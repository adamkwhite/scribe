use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::config::Config;

use super::audio_recording_event::AudioRecordingEvent;
use super::mix_buffer::MixBuffer;
use super::recorder::{
    AudioRecorder, AudioRecordingEventSink, AudioRecordingFuture, AudioRecordingInput,
    AudioRecordingOutput, RecordingControl,
};
use super::samples::{i16_to_mono_f32, to_mono_f32};

type CpalRecordingEngineFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AudioRecordingOutput>> + Send + 'a>>;

pub struct CpalAudioRecorder {
    target_sample_rate: u32,
    engine: Arc<dyn CpalRecordingEngine>,
}

impl CpalAudioRecorder {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            target_sample_rate: cfg.sample_rate,
            engine: Arc::new(RealCpalRecordingEngine),
        }
    }

    #[cfg(test)]
    fn with_engine(target_sample_rate: u32, engine: Arc<dyn CpalRecordingEngine>) -> Self {
        Self {
            target_sample_rate,
            engine,
        }
    }
}

impl AudioRecorder for CpalAudioRecorder {
    fn record(&self, input: AudioRecordingInput) -> AudioRecordingFuture<'_> {
        let engine = self.engine.clone();
        let target_sample_rate = self.target_sample_rate;
        Box::pin(async move {
            engine
                .record(target_sample_rate, input)
                .await
                .context("Audio recording failed")
        })
    }
}

trait CpalRecordingEngine: Send + Sync {
    fn record(
        &self,
        target_sample_rate: u32,
        input: AudioRecordingInput,
    ) -> CpalRecordingEngineFuture<'_>;
}

struct RealCpalRecordingEngine;

impl CpalRecordingEngine for RealCpalRecordingEngine {
    fn record(
        &self,
        target_sample_rate: u32,
        input: AudioRecordingInput,
    ) -> CpalRecordingEngineFuture<'_> {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                run_cpal_recording(
                    input.control,
                    target_sample_rate,
                    input.session_dir,
                    input.events,
                )
            })
            .await
            .context("Audio recording task failed to join")?
        })
    }
}

/// Record system audio (loopback) + microphone, mixed into one WAV file.
/// On Windows, uses WASAPI for both streams.
/// Saves to `session_dir/recording.wav`.
fn run_cpal_recording(
    control: RecordingControl,
    _target_sample_rate: u32,
    session_dir: PathBuf,
    events: AudioRecordingEventSink,
) -> Result<AudioRecordingOutput> {
    #[cfg(target_os = "windows")]
    let host = cpal::host_from_id(cpal::HostId::Wasapi).context("WASAPI host not available")?;

    #[cfg(not(target_os = "windows"))]
    let host = cpal::default_host();

    #[cfg(target_os = "windows")]
    let loopback_device = host
        .default_output_device()
        .context("No default output device found")?;

    #[cfg(not(target_os = "windows"))]
    let loopback_device = host
        .default_input_device()
        .context("No default input device found")?;

    let mic_device = host
        .default_input_device()
        .context("No default input (mic) device found")?;

    events.emit(AudioRecordingEvent::LoopbackDevice(
        loopback_device.name().unwrap_or_default(),
    ));
    events.emit(AudioRecordingEvent::MicDevice(
        mic_device.name().unwrap_or_default(),
    ));

    let loopback_config = loopback_device
        .default_output_config()
        .or_else(|_| loopback_device.default_input_config())
        .context("Failed to get loopback audio config")?;

    let mic_config = mic_device
        .default_input_config()
        .context("Failed to get mic audio config")?;

    let output_sample_rate = loopback_config.sample_rate().0;
    events.emit(AudioRecordingEvent::AudioConfig {
        loopback_sample_rate: loopback_config.sample_rate().0,
        loopback_channels: loopback_config.channels(),
        mic_sample_rate: mic_config.sample_rate().0,
        mic_channels: mic_config.channels(),
        output_sample_rate,
    });

    let spec = WavSpec {
        channels: 1,
        sample_rate: output_sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    std::fs::create_dir_all(&session_dir)?;
    let wav_path = session_dir.join("recording.wav");

    let writer = WavWriter::create(&wav_path, spec)
        .with_context(|| format!("Failed to create {}", wav_path.display()))?;
    let writer = Arc::new(Mutex::new(Some(writer)));

    let mix = Arc::new(Mutex::new(MixBuffer::new()));

    let mix_lb = mix.clone();
    let control_lb = control.clone();
    let lb_channels = loopback_config.channels();
    let report_lb_f32 = events.clone();
    let report_lb_i16 = events.clone();

    let loopback_stream = match loopback_config.sample_format() {
        cpal::SampleFormat::F32 => loopback_device.build_input_stream(
            &loopback_config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !control_lb.is_recording() {
                    return;
                }
                let mono = to_mono_f32(data, lb_channels);
                if let Ok(mut m) = mix_lb.lock() {
                    m.loopback.extend(mono);
                }
            },
            move |err| {
                report_lb_f32.emit(AudioRecordingEvent::StreamError {
                    source: "Loopback",
                    error: err.to_string(),
                })
            },
            None,
        )?,
        cpal::SampleFormat::I16 => loopback_device.build_input_stream(
            &loopback_config.config(),
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if !control_lb.is_recording() {
                    return;
                }
                let mono = i16_to_mono_f32(data, lb_channels);
                if let Ok(mut m) = mix_lb.lock() {
                    m.loopback.extend(mono);
                }
            },
            move |err| {
                report_lb_i16.emit(AudioRecordingEvent::StreamError {
                    source: "Loopback",
                    error: err.to_string(),
                })
            },
            None,
        )?,
        format => anyhow::bail!("Unsupported loopback sample format: {format:?}"),
    };

    let mix_mic = mix.clone();
    let control_mic = control.clone();
    let mic_channels = mic_config.channels();
    let report_mic_f32 = events.clone();
    let report_mic_i16 = events.clone();

    let mic_stream = match mic_config.sample_format() {
        cpal::SampleFormat::F32 => mic_device.build_input_stream(
            &mic_config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !control_mic.is_recording() {
                    return;
                }
                let mono = to_mono_f32(data, mic_channels);
                if let Ok(mut m) = mix_mic.lock() {
                    m.mic.extend(mono);
                }
            },
            move |err| {
                report_mic_f32.emit(AudioRecordingEvent::StreamError {
                    source: "Mic",
                    error: err.to_string(),
                })
            },
            None,
        )?,
        cpal::SampleFormat::I16 => mic_device.build_input_stream(
            &mic_config.config(),
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if !control_mic.is_recording() {
                    return;
                }
                let mono = i16_to_mono_f32(data, mic_channels);
                if let Ok(mut m) = mix_mic.lock() {
                    m.mic.extend(mono);
                }
            },
            move |err| {
                report_mic_i16.emit(AudioRecordingEvent::StreamError {
                    source: "Mic",
                    error: err.to_string(),
                })
            },
            None,
        )?,
        format => anyhow::bail!("Unsupported mic sample format: {format:?}"),
    };

    loopback_stream
        .play()
        .context("Failed to start loopback stream")?;
    mic_stream.play().context("Failed to start mic stream")?;

    while control.is_recording() {
        std::thread::sleep(std::time::Duration::from_millis(50));
        write_mixed_samples(&mix, &writer);
    }

    drop(loopback_stream);
    drop(mic_stream);

    write_mixed_samples(&mix, &writer);

    if let Ok(mut guard) = writer.lock()
        && let Some(w) = guard.take()
    {
        w.finalize().context("Failed to finalize WAV")?;
    }

    events.emit(AudioRecordingEvent::SavedRecording(wav_path.clone()));
    Ok(AudioRecordingOutput { wav_path })
}

fn write_mixed_samples(
    mix: &Arc<Mutex<MixBuffer>>,
    writer: &Arc<Mutex<Option<WavWriter<std::io::BufWriter<std::fs::File>>>>>,
) {
    if let Ok(mut m) = mix.lock() {
        let mixed = m.drain_mixed();
        if !mixed.is_empty()
            && let Ok(mut guard) = writer.lock()
            && let Some(ref mut w) = *guard
        {
            for sample in &mixed {
                let s = (*sample * i16::MAX as f32) as i16;
                let _ = w.write_sample(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::sync::Mutex;

    #[test]
    fn from_config_captures_sample_rate() {
        let recorder = CpalAudioRecorder::from_config(&config_with_sample_rate(22_050));

        assert_eq!(recorder.target_sample_rate, 22_050);
    }

    #[tokio::test]
    async fn record_passes_sample_rate_session_control_and_events_to_engine() {
        let engine = Arc::new(FakeCpalRecordingEngine::success());
        let recorder = CpalAudioRecorder::with_engine(44_100, engine.clone());
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_for_sink = events.clone();
        let control = RecordingControl::new_running();

        let output = recorder
            .record(AudioRecordingInput {
                control: control.clone(),
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
                wav_path: PathBuf::from("/tmp/session/recording.wav")
            }
        );
        assert_eq!(
            engine.requests(),
            vec![FakeCpalRecordingRequest {
                target_sample_rate: 44_100,
                session_dir: PathBuf::from("/tmp/session"),
                control_was_recording: true,
            }]
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[AudioRecordingEvent::SavedRecording(PathBuf::from(
                "/tmp/session/recording.wav"
            ))]
        );
    }

    #[tokio::test]
    async fn record_wraps_engine_errors_with_actionable_context() {
        let recorder = CpalAudioRecorder::with_engine(
            16_000,
            Arc::new(FakeCpalRecordingEngine::failure("device offline")),
        );

        let error = recorder
            .record(AudioRecordingInput {
                control: RecordingControl::new_running(),
                session_dir: PathBuf::from("/tmp/session"),
                events: AudioRecordingEventSink::ignoring(),
            })
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "Audio recording failed");
        assert_eq!(error.source().unwrap().to_string(), "device offline");
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct FakeCpalRecordingRequest {
        target_sample_rate: u32,
        session_dir: PathBuf,
        control_was_recording: bool,
    }

    struct FakeCpalRecordingEngine {
        requests: Arc<Mutex<Vec<FakeCpalRecordingRequest>>>,
        result: Result<(), String>,
    }

    impl FakeCpalRecordingEngine {
        fn success() -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                result: Ok(()),
            }
        }

        fn failure(message: &str) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                result: Err(message.to_string()),
            }
        }

        fn requests(&self) -> Vec<FakeCpalRecordingRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl CpalRecordingEngine for FakeCpalRecordingEngine {
        fn record(
            &self,
            target_sample_rate: u32,
            input: AudioRecordingInput,
        ) -> CpalRecordingEngineFuture<'_> {
            self.requests
                .lock()
                .unwrap()
                .push(FakeCpalRecordingRequest {
                    target_sample_rate,
                    session_dir: input.session_dir.clone(),
                    control_was_recording: input.control.is_recording(),
                });
            let result = self.result.clone();
            Box::pin(async move {
                match result {
                    Ok(()) => {
                        let wav_path = input.session_dir.join("recording.wav");
                        input
                            .events
                            .emit(AudioRecordingEvent::SavedRecording(wav_path.clone()));
                        Ok(AudioRecordingOutput { wav_path })
                    }
                    Err(message) => Err(anyhow!(message)),
                }
            })
        }
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
