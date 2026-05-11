use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub trait SessionArtifactStore: Send + Sync {
    fn recording_wav_path(&self, session_dir: &Path) -> PathBuf;
    fn write_transcript(&self, session_dir: &Path, transcript: &str) -> Result<PathBuf>;
    fn write_notes(&self, session_dir: &Path, markdown: &str) -> Result<PathBuf>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FileSystemSessionArtifactStore;

impl SessionArtifactStore for FileSystemSessionArtifactStore {
    fn recording_wav_path(&self, session_dir: &Path) -> PathBuf {
        session_dir.join("recording.wav")
    }

    fn write_transcript(&self, session_dir: &Path, transcript: &str) -> Result<PathBuf> {
        let path = session_dir.join("transcript.txt");
        std::fs::write(&path, transcript)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(path)
    }

    fn write_notes(&self, session_dir: &Path, markdown: &str) -> Result<PathBuf> {
        let path = session_dir.join("notes.md");
        std::fs::write(&path, markdown)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(path)
    }
}
