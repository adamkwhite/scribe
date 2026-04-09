mod audio;
mod config;
mod notes;
mod transcribe;

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = config::load_or_create()?;
    let recording = Arc::new(AtomicBool::new(false));

    println!("scribe — meeting transcription & notes");
    println!("Commands: [r]ecord, [s]top, [t]ranscribe last, [q]uit\n");

    // Simple CLI loop for v1 — tray icon comes later
    let mut reader = tokio::io::BufReader::new(tokio::io::stdin());
    loop {
        use tokio::io::AsyncBufReadExt;
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        match line.trim() {
            "r" | "record" => {
                if recording.load(Ordering::Relaxed) {
                    println!("Already recording.");
                    continue;
                }
                recording.store(true, Ordering::Relaxed);
                let rec = recording.clone();
                let sample_rate = cfg.sample_rate;
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = audio::record_loopback(rec, sample_rate) {
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

                // Find the most recent WAV file
                let output_dir = config::output_dir()?;
                let wav_path = audio::latest_recording(&output_dir)?;

                // Transcribe
                println!("Transcribing with whisper.cpp...");
                let transcript = transcribe::run_whisper(&wav_path, &cfg).await?;
                println!("Transcription complete ({} chars).", transcript.len());

                // Save transcript
                let txt_path = wav_path.with_extension("txt");
                std::fs::write(&txt_path, &transcript)?;
                println!("Transcript saved to: {}", txt_path.display());

                // Generate notes
                println!("Generating meeting notes...");
                let notes = notes::generate(&transcript, &cfg).await?;

                // Save notes with transcript appended
                let full_notes = format!("{notes}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
                let notes_path = wav_path.with_extension("md");
                std::fs::write(&notes_path, &full_notes)?;
                println!("Notes saved to: {}", notes_path.display());
            }
            "t" | "transcribe" => {
                let output_dir = config::output_dir()?;
                let wav_path = audio::latest_recording(&output_dir)?;
                println!("Found: {}", wav_path.display());

                println!("Transcribing with whisper.cpp...");
                let transcript = transcribe::run_whisper(&wav_path, &cfg).await?;
                println!("Transcription complete ({} chars).", transcript.len());

                // Save transcript
                let txt_path = wav_path.with_extension("txt");
                std::fs::write(&txt_path, &transcript)?;
                println!("Transcript saved to: {}", txt_path.display());

                // Generate notes
                println!("Generating meeting notes...");
                let notes = notes::generate(&transcript, &cfg).await?;

                let full_notes = format!("{notes}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
                let notes_path = wav_path.with_extension("md");
                std::fs::write(&notes_path, &full_notes)?;
                println!("Notes saved to: {}", notes_path.display());
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
