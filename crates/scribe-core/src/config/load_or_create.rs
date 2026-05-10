use anyhow::{Context, Result};

use super::default_config::default_config;
use super::managed_model::{
    ensure_managed_whisper_model, is_managed_model_path, resolve_managed_whisper_model_config,
};
use super::paths::config_path;
use super::persistence::{load_from_path, save_to_path};
use super::settings::Config;

pub async fn load_or_create() -> Result<Config> {
    let path = config_path()?;
    let config_dir = path
        .parent()
        .context("Config path has no parent directory")?
        .to_path_buf();

    let config = if path.exists() {
        tracing::info!(config_path = %path.display(), "existing config found");
        load_from_path(&path)?
    } else {
        // Create a default config for the user to fill in
        let config = default_config(&config_dir);

        save_to_path(&path, &config)?;
        tracing::info!(config_path = %path.display(), "default config created");

        println!("Created config at: {}", path.display());
        println!("Please edit it with your whisper model path and OpenRouter API key.");
        #[cfg(feature = "whisper-cli")]
        println!("Set whisper_bin to your whisper.cpp executable path.\n");
        #[cfg(not(feature = "whisper-cli"))]
        println!();

        config
    };

    let config = resolve_managed_whisper_model_config(config, &config_dir);
    if is_managed_model_path(&config.whisper_model, &config_dir) {
        ensure_managed_whisper_model().await?;
    }
    Ok(config)
}
