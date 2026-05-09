pub mod audio;
pub mod config;
pub mod notes;
pub mod opener;
pub mod transcribe;
#[cfg(target_os = "windows")]
pub mod tray;

use anyhow::{Context, Result};
use std::path::Path;

/// Process the most recent session: transcribe + generate notes.
pub async fn process_recording(cfg: &config::Config) -> Result<()> {
    let output_dir = config::effective_output_dir(cfg)?;
    let session_dir = audio::latest_session(&output_dir)?;
    process_session(cfg, &session_dir).await
}

/// Process a specific session: transcribe + generate notes.
pub async fn process_session(cfg: &config::Config, session_dir: &Path) -> Result<()> {
    let wav_path = session_dir.join("recording.wav");
    println!("Found: {}", session_dir.display());

    println!("Transcribing with whisper.cpp...");
    let transcript = transcribe::run_whisper(&wav_path, cfg).await?;
    println!("Transcription complete ({} chars).", transcript.len());

    let txt_path = session_dir.join("transcript.txt");
    std::fs::write(&txt_path, &transcript)
        .with_context(|| format!("Failed to write {}", txt_path.display()))?;
    println!("Transcript saved to: {}", txt_path.display());

    println!("Generating meeting notes...");
    let notes_text = notes::generate(&transcript, cfg).await?;

    let full_notes = format!("{notes_text}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
    let notes_path = session_dir.join("notes.md");
    std::fs::write(&notes_path, &full_notes)
        .with_context(|| format!("Failed to write {}", notes_path.display()))?;
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
