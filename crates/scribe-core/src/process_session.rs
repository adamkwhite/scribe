use crate::{config, notes, transcribe};
use anyhow::Context;
use notes::NotesGenerator;
use std::path::Path;

/// Process a specific session: transcribe + generate notes.
pub async fn process_session(cfg: &config::Config, session_dir: &Path) -> anyhow::Result<()> {
    let wav_path = session_dir.join("recording.wav");
    tracing::info!(session_dir = %session_dir.display(), "processing session");
    println!("Found: {}", session_dir.display());

    println!("Transcribing with whisper.cpp...");
    tracing::info!(wav_path = %wav_path.display(), "transcription starting");
    let transcript = match transcribe::run_whisper(&wav_path, cfg).await {
        Ok(transcript) => transcript,
        Err(error) => {
            tracing::error!(
                error = %error,
                session_dir = %session_dir.display(),
                wav_path = %wav_path.display(),
                "transcription failed"
            );
            return Err(error);
        }
    };
    tracing::info!(
        session_dir = %session_dir.display(),
        transcript_chars = transcript.len(),
        "transcription completed"
    );
    println!("Transcription complete ({} chars).", transcript.len());

    let txt_path = session_dir.join("transcript.txt");
    if let Err(error) = std::fs::write(&txt_path, &transcript)
        .with_context(|| format!("Failed to write {}", txt_path.display()))
    {
        tracing::error!(
            error = %error,
            transcript_path = %txt_path.display(),
            "transcript write failed"
        );
        return Err(error);
    }
    tracing::info!(transcript_path = %txt_path.display(), "transcript saved");
    println!("Transcript saved to: {}", txt_path.display());

    println!("Generating meeting notes...");
    tracing::info!(session_dir = %session_dir.display(), "notes generation starting");
    let notes_generator = notes::OpenRouterNotesGenerator::from_config(cfg);
    let notes_input = notes::NoteGenerationInput {
        transcript: transcript.clone(),
        context: notes::NoteGenerationContext {
            note_date: chrono::Local::now().format("%B %-d, %Y").to_string(),
            system_prompt: notes::NotesSystemPrompt::Default,
        },
    };
    let notes_text = match notes_generator.generate(notes_input).await {
        Ok(notes_output) => notes_output.markdown,
        Err(error) => {
            tracing::error!(
                error = %error,
                session_dir = %session_dir.display(),
                "notes generation failed"
            );
            return Err(error);
        }
    };
    tracing::info!(session_dir = %session_dir.display(), "notes generation completed");

    let full_notes = format!("{notes_text}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
    let notes_path = session_dir.join("notes.md");
    if let Err(error) = std::fs::write(&notes_path, &full_notes)
        .with_context(|| format!("Failed to write {}", notes_path.display()))
    {
        tracing::error!(
            error = %error,
            notes_path = %notes_path.display(),
            "notes write failed"
        );
        return Err(error);
    }
    tracing::info!(notes_path = %notes_path.display(), "notes saved");
    println!("Notes saved to: {}", notes_path.display());

    Ok(())
}
