use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::utils::sessions;

pub const ARCHIVE_DIR_NAME: &str = "archive";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FilenamePlatform {
    Unix,
    Macos,
    Windows,
}

pub fn archive_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(ARCHIVE_DIR_NAME)
}

pub fn delete_session(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("Failed to delete {}", path.display()))?;
    }
    Ok(())
}

pub fn validate_session_name(name: &str) -> Result<&str> {
    validate_session_name_for_platform(name, FilenamePlatform::current())
}

fn validate_session_name_for_platform(name: &str, platform: FilenamePlatform) -> Result<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("Session name cannot be empty");
    }
    if trimmed.chars().any(char::is_control) {
        bail!("Session name must use printable Unicode characters");
    }
    if trimmed == "." || trimmed == ".." {
        bail!("Session name must be a valid directory name");
    }
    if let Some(character) = trimmed.chars().find(|ch| platform.invalid_character(*ch)) {
        bail!("Session name contains invalid directory name character: {character}");
    }
    if platform == FilenamePlatform::Windows && trimmed.ends_with(['.', ' ']) {
        bail!("Windows session names cannot end with a space or period");
    }
    if platform == FilenamePlatform::Windows && is_windows_reserved_name(trimmed) {
        bail!("Session name is reserved on Windows");
    }
    Ok(trimmed)
}

pub fn rename_session(path: &Path, new_name: &str) -> Result<PathBuf> {
    let new_name = validate_session_name(new_name)?;
    let current_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Session path has no valid directory name")?;
    let new_dir_name = renamed_session_dir_name(current_name, new_name);
    let destination = path
        .parent()
        .context("Session path has no parent directory")?
        .join(new_dir_name);
    if destination != path && destination.exists() {
        bail!("A session named {new_name} already exists");
    }
    std::fs::rename(path, &destination).with_context(|| {
        format!(
            "Failed to rename {} to {}",
            path.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

pub fn archive_session(path: &Path, archive_root: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(archive_root)
        .with_context(|| format!("Failed to create {}", archive_root.display()))?;
    let name = path
        .file_name()
        .context("Session path has no directory name")?;
    let destination = unique_destination(archive_root, name);
    std::fs::rename(path, &destination).with_context(|| {
        format!(
            "Failed to move {} to {}",
            path.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

pub fn cleanup_archive(archive_root: &Path, retention: Duration, now: SystemTime) -> Result<usize> {
    if !archive_root.exists() {
        return Ok(0);
    }

    let mut removed = 0;
    for entry in std::fs::read_dir(archive_root)
        .with_context(|| format!("Failed to read {}", archive_root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if now
            .duration_since(modified)
            .is_ok_and(|age| age > retention)
        {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("Failed to delete archived session {}", path.display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn unique_destination(archive_root: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let base = archive_root.join(name);
    if !base.exists() {
        return base;
    }

    for suffix in 1.. {
        let candidate = archive_root.join(format!("{}-{suffix}", name.to_string_lossy()));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded suffix search should always find an unused path")
}

fn renamed_session_dir_name(current_name: &str, new_name: &str) -> String {
    if sessions::recorded_at_from_session_name(current_name).is_some()
        && let Some(prefix) = current_name.get(..17)
    {
        return format!("{prefix} — {new_name}");
    }
    new_name.to_string()
}

impl FilenamePlatform {
    fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Unix
        }
    }

    fn invalid_character(self, ch: char) -> bool {
        match self {
            Self::Unix => ch == '/',
            Self::Macos => matches!(ch, '/' | ':'),
            Self::Windows => matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'),
        }
    }
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name
        .split_once('.')
        .map_or(name, |(stem, _extension)| stem)
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem.strip_prefix("COM").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || stem.strip_prefix("LPT").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use filetime::{FileTime, set_file_mtime};

    #[test]
    fn archive_session_moves_session_into_archive_root() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        let archive = temp.path().join("config").join("archive");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("notes.md"), "notes").unwrap();

        let archived = archive_session(&session, &archive).unwrap();

        assert!(!session.exists());
        assert_eq!(archived, archive.join("2026-05-08_164949 — Test 1"));
        assert!(archived.join("notes.md").exists());
    }

    #[test]
    fn delete_session_removes_session_directory() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        std::fs::create_dir_all(&session).unwrap();

        delete_session(&session).unwrap();

        assert!(!session.exists());
    }

    #[test]
    fn cleanup_archive_removes_directories_older_than_retention() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let old = archive.join("old");
        let fresh = archive.join("fresh");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();

        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 24 * 60 * 60);
        let old_time = FileTime::from_system_time(now - Duration::from_secs(8 * 24 * 60 * 60));
        let fresh_time = FileTime::from_system_time(now - Duration::from_secs(6 * 24 * 60 * 60));
        set_file_mtime(&old, old_time).unwrap();
        set_file_mtime(&fresh, fresh_time).unwrap();

        let removed =
            cleanup_archive(&archive, Duration::from_secs(7 * 24 * 60 * 60), now).unwrap();

        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn validate_session_name_rejects_empty_names() {
        let error = validate_session_name("   ").unwrap_err();

        assert_eq!(error.to_string(), "Session name cannot be empty");
    }

    #[test]
    fn validate_session_name_rejects_non_printable_characters() {
        let error = validate_session_name("Team\nStandup").unwrap_err();

        assert_eq!(
            error.to_string(),
            "Session name must use printable Unicode characters"
        );
    }

    #[test]
    fn validate_session_name_accepts_printable_unicode() {
        assert_eq!(
            validate_session_name(" Démo – lancement  ").unwrap(),
            "Démo – lancement"
        );
    }

    #[test]
    fn platform_validation_rejects_dot_directory_names() {
        let error = validate_session_name_for_platform(".", FilenamePlatform::Unix).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Session name must be a valid directory name"
        );
    }

    #[test]
    fn platform_validation_rejects_macos_colon() {
        let error = validate_session_name_for_platform("Team:Standup", FilenamePlatform::Macos)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Session name contains invalid directory name character: :"
        );
    }

    #[test]
    fn platform_validation_rejects_windows_reserved_name() {
        let error =
            validate_session_name_for_platform("CON.txt", FilenamePlatform::Windows).unwrap_err();

        assert_eq!(error.to_string(), "Session name is reserved on Windows");
    }

    #[test]
    fn platform_validation_rejects_windows_invalid_characters() {
        let error = validate_session_name_for_platform("Team?Standup", FilenamePlatform::Windows)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Session name contains invalid directory name character: ?"
        );
    }

    #[test]
    fn platform_validation_rejects_windows_trailing_space_or_period() {
        let error = validate_session_name_for_platform("Team standup.", FilenamePlatform::Windows)
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Windows session names cannot end with a space or period"
        );
    }

    #[test]
    fn rename_session_preserves_timestamp_prefix_and_contents() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("2026-05-08_164949 — Test 1");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("notes.md"), "notes").unwrap();

        let renamed = rename_session(&session, "Team standup").unwrap();

        assert!(!session.exists());
        assert_eq!(
            renamed.file_name().unwrap().to_string_lossy(),
            "2026-05-08_164949 — Team standup"
        );
        assert!(renamed.join("notes.md").exists());
    }

    #[test]
    fn rename_session_without_timestamp_renames_to_valid_name() {
        let temp = tempfile::tempdir().unwrap();
        let session = temp.path().join("Imported session");
        std::fs::create_dir_all(&session).unwrap();

        let renamed = rename_session(&session, "Imported notes").unwrap();

        assert!(!session.exists());
        assert_eq!(
            renamed.file_name().unwrap().to_string_lossy(),
            "Imported notes"
        );
    }
}
