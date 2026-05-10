use anyhow::Result;
use std::{future::Future, path::PathBuf, pin::Pin};

pub type TranscriptionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TranscriptionOutput>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptionInput {
    pub wav_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptionOutput {
    pub transcript: String,
}

pub trait TranscriptionProvider: Send + Sync {
    fn transcribe(&self, input: TranscriptionInput) -> TranscriptionFuture<'_>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_input_and_output_support_deterministic_assertions() {
        let input = TranscriptionInput {
            wav_path: PathBuf::from("/tmp/recording.wav"),
        };
        let output = TranscriptionOutput {
            transcript: "meeting notes transcript".to_string(),
        };

        assert_eq!(
            input,
            TranscriptionInput {
                wav_path: PathBuf::from("/tmp/recording.wav")
            }
        );
        assert_eq!(
            output,
            TranscriptionOutput {
                transcript: "meeting notes transcript".to_string()
            }
        );
    }
}
