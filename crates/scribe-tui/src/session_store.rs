use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const ARCHIVE_DIR_NAME: &str = "archive";

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
}
