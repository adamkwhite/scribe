use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, filter::LevelFilter};

use crate::config;

pub fn log_path() -> Result<PathBuf> {
    Ok(log_path_in_dir(&config::config_dir()?))
}

pub fn init_file_logging(binary_name: &'static str) -> Result<PathBuf> {
    let path = log_path()?;
    init_file_logging_at_path(binary_name, &path)?;
    Ok(path)
}

fn init_file_logging_at_path(binary_name: &'static str, path: &Path) -> Result<()> {
    let file = prepare_log_file(path)?;
    drop(file);
    let parent = path.parent().context("Log path has no parent directory")?;
    let file_name = path.file_name().context("Log path has no file name")?;
    let writer = tracing_appender::rolling::never(parent, file_name);
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(LevelFilter::DEBUG),
    );

    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to initialize file logging subscriber")?;

    tracing::info!(
        binary = binary_name,
        log_path = %path.display(),
        "file logging initialized"
    );
    Ok(())
}

fn log_path_in_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("scribe.log")
}

fn prepare_log_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("Failed to open log file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn log_path_in_dir_uses_scribe_log_filename() {
        let path = log_path_in_dir(std::path::Path::new("/tmp/scribe-config"));

        assert_eq!(path, std::path::Path::new("/tmp/scribe-config/scribe.log"));
    }

    #[test]
    fn prepare_log_file_truncates_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scribe.log");
        fs::write(&path, "old log contents").unwrap();

        prepare_log_file(&path).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }
}
