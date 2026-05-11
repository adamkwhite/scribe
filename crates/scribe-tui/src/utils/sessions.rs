use std::path::PathBuf;
use std::time::SystemTime;

use scribe_core::audio;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEntry {
    pub path: PathBuf,
    pub name: String,
    pub status: audio::SessionStatus,
    pub modified: SystemTime,
    pub recorded_at: Option<SystemTime>,
}

impl From<audio::SessionEntry> for SessionEntry {
    fn from(session: audio::SessionEntry) -> Self {
        let recorded_at = recorded_at_from_session_name(&session.name);
        Self {
            path: session.path,
            name: session.name,
            status: session.status,
            modified: session.modified,
            recorded_at,
        }
    }
}

pub fn recorded_at_from_session_name(name: &str) -> Option<SystemTime> {
    let prefix = name.get(..17)?;
    let parsed = chrono::NaiveDateTime::parse_from_str(prefix, "%Y-%m-%d_%H%M%S").ok()?;
    let local = parsed.and_local_timezone(chrono::Local).single()?;
    Some(local.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_recorded_time_from_session_directory_prefix() {
        let recorded_at = recorded_at_from_session_name("2026-05-08_164949 — Test 1").unwrap();
        let datetime: chrono::DateTime<chrono::Local> = recorded_at.into();

        assert_eq!(
            datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-05-08 16:49:49"
        );
    }

    #[test]
    fn recorded_time_returns_none_for_non_scribe_directory_name() {
        assert_eq!(recorded_at_from_session_name("not-a-session"), None);
    }

    #[test]
    fn session_entry_from_core_populates_recorded_at_from_directory_name() {
        let session = SessionEntry::from(audio::SessionEntry {
            path: PathBuf::from("2026-05-08_164949 — Test 1"),
            name: "2026-05-08_164949 — Test 1".to_string(),
            status: audio::SessionStatus::RecordingOnly,
            modified: SystemTime::UNIX_EPOCH,
        });

        assert!(session.recorded_at.is_some());
    }
}
