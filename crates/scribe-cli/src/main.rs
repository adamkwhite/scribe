use anyhow::{Context, Result};
use scribe_core::{audio, config, process_recording};

#[tokio::main]
async fn main() -> Result<()> {
    let log_path = scribe_core::logging::init_file_logging("scribe-cli")?;
    tracing::info!(log_path = %log_path.display(), "scribe CLI starting");
    let cfg = config::load_or_create().await?;
    run_cli(cfg).await
}

async fn run_cli(cfg: config::Config) -> Result<()> {
    let mut recording_control: Option<audio::RecordingControl> = None;
    let mut recording_task: Option<tokio::task::JoinHandle<Result<()>>> = None;

    println!("scribe — meeting transcription & notes");
    println!("Commands: [r]ecord, [s]top, [t]ranscribe last, [q]uit\n");

    let mut reader = tokio::io::BufReader::new(tokio::io::stdin());
    loop {
        use tokio::io::AsyncBufReadExt;
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        match line.trim() {
            cmd if cmd.starts_with("r") => {
                if recording_control
                    .as_ref()
                    .is_some_and(|control| control.is_recording())
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

                let session_store = audio::audio_session_store_from_config(&cfg)?;
                let session_dir = session_store
                    .create_session(audio::CreateAudioSessionInput {
                        name,
                        context: audio::CreateAudioSessionContext {
                            timestamp: audio::AudioSessionTimestamp::now_local(),
                        },
                    })?
                    .session_dir;
                tracing::info!(session_dir = %session_dir.display(), "CLI recording session created");
                println!("Session: {}", session_dir.display());

                let recorder = audio::audio_recorder_from_config(&cfg)?;
                let control = audio::RecordingControl::new_running();
                let input = audio::AudioRecordingInput {
                    control: control.clone(),
                    session_dir,
                    events: audio::AudioRecordingEventSink::printing(),
                };
                recording_control = Some(control);
                recording_task = Some(tokio::spawn(async move {
                    recorder.record(input).await.map(|_| ())
                }));
                tracing::info!("CLI recording started");
                println!("Recording started. Press 's' to stop.");
            }
            "s" | "stop" => {
                if !recording_control
                    .as_ref()
                    .is_some_and(|control| control.is_recording())
                {
                    tracing::info!("stop command ignored because no recording is active");
                    println!("Not recording.");
                    continue;
                }
                if let Some(control) = &recording_control {
                    control.stop();
                }
                tracing::info!("CLI recording stop requested");
                println!("Recording stopped. Finalizing...");
                wait_for_recording_task(&mut recording_task).await?;
                recording_control = None;
                println!("Processing...");
                process_recording(&cfg).await?;
            }
            "t" | "transcribe" => {
                tracing::info!("CLI process latest session requested");
                process_recording(&cfg).await?;
            }
            "q" | "quit" => {
                if let Some(control) = &recording_control {
                    control.stop();
                }
                wait_for_recording_task(&mut recording_task).await?;
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

async fn wait_for_recording_task(
    recording_task: &mut Option<tokio::task::JoinHandle<Result<()>>>,
) -> Result<()> {
    if let Some(task) = recording_task.take() {
        task.await.context("Recording task failed to join")??;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn wait_for_recording_task_awaits_finalization_before_returning() {
        let finalized = Arc::new(AtomicBool::new(false));
        let finalized_in_task = finalized.clone();
        let mut recording_task = Some(tokio::task::spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(25));
            finalized_in_task.store(true, Ordering::SeqCst);
            Ok(())
        }));

        wait_for_recording_task(&mut recording_task).await.unwrap();

        assert!(recording_task.is_none());
        assert!(finalized.load(Ordering::SeqCst));
    }
}
