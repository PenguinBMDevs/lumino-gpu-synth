//! Waveform-shape probe: renders N sustained voices and checks whether the
//! output is square-wave-like (saturation flat-topping) vs. dynamically
//! scaled. Flat-topped waveforms (many samples near the ceiling) sound like
//! distortion/crackle even at modest peak values.
//!
//! Usage: diag_peakshape <voices>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let voices: usize = std::env::args()
        .nth(1)
        .unwrap_or("256".into())
        .parse()
        .unwrap();
    let block = 2048usize;
    let sr = 64_000usize;
    let config = SynthConfig {
        sample_rate: sr as u32,
        block_size: block,
        max_voices: 16384,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    for i in 0..voices {
        let ch = (i / 128) as u8;
        let key = (i % 128) as u8;
        let vel = 90 + ((i * 7) % 37) as u8;
        synth.note_on(ch, key, vel);
    }

    let mut buf = vec![0.0f32; block * 2];
    let mut peak = 0.0f32;
    let mut over09 = 0usize;
    let mut total = 0usize;
    let mut sq = 0.0f64;

    for _ in 0..12 {
        // 12 blocks * 32ms = ~384 ms of sustained polyphony
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
        for &s in &buf {
            let a = s.abs();
            peak = peak.max(a);
            if a > 0.9 {
                over09 += 1;
            }
            sq += s as f64 * s as f64;
            total += 1;
        }
    }
    let rms = (sq / total as f64).sqrt();
    // How much of the signal sits at >90% of its own peak: for a sine,
    // ~3.6% of samples exceed 0.9*peak; for a square wave, ~100% do.
    let near = over09 as f64 / total as f64;
    println!(
        "voices={voices:>5} peak={peak:.3} rms={rms:.4} p/r={:.1} over0.9_abs={over09}/{total} over90pctOfPeak={:.1}% {}",
        if peak > 0.0 { peak / rms as f32 } else { 0.0 },
        near * 100.0,
        if near > 0.5 {
            "SQUARE-WAVE FLAT-TOP (distorted)"
        } else if peak > 1.0 {
            "clipped"
        } else {
            "dynamic (ok)"
        }
    );
    Ok(())
}
