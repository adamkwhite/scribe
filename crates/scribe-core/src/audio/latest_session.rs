use anyhow::{Context, Result};
use std::path::PathBuf;

/// Find the most recent session directory containing a recording.wav.
pub fn latest_session(base_dir: &PathBuf) -> Result<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(base_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && e.path().join("recording.wav").exists())
        .collect();

    entries.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    entries
        .last()
        .map(|e| e.path())
        .context("No recordings found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn latest_session_returns_most_recent_with_recording() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_path_buf();

        let older = base.join("session-1");
        fs::create_dir_all(&older).unwrap();
        fs::write(older.join("recording.wav"), b"fake").unwrap();

        sleep(Duration::from_millis(20));

        let newer = base.join("session-2");
        fs::create_dir_all(&newer).unwrap();
        fs::write(newer.join("recording.wav"), b"fake").unwrap();

        let result = latest_session(&base).unwrap();
        assert_eq!(result, newer);
    }

    #[test]
    fn latest_session_skips_dirs_without_recording() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_path_buf();

        let with_recording = base.join("good");
        fs::create_dir_all(&with_recording).unwrap();
        fs::write(with_recording.join("recording.wav"), b"fake").unwrap();

        sleep(Duration::from_millis(20));

        // Newer directory but no recording.wav - should be skipped
        let without_recording = base.join("empty");
        fs::create_dir_all(&without_recording).unwrap();

        let result = latest_session(&base).unwrap();
        assert_eq!(result, with_recording);
    }

    #[test]
    fn latest_session_errors_when_no_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_path_buf();
        let result = latest_session(&base);
        assert!(result.is_err());
    }
}
