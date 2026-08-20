//! Probes peak/NaN behavior with use_effects = true (the default): filters
//! can resonate (gain > 1 near cutoff) and even diverge to NaN/Inf on
//! pathological coefficients.
//! Usage: diag_filterpeak <voices>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let voices: usize = std::env::args()
        .nth(1)
        .unwrap_or("256".into())
        .parse()
        .unwrap();
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        max_voices: 16384,
        use_effects: true, // default config - filters active
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

    let mut buf = vec![0.0f32; 2048 * 2];
    let mut peak = 0.0f32;
    let mut nan_count = 0usize;
    let mut inf_count = 0usize;
    for _ in 0..12 {
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
        for &s in &buf {
            let a = s.abs();
            if a.is_nan() {
                nan_count += 1;
            } else if a.is_infinite() {
                inf_count += 1;
            } else {
                peak = peak.max(a);
            }
        }
    }
    println!("filtered voices={voices:>5} peak={peak:.3} NaN={nan_count} Inf={inf_count}",);
    Ok(())
}
