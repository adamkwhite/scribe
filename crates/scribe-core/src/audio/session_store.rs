use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::config;

use super::session_entry::SessionEntry;
use super::session_status::SessionStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateAudioSessionInput {
    pub name: Option<String>,
    pub context: CreateAudioSessionContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateAudioSessionContext {
    pub timestamp: AudioSessionTimestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioSessionTimestamp {
    value: String,
}

impl AudioSessionTimestamp {
    pub fn now_local() -> Self {
        Self {
            value: chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string(),
        }
    }

    pub fn fixed(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateAudioSessionOutput {
    pub session_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListAudioSessionsOutput {
    pub sessions: Vec<SessionEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatestAudioSessionOutput {
    pub session_dir: PathBuf,
}

pub trait AudioSessionStore: Send + Sync {
    fn create_session(&self, input: CreateAudioSessionInput) -> Result<CreateAudioSessionOutput>;
    fn list_sessions(&self) -> Result<ListAudioSessionsOutput>;
    fn latest_recording_session(&self) -> Result<LatestAudioSessionOutput>;
}

pub struct FileSystemAudioSessionStore {
    base_dir: PathBuf,
    filesystem: Arc<dyn AudioSessionFileSystem>,
}

impl FileSystemAudioSessionStore {
    pub fn from_config(cfg: &config::Config) -> Result<Self> {
        Ok(Self::new(config::effective_output_dir(cfg)?))
    }

    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            filesystem: Arc::new(RealAudioSessionFileSystem),
        }
    }

    #[cfg(test)]
    fn with_filesystem(base_dir: PathBuf, filesystem: Arc<dyn AudioSessionFileSystem>) -> Self {
        Self {
            base_dir,
            filesystem,
        }
    }

    fn session_directories(&self) -> Result<Vec<AudioSessionDirectory>> {
        let mut directories = self.filesystem.list_session_dirs(&self.base_dir)?;
        directories.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| b.name.cmp(&a.name))
        });
        Ok(directories)
    }
}

impl AudioSessionStore for FileSystemAudioSessionStore {
    fn create_session(&self, input: CreateAudioSessionInput) -> Result<CreateAudioSessionOutput> {
        let timestamp = input.context.timestamp.as_str();
        let dir_name = match input.name.as_deref() {
            Some(name) if !name.trim().is_empty() => format!("{timestamp} — {name}"),
            _ => timestamp.to_string(),
        };
        let session_dir = self.base_dir.join(dir_name);
        self.filesystem.create_dir_all(&session_dir)?;
        Ok(CreateAudioSessionOutput { session_dir })
    }

    fn list_sessions(&self) -> Result<ListAudioSessionsOutput> {
        let sessions = self
            .session_directories()?
            .into_iter()
            .map(|directory| {
                let status = directory.status();
                SessionEntry {
                    path: directory.path,
                    name: directory.name,
                    status,
                    modified: directory.modified,
                }
            })
            .collect();
        Ok(ListAudioSessionsOutput { sessions })
    }

    fn latest_recording_session(&self) -> Result<LatestAudioSessionOutput> {
        self.session_directories()?
            .into_iter()
            .find(|directory| directory.has_recording)
            .map(|directory| LatestAudioSessionOutput {
                session_dir: directory.path,
            })
            .context("No recordings found")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AudioSessionDirectory {
    path: PathBuf,
    name: String,
    modified: SystemTime,
    has_recording: bool,
    has_transcript: bool,
    has_notes: bool,
}

impl AudioSessionDirectory {
    fn status(&self) -> SessionStatus {
        if self.has_notes {
            SessionStatus::NotesReady
        } else if self.has_transcript {
            SessionStatus::TranscriptReady
        } else if self.has_recording {
            SessionStatus::RecordingOnly
        } else {
            SessionStatus::Empty
        }
    }
}

trait AudioSessionFileSystem: Send + Sync {
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn list_session_dirs(&self, base_dir: &Path) -> Result<Vec<AudioSessionDirectory>>;
}

struct RealAudioSessionFileSystem;

impl AudioSessionFileSystem for RealAudioSessionFileSystem {
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)
            .with_context(|| format!("Failed to create session directory {}", path.display()))
    }

    fn list_session_dirs(&self, base_dir: &Path) -> Result<Vec<AudioSessionDirectory>> {
        if !base_dir.exists() {
            return Ok(Vec::new());
        }

        let directories = std::fs::read_dir(base_dir)
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
                Some(AudioSessionDirectory {
                    has_recording: path.join("recording.wav").exists(),
                    has_transcript: path.join("transcript.txt").exists(),
                    has_notes: path.join("notes.md").exists(),
                    path,
                    name,
                    modified,
                })
            })
            .collect();
        Ok(directories)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    #[test]
    fn typed_timestamp_supports_fixed_and_local_values() {
        let fixed = AudioSessionTimestamp::fixed("2026-05-10_120000");

        assert_eq!(fixed.as_str(), "2026-05-10_120000");
        assert!(!AudioSessionTimestamp::now_local().as_str().is_empty());
    }

    #[test]
    fn create_session_uses_fixed_timestamp_and_name_deterministically() {
        let filesystem = Arc::new(FakeAudioSessionFileSystem::default());
        let store = FileSystemAudioSessionStore::with_filesystem(
            PathBuf::from("/sessions"),
            filesystem.clone(),
        );

        let output = store
            .create_session(CreateAudioSessionInput {
                name: Some("Planning".to_string()),
                context: CreateAudioSessionContext {
                    timestamp: AudioSessionTimestamp::fixed("2026-05-10_120000"),
                },
            })
            .unwrap();

        assert_eq!(
            output,
            CreateAudioSessionOutput {
                session_dir: PathBuf::from("/sessions/2026-05-10_120000 — Planning"),
            }
        );
        assert_eq!(
            filesystem.created_paths(),
            vec![PathBuf::from("/sessions/2026-05-10_120000 — Planning")]
        );
    }

    #[test]
    fn create_session_uses_timestamp_only_for_missing_or_blank_names() {
        let filesystem = Arc::new(FakeAudioSessionFileSystem::default());
        let store = FileSystemAudioSessionStore::with_filesystem(
            PathBuf::from("/sessions"),
            filesystem.clone(),
        );

        let unnamed = store
            .create_session(CreateAudioSessionInput {
                name: None,
                context: CreateAudioSessionContext {
                    timestamp: AudioSessionTimestamp::fixed("2026-05-10_120000"),
                },
            })
            .unwrap();
        let blank = store
            .create_session(CreateAudioSessionInput {
                name: Some("   ".to_string()),
                context: CreateAudioSessionContext {
                    timestamp: AudioSessionTimestamp::fixed("2026-05-10_120001"),
                },
            })
            .unwrap();

        assert_eq!(
            unnamed.session_dir,
            PathBuf::from("/sessions/2026-05-10_120000")
        );
        assert_eq!(
            blank.session_dir,
            PathBuf::from("/sessions/2026-05-10_120001")
        );
    }

    #[test]
    fn list_sessions_maps_statuses_and_sorts_without_sleeping() {
        let filesystem = Arc::new(FakeAudioSessionFileSystem::with_dirs(vec![
            fake_dir("older", 10, true, false, false),
            fake_dir("newer", 20, true, true, true),
            fake_dir("empty", 30, false, false, false),
        ]));
        let store =
            FileSystemAudioSessionStore::with_filesystem(PathBuf::from("/sessions"), filesystem);

        let output = store.list_sessions().unwrap();

        assert_eq!(
            output.sessions,
            vec![
                SessionEntry {
                    path: PathBuf::from("/sessions/empty"),
                    name: "empty".to_string(),
                    status: SessionStatus::Empty,
                    modified: time(30),
                },
                SessionEntry {
                    path: PathBuf::from("/sessions/newer"),
                    name: "newer".to_string(),
                    status: SessionStatus::NotesReady,
                    modified: time(20),
                },
                SessionEntry {
                    path: PathBuf::from("/sessions/older"),
                    name: "older".to_string(),
                    status: SessionStatus::RecordingOnly,
                    modified: time(10),
                },
            ]
        );
    }

    #[test]
    fn latest_recording_session_skips_empty_sessions() {
        let filesystem = Arc::new(FakeAudioSessionFileSystem::with_dirs(vec![
            fake_dir("recorded", 10, true, false, false),
            fake_dir("empty", 20, false, false, false),
        ]));
        let store =
            FileSystemAudioSessionStore::with_filesystem(PathBuf::from("/sessions"), filesystem);

        let output = store.latest_recording_session().unwrap();

        assert_eq!(
            output,
            LatestAudioSessionOutput {
                session_dir: PathBuf::from("/sessions/recorded")
            }
        );
    }

    #[test]
    fn latest_recording_session_errors_when_no_recording_exists() {
        let filesystem = Arc::new(FakeAudioSessionFileSystem::with_dirs(vec![fake_dir(
            "empty", 20, false, false, false,
        )]));
        let store =
            FileSystemAudioSessionStore::with_filesystem(PathBuf::from("/sessions"), filesystem);

        let error = store.latest_recording_session().unwrap_err();

        assert_eq!(error.to_string(), "No recordings found");
    }

    #[derive(Default)]
    struct FakeAudioSessionFileSystem {
        created_paths: Mutex<Vec<PathBuf>>,
        directories: Vec<AudioSessionDirectory>,
    }

    impl FakeAudioSessionFileSystem {
        fn with_dirs(directories: Vec<AudioSessionDirectory>) -> Self {
            Self {
                created_paths: Mutex::new(Vec::new()),
                directories,
            }
        }

        fn created_paths(&self) -> Vec<PathBuf> {
            self.created_paths.lock().unwrap().clone()
        }
    }

    impl AudioSessionFileSystem for FakeAudioSessionFileSystem {
        fn create_dir_all(&self, path: &Path) -> Result<()> {
            self.created_paths.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }

        fn list_session_dirs(&self, _base_dir: &Path) -> Result<Vec<AudioSessionDirectory>> {
            Ok(self.directories.clone())
        }
    }

    fn fake_dir(
        name: &str,
        modified_secs: u64,
        has_recording: bool,
        has_transcript: bool,
        has_notes: bool,
    ) -> AudioSessionDirectory {
        AudioSessionDirectory {
            path: PathBuf::from(format!("/sessions/{name}")),
            name: name.to_string(),
            modified: time(modified_secs),
            has_recording,
            has_transcript,
            has_notes,
        }
    }

    fn time(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }
}
