use anyhow::Result;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::audio_recording_event::AudioRecordingEvent;

pub type AudioRecordingFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AudioRecordingOutput>> + Send + 'a>>;

#[derive(Clone)]
pub struct AudioRecordingInput {
    pub control: RecordingControl,
    pub session_dir: PathBuf,
    pub events: AudioRecordingEventSink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioRecordingOutput {
    pub wav_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct RecordingControl {
    recording: Arc<AtomicBool>,
}

impl RecordingControl {
    pub fn new_running() -> Self {
        Self {
            recording: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn stop(&self) {
        self.recording.store(false, Ordering::Relaxed);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct AudioRecordingEventSink {
    on_event: Arc<dyn Fn(AudioRecordingEvent) + Send + Sync>,
}

impl AudioRecordingEventSink {
    pub fn custom<F>(on_event: F) -> Self
    where
        F: Fn(AudioRecordingEvent) + Send + Sync + 'static,
    {
        Self {
            on_event: Arc::new(on_event),
        }
    }

    pub fn printing() -> Self {
        Self::custom(|event| event.print())
    }

    pub fn ignoring() -> Self {
        Self::custom(|_| {})
    }

    pub fn emit(&self, event: AudioRecordingEvent) {
        match &event {
            AudioRecordingEvent::StreamError { .. } => {
                tracing::warn!(message = %event.message(), "audio recording event");
            }
            _ => {
                tracing::info!(message = %event.message(), "audio recording event");
            }
        }
        (self.on_event)(event);
    }
}

pub trait AudioRecorder: Send + Sync {
    fn record(&self, input: AudioRecordingInput) -> AudioRecordingFuture<'_>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn recording_control_starts_running_stops_and_shares_state_across_clones() {
        let control = RecordingControl::new_running();
        let cloned = control.clone();

        assert!(control.is_recording());
        assert!(cloned.is_recording());

        cloned.stop();

        assert!(!control.is_recording());
        assert!(!cloned.is_recording());
    }

    #[test]
    fn event_sink_forwards_custom_events() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_sink = events.clone();
        let sink = AudioRecordingEventSink::custom(move |event| {
            events_for_sink.lock().unwrap().push(event);
        });

        sink.emit(AudioRecordingEvent::MicDevice("Studio Mic".to_string()));

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[AudioRecordingEvent::MicDevice("Studio Mic".to_string())]
        );
    }

    #[tokio::test]
    async fn fake_recorder_can_assert_typed_input_and_return_output() {
        struct FakeRecorder;

        impl AudioRecorder for FakeRecorder {
            fn record(&self, input: AudioRecordingInput) -> AudioRecordingFuture<'_> {
                Box::pin(async move {
                    assert!(input.control.is_recording());
                    assert_eq!(input.session_dir, PathBuf::from("/tmp/session"));
                    input.events.emit(AudioRecordingEvent::SavedRecording(
                        input.session_dir.join("recording.wav"),
                    ));
                    Ok(AudioRecordingOutput {
                        wav_path: PathBuf::from("/tmp/session/recording.wav"),
                    })
                })
            }
        }

        let recorder = FakeRecorder;
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_sink = events.clone();

        let output = recorder
            .record(AudioRecordingInput {
                control: RecordingControl::new_running(),
                session_dir: PathBuf::from("/tmp/session"),
                events: AudioRecordingEventSink::custom(move |event| {
                    events_for_sink.lock().unwrap().push(event);
                }),
            })
            .await
            .unwrap();

        assert_eq!(
            output,
            AudioRecordingOutput {
                wav_path: PathBuf::from("/tmp/session/recording.wav"),
            }
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[AudioRecordingEvent::SavedRecording(PathBuf::from(
                "/tmp/session/recording.wav"
            ))]
        );
    }
}
