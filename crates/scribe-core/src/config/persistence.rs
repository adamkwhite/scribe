use anyhow::{Context, Result};
use std::path::Path;
#[cfg(feature = "tui")]
use std::path::PathBuf;

#[cfg(feature = "tui")]
use super::managed_model::resolve_managed_whisper_model_config;
#[cfg(feature = "tui")]
use super::paths::config_path;
use super::settings::Config;

#[cfg(feature = "tui")]
pub fn load_existing() -> Result<Option<Config>> {
    let path = config_path()?;
    if path.exists() {
        let config_dir = path
            .parent()
            .context("Config path has no parent directory")?
            .to_path_buf();
        load_from_path(&path)
            .map(|config| resolve_managed_whisper_model_config(config, &config_dir))
            .map(Some)
    } else {
        Ok(None)
    }
}

pub fn load_from_path(path: &Path) -> Result<Config> {
    tracing::info!(config_path = %path.display(), "loading config");
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    toml::from_str(&contents).with_context(|| format!("Failed to parse {}", path.display()))
}

#[cfg(feature = "tui")]
pub fn save(cfg: &Config) -> Result<PathBuf> {
    let path = config_path()?;
    save_to_path(&path, cfg)?;
    Ok(path)
}

pub fn save_to_path(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let toml_str = toml::to_string_pretty(cfg)?;
    std::fs::write(path, toml_str)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    tracing::info!(config_path = %path.display(), "config saved");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_existing_config_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: "model.bin".into(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 22050,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        save_to_path(&path, &cfg).unwrap();
        let loaded = load_from_path(&path).unwrap();

        assert_eq!(loaded.whisper_bin, cfg.whisper_bin);
        assert_eq!(loaded.whisper_model, cfg.whisper_model);
        assert_eq!(loaded.openrouter_api_key, cfg.openrouter_api_key);
        assert_eq!(loaded.model, cfg.model);
        assert_eq!(loaded.sample_rate, cfg.sample_rate);
        assert_eq!(loaded.output_dir, cfg.output_dir);
    }
}
