use anyhow::Result;
use scribe_core::{audio, config, notes, runtime};

#[tokio::main]
async fn main() -> Result<()> {
    let log_path = scribe_core::logging::init_file_logging("scribe-cli")?;
    tracing::info!(log_path = %log_path.display(), "scribe CLI starting");
    let (cfg, origin) = config::load_or_create()?;
    if let config::ConfigOrigin::JustCreated(path) = origin {
        println!("Created config at: {}", path.display());
        println!("Please edit it with your whisper model path and OpenRouter API key.");
        println!();
    }
    let runtime = runtime::ScribeRuntime::from_config(&cfg)?;
    run_cli(runtime).await
}

async fn run_cli(runtime: runtime::ScribeRuntime) -> Result<()> {
    let mut recording: Option<runtime::ActiveRecording> = None;

    println!("scribe — meeting transcription & notes");
    println!("Commands: [r]ecord, [s]top, [t]ranscribe last, [q]uit\n");

    let mut reader = tokio::io::BufReader::new(tokio::io::stdin());
    loop {
        use tokio::io::AsyncBufReadExt;
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        match line.trim() {
            cmd if cmd.starts_with("r") => {
                if recording
                    .as_ref()
                    .is_some_and(|recording| recording.is_recording())
                {
                    tracing::info!("record command ignored because recording is already active");
                    println!("Already recording.");
                    continue;
                }

                // Extract optional name: "r My Meeting" or just "r"
                let name = cmd
                    .strip_prefix("record")
                    .or(cmd.strip_prefix("r"))
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                if name.is_none() {
                    println!("Tip: use 'r Meeting Name' to name your recording.");
                }

                let active_recording = runtime.start_recording(runtime::StartRecordingInput {
                    name,
                    context: runtime.recording_context_now(),
                    events: audio::AudioRecordingEventSink::printing(),
                })?;
                let session_dir = active_recording.session_dir().to_path_buf();
                tracing::info!(session_dir = %session_dir.display(), "CLI recording session created");
                println!("Session: {}", session_dir.display());
                recording = Some(active_recording);
                tracing::info!("CLI recording started");
                println!("Recording started. Press 's' to stop.");
            }
            "s" | "stop" => {
                if !recording
                    .as_ref()
                    .is_some_and(|recording| recording.is_recording())
                {
                    tracing::info!("stop command ignored because no recording is active");
                    println!("Not recording.");
                    continue;
                }
                let active_recording = recording.take().expect("recording presence checked above");
                let session_dir = active_recording.session_dir().to_path_buf();
                active_recording.stop();
                tracing::info!("CLI recording stop requested");
                println!("Recording stopped. Finalizing...");
                active_recording.wait().await?;
                println!("Processing...");
                process_session(&runtime, session_dir).await?;
            }
            "t" | "transcribe" => {
                tracing::info!("CLI process latest session requested");
                process_latest_recording(&runtime).await?;
            }
            "q" | "quit" => {
                if let Some(active_recording) = recording.take() {
                    active_recording.stop();
                    active_recording.wait().await?;
                }
                tracing::info!("scribe CLI exiting");
                println!("Bye.");
                break;
            }
            "" => {}
            other => println!("Unknown command: {other}"),
        }
    }

    Ok(())
}

async fn process_session(
    runtime: &runtime::ScribeRuntime,
    session_dir: std::path::PathBuf,
) -> Result<()> {
    runtime
        .process_session(runtime::ProcessSessionInput {
            session_dir,
            context: runtime.note_generation_context_now(notes::NotesSystemPrompt::Default),
            events: runtime::SessionProcessingEventSink::printing(),
        })
        .await?;

    Ok(())
}

async fn process_latest_recording(runtime: &runtime::ScribeRuntime) -> Result<()> {
    runtime
        .process_latest_recording(runtime::ProcessLatestRecordingInput {
            context: runtime.note_generation_context_now(notes::NotesSystemPrompt::Default),
            events: runtime::SessionProcessingEventSink::printing(),
        })
        .await?;

    Ok(())
}
