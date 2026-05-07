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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_config() {
        let toml_str = r#"
            whisper_bin = "/usr/bin/whisper-cli"
            whisper_model = "/models/ggml-base.en.bin"
            openrouter_api_key = "sk-or-test"
            model = "anthropic/claude-3-5-sonnet"
            sample_rate = 44100
            output_dir = "/tmp/scribe-out"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.whisper_bin, "/usr/bin/whisper-cli");
        assert_eq!(cfg.whisper_model, "/models/ggml-base.en.bin");
        assert_eq!(cfg.openrouter_api_key, "sk-or-test");
        assert_eq!(cfg.model, "anthropic/claude-3-5-sonnet");
        assert_eq!(cfg.sample_rate, 44100);
        assert_eq!(cfg.output_dir.as_deref(), Some("/tmp/scribe-out"));
    }

    #[test]
    fn parses_minimal_config_with_defaults() {
        let toml_str = r#"
            whisper_bin = "whisper-cli"
            whisper_model = "ggml-base.en.bin"
            openrouter_api_key = "sk-or-test"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.model, "google/gemini-2.5-flash");
        assert_eq!(cfg.sample_rate, 16000);
        assert!(cfg.output_dir.is_none());
    }

    #[test]
    fn rejects_config_missing_required_field() {
        // openrouter_api_key is required (no default)
        let toml_str = r#"
            whisper_bin = "whisper-cli"
            whisper_model = "ggml-base.en.bin"
        "#;
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn round_trips_through_toml() {
        let original = Config {
            whisper_bin: "whisper".into(),
            whisper_model: "model.bin".into(),
            openrouter_api_key: "key".into(),
            model: "some/model".into(),
            sample_rate: 22050,
            output_dir: Some("/data/out".into()),
        };
        let serialized = toml::to_string(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.whisper_bin, original.whisper_bin);
        assert_eq!(parsed.whisper_model, original.whisper_model);
        assert_eq!(parsed.openrouter_api_key, original.openrouter_api_key);
        assert_eq!(parsed.model, original.model);
        assert_eq!(parsed.sample_rate, original.sample_rate);
        assert_eq!(parsed.output_dir, original.output_dir);
    }
}
