//! Sweeps configuration axes (block_size, interpolation, effects) against
//! high polyphony and reports peak / NaN / Inf, to find any combination
//! that still escapes the limiter.

use lumino_gpu_synth::{GpuSynth, InterpolationMode, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let voices: usize = std::env::args()
        .nth(1)
        .unwrap_or("1024".into())
        .parse()
        .unwrap();
    for block in [512usize, 1024, 2048] {
        for interp in [InterpolationMode::Linear, InterpolationMode::Point64Sinc] {
            for effects in [false, true] {
                let config = SynthConfig {
                    sample_rate: 64_000,
                    block_size: block,
                    max_voices: 16384,
                    interpolation: interp,
                    use_effects: effects,
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
                let mut bad = 0usize;
                for _ in 0..12 {
                    buf.fill(0.0);
                    synth.render_block(&mut buf)?;
                    for &s in &buf {
                        if s.is_finite() {
                            peak = peak.max(s.abs());
                        } else {
                            bad += 1;
                        }
                    }
                }
                let status = if bad > 0 {
                    "NaN/Inf!"
                } else if peak > 0.99 {
                    "OVER 0.99!"
                } else {
                    "ok"
                };
                let interp_s = format!("{interp:?}");
                println!(
                    "block={block:>4} interp={interp_s:<22} effects={effects:<5} voices={voices:>5} peak={peak:.3} bad={bad} {status}",
                );
            }
        }
    }
    Ok(())
}
