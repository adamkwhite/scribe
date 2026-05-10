use anyhow::{Context, Result};
use std::path::Path;

use super::output_dir::effective_output_dir;
use super::settings::Config;

pub fn validate_setup(cfg: &Config) -> Result<()> {
    let key = cfg.openrouter_api_key.trim();
    if key.is_empty() || key == "YOUR_KEY_HERE" {
        anyhow::bail!("OpenRouter API key is required");
    }

    if cfg.model.trim().is_empty() {
        anyhow::bail!("Notes model is required");
    }

    #[cfg(feature = "whisper-cli")]
    {
        if cfg
            .whisper_bin
            .as_deref()
            .map(str::trim)
            .filter(|bin| !bin.is_empty())
            .is_none()
        {
            anyhow::bail!("whisper_bin is required when the whisper-cli backend is enabled");
        }
    }

    let model_path = Path::new(&cfg.whisper_model);
    if !model_path.exists() {
        anyhow::bail!("Whisper model does not exist: {}", model_path.display());
    }

    let output = effective_output_dir(cfg)?;
    let metadata = std::fs::metadata(&output)
        .with_context(|| format!("Failed to inspect {}", output.display()))?;
    if !metadata.is_dir() {
        anyhow::bail!("Output path is not a directory: {}", output.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_setup_rejects_placeholder_api_key() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = temp.path().join("model.bin");
        std::fs::write(&model_path, b"model").unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: model_path.to_string_lossy().into_owned(),
            openrouter_api_key: "YOUR_KEY_HERE".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        let error = validate_setup(&cfg).unwrap_err();

        assert!(error.to_string().contains("OpenRouter API key is required"));
    }

    #[cfg(feature = "whisper-cli")]
    #[test]
    fn validate_setup_requires_whisper_bin_for_cli_backend() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = temp.path().join("model.bin");
        std::fs::write(&model_path, b"model").unwrap();
        let cfg = Config {
            whisper_bin: None,
            whisper_model: model_path.to_string_lossy().into_owned(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        let error = validate_setup(&cfg).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("whisper_bin is required when the whisper-cli backend is enabled")
        );
    }

    #[test]
    fn validate_setup_rejects_missing_whisper_model() {
        let temp = tempfile::tempdir().unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: temp
                .path()
                .join("missing.bin")
                .to_string_lossy()
                .into_owned(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        let error = validate_setup(&cfg).unwrap_err();

        assert!(error.to_string().contains("Whisper model does not exist"));
    }

    #[test]
    fn setup_accepts_managed_model_filename_next_to_config() {
        use super::super::managed_model::resolve_managed_whisper_model_config;

        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("ggml-base.en.bin"), b"model").unwrap();
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: "ggml-base.en.bin".into(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(temp.path().join("out").to_string_lossy().into_owned()),
        };

        let cfg = resolve_managed_whisper_model_config(cfg, temp.path());

        validate_setup(&cfg).unwrap();
        assert_eq!(
            cfg.whisper_model,
            temp.path().join("ggml-base.en.bin").to_string_lossy()
        );
    }
}
