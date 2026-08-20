//! Measures GPU render cost vs voice count (the `collect_pending_readback`
//! wait). Determines the polyphony ceiling that keeps realtime safe.
//! Usage: diag_gpucost [voices...]

use lumino_gpu_synth::{GpuSynth, SynthConfig};
use std::time::Instant;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let voices_list: Vec<usize> = if args.is_empty() {
        vec![256, 512, 1024, 2048, 4096, 6144]
    } else {
        args
    };

    for &voices in &voices_list {
        let config = SynthConfig {
            sample_rate: 64_000,
            block_size: 2048,
            max_voices: voices.max(64),
            max_voices_per_key: 0, // no per-key trim: measure raw GPU cost
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
        let mut buf = vec![0.0f32; 2048 * 2];
        let mut total = 0.0f64;
        let mut n = 0u32;
        let mut peak = 0.0f32;
        let mut per_block = String::new();
        for b in 0..12 {
            buf.fill(0.0);
            let t0 = Instant::now();
            synth.render_block(&mut buf)?;
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            if b >= 2 {
                // Skip the cold-start blocks (shader/pipeline compilation).
                total += dt;
                n += 1;
            }
            if b < 4 {
                per_block.push_str(&format!(" b{b}={dt:.0}ms"));
            }
            for &s in &buf {
                peak = peak.max(s.abs());
            }
        }
        println!(
            "voices={voices:>5}  steady={:.1}ms/block{per_block}  peak={peak:.3}",
            total / n as f64
        );
    }
    Ok(())
}
