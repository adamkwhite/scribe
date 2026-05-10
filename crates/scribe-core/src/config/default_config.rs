use std::path::Path;

use super::managed_model::managed_model_path_in_dir;
use super::settings::Config;

pub fn default_config(config_dir: &Path) -> Config {
    Config {
        whisper_bin: default_whisper_bin(),
        whisper_model: default_whisper_model(config_dir),
        openrouter_api_key: "YOUR_KEY_HERE".to_string(),
        model: "google/gemini-2.5-flash".to_string(),
        sample_rate: 16000,
        output_dir: None,
    }
}

fn default_whisper_model(config_dir: &Path) -> String {
    managed_model_path_in_dir(config_dir)
        .to_string_lossy()
        .into_owned()
}

fn default_whisper_bin() -> Option<String> {
    #[cfg(feature = "whisper-cli")]
    {
        Some("whisper-cli".to_string())
    }

    #[cfg(not(feature = "whisper-cli"))]
    {
        None
    }
}

#[cfg(all(test, not(feature = "whisper-cli")))]
mod tests {
    use super::*;

    #[cfg(not(feature = "whisper-cli"))]
    #[test]
    fn default_config_uses_embedded_backend_and_managed_model_path() {
        let temp = tempfile::tempdir().unwrap();

        let cfg = default_config(temp.path());

        assert!(cfg.whisper_bin.is_none());
        assert_eq!(
            cfg.whisper_model,
            managed_model_path_in_dir(temp.path()).to_string_lossy()
        );
    }
}
