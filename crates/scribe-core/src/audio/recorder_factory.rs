use anyhow::Result;

use crate::config::Config;

use super::{AudioRecorder, CpalAudioRecorder};

pub fn audio_recorder_from_config(cfg: &Config) -> Result<Box<dyn AudioRecorder>> {
    Ok(Box::new(CpalAudioRecorder::from_config(cfg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_cpal_recorder_from_config() {
        audio_recorder_from_config(&config_with_sample_rate(16_000)).unwrap();
    }

    fn config_with_sample_rate(sample_rate: u32) -> Config {
        Config {
            whisper_bin: None,
            whisper_model: "model.bin".to_string(),
            openrouter_api_key: "key".to_string(),
            model: "notes/model".to_string(),
            sample_rate,
            output_dir: None,
        }
    }
}
