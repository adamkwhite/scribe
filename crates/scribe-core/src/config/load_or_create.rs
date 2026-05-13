use anyhow::{Context, Result};
use std::path::Path;

use super::default_config::default_config;
use super::managed_model::resolve_managed_whisper_model_config;
use super::paths::config_path;
use super::persistence::{load_from_path, save_to_path};
use super::settings::Config;

pub fn load_or_create() -> Result<Config> {
    let path = config_path()?;
    load_or_create_from_path(&path)
}

fn load_or_create_from_path(path: &Path) -> Result<Config> {
    let config_dir = path
        .parent()
        .context("Config path has no parent directory")?
        .to_path_buf();

    let config = if path.exists() {
        tracing::info!(config_path = %path.display(), "existing config found");
        load_from_path(path)?
    } else {
        // Create a default config for the user to fill in
        let config = default_config(&config_dir);

        save_to_path(path, &config)?;
        tracing::info!(config_path = %path.display(), "default config created");

        println!("Created config at: {}", path.display());
        println!("Please edit it with your whisper model path and OpenRouter API key.");
        println!();

        config
    };

    Ok(resolve_managed_whisper_model_config(config, &config_dir))
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

        let loaded = load_or_create_from_path(&config_path).unwrap();

        assert_eq!(
            loaded.whisper_model,
            Path::new(temp.path())
                .join("ggml-base.en.bin")
                .to_string_lossy()
        );
        assert!(!temp.path().join("ggml-base.en.bin").exists());
    }
}
