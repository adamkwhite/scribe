use anyhow::{Context, Result};
use std::path::PathBuf;

use super::settings::Config;

fn default_output_dir() -> Result<PathBuf> {
    let dir = dirs::document_dir()
        .or_else(dirs::home_dir)
        .context("Could not find home directory")?
        .join("scribe");
    Ok(dir)
}

pub fn effective_output_dir(cfg: &Config) -> Result<PathBuf> {
    let dir = match cfg.output_dir.as_deref() {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => default_output_dir()?,
    };
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create output directory {}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_output_dir_uses_configured_output_dir() {
        let temp = tempfile::tempdir().unwrap();
        let configured = temp.path().join("custom-scribe");
        let cfg = Config {
            whisper_bin: Some("whisper-cli".into()),
            whisper_model: "model.bin".into(),
            openrouter_api_key: "sk-or-test".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: Some(configured.to_string_lossy().into_owned()),
        };

        let result = effective_output_dir(&cfg).unwrap();

        assert_eq!(result, configured);
        assert!(result.exists());
    }
}
