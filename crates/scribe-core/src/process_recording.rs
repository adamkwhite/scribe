use crate::{audio, config};

/// Process the most recent session: transcribe + generate notes.
pub async fn process_recording(cfg: &config::Config) -> anyhow::Result<()> {
    let session_store = audio::audio_session_store_from_config(cfg)?;
    let session_dir = session_store.latest_recording_session()?.session_dir;
    tracing::info!(session_dir = %session_dir.display(), "processing latest recording");
    crate::process_session::process_session(cfg, &session_dir).await
}
