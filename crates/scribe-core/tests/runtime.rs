use anyhow::{Result, anyhow};
use scribe_core::audio::{
    AudioRecorder, AudioRecordingEvent, AudioRecordingEventSink, AudioRecordingFuture,
    AudioRecordingInput, AudioRecordingOutput, AudioSessionStore, AudioSessionTimestamp,
    CreateAudioSessionContext, CreateAudioSessionInput, CreateAudioSessionOutput,
    LatestAudioSessionOutput, ListAudioSessionsOutput, RecordingControl,
};
use scribe_core::notes::{
    NoteGenerationContext, NoteGenerationFuture, NoteGenerationInput, NoteGenerationOutput,
    NotesGenerator, NotesSystemPrompt,
};
use scribe_core::runtime::{
    ProcessLatestRecordingInput, ProcessSessionInput, ProcessSessionOutput, RuntimeClock,
    ScribeRuntime, ScribeRuntimeParts, SessionArtifactStore, SessionProcessingEvent,
    SessionProcessingEventSink, StartRecordingInput,
};
use scribe_core::transcribe::{
    TranscriptionFuture, TranscriptionInput, TranscriptionOutput, TranscriptionProvider,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[test]
fn runtime_from_parts_uses_injected_clock_without_config() {
    let runtime = runtime_with_fakes(FakeRuntimeOptions::default());

    assert_eq!(
        runtime.recording_context_now(),
        CreateAudioSessionContext {
            timestamp: AudioSessionTimestamp::fixed("2026-05-11_090000"),
        }
    );
    assert_eq!(
        runtime.note_generation_context_now(NotesSystemPrompt::Default),
        NoteGenerationContext {
            note_date: "May 11, 2026".to_string(),
            system_prompt: NotesSystemPrompt::Default,
        }
    );
}

#[tokio::test]
async fn start_recording_creates_session_and_passes_recorder_input() {
    let options = FakeRuntimeOptions::default();
    let session_store = options.session_store.clone();
    let recorder = options.recorder.clone();
    let runtime = runtime_with_fakes(options);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_sink = events.clone();

    let recording = runtime
        .start_recording(StartRecordingInput {
            name: Some("Planning".to_string()),
            context: CreateAudioSessionContext {
                timestamp: AudioSessionTimestamp::fixed("2026-05-11_090000"),
            },
            events: AudioRecordingEventSink::custom(move |event| {
                events_for_sink.lock().unwrap().push(event);
            }),
        })
        .unwrap();
    let output = recording.wait().await.unwrap();

    assert_eq!(
        session_store.created_inputs(),
        vec![CreateAudioSessionInput {
            name: Some("Planning".to_string()),
            context: CreateAudioSessionContext {
                timestamp: AudioSessionTimestamp::fixed("2026-05-11_090000"),
            },
        }]
    );
    assert_eq!(
        recorder.recorded_session_dirs(),
        vec![PathBuf::from("/sessions/2026-05-11_090000 — Planning")]
    );
    assert_eq!(
        output,
        AudioRecordingOutput {
            wav_path: PathBuf::from("/sessions/2026-05-11_090000 — Planning/recording.wav"),
        }
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        &[AudioRecordingEvent::SavedRecording(PathBuf::from(
            "/sessions/2026-05-11_090000 — Planning/recording.wav"
        ))]
    );
}

#[tokio::test]
async fn active_recording_stop_shares_control_with_recorder_task() {
    let options = FakeRuntimeOptions::default();
    let recorder = options.recorder.clone();
    recorder.block_until_released();
    let runtime = runtime_with_fakes(options);

    let recording = runtime
        .start_recording(StartRecordingInput {
            name: None,
            context: runtime.recording_context_now(),
            events: AudioRecordingEventSink::ignoring(),
        })
        .unwrap();
    recorder.wait_until_started().await;

    assert!(recording.is_recording());
    recording.stop();
    assert!(!recording.is_recording());
    assert!(!recorder.last_control().unwrap().is_recording());

    recorder.release();
    recording.wait().await.unwrap();
}

#[tokio::test]
async fn process_session_transcribes_generates_notes_writes_artifacts_and_emits_events() {
    let options = FakeRuntimeOptions::default();
    let transcription = options.transcription.clone();
    let notes = options.notes.clone();
    let artifacts = options.artifacts.clone();
    let runtime = runtime_with_fakes(options);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_sink = events.clone();
    let session_dir = PathBuf::from("/sessions/session-1");
    let context = NoteGenerationContext {
        note_date: "May 11, 2026".to_string(),
        system_prompt: NotesSystemPrompt::Custom("Use this prompt exactly.".to_string()),
    };

    let output = runtime
        .process_session(ProcessSessionInput {
            session_dir: session_dir.clone(),
            context: context.clone(),
            events: SessionProcessingEventSink::custom(move |event| {
                events_for_sink.lock().unwrap().push(event);
            }),
        })
        .await
        .unwrap();

    assert_eq!(
        transcription.inputs(),
        vec![TranscriptionInput {
            wav_path: session_dir.join("recording.wav"),
        }]
    );
    assert_eq!(
        notes.inputs(),
        vec![NoteGenerationInput {
            transcript: "transcript text".to_string(),
            context,
        }]
    );
    assert_eq!(
        artifacts.writes(),
        vec![
            ArtifactWrite::Transcript {
                path: session_dir.join("transcript.txt"),
                contents: "transcript text".to_string(),
            },
            ArtifactWrite::Notes {
                path: session_dir.join("notes.md"),
                contents: "notes text\n\n---\n\n## Raw Transcript\n\ntranscript text\n".to_string(),
            },
        ]
    );
    assert_eq!(
        output,
        ProcessSessionOutput {
            session_dir: session_dir.clone(),
            transcript_path: session_dir.join("transcript.txt"),
            notes_path: session_dir.join("notes.md"),
            transcript_chars: 15,
        }
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        &[
            SessionProcessingEvent::SelectedSession {
                session_dir: session_dir.clone(),
            },
            SessionProcessingEvent::TranscriptionStarted {
                session_dir: session_dir.clone(),
                wav_path: session_dir.join("recording.wav"),
            },
            SessionProcessingEvent::TranscriptionCompleted {
                session_dir: session_dir.clone(),
                transcript_chars: 15,
            },
            SessionProcessingEvent::TranscriptSaved {
                transcript_path: session_dir.join("transcript.txt"),
            },
            SessionProcessingEvent::NotesGenerationStarted {
                session_dir: session_dir.clone(),
            },
            SessionProcessingEvent::NotesGenerationCompleted {
                session_dir: session_dir.clone(),
            },
            SessionProcessingEvent::NotesSaved {
                notes_path: session_dir.join("notes.md"),
            },
        ]
    );
}

#[tokio::test]
async fn process_latest_recording_uses_runtime_session_store() {
    let options = FakeRuntimeOptions::default();
    let session_store = options.session_store.clone();
    let runtime = runtime_with_fakes(options);

    let output = runtime
        .process_latest_recording(ProcessLatestRecordingInput {
            context: runtime.note_generation_context_now(NotesSystemPrompt::Default),
            events: SessionProcessingEventSink::ignoring(),
        })
        .await
        .unwrap();

    assert_eq!(session_store.latest_calls(), 1);
    assert_eq!(output.session_dir, PathBuf::from("/sessions/latest"));
}

#[tokio::test]
async fn process_session_transcription_failure_stops_before_writing_artifacts() {
    let options = FakeRuntimeOptions {
        transcription_error: Some("transcription failed".to_string()),
        ..FakeRuntimeOptions::default()
    };
    let artifacts = options.artifacts.clone();
    let runtime = runtime_with_fakes(options);

    let error = runtime
        .process_session(ProcessSessionInput {
            session_dir: PathBuf::from("/sessions/session-1"),
            context: runtime.note_generation_context_now(NotesSystemPrompt::Default),
            events: SessionProcessingEventSink::ignoring(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "transcription failed");
    assert!(artifacts.writes().is_empty());
}

#[derive(Clone)]
struct FakeRuntimeOptions {
    clock: Arc<FakeClock>,
    session_store: Arc<FakeSessionStore>,
    recorder: Arc<FakeRecorder>,
    transcription: Arc<FakeTranscriptionProvider>,
    notes: Arc<FakeNotesGenerator>,
    artifacts: Arc<FakeArtifactStore>,
    transcription_error: Option<String>,
}

impl Default for FakeRuntimeOptions {
    fn default() -> Self {
        Self {
            clock: Arc::new(FakeClock),
            session_store: Arc::new(FakeSessionStore::default()),
            recorder: Arc::new(FakeRecorder::default()),
            transcription: Arc::new(FakeTranscriptionProvider::default()),
            notes: Arc::new(FakeNotesGenerator::default()),
            artifacts: Arc::new(FakeArtifactStore::default()),
            transcription_error: None,
        }
    }
}

fn runtime_with_fakes(options: FakeRuntimeOptions) -> ScribeRuntime {
    if let Some(error) = options.transcription_error.clone() {
        options.transcription.set_error(error);
    }

    ScribeRuntime::from_parts(ScribeRuntimeParts {
        session_store: options.session_store,
        audio_recorder: options.recorder,
        transcription_provider: options.transcription,
        notes_generator: options.notes,
        clock: options.clock,
        artifacts: options.artifacts,
    })
}

struct FakeClock;

impl RuntimeClock for FakeClock {
    fn recording_timestamp(&self) -> AudioSessionTimestamp {
        AudioSessionTimestamp::fixed("2026-05-11_090000")
    }

    fn note_date(&self) -> String {
        "May 11, 2026".to_string()
    }
}

#[derive(Default)]
struct FakeSessionStore {
    created: Mutex<Vec<CreateAudioSessionInput>>,
    latest_calls: AtomicUsize,
}

impl FakeSessionStore {
    fn created_inputs(&self) -> Vec<CreateAudioSessionInput> {
        self.created.lock().unwrap().clone()
    }

    fn latest_calls(&self) -> usize {
        self.latest_calls.load(Ordering::SeqCst)
    }
}

impl AudioSessionStore for FakeSessionStore {
    fn create_session(&self, input: CreateAudioSessionInput) -> Result<CreateAudioSessionOutput> {
        self.created.lock().unwrap().push(input.clone());
        let timestamp = input.context.timestamp.as_str();
        let name = input
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map_or_else(String::new, |name| format!(" — {name}"));
        Ok(CreateAudioSessionOutput {
            session_dir: PathBuf::from(format!("/sessions/{timestamp}{name}")),
        })
    }

    fn list_sessions(&self) -> Result<ListAudioSessionsOutput> {
        Ok(ListAudioSessionsOutput {
            sessions: Vec::new(),
        })
    }

    fn latest_recording_session(&self) -> Result<LatestAudioSessionOutput> {
        self.latest_calls.fetch_add(1, Ordering::SeqCst);
        Ok(LatestAudioSessionOutput {
            session_dir: PathBuf::from("/sessions/latest"),
        })
    }
}

#[derive(Default)]
struct FakeRecorder {
    session_dirs: Mutex<Vec<PathBuf>>,
    last_control: Mutex<Option<RecordingControl>>,
    started: Notify,
    release: Notify,
    block: Mutex<bool>,
}

impl FakeRecorder {
    fn recorded_session_dirs(&self) -> Vec<PathBuf> {
        self.session_dirs.lock().unwrap().clone()
    }

    fn last_control(&self) -> Option<RecordingControl> {
        self.last_control.lock().unwrap().clone()
    }

    fn block_until_released(&self) {
        *self.block.lock().unwrap() = true;
    }

    async fn wait_until_started(&self) {
        self.started.notified().await;
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

impl AudioRecorder for FakeRecorder {
    fn record(&self, input: AudioRecordingInput) -> AudioRecordingFuture<'_> {
        Box::pin(async move {
            self.session_dirs
                .lock()
                .unwrap()
                .push(input.session_dir.clone());
            *self.last_control.lock().unwrap() = Some(input.control.clone());
            self.started.notify_one();
            if *self.block.lock().unwrap() {
                self.release.notified().await;
            }
            let wav_path = input.session_dir.join("recording.wav");
            input
                .events
                .emit(AudioRecordingEvent::SavedRecording(wav_path.clone()));
            Ok(AudioRecordingOutput { wav_path })
        })
    }
}

#[derive(Default)]
struct FakeTranscriptionProvider {
    inputs: Mutex<Vec<TranscriptionInput>>,
    error: Mutex<Option<String>>,
}

impl FakeTranscriptionProvider {
    fn inputs(&self) -> Vec<TranscriptionInput> {
        self.inputs.lock().unwrap().clone()
    }

    fn set_error(&self, error: String) {
        *self.error.lock().unwrap() = Some(error);
    }
}

impl TranscriptionProvider for FakeTranscriptionProvider {
    fn transcribe(&self, input: TranscriptionInput) -> TranscriptionFuture<'_> {
        Box::pin(async move {
            self.inputs.lock().unwrap().push(input);
            if let Some(error) = self.error.lock().unwrap().clone() {
                return Err(anyhow!(error));
            }
            Ok(TranscriptionOutput {
                transcript: "transcript text".to_string(),
            })
        })
    }
}

#[derive(Default)]
struct FakeNotesGenerator {
    inputs: Mutex<Vec<NoteGenerationInput>>,
}

impl FakeNotesGenerator {
    fn inputs(&self) -> Vec<NoteGenerationInput> {
        self.inputs.lock().unwrap().clone()
    }
}

impl NotesGenerator for FakeNotesGenerator {
    fn generate(&self, input: NoteGenerationInput) -> NoteGenerationFuture<'_> {
        Box::pin(async move {
            self.inputs.lock().unwrap().push(input);
            Ok(NoteGenerationOutput {
                markdown: "notes text".to_string(),
            })
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ArtifactWrite {
    Transcript { path: PathBuf, contents: String },
    Notes { path: PathBuf, contents: String },
}

#[derive(Default)]
struct FakeArtifactStore {
    writes: Mutex<Vec<ArtifactWrite>>,
}

impl FakeArtifactStore {
    fn writes(&self) -> Vec<ArtifactWrite> {
        self.writes.lock().unwrap().clone()
    }
}

impl SessionArtifactStore for FakeArtifactStore {
    fn recording_wav_path(&self, session_dir: &Path) -> PathBuf {
        session_dir.join("recording.wav")
    }

    fn write_transcript(&self, session_dir: &Path, transcript: &str) -> Result<PathBuf> {
        let path = session_dir.join("transcript.txt");
        self.writes.lock().unwrap().push(ArtifactWrite::Transcript {
            path: path.clone(),
            contents: transcript.to_string(),
        });
        Ok(path)
    }

    fn write_notes(&self, session_dir: &Path, markdown: &str) -> Result<PathBuf> {
        let path = session_dir.join("notes.md");
        self.writes.lock().unwrap().push(ArtifactWrite::Notes {
            path: path.clone(),
            contents: markdown.to_string(),
        });
        Ok(path)
    }
}
