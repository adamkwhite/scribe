use anyhow::{Context, Result};
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};

use crate::config::Config;

use super::provider::{
    TranscriptionFuture, TranscriptionInput, TranscriptionOutput, TranscriptionProvider,
};

pub struct EmbeddedWhisperTranscriptionProvider {
    model_path: String,
    engine: Arc<dyn EmbeddedWhisperEngine>,
}

impl EmbeddedWhisperTranscriptionProvider {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            model_path: cfg.whisper_model.clone(),
            engine: Arc::new(WhisperRsEmbeddedWhisperEngine),
        }
    }

    #[cfg(test)]
    fn new(model_path: String, engine: Arc<dyn EmbeddedWhisperEngine>) -> Self {
        Self { model_path, engine }
    }
}

impl TranscriptionProvider for EmbeddedWhisperTranscriptionProvider {
    fn transcribe(&self, input: TranscriptionInput) -> TranscriptionFuture<'_> {
        Box::pin(async move {
            tracing::info!(
                wav_path = %input.wav_path.display(),
                whisper_model = %self.model_path,
                "running embedded whisper"
            );

            let transcript = self
                .engine
                .transcribe(self.model_path.clone(), input.wav_path)
                .await
                .context("Embedded whisper transcription failed")?
                .trim()
                .to_string();

            tracing::info!(
                transcript_chars = transcript.len(),
                "embedded whisper transcription completed"
            );
            Ok(TranscriptionOutput { transcript })
        })
    }
}

type EmbeddedWhisperEngineFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

trait EmbeddedWhisperEngine: Send + Sync {
    fn transcribe(&self, model_path: String, wav_path: PathBuf) -> EmbeddedWhisperEngineFuture<'_>;
}

struct WhisperRsEmbeddedWhisperEngine;

impl EmbeddedWhisperEngine for WhisperRsEmbeddedWhisperEngine {
    fn transcribe(&self, model_path: String, wav_path: PathBuf) -> EmbeddedWhisperEngineFuture<'_> {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || run_embedded_whisper(&wav_path, &model_path))
                .await
                .context("Embedded whisper task failed")?
        })
    }
}

fn run_embedded_whisper(wav_path: &std::path::Path, model_path: &str) -> Result<String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    use super::wav_loader::load_wav_as_mono_16khz_f32;

    whisper_rs::install_logging_hooks();

    let audio = load_wav_as_mono_16khz_f32(wav_path)?;
    let ctx = WhisperContext::new_with_params(
        std::path::Path::new(model_path),
        WhisperContextParameters::default(),
    )
    .with_context(|| format!("Failed to load whisper model '{}'", model_path))?;
    let mut state = ctx
        .create_state()
        .context("Failed to create whisper state")?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(
        std::thread::available_parallelism()
            .map(|threads| threads.get())
            .unwrap_or(1) as i32,
    );
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, &audio)
        .context("Embedded whisper model inference failed")?;

    let transcript = state
        .as_iter()
        .map(|segment| segment.to_string())
        .collect::<Vec<_>>()
        .join("");
    Ok(transcript.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::{
        collections::VecDeque,
        sync::{Mutex, MutexGuard},
    };

    #[tokio::test]
    async fn passes_model_and_wav_path_to_engine_and_returns_transcript() {
        let engine = FakeEngine::with_response("  Embedded transcript\n".to_string());
        let provider = EmbeddedWhisperTranscriptionProvider::new(
            "model.bin".to_string(),
            Arc::new(engine.clone()),
        );
        let wav_path = PathBuf::from("/tmp/recording.wav");

        let output = provider
            .transcribe(TranscriptionInput {
                wav_path: wav_path.clone(),
            })
            .await
            .unwrap();

        assert_eq!(
            output,
            TranscriptionOutput {
                transcript: "Embedded transcript".to_string()
            }
        );
        let requests = engine.requests();
        assert_eq!(requests.as_slice(), &[("model.bin".to_string(), wav_path)]);
    }

    #[tokio::test]
    async fn engine_error_is_wrapped_with_embedded_context() {
        let engine = FakeEngine::with_result(Err("decode failed".to_string()));
        let provider =
            EmbeddedWhisperTranscriptionProvider::new("model.bin".to_string(), Arc::new(engine));

        let error = provider
            .transcribe(TranscriptionInput {
                wav_path: PathBuf::from("/tmp/recording.wav"),
            })
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "Embedded whisper transcription failed");
    }

    #[derive(Clone)]
    struct FakeEngine {
        requests: Arc<Mutex<Vec<(String, PathBuf)>>>,
        responses: Arc<Mutex<VecDeque<Result<String, String>>>>,
    }

    impl FakeEngine {
        fn with_response(response: String) -> Self {
            Self::with_result(Ok(response))
        }

        fn with_result(response: Result<String, String>) -> Self {
            let mut responses = VecDeque::new();
            responses.push_back(response);
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn requests(&self) -> MutexGuard<'_, Vec<(String, PathBuf)>> {
            self.requests.lock().unwrap()
        }
    }

    impl EmbeddedWhisperEngine for FakeEngine {
        fn transcribe(
            &self,
            model_path: String,
            wav_path: PathBuf,
        ) -> EmbeddedWhisperEngineFuture<'_> {
            self.requests.lock().unwrap().push((model_path, wav_path));
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake response");
            Box::pin(async move { response.map_err(|message| anyhow!(message)) })
        }
    }
}
