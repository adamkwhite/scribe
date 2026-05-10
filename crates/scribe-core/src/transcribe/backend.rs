use crate::config::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptionBackend {
    EmbeddedWhisper,
    WhisperCli,
}

pub fn transcription_backend_from_config(cfg: &Config) -> TranscriptionBackend {
    if cfg
        .whisper_bin
        .as_deref()
        .map(str::trim)
        .filter(|bin| !bin.is_empty())
        .is_some()
    {
        TranscriptionBackend::WhisperCli
    } else {
        TranscriptionBackend::EmbeddedWhisper
    }
}

pub fn transcription_backend_label_from_config(cfg: &Config) -> &'static str {
    match transcription_backend_from_config(cfg) {
        TranscriptionBackend::EmbeddedWhisper => "embedded whisper.cpp",
        TranscriptionBackend::WhisperCli => "whisper.cpp CLI",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_selection_uses_embedded_when_whisper_bin_is_missing() {
        let cfg = config_with_bin(None);

        assert_eq!(
            transcription_backend_from_config(&cfg),
            TranscriptionBackend::EmbeddedWhisper
        );
        assert_eq!(
            transcription_backend_label_from_config(&cfg),
            "embedded whisper.cpp"
        );
    }

    #[test]
    fn backend_selection_uses_embedded_when_whisper_bin_is_blank() {
        let cfg = config_with_bin(Some("   ".to_string()));

        assert_eq!(
            transcription_backend_from_config(&cfg),
            TranscriptionBackend::EmbeddedWhisper
        );
        assert_eq!(
            transcription_backend_label_from_config(&cfg),
            "embedded whisper.cpp"
        );
    }

    #[test]
    fn backend_selection_uses_cli_when_whisper_bin_is_configured() {
        let cfg = config_with_bin(Some("whisper-cli".to_string()));

        assert_eq!(
            transcription_backend_from_config(&cfg),
            TranscriptionBackend::WhisperCli
        );
        assert_eq!(
            transcription_backend_label_from_config(&cfg),
            "whisper.cpp CLI"
        );
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
