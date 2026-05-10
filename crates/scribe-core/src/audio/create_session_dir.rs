use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config;

/// Create a new session directory with optional name.
pub fn create_session_dir(cfg: &config::Config, name: Option<&str>) -> Result<PathBuf> {
    let base = config::effective_output_dir(cfg)?;
    create_session_dir_in(&base, name)
}

pub fn create_session_dir_in(base: &Path, name: Option<&str>) -> Result<PathBuf> {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
    let dir_name = match name {
        Some(n) if !n.is_empty() => format!("{timestamp} — {n}"),
        _ => format!("{timestamp}"),
    };
    let session_dir = base.join(dir_name);
    std::fs::create_dir_all(&session_dir)?;
    Ok(session_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_session_dir_in_uses_configured_base_dir() {
        let temp = tempfile::tempdir().unwrap();

        let session_dir = create_session_dir_in(temp.path(), Some("Planning")).unwrap();

        assert!(session_dir.starts_with(temp.path()));
        assert!(session_dir.exists());
        assert!(
            session_dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("Planning")
        );
    }
}
