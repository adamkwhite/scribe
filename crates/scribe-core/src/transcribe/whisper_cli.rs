use anyhow::{Context, Result};
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};

use crate::config::Config;

use super::provider::{
    TranscriptionFuture, TranscriptionInput, TranscriptionOutput, TranscriptionProvider,
};

pub struct WhisperCliTranscriptionProvider {
    whisper_bin: String,
    whisper_model: String,
    runner: Arc<dyn WhisperCliCommandRunner>,
}

impl WhisperCliTranscriptionProvider {
    pub fn from_config(cfg: &Config) -> Result<Self> {
        let whisper_bin = cfg
            .whisper_bin
            .as_deref()
            .map(str::trim)
            .filter(|bin| !bin.is_empty())
            .context("whisper_bin is required to use the whisper-cli transcription backend")?;

        Ok(Self {
            whisper_bin: whisper_bin.to_string(),
            whisper_model: cfg.whisper_model.clone(),
            runner: Arc::new(TokioWhisperCliCommandRunner),
        })
    }

    #[cfg(test)]
    fn new(
        whisper_bin: String,
        whisper_model: String,
        runner: Arc<dyn WhisperCliCommandRunner>,
    ) -> Self {
        Self {
            whisper_bin,
            whisper_model,
            runner,
        }
    }
}

impl TranscriptionProvider for WhisperCliTranscriptionProvider {
    fn transcribe(&self, input: TranscriptionInput) -> TranscriptionFuture<'_> {
        Box::pin(async move {
            tracing::info!(
                whisper_bin = %self.whisper_bin,
                whisper_model = %self.whisper_model,
                wav_path = %input.wav_path.display(),
                "running external whisper"
            );

            let request = WhisperCliCommandRequest {
                whisper_bin: self.whisper_bin.clone(),
                args: vec![
                    "--model".to_string(),
                    self.whisper_model.clone(),
                    "--output-txt".to_string(),
                    "--no-timestamps".to_string(),
                    input.wav_path.to_string_lossy().into_owned(),
                ],
                wav_path: input.wav_path.clone(),
            };

            let response = self.runner.run(request).await.with_context(|| {
                format!(
                    "Failed to run '{}'. Is whisper.cpp installed?",
                    self.whisper_bin
                )
            })?;

            if !response.success {
                let stderr = String::from_utf8_lossy(&response.stderr);
                tracing::error!(
                    status = %response.status,
                    stderr = %stderr,
                    "external whisper failed"
                );
                anyhow::bail!("whisper.cpp failed: {stderr}");
            }

            let transcript = read_whisper_transcript(&input.wav_path, response.stdout)?;
            tracing::info!(
                transcript_chars = transcript.len(),
                "external whisper transcription completed"
            );
            Ok(TranscriptionOutput { transcript })
        })
    }
}

type WhisperCliCommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<WhisperCliCommandResponse>> + Send + 'a>>;

trait WhisperCliCommandRunner: Send + Sync {
    fn run(&self, request: WhisperCliCommandRequest) -> WhisperCliCommandFuture<'_>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WhisperCliCommandRequest {
    whisper_bin: String,
    args: Vec<String>,
    wav_path: PathBuf,
}

struct WhisperCliCommandResponse {
    success: bool,
    status: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct TokioWhisperCliCommandRunner;

impl WhisperCliCommandRunner for TokioWhisperCliCommandRunner {
    fn run(&self, request: WhisperCliCommandRequest) -> WhisperCliCommandFuture<'_> {
        Box::pin(async move {
            let output = tokio::process::Command::new(&request.whisper_bin)
                .args(&request.args)
                .output()
                .await?;

            Ok(WhisperCliCommandResponse {
                success: output.status.success(),
                status: output.status.to_string(),
                stdout: output.stdout,
                stderr: output.stderr,
            })
        })
    }
}

fn read_whisper_transcript(wav_path: &std::path::Path, stdout: Vec<u8>) -> Result<String> {
    let txt_path = wav_path.with_extension("wav.txt");
    if txt_path.exists() {
        let transcript =
            std::fs::read_to_string(&txt_path).context("Failed to read whisper output")?;
        let _ = std::fs::remove_file(&txt_path);
        tracing::info!("external whisper transcript read from output file");
        return Ok(transcript.trim().to_string());
    }

    tracing::info!("external whisper transcript read from stdout");
    Ok(String::from_utf8_lossy(&stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::{
        collections::VecDeque,
        sync::{Mutex, MutexGuard},
    };

    #[test]
    fn from_config_rejects_missing_whisper_bin() {
        let cfg = config_with_bin(None);

        let error = match WhisperCliTranscriptionProvider::from_config(&cfg) {
            Ok(_) => panic!("expected missing whisper_bin to fail"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "whisper_bin is required to use the whisper-cli transcription backend"
        );
    }

    #[test]
    fn from_config_rejects_blank_whisper_bin() {
        let cfg = config_with_bin(Some("   ".to_string()));

        let error = match WhisperCliTranscriptionProvider::from_config(&cfg) {
            Ok(_) => panic!("expected blank whisper_bin to fail"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "whisper_bin is required to use the whisper-cli transcription backend"
        );
    }

    #[tokio::test]
    async fn sends_expected_command_request_and_returns_transcript_from_output_file() {
        let temp = tempfile::tempdir().unwrap();
        let wav_path = temp.path().join("recording.wav");
        std::fs::write(&wav_path, b"wav").unwrap();
        std::fs::write(wav_path.with_extension("wav.txt"), "  File transcript\n").unwrap();
        let runner = FakeRunner::with_response(WhisperCliCommandResponse {
            success: true,
            status: "exit status: 0".to_string(),
            stdout: b"stdout transcript".to_vec(),
            stderr: Vec::new(),
        });
        let provider = WhisperCliTranscriptionProvider::new(
            "whisper-cli".to_string(),
            "model.bin".to_string(),
            Arc::new(runner.clone()),
        );

        let output = provider
            .transcribe(TranscriptionInput {
                wav_path: wav_path.clone(),
            })
            .await
            .unwrap();

        assert_eq!(
            output,
            TranscriptionOutput {
                transcript: "File transcript".to_string()
            }
        );
        assert!(!wav_path.with_extension("wav.txt").exists());
        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0],
            WhisperCliCommandRequest {
                whisper_bin: "whisper-cli".to_string(),
                args: vec![
                    "--model".to_string(),
                    "model.bin".to_string(),
                    "--output-txt".to_string(),
                    "--no-timestamps".to_string(),
                    wav_path.to_string_lossy().into_owned(),
                ],
                wav_path,
            }
        );
    }

    #[tokio::test]
    async fn returns_trimmed_stdout_when_output_file_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let wav_path = temp.path().join("recording.wav");
        std::fs::write(&wav_path, b"wav").unwrap();
        let runner = FakeRunner::with_response(WhisperCliCommandResponse {
            success: true,
            status: "exit status: 0".to_string(),
            stdout: b"  Stdout transcript\n".to_vec(),
            stderr: Vec::new(),
        });
        let provider = WhisperCliTranscriptionProvider::new(
            "whisper-cli".to_string(),
            "model.bin".to_string(),
            Arc::new(runner),
        );

        let output = provider
            .transcribe(TranscriptionInput { wav_path })
            .await
            .unwrap();

        assert_eq!(
            output,
            TranscriptionOutput {
                transcript: "Stdout transcript".to_string()
            }
        );
    }

    #[tokio::test]
    async fn non_zero_exit_maps_to_whisper_failure() {
        let temp = tempfile::tempdir().unwrap();
        let wav_path = temp.path().join("recording.wav");
        let runner = FakeRunner::with_response(WhisperCliCommandResponse {
            success: false,
            status: "exit status: 1".to_string(),
            stdout: Vec::new(),
            stderr: b"bad model".to_vec(),
        });
        let provider = WhisperCliTranscriptionProvider::new(
            "whisper-cli".to_string(),
            "model.bin".to_string(),
            Arc::new(runner),
        );

        let error = provider
            .transcribe(TranscriptionInput { wav_path })
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "whisper.cpp failed: bad model");
    }

    #[tokio::test]
    async fn runner_failure_is_wrapped_with_binary_context() {
        let temp = tempfile::tempdir().unwrap();
        let wav_path = temp.path().join("recording.wav");
        let runner = FakeRunner::with_result(Err("spawn failed".to_string()));
        let provider = WhisperCliTranscriptionProvider::new(
            "whisper-cli".to_string(),
            "model.bin".to_string(),
            Arc::new(runner),
        );

        let error = provider
            .transcribe(TranscriptionInput { wav_path })
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Failed to run 'whisper-cli'. Is whisper.cpp installed?"
        );
    }

    #[derive(Clone)]
    struct FakeRunner {
        requests: Arc<Mutex<Vec<WhisperCliCommandRequest>>>,
        responses: Arc<Mutex<VecDeque<Result<WhisperCliCommandResponse, String>>>>,
    }

    impl FakeRunner {
        fn with_response(response: WhisperCliCommandResponse) -> Self {
            Self::with_result(Ok(response))
        }

        fn with_result(response: Result<WhisperCliCommandResponse, String>) -> Self {
            let mut responses = VecDeque::new();
            responses.push_back(response);
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn requests(&self) -> MutexGuard<'_, Vec<WhisperCliCommandRequest>> {
            self.requests.lock().unwrap()
        }
    }

    impl WhisperCliCommandRunner for FakeRunner {
        fn run(&self, request: WhisperCliCommandRequest) -> WhisperCliCommandFuture<'_> {
            self.requests.lock().unwrap().push(request);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake response");
            Box::pin(async move { response.map_err(|message| anyhow!(message)) })
        }
    }

    fn config_with_bin(whisper_bin: Option<String>) -> Config {
        Config {
            whisper_bin,
            whisper_model: "model.bin".to_string(),
            openrouter_api_key: "key".to_string(),
            model: "notes/model".to_string(),
            sample_rate: 16000,
            output_dir: None,
        }
    }
}
