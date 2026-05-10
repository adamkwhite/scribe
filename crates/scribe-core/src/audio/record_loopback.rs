use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::audio_recording_event::AudioRecordingEvent;
use super::mix_buffer::MixBuffer;
use super::samples::{i16_to_mono_f32, to_mono_f32};

fn emit_recording_event<F>(on_event: &F, event: AudioRecordingEvent)
where
    F: Fn(AudioRecordingEvent),
{
    match &event {
        AudioRecordingEvent::StreamError { .. } => {
            tracing::warn!(message = %event.message(), "audio recording event");
        }
        _ => {
            tracing::info!(message = %event.message(), "audio recording event");
        }
    }
    on_event(event);
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

    // Loopback device (system audio - other person's voice)
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

    emit_recording_event(
        &*on_event,
        AudioRecordingEvent::LoopbackDevice(loopback_device.name().unwrap_or_default()),
    );
    emit_recording_event(
        &*on_event,
        AudioRecordingEvent::MicDevice(mic_device.name().unwrap_or_default()),
    );

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
    emit_recording_event(
        &*on_event,
        AudioRecordingEvent::AudioConfig {
            loopback_sample_rate: loopback_config.sample_rate().0,
            loopback_channels: loopback_config.channels(),
            mic_sample_rate: mic_config.sample_rate().0,
            mic_channels: mic_config.channels(),
            output_sample_rate,
        },
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
                emit_recording_event(
                    &*report_lb_f32,
                    AudioRecordingEvent::StreamError {
                        source: "Loopback",
                        error: err.to_string(),
                    },
                )
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
                emit_recording_event(
                    &*report_lb_i16,
                    AudioRecordingEvent::StreamError {
                        source: "Loopback",
                        error: err.to_string(),
                    },
                )
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
                emit_recording_event(
                    &*report_mic_f32,
                    AudioRecordingEvent::StreamError {
                        source: "Mic",
                        error: err.to_string(),
                    },
                )
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
                emit_recording_event(
                    &*report_mic_i16,
                    AudioRecordingEvent::StreamError {
                        source: "Mic",
                        error: err.to_string(),
                    },
                )
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

    emit_recording_event(&*on_event, AudioRecordingEvent::SavedRecording(wav_path));
    Ok(())
}
