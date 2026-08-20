//! Measures high-frequency content of a rendered voice and the alias
//! energy introduced by 64k -> 48k linear resampling (no anti-alias filter).
//! Usage: diag_alias <key>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

/// Inline copy of the playback linear resampler (the crate's is pub(crate)).
fn resample_linear(input: &[f32], from: u32, to: u32, channels: usize) -> Vec<f32> {
    let ratio = to as f64 / from as f64;
    let n_in = input.len() / channels;
    let n_out = ((n_in as f64) * ratio) as usize;
    let mut out = vec![0.0f32; n_out * channels];
    for (o, chunk) in out.chunks_exact_mut(channels).enumerate() {
        let base = (o as f64) / ratio;
        let i0 = base.floor() as usize;
        let frac = (base - i0 as f64) as f32;
        for (c, dst) in chunk.iter_mut().enumerate() {
            let a = input.get(i0 * channels + c).copied().unwrap_or(0.0);
            let b = input.get((i0 + 1) * channels + c).copied().unwrap_or(a);
            *dst = a + (b - a) * frac;
        }
    }
    out
}

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let key: u8 = std::env::args()
        .nth(1)
        .unwrap_or("110".into())
        .parse()
        .unwrap();
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        max_voices: 64,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    synth.note_on(0, key, 110);
    let mut buf = vec![0.0f32; 2048 * 2];
    let mut raw = Vec::new();
    for _ in 0..8 {
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
        raw.extend_from_slice(&buf);
    }
    // Skip the first 2048 frames (attack + pipeline warm-up).
    let raw = &raw[2048 * 2..];

    // HF metric: sample-to-sample delta energy (rich in HF content).
    let mut delta_energy = 0.0f64;
    for i in (2..raw.len()).step_by(2) {
        let d = raw[i] - raw[i - 2];
        delta_energy += (d * d) as f64;
    }
    let n = (raw.len() / 2 - 1) as f64;
    let delta_rms = (delta_energy / n).sqrt();
    let mut peak = 0.0f32;
    for &s in raw {
        peak = peak.max(s.abs());
    }

    // Resample to 48k and measure the same HF metric plus the alias band
    // (16k-24k) via a crude Goertzel-ish check: first-difference RMS of the
    // 48k output (linear interp averages adjacent 64k samples, so its HF
    // content is what survives/aliases).
    let out48 = resample_linear(raw, 64_000, 48_000, 2);
    let mut d48 = 0.0f64;
    for i in (2..out48.len()).step_by(2) {
        let d = out48[i] - out48[i - 2];
        d48 += (d * d) as f64;
    }
    let n48 = (out48.len() / 2 - 1) as f64;
    let d48_rms = (d48 / n48).sqrt();

    println!(
        "key={key} peak={peak:.3} raw_delta_rms={delta_rms:.4} (64k HF content)\n  resampled_to_48k delta_rms={d48_rms:.4} (aliased HF survives in 0-24k)",
    );
    Ok(())
}
