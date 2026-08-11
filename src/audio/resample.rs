//! Sample-rate conversion for realtime playback.
//!
//! The engine renders at its configured sample rate; output devices usually
//! run at a different rate (e.g. 48 kHz). [`LinearResampler`] bridges the
//! two with linear interpolation, keeping the previous block's tail so
//! interpolation stays continuous across block boundaries.

/// Minimal linear-interpolation resampler for engine -> device sample-rate
/// conversion. Keeps the previous block's tail so interpolation is continuous
/// across block boundaries.
pub(crate) struct LinearResampler {
    ratio: f64,
    channels: usize,
    /// Last input block, kept for interpolation across block boundaries.
    prev: Vec<f32>,
}

impl LinearResampler {
    pub(crate) fn new(from: u32, to: u32, channels: usize) -> Self {
        Self {
            ratio: if from == 0 {
                1.0
            } else {
                to as f64 / from as f64
            },
            channels,
            prev: vec![0.0; channels],
        }
    }

    /// Resamples one interleaved block; output length = round(input * ratio).
    pub(crate) fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if (self.ratio - 1.0).abs() < 1e-9 {
            return input.to_vec();
        }
        let n_in = input.len() / self.channels;
        let n_out = ((n_in as f64) * self.ratio) as usize;
        let mut out = vec![0.0f32; n_out * self.channels];
        for (o, chunk) in out.chunks_exact_mut(self.channels).enumerate() {
            let base = (o as f64) / self.ratio;
            let i0 = base.floor() as isize;
            let frac = (base - i0 as f64) as f32;
            for (c, dst) in chunk.iter_mut().enumerate() {
                let a = sample_at(input, self.prev.as_slice(), self.channels, i0, c);
                let b = sample_at(input, self.prev.as_slice(), self.channels, i0 + 1, c);
                *dst = a + (b - a) * frac;
            }
        }
        // Keep the tail of this block as the interpolation boundary.
        if n_in > 0 {
            for c in 0..self.channels {
                self.prev[c] = input[(n_in - 1) * self.channels + c];
            }
        }
        out
    }
}

/// Reads sample `i` from either the current input block or the previous
/// block's tail. Negative indices refer to `prev` (continuity across blocks);
/// indices past the end of the current block clamp to its last sample.
#[inline]
fn sample_at(input: &[f32], prev: &[f32], channels: usize, i: isize, c: usize) -> f32 {
    if i < 0 {
        let idx = (i + 1) as usize * channels + c;
        prev.get(idx).copied().unwrap_or(0.0)
    } else if i as usize >= input.len() / channels {
        let last = (input.len() / channels).saturating_sub(1);
        input.get(last * channels + c).copied().unwrap_or(0.0)
    } else {
        let idx = i as usize * channels + c;
        input.get(idx).copied().unwrap_or(prev[c])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_identity_rate() {
        let mut r = LinearResampler::new(48_000, 48_000, 2);
        let input = vec![1.0f32, -1.0, 0.5, -0.5];
        let out = r.process(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn resampler_output_length_tracks_ratio() {
        // Downsample 48k -> 44.1k: ratio = 0.91875.
        let mut r = LinearResampler::new(48_000, 44_100, 2);
        let n = 10_000;
        let mut input = Vec::with_capacity(n * 2);
        for i in 0..n {
            input.push((i as f32 * 0.01).sin());
            input.push((i as f32 * 0.013).cos());
        }
        let out = r.process(&input);
        let expected = (n as f64 * (44_100.0 / 48_000.0)) as usize;
        assert_eq!(out.len(), expected * 2);
    }

    #[test]
    fn resampler_upsample_ratio() {
        // Upsample 44.1k -> 48k: ratio = 1.088...
        let mut r = LinearResampler::new(44_100, 48_000, 1);
        let n = 5_000;
        let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = r.process(&input);
        let expected = (n as f64 * (48_000.0 / 44_100.0)) as usize;
        assert_eq!(out.len(), expected);
        // Linear interpolation of a smooth sine stays bounded by its range.
        assert!(out.iter().all(|s| s.abs() <= 1.001));
    }

    #[test]
    fn resampler_continuous_across_blocks() {
        // Two consecutive blocks must not jump at the seam: feed a linear
        // ramp and check the interpolated values stay monotonic-ish.
        let mut r = LinearResampler::new(48_000, 32_000, 1);
        let mut prev_end = 0.0f32;
        for block in 0..4 {
            let start = block * 512;
            let input: Vec<f32> = (0..512).map(|i| (start + i) as f32 * 0.001).collect();
            let out = r.process(&input);
            let first = out.first().copied().unwrap_or(0.0);
            assert!(
                (first - prev_end).abs() < 0.01,
                "seam jump {first} vs {prev_end}"
            );
            prev_end = *out.last().unwrap_or(&0.0);
        }
    }
}
