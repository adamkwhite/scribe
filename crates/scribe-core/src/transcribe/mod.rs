mod backend;
mod embedded_whisper;
mod provider;
mod provider_factory;
mod resample;
mod wav_loader;
mod whisper_cli;

pub use backend::{
    TranscriptionBackend, transcription_backend_from_config,
    transcription_backend_label_from_config,
};
pub use embedded_whisper::EmbeddedWhisperTranscriptionProvider;
pub use provider::{
    TranscriptionFuture, TranscriptionInput, TranscriptionOutput, TranscriptionProvider,
};
pub use provider_factory::transcription_provider_from_config;
pub use whisper_cli::WhisperCliTranscriptionProvider;
