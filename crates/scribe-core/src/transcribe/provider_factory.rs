use anyhow::Result;

use crate::config::Config;

use super::{
    EmbeddedWhisperTranscriptionProvider, TranscriptionBackend, TranscriptionProvider,
    WhisperCliTranscriptionProvider, transcription_backend_from_config,
};

pub fn transcription_provider_from_config(cfg: &Config) -> Result<Box<dyn TranscriptionProvider>> {
    match transcription_backend_from_config(cfg) {
        TranscriptionBackend::EmbeddedWhisper => Ok(Box::new(
            EmbeddedWhisperTranscriptionProvider::from_config(cfg),
        )),
        TranscriptionBackend::WhisperCli => {
            Ok(Box::new(WhisperCliTranscriptionProvider::from_config(cfg)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_embedded_provider_when_whisper_bin_is_missing() {
        transcription_provider_from_config(&config_with_bin(None)).unwrap();
    }

    #[test]
    fn creates_embedded_provider_when_whisper_bin_is_blank() {
        transcription_provider_from_config(&config_with_bin(Some("   ".to_string()))).unwrap();
    }

    #[test]
    fn creates_cli_provider_when_whisper_bin_is_configured() {
        transcription_provider_from_config(&config_with_bin(Some("whisper-cli".to_string())))
            .unwrap();
    }

    fn config_with_bin(whisper_bin: Option<String>) -> Config {
        Config {
            whisper_bin,
            whisper_model: "model.bin".to_string(),
            openrouter_api_key: "key".to_string(),
            model: "notes/model".to_string(),
            sample_rate: 16000,
            output_dir: None,
        }
    }
}
