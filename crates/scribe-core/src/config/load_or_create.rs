use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::default_config::default_config;
use super::managed_model::resolve_managed_whisper_model_config;
use super::paths::config_path;
use super::persistence::{load_from_path, save_to_path};
use super::settings::Config;

/// Indicates whether `load_or_create` loaded an existing config or wrote a
/// fresh default. Callers use this to decide whether to print a first-run
/// message (CLI) or surface a setup prompt (tray) — keeps the library out
/// of stdout.
pub enum ConfigOrigin {
    Existing,
    JustCreated(PathBuf),
}

pub fn load_or_create() -> Result<(Config, ConfigOrigin)> {
    let path = config_path()?;
    load_or_create_from_path(&path)
}

fn load_or_create_from_path(path: &Path) -> Result<(Config, ConfigOrigin)> {
    let config_dir = path
        .parent()
        .context("Config path has no parent directory")?
        .to_path_buf();

    let (config, origin) = if path.exists() {
        tracing::info!(config_path = %path.display(), "existing config found");
        (load_from_path(path)?, ConfigOrigin::Existing)
    } else {
        let config = default_config(&config_dir);
        save_to_path(path, &config)?;
        tracing::info!(config_path = %path.display(), "default config created");
        (config, ConfigOrigin::JustCreated(path.to_path_buf()))
    };

    Ok((
        resolve_managed_whisper_model_config(config, &config_dir),
        origin,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn existing_managed_config_loads_without_downloading_model() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        let cfg = Config {
            whisper_bin: None,
            whisper_model: "ggml-base.en.bin".into(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };
        save_to_path(&config_path, &cfg).unwrap();

        let (loaded, origin) = load_or_create_from_path(&config_path).unwrap();

        assert_eq!(
            loaded.whisper_model,
            Path::new(temp.path())
                .join("ggml-base.en.bin")
                .to_string_lossy()
        );
        assert!(!temp.path().join("ggml-base.en.bin").exists());
        assert!(matches!(origin, ConfigOrigin::Existing));
    }

    #[test]
    fn missing_config_creates_default_and_reports_just_created() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");

        let (_, origin) = load_or_create_from_path(&config_path).unwrap();

        assert!(config_path.exists(), "config.toml should be written");
        match origin {
            ConfigOrigin::JustCreated(p) => assert_eq!(p, config_path),
            ConfigOrigin::Existing => panic!("expected JustCreated"),
        }
    }
}
