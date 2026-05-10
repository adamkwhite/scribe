use std::collections::VecDeque;

/// Shared buffer for mixing two audio streams.
/// Each stream pushes mono f32 samples; the writer drains and mixes them.
pub(super) struct MixBuffer {
    pub(super) loopback: VecDeque<f32>,
    pub(super) mic: VecDeque<f32>,
}

impl MixBuffer {
    pub(super) fn new() -> Self {
        Self {
            loopback: VecDeque::new(),
            mic: VecDeque::new(),
        }
    }

    /// Drain all available samples, mixing both streams.
    /// If one stream has no data, the other still produces output (mixed with silence).
    pub(super) fn drain_mixed(&mut self) -> Vec<f32> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        // 0.9 + 0.9*1.5 = 2.25 -> clamped to 1.0; -2.25 -> -1.0
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
}
