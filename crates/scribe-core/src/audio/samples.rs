/// Convert a multi-channel f32 buffer to mono by averaging channels.
pub(super) fn to_mono_f32(data: &[f32], channels: u16) -> Vec<f32> {
    if channels == 1 {
        return data.to_vec();
    }
    data.chunks(channels as usize)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Convert a multi-channel i16 buffer to mono f32.
pub(super) fn i16_to_mono_f32(data: &[i16], channels: u16) -> Vec<f32> {
    let as_f32: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    to_mono_f32(&as_f32, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
