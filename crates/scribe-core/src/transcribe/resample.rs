const WHISPER_SAMPLE_RATE: u32 = 16_000;

pub(super) fn resample_to_16khz(samples: &[f32], source_rate: u32) -> Vec<f32> {
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
