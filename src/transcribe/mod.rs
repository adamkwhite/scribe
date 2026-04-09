use anyhow::{Context, Result};
use std::path::Path;

use crate::config::Config;

/// Run whisper.cpp CLI on a WAV file and return the transcript text.
pub async fn run_whisper(wav_path: &Path, cfg: &Config) -> Result<String> {
    let output = tokio::process::Command::new(&cfg.whisper_bin)
        .args([
            "--model", &cfg.whisper_model,
            "--output-txt",
            "--no-timestamps",
            &wav_path.to_string_lossy(),
        ])
        .output()
        .await
        .with_context(|| format!("Failed to run '{}'. Is whisper.cpp installed?", cfg.whisper_bin))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("whisper.cpp failed: {stderr}");
    }

    // whisper.cpp with --output-txt creates a .txt file next to the input
    let txt_path = wav_path.with_extension("wav.txt");
    if txt_path.exists() {
        let transcript = std::fs::read_to_string(&txt_path)
            .context("Failed to read whisper output")?;
        // Clean up the intermediate txt file
        let _ = std::fs::remove_file(&txt_path);
        Ok(transcript.trim().to_string())
    } else {
        // Some versions write to stdout instead
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().to_string())
    }
}
