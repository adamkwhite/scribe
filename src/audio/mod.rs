use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::config;

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
    _target_sample_rate: u32,
    session_dir: PathBuf,
) -> Result<()> {
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

    println!("Loopback: {}", loopback_device.name().unwrap_or_default());
    println!("Mic: {}", mic_device.name().unwrap_or_default());

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
    println!(
        "Loopback: {}Hz {}ch, Mic: {}Hz {}ch, Output: {}Hz mono",
        loopback_config.sample_rate().0,
        loopback_config.channels(),
        mic_config.sample_rate().0,
        mic_config.channels(),
        output_sample_rate,
    );

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
            |err| eprintln!("Loopback stream error: {err}"),
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
            |err| eprintln!("Loopback stream error: {err}"),
            None,
        )?,
        format => anyhow::bail!("Unsupported loopback sample format: {format:?}"),
    };

    // --- Mic stream ---
    let mix_mic = mix.clone();
    let rec_mic = recording.clone();
    let mic_channels = mic_config.channels();

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
            |err| eprintln!("Mic stream error: {err}"),
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
            |err| eprintln!("Mic stream error: {err}"),
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

    println!("Saved recording to: {}", wav_path.display());
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
pub fn create_session_dir(name: Option<&str>) -> Result<PathBuf> {
    let base = config::output_dir()?;
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
    let dir_name = match name {
        Some(n) if !n.is_empty() => format!("{timestamp} — {n}"),
        _ => format!("{timestamp}"),
    };
    let session_dir = base.join(dir_name);
    std::fs::create_dir_all(&session_dir)?;
    Ok(session_dir)
}
