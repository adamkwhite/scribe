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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_wav_path_appends_recording_wav_to_session_dir() {
        let store = FileSystemSessionArtifactStore;
        let path = store.recording_wav_path(Path::new("/tmp/session"));
        assert_eq!(path, PathBuf::from("/tmp/session/recording.wav"));
    }

    #[test]
    fn write_transcript_writes_file_and_returns_path() {
        let store = FileSystemSessionArtifactStore;
        let temp = tempfile::tempdir().unwrap();

        let path = store.write_transcript(temp.path(), "hello world").unwrap();

        assert_eq!(path, temp.path().join("transcript.txt"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn write_notes_writes_file_and_returns_path() {
        let store = FileSystemSessionArtifactStore;
        let temp = tempfile::tempdir().unwrap();

        let path = store.write_notes(temp.path(), "# Notes\n\nbody").unwrap();

        assert_eq!(path, temp.path().join("notes.md"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Notes\n\nbody");
    }

    #[test]
    fn write_transcript_fails_when_session_dir_does_not_exist() {
        let store = FileSystemSessionArtifactStore;
        let missing = Path::new("/does/not/exist/at/all");

        let result = store.write_transcript(missing, "x");

        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("Failed to write"), "got: {err}");
    }
}
