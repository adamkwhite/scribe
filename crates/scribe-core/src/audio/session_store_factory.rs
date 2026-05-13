use anyhow::Result;

use crate::config::Config;

use super::{AudioSessionStore, FileSystemAudioSessionStore};

pub fn audio_session_store_from_config(cfg: &Config) -> Result<Box<dyn AudioSessionStore>> {
    Ok(Box::new(FileSystemAudioSessionStore::from_config(cfg)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_filesystem_session_store_from_config() {
        let temp = tempfile::tempdir().unwrap();

        audio_session_store_from_config(&config_with_output_dir(temp.path().to_string_lossy()))
            .unwrap();
    }

    fn config_with_output_dir(output_dir: impl ToString) -> Config {
        Config {
            whisper_bin: None,
            whisper_model: "model.bin".to_string(),
            openrouter_api_key: "key".to_string(),
            model: "notes/model".to_string(),
            sample_rate: 16_000,
            output_dir: Some(output_dir.to_string()),
        }
    }
}
