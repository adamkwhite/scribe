use anyhow::{Context, Result};
use std::path::Path;

use super::resample::resample_to_16khz;

pub(super) fn load_wav_as_mono_16khz_f32(wav_path: &Path) -> Result<Vec<f32>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

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
