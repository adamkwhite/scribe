mod audio;
mod config;
mod notes;
mod transcribe;
#[cfg(target_os = "windows")]
mod tray;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Parser)]
#[command(name = "scribe", about = "Meeting transcription & notes")]
struct Args {
    /// Run in CLI mode instead of system tray
    #[arg(long)]
    cli: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = config::load_or_create().await?;

    if args.cli {
        run_cli(cfg).await
    } else {
        #[cfg(target_os = "windows")]
        {
            tray::run(cfg).await
        }
        #[cfg(not(target_os = "windows"))]
        {
            println!("System tray only available on Windows. Use --cli mode.");
            run_cli(cfg).await
        }
    }
}

/// Process the most recent session: transcribe + generate notes.
pub async fn process_recording(cfg: &config::Config) -> Result<()> {
    let output_dir = config::output_dir()?;
    let session_dir = audio::latest_session(&output_dir)?;
    let wav_path = session_dir.join("recording.wav");
    println!("Found: {}", session_dir.display());

    println!("Transcribing with whisper.cpp...");
    let transcript = transcribe::run_whisper(&wav_path, cfg).await?;
    println!("Transcription complete ({} chars).", transcript.len());

    let txt_path = session_dir.join("transcript.txt");
    std::fs::write(&txt_path, &transcript)?;
    println!("Transcript saved to: {}", txt_path.display());

    println!("Generating meeting notes...");
    let notes_text = notes::generate(&transcript, cfg).await?;

    let full_notes = format!("{notes_text}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
    let notes_path = session_dir.join("notes.md");
    std::fs::write(&notes_path, &full_notes)?;
    println!("Notes saved to: {}", notes_path.display());

    Ok(())
}

/// Prompt for a session name via Windows input dialog.
#[cfg(target_os = "windows")]
pub fn prompt_session_name_gui() -> Option<String> {
    let script = r#"
Add-Type -AssemblyName Microsoft.VisualBasic
$name = [Microsoft.VisualBasic.Interaction]::InputBox("Enter a name for this recording (or leave blank):", "Scribe — New Recording", "")
Write-Output $name
"#;
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .output()
        .ok()?;

    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

async fn run_cli(cfg: config::Config) -> Result<()> {
    let recording = Arc::new(AtomicBool::new(false));
    let current_session: Arc<std::sync::Mutex<Option<PathBuf>>> =
        Arc::new(std::sync::Mutex::new(None));

    println!("scribe — meeting transcription & notes");
    println!("Commands: [r]ecord, [s]top, [t]ranscribe last, [q]uit\n");

    let mut reader = tokio::io::BufReader::new(tokio::io::stdin());
    loop {
        use tokio::io::AsyncBufReadExt;
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        match line.trim() {
            cmd if cmd.starts_with("r") => {
                if recording.load(Ordering::Relaxed) {
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

                let session_dir = audio::create_session_dir(name.as_deref())?;
                println!("Session: {}", session_dir.display());

                *current_session.lock().unwrap() = Some(session_dir.clone());

                recording.store(true, Ordering::Relaxed);
                let rec = recording.clone();
                let sample_rate = cfg.sample_rate;
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = audio::record_loopback(rec, sample_rate, session_dir) {
                        eprintln!("Recording error: {e}");
                    }
                });
                println!("Recording started. Press 's' to stop.");
            }
            "s" | "stop" => {
                if !recording.load(Ordering::Relaxed) {
                    println!("Not recording.");
                    continue;
                }
                recording.store(false, Ordering::Relaxed);
                println!("Recording stopped. Processing...");
                process_recording(&cfg).await?;
            }
            "t" | "transcribe" => {
                process_recording(&cfg).await?;
            }
            "q" | "quit" => {
                recording.store(false, Ordering::Relaxed);
                println!("Bye.");
                break;
            }
            "" => {}
            other => println!("Unknown command: {other}"),
        }
    }

    Ok(())
}
