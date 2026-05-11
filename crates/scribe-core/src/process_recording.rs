use crate::{config, notes, runtime};

/// Process the most recent session: transcribe + generate notes.
pub async fn process_recording(cfg: &config::Config) -> anyhow::Result<()> {
    let runtime = runtime::ScribeRuntime::from_config(cfg)?;
    runtime
        .process_latest_recording(runtime::ProcessLatestRecordingInput {
            context: runtime.note_generation_context_now(notes::NotesSystemPrompt::Default),
            events: runtime::SessionProcessingEventSink::printing(),
        })
        .await
        .map(|_| ())
}
