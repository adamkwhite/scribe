use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Path to whisper.cpp executable
    pub whisper_bin: String,

    /// Path to whisper model file (e.g., ggml-base.en.bin)
    pub whisper_model: String,

    /// OpenRouter API key
    pub openrouter_api_key: String,

    /// Model to use for note generation
    #[serde(default = "default_model")]
    pub model: String,

    /// Audio sample rate
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    /// Output directory for recordings and notes
    #[serde(default)]
    pub output_dir: Option<String>,
}

fn default_model() -> String {
    "google/gemini-2.5-flash".to_string()
}

fn default_sample_rate() -> u32 {
    16000
}

pub fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("Could not find config directory")?
        .join("scribe");
    Ok(dir.join("config.toml"))
}

pub fn output_dir() -> Result<PathBuf> {
    let dir = dirs::document_dir()
        .or_else(dirs::home_dir)
        .context("Could not find home directory")?
        .join("scribe");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn load_or_create() -> Result<Config> {
    let path = config_path()?;

    if path.exists() {
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(config)
    } else {
        // Create a default config for the user to fill in
        let config = Config {
            whisper_bin: "whisper-cli".to_string(),
            whisper_model: "ggml-base.en.bin".to_string(),
            openrouter_api_key: "YOUR_KEY_HERE".to_string(),
            model: default_model(),
            sample_rate: default_sample_rate(),
            output_dir: None,
        };

        std::fs::create_dir_all(path.parent().unwrap())?;
        let toml_str = toml::to_string_pretty(&config)?;
        std::fs::write(&path, &toml_str)?;

        println!("Created config at: {}", path.display());
        println!("Please edit it with your whisper.cpp path and OpenRouter API key.\n");

        Ok(config)
    }
}
