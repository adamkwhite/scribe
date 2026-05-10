use anyhow::{Context, Result};
use std::path::Path;
use std::time::SystemTime;

use super::session_entry::SessionEntry;
use super::session_status::SessionStatus;

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn list_sessions(base_dir: &Path) -> Result<Vec<SessionEntry>> {
    if !base_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(base_dir)
        .with_context(|| format!("Failed to read {}", base_dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }

            let metadata = entry.metadata().ok()?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let name = path.file_name()?.to_string_lossy().into_owned();
            let status = session_status(&path);
            #[cfg(feature = "tui")]
            let recorded_at = recorded_at_from_session_name(&name);

            Some(SessionEntry {
                path,
                name,
                status,
                modified,
                #[cfg(feature = "tui")]
                recorded_at,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(entries)
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
fn session_status(path: &Path) -> SessionStatus {
    if path.join("notes.md").exists() {
        SessionStatus::NotesReady
    } else if path.join("transcript.txt").exists() {
        SessionStatus::TranscriptReady
    } else if path.join("recording.wav").exists() {
        SessionStatus::RecordingOnly
    } else {
        SessionStatus::Empty
    }
}

#[cfg(feature = "tui")]
pub fn recorded_at_from_session_name(name: &str) -> Option<SystemTime> {
    let prefix = name.get(..17)?;
    let parsed = chrono::NaiveDateTime::parse_from_str(prefix, "%Y-%m-%d_%H%M%S").ok()?;
    let local = parsed.and_local_timezone(chrono::Local).single()?;
    Some(local.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn list_sessions_returns_directories_newest_first_with_status() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();

        let older = base.join("2026-05-07_100000 — Older");
        fs::create_dir_all(&older).unwrap();
        fs::write(older.join("recording.wav"), b"fake").unwrap();

        sleep(Duration::from_millis(20));

        let newer = base.join("2026-05-08_100000 — Newer");
        fs::create_dir_all(&newer).unwrap();
        fs::write(newer.join("recording.wav"), b"fake").unwrap();
        fs::write(newer.join("transcript.txt"), b"transcript").unwrap();
        fs::write(newer.join("notes.md"), b"notes").unwrap();

        let sessions = list_sessions(base).unwrap();

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].path, newer);
        assert_eq!(sessions[0].status, SessionStatus::NotesReady);
        assert_eq!(sessions[1].path, older);
        assert_eq!(sessions[1].status, SessionStatus::RecordingOnly);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn parses_recorded_time_from_session_directory_prefix() {
        let recorded_at = recorded_at_from_session_name("2026-05-08_164949 — Test 1").unwrap();
        let datetime: chrono::DateTime<chrono::Local> = recorded_at.into();

        assert_eq!(
            datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-05-08 16:49:49"
        );
    }

    #[cfg(feature = "tui")]
    #[test]
    fn recorded_time_returns_none_for_non_scribe_directory_name() {
        assert_eq!(recorded_at_from_session_name("not-a-session"), None);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn list_sessions_populates_recorded_at_from_directory_name() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let session = base.join("2026-05-08_164949 — Test 1");
        fs::create_dir_all(&session).unwrap();
        fs::write(session.join("recording.wav"), b"fake").unwrap();

        let sessions = list_sessions(base).unwrap();

        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].recorded_at.is_some());
    }

    #[test]
    fn list_sessions_includes_directories_without_recordings_as_empty() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let empty = base.join("empty-session");
        fs::create_dir_all(&empty).unwrap();

        let sessions = list_sessions(base).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Empty);
    }
}
