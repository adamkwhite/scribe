use crate::{config, notes, runtime};
use std::path::Path;

/// Process a specific session: transcribe + generate notes.
pub async fn process_session(cfg: &config::Config, session_dir: &Path) -> anyhow::Result<()> {
    let runtime = runtime::ScribeRuntime::from_config(cfg)?;
    runtime
        .process_session(runtime::ProcessSessionInput {
            session_dir: session_dir.to_path_buf(),
            context: runtime.note_generation_context_now(notes::NotesSystemPrompt::Default),
            events: runtime::SessionProcessingEventSink::printing(),
        })
        .await
        .map(|_| ())
}
