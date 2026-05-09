pub mod audio;
pub mod config;
pub mod notes;
pub mod opener;
pub mod transcribe;

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
