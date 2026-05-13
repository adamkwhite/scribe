use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Path to whisper.cpp executable
    #[serde(default)]
    pub whisper_bin: Option<String>,

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
        assert_eq!(cfg.whisper_bin.as_deref(), Some("/usr/bin/whisper-cli"));
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
        assert_eq!(cfg.whisper_bin.as_deref(), Some("whisper-cli"));
        assert_eq!(cfg.model, "google/gemini-2.5-flash");
        assert_eq!(cfg.sample_rate, 16000);
        assert!(cfg.output_dir.is_none());
    }

    #[test]
    fn parses_config_without_whisper_bin() {
        let toml_str = r#"
            whisper_model = "ggml-base.en.bin"
            openrouter_api_key = "sk-or-test"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.whisper_bin.is_none());
        assert_eq!(cfg.whisper_model, "ggml-base.en.bin");
    }

    #[test]
    fn rejects_config_missing_api_key() {
        let toml_str = r#"
            whisper_model = "ggml-base.en.bin"
        "#;
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn round_trips_through_toml() {
        let original = Config {
            whisper_bin: Some("whisper".into()),
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
