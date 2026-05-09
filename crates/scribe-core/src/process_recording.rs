use crate::{audio, config};

/// Process the most recent session: transcribe + generate notes.
pub async fn process_recording(cfg: &config::Config) -> anyhow::Result<()> {
    let output_dir = config::effective_output_dir(cfg)?;
    tracing::info!(output_dir = %output_dir.display(), "processing latest recording");
    let session_dir = audio::latest_session(&output_dir)?;
    crate::process_session::process_session(cfg, &session_dir).await
}
