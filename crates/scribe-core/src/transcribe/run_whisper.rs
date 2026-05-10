#[cfg(any(
    feature = "whisper-cli",
    all(not(feature = "whisper-cli"), feature = "__embedded-whisper")
))]
use anyhow::Context;
use anyhow::Result;
use std::path::Path;

use crate::config::Config;

#[cfg(all(not(feature = "whisper-cli"), not(feature = "__embedded-whisper")))]
compile_error!(
    "scribe-core requires either the default embedded Whisper backend or the `whisper-cli` feature"
);

#[cfg(all(not(feature = "whisper-cli"), not(feature = "__embedded-whisper")))]
pub async fn run_whisper(_wav_path: &Path, _cfg: &Config) -> Result<String> {
    unreachable!("invalid backend feature configuration")
}

/// Run the configured Whisper backend on a WAV file and return the transcript text.
#[cfg(feature = "whisper-cli")]
pub async fn run_whisper(wav_path: &Path, cfg: &Config) -> Result<String> {
    let whisper_bin = external_whisper_bin(cfg)?;
    tracing::info!(
        whisper_bin,
        whisper_model = %cfg.whisper_model,
        wav_path = %wav_path.display(),
        "running external whisper"
    );
    let output = tokio::process::Command::new(whisper_bin)
        .args([
            "--model",
            &cfg.whisper_model,
            "--output-txt",
            "--no-timestamps",
            &wav_path.to_string_lossy(),
        ])
        .output()
        .await
        .with_context(|| format!("Failed to run '{}'. Is whisper.cpp installed?", whisper_bin))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(
            status = ?output.status,
            stderr = %stderr,
            "external whisper failed"
        );
        anyhow::bail!("whisper.cpp failed: {stderr}");
    }

    // whisper.cpp with --output-txt creates a .txt file next to the input
    let txt_path = wav_path.with_extension("wav.txt");
    if txt_path.exists() {
        let transcript =
            std::fs::read_to_string(&txt_path).context("Failed to read whisper output")?;
        // Clean up the intermediate txt file
        let _ = std::fs::remove_file(&txt_path);
        tracing::info!(
            transcript_chars = transcript.trim().len(),
            "external whisper transcript read from output file"
        );
        Ok(transcript.trim().to_string())
    } else {
        // Some versions write to stdout instead
        let stdout = String::from_utf8_lossy(&output.stdout);
        tracing::info!(
            transcript_chars = stdout.trim().len(),
            "external whisper transcript read from stdout"
        );
        Ok(stdout.trim().to_string())
    }
}

/// Run the embedded whisper.cpp backend on a WAV file and return the transcript text.
#[cfg(all(not(feature = "whisper-cli"), feature = "__embedded-whisper"))]
pub async fn run_whisper(wav_path: &Path, cfg: &Config) -> Result<String> {
    let wav_path = wav_path.to_path_buf();
    let model_path = cfg.whisper_model.clone();
    tracing::info!(
        wav_path = %wav_path.display(),
        whisper_model = %model_path,
        "running embedded whisper"
    );

    tokio::task::spawn_blocking(move || run_embedded_whisper(&wav_path, &model_path))
        .await
        .context("Embedded whisper task failed")?
}

#[cfg(any(test, feature = "whisper-cli"))]
fn external_whisper_bin(cfg: &Config) -> Result<&str> {
    cfg.whisper_bin
        .as_deref()
        .context("whisper_bin is required when the whisper-cli backend is enabled")
}

#[cfg(all(not(feature = "whisper-cli"), feature = "__embedded-whisper"))]
fn run_embedded_whisper(wav_path: &Path, model_path: &str) -> Result<String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    use super::wav_loader::load_wav_as_mono_16khz_f32;

    whisper_rs::install_logging_hooks();

    let audio = load_wav_as_mono_16khz_f32(wav_path)?;
    let ctx =
        WhisperContext::new_with_params(Path::new(model_path), WhisperContextParameters::default())
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
        .context("Embedded whisper transcription failed")?;

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

    #[test]
    fn external_whisper_bin_errors_when_missing() {
        let cfg = Config {
            whisper_bin: None,
            whisper_model: "model.bin".into(),
            openrouter_api_key: "key".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: None,
        };

        let error = external_whisper_bin(&cfg).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("whisper_bin is required when the whisper-cli backend is enabled")
        );
    }
}
