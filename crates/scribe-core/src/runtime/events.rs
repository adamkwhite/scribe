use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionProcessingEvent {
    SelectedSession {
        session_dir: PathBuf,
    },
    TranscriptionStarted {
        session_dir: PathBuf,
        wav_path: PathBuf,
    },
    TranscriptionCompleted {
        session_dir: PathBuf,
        transcript_chars: usize,
    },
    TranscriptSaved {
        transcript_path: PathBuf,
    },
    NotesGenerationStarted {
        session_dir: PathBuf,
    },
    NotesGenerationCompleted {
        session_dir: PathBuf,
    },
    NotesSaved {
        notes_path: PathBuf,
    },
}

impl SessionProcessingEvent {
    pub fn message(&self) -> String {
        match self {
            Self::SelectedSession { session_dir } => {
                format!("Found: {}", session_dir.display())
            }
            Self::TranscriptionStarted { .. } => "Transcribing...".to_string(),
            Self::TranscriptionCompleted {
                transcript_chars, ..
            } => {
                format!("Transcription complete ({transcript_chars} chars).")
            }
            Self::TranscriptSaved { transcript_path } => {
                format!("Transcript saved to: {}", transcript_path.display())
            }
            Self::NotesGenerationStarted { .. } => "Generating meeting notes...".to_string(),
            Self::NotesGenerationCompleted { .. } => "Meeting notes generated.".to_string(),
            Self::NotesSaved { notes_path } => {
                format!("Notes saved to: {}", notes_path.display())
            }
        }
    }
}

#[derive(Clone)]
pub struct SessionProcessingEventSink {
    on_event: Arc<dyn Fn(SessionProcessingEvent) + Send + Sync>,
}

impl SessionProcessingEventSink {
    pub fn custom<F>(on_event: F) -> Self
    where
        F: Fn(SessionProcessingEvent) + Send + Sync + 'static,
    {
        Self {
            on_event: Arc::new(on_event),
        }
    }

    pub fn printing() -> Self {
        Self::custom(|event| println!("{}", event.message()))
    }

    pub fn ignoring() -> Self {
        Self::custom(|_| {})
    }

    pub fn emit(&self, event: SessionProcessingEvent) {
        tracing::info!(message = %event.message(), "session processing event");
        (self.on_event)(event);
    }
}
