use super::{
    ActiveRecording, FileSystemSessionArtifactStore, LocalRuntimeClock, RuntimeClock,
    SessionArtifactStore, SessionProcessingEvent, SessionProcessingEventSink,
};
use crate::audio::{
    self, AudioRecorder, AudioRecordingEventSink, AudioRecordingInput, AudioSessionStore,
    CreateAudioSessionContext, CreateAudioSessionInput,
};
use crate::config::Config;
use crate::notes::{
    self, NoteGenerationContext, NoteGenerationInput, NotesGenerator, NotesSystemPrompt,
};
use crate::transcribe::{self, TranscriptionInput, TranscriptionProvider};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct ScribeRuntime {
    session_store: Arc<dyn AudioSessionStore>,
    audio_recorder: Arc<dyn AudioRecorder>,
    transcription_provider: Arc<dyn TranscriptionProvider>,
    notes_generator: Arc<dyn NotesGenerator>,
    clock: Arc<dyn RuntimeClock>,
    artifacts: Arc<dyn SessionArtifactStore>,
}

pub struct ScribeRuntimeParts {
    pub session_store: Arc<dyn AudioSessionStore>,
    pub audio_recorder: Arc<dyn AudioRecorder>,
    pub transcription_provider: Arc<dyn TranscriptionProvider>,
    pub notes_generator: Arc<dyn NotesGenerator>,
    pub clock: Arc<dyn RuntimeClock>,
    pub artifacts: Arc<dyn SessionArtifactStore>,
}

#[derive(Clone)]
pub struct StartRecordingInput {
    pub name: Option<String>,
    pub context: CreateAudioSessionContext,
    pub events: AudioRecordingEventSink,
}

#[derive(Clone)]
pub struct ProcessSessionInput {
    pub session_dir: PathBuf,
    pub context: NoteGenerationContext,
    pub events: SessionProcessingEventSink,
}

#[derive(Clone)]
pub struct ProcessLatestRecordingInput {
    pub context: NoteGenerationContext,
    pub events: SessionProcessingEventSink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSessionOutput {
    pub session_dir: PathBuf,
    pub transcript_path: PathBuf,
    pub notes_path: PathBuf,
    pub transcript_chars: usize,
}

impl ScribeRuntime {
    pub fn from_config(cfg: &Config) -> Result<Self> {
        Ok(Self::from_parts(ScribeRuntimeParts {
            session_store: Arc::new(audio::FileSystemAudioSessionStore::from_config(cfg)?),
            audio_recorder: Arc::new(audio::CpalAudioRecorder::from_config(cfg)),
            transcription_provider: transcribe::transcription_provider_from_config(cfg)?.into(),
            notes_generator: Arc::new(notes::OpenRouterNotesGenerator::from_config(cfg)),
            clock: Arc::new(LocalRuntimeClock),
            artifacts: Arc::new(FileSystemSessionArtifactStore),
        }))
    }

    pub fn from_parts(parts: ScribeRuntimeParts) -> Self {
        Self {
            session_store: parts.session_store,
            audio_recorder: parts.audio_recorder,
            transcription_provider: parts.transcription_provider,
            notes_generator: parts.notes_generator,
            clock: parts.clock,
            artifacts: parts.artifacts,
        }
    }

    pub fn session_store(&self) -> Arc<dyn AudioSessionStore> {
        self.session_store.clone()
    }

    pub fn recording_context_now(&self) -> CreateAudioSessionContext {
        CreateAudioSessionContext {
            timestamp: self.clock.recording_timestamp(),
        }
    }

    pub fn note_generation_context_now(
        &self,
        system_prompt: NotesSystemPrompt,
    ) -> NoteGenerationContext {
        NoteGenerationContext {
            note_date: self.clock.note_date(),
            system_prompt,
        }
    }

    pub fn start_recording(&self, input: StartRecordingInput) -> Result<ActiveRecording> {
        let session_dir = self
            .session_store
            .create_session(CreateAudioSessionInput {
                name: input.name,
                context: input.context,
            })?
            .session_dir;
        let control = audio::RecordingControl::new_running();
        let recording_input = AudioRecordingInput {
            control: control.clone(),
            session_dir: session_dir.clone(),
            events: input.events,
        };
        let audio_recorder = self.audio_recorder.clone();
        let task = tokio::spawn(async move { audio_recorder.record(recording_input).await });

        Ok(ActiveRecording::new(session_dir, control, task))
    }

    pub async fn process_latest_recording(
        &self,
        input: ProcessLatestRecordingInput,
    ) -> Result<ProcessSessionOutput> {
        let session_dir = self.session_store.latest_recording_session()?.session_dir;
        self.process_session(ProcessSessionInput {
            session_dir,
            context: input.context,
            events: input.events,
        })
        .await
    }

    pub async fn process_session(
        &self,
        input: ProcessSessionInput,
    ) -> Result<ProcessSessionOutput> {
        let ProcessSessionInput {
            session_dir,
            context,
            events,
        } = input;
        let wav_path = self.artifacts.recording_wav_path(&session_dir);

        tracing::info!(session_dir = %session_dir.display(), "processing session");
        events.emit(SessionProcessingEvent::SelectedSession {
            session_dir: session_dir.clone(),
        });
        tracing::info!(wav_path = %wav_path.display(), "transcription starting");
        events.emit(SessionProcessingEvent::TranscriptionStarted {
            session_dir: session_dir.clone(),
            wav_path: wav_path.clone(),
        });
        let transcript = self
            .transcription_provider
            .transcribe(TranscriptionInput { wav_path })
            .await?
            .transcript;
        let transcript_chars = transcript.len();
        tracing::info!(
            session_dir = %session_dir.display(),
            transcript_chars,
            "transcription completed"
        );
        events.emit(SessionProcessingEvent::TranscriptionCompleted {
            session_dir: session_dir.clone(),
            transcript_chars,
        });

        let transcript_path = self.artifacts.write_transcript(&session_dir, &transcript)?;
        tracing::info!(transcript_path = %transcript_path.display(), "transcript saved");
        events.emit(SessionProcessingEvent::TranscriptSaved {
            transcript_path: transcript_path.clone(),
        });

        tracing::info!(session_dir = %session_dir.display(), "notes generation starting");
        events.emit(SessionProcessingEvent::NotesGenerationStarted {
            session_dir: session_dir.clone(),
        });
        let notes_text = self
            .notes_generator
            .generate(NoteGenerationInput {
                transcript: transcript.clone(),
                context,
            })
            .await?
            .markdown;
        tracing::info!(session_dir = %session_dir.display(), "notes generation completed");
        events.emit(SessionProcessingEvent::NotesGenerationCompleted {
            session_dir: session_dir.clone(),
        });

        let full_notes = format!("{notes_text}\n\n---\n\n## Raw Transcript\n\n{transcript}\n");
        let notes_path = self.artifacts.write_notes(&session_dir, &full_notes)?;
        tracing::info!(notes_path = %notes_path.display(), "notes saved");
        events.emit(SessionProcessingEvent::NotesSaved {
            notes_path: notes_path.clone(),
        });

        Ok(ProcessSessionOutput {
            session_dir,
            transcript_path,
            notes_path,
            transcript_chars,
        })
    }
}
