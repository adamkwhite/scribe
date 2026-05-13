use anyhow::{Context, Result};
use std::path::PathBuf;

use scribe_core::config::{self, Config};

pub fn load_existing_config() -> Result<Option<Config>> {
    let path = config::config_path()?;
    if path.exists() {
        let config_dir = path
            .parent()
            .context("Config path has no parent directory")?
            .to_path_buf();
        config::load_from_path(&path)
            .map(|config| config::resolve_managed_whisper_model_config(config, &config_dir))
            .map(Some)
    } else {
        Ok(None)
    }
}

pub fn save_config(cfg: &Config) -> Result<PathBuf> {
    let path = config::config_path()?;
    config::save_to_path(&path, cfg)?;
    Ok(path)
}
