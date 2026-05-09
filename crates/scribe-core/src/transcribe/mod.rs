#[cfg(any(
    feature = "whisper-cli",
    all(not(feature = "whisper-cli"), feature = "__embedded-whisper")
))]
use anyhow::Context;
use anyhow::Result;
use std::path::Path;

use crate::config::Config;

#[cfg(all(not(feature = "whisper-cli"), not(feature = "__embedded-whisper")))]
compile_error!(
    "scribe-core requires either the default embedded Whisper backend or the `whisper-cli` feature"
);

#[cfg(all(not(feature = "whisper-cli"), not(feature = "__embedded-whisper")))]
pub async fn run_whisper(_wav_path: &Path, _cfg: &Config) -> Result<String> {
    unreachable!("invalid backend feature configuration")
}

#[cfg(any(
    test,
    all(not(feature = "whisper-cli"), feature = "__embedded-whisper")
))]
const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Run the configured Whisper backend on a WAV file and return the transcript text.
#[cfg(feature = "whisper-cli")]
pub async fn run_whisper(wav_path: &Path, cfg: &Config) -> Result<String> {
    let whisper_bin = external_whisper_bin(cfg)?;
    tracing::info!(
        whisper_bin,
        whisper_model = %cfg.whisper_model,
        wav_path = %wav_path.display(),
        "running external whisper"
    );
    let output = tokio::process::Command::new(whisper_bin)
        .args([
            "--model",
            &cfg.whisper_model,
            "--output-txt",
            "--no-timestamps",
            &wav_path.to_string_lossy(),
        ])
        .output()
        .await
        .with_context(|| format!("Failed to run '{}'. Is whisper.cpp installed?", whisper_bin))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(
            status = ?output.status,
            stderr = %stderr,
            "external whisper failed"
        );
        anyhow::bail!("whisper.cpp failed: {stderr}");
    }

    // whisper.cpp with --output-txt creates a .txt file next to the input
    let txt_path = wav_path.with_extension("wav.txt");
    if txt_path.exists() {
        let transcript =
            std::fs::read_to_string(&txt_path).context("Failed to read whisper output")?;
        // Clean up the intermediate txt file
        let _ = std::fs::remove_file(&txt_path);
        tracing::info!(
            transcript_chars = transcript.trim().len(),
            "external whisper transcript read from output file"
        );
        Ok(transcript.trim().to_string())
    } else {
        // Some versions write to stdout instead
        let stdout = String::from_utf8_lossy(&output.stdout);
        tracing::info!(
            transcript_chars = stdout.trim().len(),
            "external whisper transcript read from stdout"
        );
        Ok(stdout.trim().to_string())
    }
}

/// Run the embedded whisper.cpp backend on a WAV file and return the transcript text.
#[cfg(all(not(feature = "whisper-cli"), feature = "__embedded-whisper"))]
pub async fn run_whisper(wav_path: &Path, cfg: &Config) -> Result<String> {
    let wav_path = wav_path.to_path_buf();
    let model_path = cfg.whisper_model.clone();
    tracing::info!(
        wav_path = %wav_path.display(),
        whisper_model = %model_path,
        "running embedded whisper"
    );

    tokio::task::spawn_blocking(move || run_embedded_whisper(&wav_path, &model_path))
        .await
        .context("Embedded whisper task failed")?
}

#[cfg(any(test, feature = "whisper-cli"))]
fn external_whisper_bin(cfg: &Config) -> Result<&str> {
    cfg.whisper_bin
        .as_deref()
        .context("whisper_bin is required when the whisper-cli backend is enabled")
}

#[cfg(all(not(feature = "whisper-cli"), feature = "__embedded-whisper"))]
fn run_embedded_whisper(wav_path: &Path, model_path: &str) -> Result<String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    whisper_rs::install_logging_hooks();

    let audio = load_wav_as_mono_16khz_f32(wav_path)?;
    let ctx =
        WhisperContext::new_with_params(Path::new(model_path), WhisperContextParameters::default())
            .with_context(|| format!("Failed to load whisper model '{}'", model_path))?;
    let mut state = ctx
        .create_state()
        .context("Failed to create whisper state")?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(
        std::thread::available_parallelism()
            .map(|threads| threads.get())
            .unwrap_or(1) as i32,
    );
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, &audio)
        .context("Embedded whisper transcription failed")?;

    let transcript = state
        .as_iter()
        .map(|segment| segment.to_string())
        .collect::<Vec<_>>()
        .join("");
    Ok(transcript.trim().to_string())
}

#[cfg(any(
    test,
    all(not(feature = "whisper-cli"), feature = "__embedded-whisper")
))]
fn load_wav_as_mono_16khz_f32(wav_path: &Path) -> Result<Vec<f32>> {
    let reader = hound::WavReader::open(wav_path)
        .with_context(|| format!("Failed to open {}", wav_path.display()))?;
    let spec = reader.spec();

    if spec.channels == 0 {
        anyhow::bail!("WAV file has no audio channels");
    }

    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader
            .into_samples::<i16>()
            .map(|sample| sample.map(|sample| sample as f32 / i16::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to read 16-bit WAV samples")?,
        (hound::SampleFormat::Float, 32) => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to read 32-bit float WAV samples")?,
        _ => anyhow::bail!(
            "Unsupported WAV format: {:?} {}-bit",
            spec.sample_format,
            spec.bits_per_sample
        ),
    };

    let channel_count = spec.channels as usize;
    let mono = if channel_count == 1 {
        samples
    } else {
        samples
            .chunks(channel_count)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    Ok(resample_to_16khz(&mono, spec.sample_rate))
}

#[cfg(any(
    test,
    all(not(feature = "whisper-cli"), feature = "__embedded-whisper")
))]
fn resample_to_16khz(samples: &[f32], source_rate: u32) -> Vec<f32> {
    if source_rate == WHISPER_SAMPLE_RATE || samples.is_empty() {
        return samples.to_vec();
    }

    let output_len =
        (samples.len() as f64 * WHISPER_SAMPLE_RATE as f64 / source_rate as f64).round() as usize;
    if output_len <= 1 {
        return vec![samples[0]];
    }

    (0..output_len)
        .map(|idx| {
            let source_pos = idx as f64 * source_rate as f64 / WHISPER_SAMPLE_RATE as f64;
            let lower = source_pos.floor() as usize;
            let upper = (lower + 1).min(samples.len() - 1);
            let fraction = (source_pos - lower as f64) as f32;
            samples[lower] * (1.0 - fraction) + samples[upper] * fraction
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    #[test]
    fn external_whisper_bin_errors_when_missing() {
        let cfg = Config {
            whisper_bin: None,
            whisper_model: "model.bin".into(),
            openrouter_api_key: "key".into(),
            model: "some/model".into(),
            sample_rate: 16000,
            output_dir: None,
        };

        let error = external_whisper_bin(&cfg).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("whisper_bin is required when the whisper-cli backend is enabled")
        );
    }

    #[test]
    fn loads_mono_i16_wav_as_16khz_f32() {
        let temp = tempfile::tempdir().unwrap();
        let wav_path = temp.path().join("mono.wav");
        write_test_wav(&wav_path, 16_000, 1, &[i16::MAX, 0, i16::MIN]);

        let samples = load_wav_as_mono_16khz_f32(&wav_path).unwrap();

        assert_eq!(samples.len(), 3);
        assert!((samples[0] - 1.0).abs() < 1e-4);
        assert_eq!(samples[1], 0.0);
        assert!(samples[2] < -0.999);
    }

    #[test]
    fn mixes_stereo_i16_wav_to_mono() {
        let temp = tempfile::tempdir().unwrap();
        let wav_path = temp.path().join("stereo.wav");
        write_test_wav(&wav_path, 16_000, 2, &[i16::MAX, 0, 0, i16::MAX]);

        let samples = load_wav_as_mono_16khz_f32(&wav_path).unwrap();

        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 0.5).abs() < 1e-4);
        assert!((samples[1] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn resamples_wav_to_16khz() {
        let temp = tempfile::tempdir().unwrap();
        let wav_path = temp.path().join("resample.wav");
        let samples = vec![0; 48_000];
        write_test_wav(&wav_path, 48_000, 1, &samples);

        let samples = load_wav_as_mono_16khz_f32(&wav_path).unwrap();

        assert_eq!(samples.len(), 16_000);
    }

    fn write_test_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[i16]) {
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }
}
