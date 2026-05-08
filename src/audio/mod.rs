use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::config;

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub enum SessionStatus {
    Empty,
    RecordingOnly,
    TranscriptReady,
    NotesReady,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub struct SessionEntry {
    pub path: PathBuf,
    pub name: String,
    pub status: SessionStatus,
    pub modified: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioRecordingEvent {
    LoopbackDevice(String),
    MicDevice(String),
    AudioConfig {
        loopback_sample_rate: u32,
        loopback_channels: u16,
        mic_sample_rate: u32,
        mic_channels: u16,
        output_sample_rate: u32,
    },
    StreamError {
        source: &'static str,
        error: String,
    },
    SavedRecording(PathBuf),
}

impl AudioRecordingEvent {
    pub fn message(&self) -> String {
        match self {
            Self::LoopbackDevice(name) => format!("Loopback: {name}"),
            Self::MicDevice(name) => format!("Mic: {name}"),
            Self::AudioConfig {
                loopback_sample_rate,
                loopback_channels,
                mic_sample_rate,
                mic_channels,
                output_sample_rate,
            } => format!(
                "Loopback: {loopback_sample_rate}Hz {loopback_channels}ch, Mic: {mic_sample_rate}Hz {mic_channels}ch, Output: {output_sample_rate}Hz mono"
            ),
            Self::StreamError { source, error } => format!("{source} stream error: {error}"),
            Self::SavedRecording(path) => format!("Saved recording to: {}", path.display()),
        }
    }

    fn print(&self) {
        match self {
            Self::StreamError { .. } => eprintln!("{}", self.message()),
            _ => println!("{}", self.message()),
        }
    }
}

/// Shared buffer for mixing two audio streams.
/// Each stream pushes mono f32 samples; the writer drains and mixes them.
struct MixBuffer {
    loopback: VecDeque<f32>,
    mic: VecDeque<f32>,
}

impl MixBuffer {
    fn new() -> Self {
        Self {
            loopback: VecDeque::new(),
            mic: VecDeque::new(),
        }
    }

    /// Drain all available samples, mixing both streams.
    /// If one stream has no data, the other still produces output (mixed with silence).
    fn drain_mixed(&mut self) -> Vec<f32> {
        let count = self.loopback.len().max(self.mic.len());
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let l = self.loopback.pop_front().unwrap_or(0.0);
            let m = self.mic.pop_front().unwrap_or(0.0);
            // Mix with slight mic boost (mic is usually quieter than system audio)
            out.push((l + m * 1.5).clamp(-1.0, 1.0));
        }
        out
    }
}

/// Convert a multi-channel f32 buffer to mono by averaging channels.
fn to_mono_f32(data: &[f32], channels: u16) -> Vec<f32> {
    if channels == 1 {
        return data.to_vec();
    }
    data.chunks(channels as usize)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Convert a multi-channel i16 buffer to mono f32.
fn i16_to_mono_f32(data: &[i16], channels: u16) -> Vec<f32> {
    let as_f32: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    to_mono_f32(&as_f32, channels)
}

/// Record system audio (loopback) + microphone, mixed into one WAV file.
/// On Windows, uses WASAPI for both streams.
/// Saves to `session_dir/recording.wav`.
pub fn record_loopback(
    recording: Arc<AtomicBool>,
    target_sample_rate: u32,
    session_dir: PathBuf,
) -> Result<()> {
    record_loopback_with_events(recording, target_sample_rate, session_dir, |event| {
        event.print();
    })
}

pub fn record_loopback_with_events<F>(
    recording: Arc<AtomicBool>,
    _target_sample_rate: u32,
    session_dir: PathBuf,
    on_event: F,
) -> Result<()>
where
    F: Fn(AudioRecordingEvent) + Send + Sync + 'static,
{
    let on_event = Arc::new(on_event);

    #[cfg(target_os = "windows")]
    let host = cpal::host_from_id(cpal::HostId::Wasapi).context("WASAPI host not available")?;

    #[cfg(not(target_os = "windows"))]
    let host = cpal::default_host();

    // Loopback device (system audio — other person's voice)
    #[cfg(target_os = "windows")]
    let loopback_device = host
        .default_output_device()
        .context("No default output device found")?;

    #[cfg(not(target_os = "windows"))]
    let loopback_device = host
        .default_input_device()
        .context("No default input device found")?;

    // Microphone device (your voice)
    let mic_device = host
        .default_input_device()
        .context("No default input (mic) device found")?;

    on_event(AudioRecordingEvent::LoopbackDevice(
        loopback_device.name().unwrap_or_default(),
    ));
    on_event(AudioRecordingEvent::MicDevice(
        mic_device.name().unwrap_or_default(),
    ));

    let loopback_config = loopback_device
        .default_output_config()
        .or_else(|_| loopback_device.default_input_config())
        .context("Failed to get loopback audio config")?;

    let mic_config = mic_device
        .default_input_config()
        .context("Failed to get mic audio config")?;

    // Use the loopback sample rate for the output WAV (mic will be at its native rate,
    // but since both are typically 48kHz or 44.1kHz on Windows this usually matches)
    let output_sample_rate = loopback_config.sample_rate().0;
    on_event(AudioRecordingEvent::AudioConfig {
        loopback_sample_rate: loopback_config.sample_rate().0,
        loopback_channels: loopback_config.channels(),
        mic_sample_rate: mic_config.sample_rate().0,
        mic_channels: mic_config.channels(),
        output_sample_rate,
    });

    let spec = WavSpec {
        channels: 1, // mono mix
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

    // --- Loopback stream ---
    let mix_lb = mix.clone();
    let rec_lb = recording.clone();
    let lb_channels = loopback_config.channels();
    let report_lb_f32 = on_event.clone();
    let report_lb_i16 = on_event.clone();

    let loopback_stream = match loopback_config.sample_format() {
        cpal::SampleFormat::F32 => loopback_device.build_input_stream(
            &loopback_config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !rec_lb.load(Ordering::Relaxed) {
                    return;
                }
                let mono = to_mono_f32(data, lb_channels);
                if let Ok(mut m) = mix_lb.lock() {
                    m.loopback.extend(mono);
                }
            },
            move |err| {
                report_lb_f32(AudioRecordingEvent::StreamError {
                    source: "Loopback",
                    error: err.to_string(),
                })
            },
            None,
        )?,
        cpal::SampleFormat::I16 => loopback_device.build_input_stream(
            &loopback_config.config(),
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if !rec_lb.load(Ordering::Relaxed) {
                    return;
                }
                let mono = i16_to_mono_f32(data, lb_channels);
                if let Ok(mut m) = mix_lb.lock() {
                    m.loopback.extend(mono);
                }
            },
            move |err| {
                report_lb_i16(AudioRecordingEvent::StreamError {
                    source: "Loopback",
                    error: err.to_string(),
                })
            },
            None,
        )?,
        format => anyhow::bail!("Unsupported loopback sample format: {format:?}"),
    };

    // --- Mic stream ---
    let mix_mic = mix.clone();
    let rec_mic = recording.clone();
    let mic_channels = mic_config.channels();
    let report_mic_f32 = on_event.clone();
    let report_mic_i16 = on_event.clone();

    let mic_stream = match mic_config.sample_format() {
        cpal::SampleFormat::F32 => mic_device.build_input_stream(
            &mic_config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !rec_mic.load(Ordering::Relaxed) {
                    return;
                }
                let mono = to_mono_f32(data, mic_channels);
                if let Ok(mut m) = mix_mic.lock() {
                    m.mic.extend(mono);
                }
            },
            move |err| {
                report_mic_f32(AudioRecordingEvent::StreamError {
                    source: "Mic",
                    error: err.to_string(),
                })
            },
            None,
        )?,
        cpal::SampleFormat::I16 => mic_device.build_input_stream(
            &mic_config.config(),
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if !rec_mic.load(Ordering::Relaxed) {
                    return;
                }
                let mono = i16_to_mono_f32(data, mic_channels);
                if let Ok(mut m) = mix_mic.lock() {
                    m.mic.extend(mono);
                }
            },
            move |err| {
                report_mic_i16(AudioRecordingEvent::StreamError {
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

    // Drain and write mixed audio periodically
    while recording.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(50));

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

    // Stop streams
    drop(loopback_stream);
    drop(mic_stream);

    // Drain any remaining samples
    if let Ok(mut m) = mix.lock() {
        // Write remaining from whichever buffer still has data
        let mixed = m.drain_mixed();
        if let Ok(mut guard) = writer.lock()
            && let Some(ref mut w) = *guard
        {
            for sample in &mixed {
                let s = (*sample * i16::MAX as f32) as i16;
                let _ = w.write_sample(s);
            }
        }
    }

    // Finalize the WAV file
    if let Ok(mut guard) = writer.lock()
        && let Some(w) = guard.take()
    {
        w.finalize().context("Failed to finalize WAV")?;
    }

    on_event(AudioRecordingEvent::SavedRecording(wav_path));
    Ok(())
}

/// Find the most recent session directory containing a recording.wav.
pub fn latest_session(base_dir: &PathBuf) -> Result<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(base_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && e.path().join("recording.wav").exists())
        .collect();

    entries.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    entries
        .last()
        .map(|e| e.path())
        .context("No recordings found")
}

/// Create a new session directory with optional name.
pub fn create_session_dir(cfg: &config::Config, name: Option<&str>) -> Result<PathBuf> {
    let base = config::effective_output_dir(cfg)?;
    create_session_dir_in(&base, name)
}

pub fn create_session_dir_in(base: &Path, name: Option<&str>) -> Result<PathBuf> {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
    let dir_name = match name {
        Some(n) if !n.is_empty() => format!("{timestamp} — {n}"),
        _ => format!("{timestamp}"),
    };
    let session_dir = base.join(dir_name);
    std::fs::create_dir_all(&session_dir)?;
    Ok(session_dir)
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn list_sessions(base_dir: &Path) -> Result<Vec<SessionEntry>> {
    if !base_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(base_dir)
        .with_context(|| format!("Failed to read {}", base_dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }

            let metadata = entry.metadata().ok()?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let name = path.file_name()?.to_string_lossy().into_owned();
            let status = session_status(&path);

            Some(SessionEntry {
                path,
                name,
                status,
                modified,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(entries)
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
fn session_status(path: &Path) -> SessionStatus {
    if path.join("notes.md").exists() {
        SessionStatus::NotesReady
    } else if path.join("transcript.txt").exists() {
        SessionStatus::TranscriptReady
    } else if path.join("recording.wav").exists() {
        SessionStatus::RecordingOnly
    } else {
        SessionStatus::Empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn to_mono_passes_through_single_channel() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(to_mono_f32(&input, 1), input);
    }

    #[test]
    fn to_mono_averages_stereo_channels() {
        let input = vec![0.0, 1.0, 0.5, 0.5, -1.0, 1.0];
        let result = to_mono_f32(&input, 2);
        assert_eq!(result, vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn to_mono_handles_empty_input() {
        let input: Vec<f32> = vec![];
        assert_eq!(to_mono_f32(&input, 2), Vec::<f32>::new());
    }

    #[test]
    fn i16_conversion_preserves_amplitude() {
        let input: Vec<i16> = vec![i16::MAX, 0, i16::MIN];
        let result = i16_to_mono_f32(&input, 1);
        assert!((result[0] - 1.0).abs() < 1e-4);
        assert_eq!(result[1], 0.0);
        // i16::MIN / i16::MAX is slightly less than -1.0 (asymmetric range)
        assert!(result[2] < -0.999);
    }

    #[test]
    fn mix_buffer_combines_both_streams() {
        let mut buf = MixBuffer::new();
        buf.loopback.extend([0.2, 0.4]);
        buf.mic.extend([0.1, 0.2]);
        let mixed = buf.drain_mixed();
        // Output: loopback + mic * 1.5, clamped
        assert_eq!(mixed.len(), 2);
        assert!((mixed[0] - (0.2 + 0.1 * 1.5)).abs() < 1e-6);
        assert!((mixed[1] - (0.4 + 0.2 * 1.5)).abs() < 1e-6);
    }

    #[test]
    fn mix_buffer_pads_shorter_stream_with_silence() {
        let mut buf = MixBuffer::new();
        buf.loopback.extend([0.5, 0.5, 0.5]);
        buf.mic.extend([0.1]);
        let mixed = buf.drain_mixed();
        // Three samples out: first uses both, last two pad mic with 0.0
        assert_eq!(mixed.len(), 3);
        assert!((mixed[0] - (0.5 + 0.1 * 1.5)).abs() < 1e-6);
        assert!((mixed[1] - 0.5).abs() < 1e-6);
        assert!((mixed[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mix_buffer_clamps_to_unit_range() {
        let mut buf = MixBuffer::new();
        buf.loopback.extend([0.9, -0.9]);
        buf.mic.extend([0.9, -0.9]);
        let mixed = buf.drain_mixed();
        // 0.9 + 0.9*1.5 = 2.25 → clamped to 1.0; -2.25 → -1.0
        assert_eq!(mixed[0], 1.0);
        assert_eq!(mixed[1], -1.0);
    }

    #[test]
    fn mix_buffer_drains_to_empty() {
        let mut buf = MixBuffer::new();
        buf.loopback.extend([0.1]);
        buf.mic.extend([0.2]);
        let _ = buf.drain_mixed();
        assert!(buf.loopback.is_empty());
        assert!(buf.mic.is_empty());
    }

    #[test]
    fn latest_session_returns_most_recent_with_recording() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_path_buf();

        let older = base.join("session-1");
        fs::create_dir_all(&older).unwrap();
        fs::write(older.join("recording.wav"), b"fake").unwrap();

        sleep(Duration::from_millis(20));

        let newer = base.join("session-2");
        fs::create_dir_all(&newer).unwrap();
        fs::write(newer.join("recording.wav"), b"fake").unwrap();

        let result = latest_session(&base).unwrap();
        assert_eq!(result, newer);
    }

    #[test]
    fn latest_session_skips_dirs_without_recording() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_path_buf();

        let with_recording = base.join("good");
        fs::create_dir_all(&with_recording).unwrap();
        fs::write(with_recording.join("recording.wav"), b"fake").unwrap();

        sleep(Duration::from_millis(20));

        // Newer directory but no recording.wav — should be skipped
        let without_recording = base.join("empty");
        fs::create_dir_all(&without_recording).unwrap();

        let result = latest_session(&base).unwrap();
        assert_eq!(result, with_recording);
    }

    #[test]
    fn latest_session_errors_when_no_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().to_path_buf();
        let result = latest_session(&base);
        assert!(result.is_err());
    }

    #[test]
    fn create_session_dir_in_uses_configured_base_dir() {
        let temp = tempfile::tempdir().unwrap();

        let session_dir = create_session_dir_in(temp.path(), Some("Planning")).unwrap();

        assert!(session_dir.starts_with(temp.path()));
        assert!(session_dir.exists());
        assert!(
            session_dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("Planning")
        );
    }

    #[test]
    fn list_sessions_returns_directories_newest_first_with_status() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();

        let older = base.join("2026-05-07_100000 - Older");
        fs::create_dir_all(&older).unwrap();
        fs::write(older.join("recording.wav"), b"fake").unwrap();

        sleep(Duration::from_millis(20));

        let newer = base.join("2026-05-08_100000 - Newer");
        fs::create_dir_all(&newer).unwrap();
        fs::write(newer.join("recording.wav"), b"fake").unwrap();
        fs::write(newer.join("transcript.txt"), b"transcript").unwrap();
        fs::write(newer.join("notes.md"), b"notes").unwrap();

        let sessions = list_sessions(base).unwrap();

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].path, newer);
        assert_eq!(sessions[0].status, SessionStatus::NotesReady);
        assert_eq!(sessions[1].path, older);
        assert_eq!(sessions[1].status, SessionStatus::RecordingOnly);
    }

    #[test]
    fn list_sessions_includes_directories_without_recordings_as_empty() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let empty = base.join("empty-session");
        fs::create_dir_all(&empty).unwrap();

        let sessions = list_sessions(base).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Empty);
    }

    #[test]
    fn audio_recording_events_format_user_visible_messages() {
        let saved_path = PathBuf::from("/tmp/scribe/recording.wav");

        assert_eq!(
            AudioRecordingEvent::LoopbackDevice("HD Pro Webcam C920".into()).message(),
            "Loopback: HD Pro Webcam C920"
        );
        assert_eq!(
            AudioRecordingEvent::MicDevice("Studio Mic".into()).message(),
            "Mic: Studio Mic"
        );
        assert_eq!(
            AudioRecordingEvent::AudioConfig {
                loopback_sample_rate: 16000,
                loopback_channels: 2,
                mic_sample_rate: 48000,
                mic_channels: 1,
                output_sample_rate: 16000,
            }
            .message(),
            "Loopback: 16000Hz 2ch, Mic: 48000Hz 1ch, Output: 16000Hz mono"
        );
        assert_eq!(
            AudioRecordingEvent::SavedRecording(saved_path.clone()).message(),
            format!("Saved recording to: {}", saved_path.display())
        );
        assert_eq!(
            AudioRecordingEvent::StreamError {
                source: "Mic",
                error: "device lost".into(),
            }
            .message(),
            "Mic stream error: device lost"
        );
    }
}
