//! Detects crackle/pops in the rendered output vs voice count.
//!
//! Renders a dense chord of N sustained voices for several blocks and
//! measures:
//!   - `peak`: output peak (limiter should hold it at ~0.98)
//!   - `clip`: samples at the 1.0 ceiling (hard-clipped -> square waves)
//!   - `jumps`: sample-to-sample discontinuities > 0.25 (audible click/pop)
//!   - `blockjump`: discontinuity at block boundaries (limiter gain steps)
//!
//! Usage: diag_crackle [voices...]

use lumino_gpu_synth::{GpuSynth, SynthConfig};
use std::time::Instant;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let voices_list: Vec<usize> = if args.is_empty() {
        vec![128, 256, 512, 800, 1024, 1536, 2048, 4096]
    } else {
        args
    };

    for &voices in &voices_list {
        let config = SynthConfig {
            sample_rate: 64_000,
            block_size: 2048,
            max_voices: (voices * 3 / 2).max(64), // pool = 1.5x, no pool-trim pressure
            max_voices_per_key: 0,                // no per-key trim: pure summation
            use_effects: false,
            show_progress: false,
            ..SynthConfig::default()
        };
        let mut synth = GpuSynth::new(config)?;
        synth.load_soundfont("assets/test.sf2", 0, 0)?;
        // Spread voices across channels/keys/velocities so they are
        // independent signals (random-phase summation like real music).
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
        let mut clip = 0usize;
        let mut jumps = 0usize;
        let mut blockjump = 0usize;
        let mut prev = 0.0f32; // last sample of the previous block (left)
        for b in 0..14 {
            buf.fill(0.0);
            let t0 = Instant::now();
            synth.render_block(&mut buf)?;
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            if b >= 2 {
                total += dt;
                n += 1;
            }
            if b > 0 {
                // Block-boundary continuity (limiter gain stepping).
                if (buf[0] - prev).abs() > 0.25 {
                    blockjump += 1;
                }
            }
            prev = buf[buf.len() - 2];
            for pair in buf.chunks_exact(2) {
                let l = pair[0];
                let r = pair[1];
                let a = l.abs();
                let b = r.abs();
                peak = peak.max(a).max(b);
                if a >= 0.999 || b >= 0.999 {
                    clip += 1;
                }
            }
            // Sample-to-sample jumps within the block (stereo, left only).
            for i in 2..buf.len() {
                if (buf[i] - buf[i - 2]).abs() > 0.25 {
                    jumps += 1;
                }
            }
        }
        let steady = total / n as f64;
        let flag = if clip > 0 || jumps > 0 || blockjump > 0 {
            "  <<< CRACKLE"
        } else {
            ""
        };
        println!(
            "voices={voices:>4}  steady={steady:>5.1}ms  peak={peak:.3}  clip={clip:>6}  jumps={jumps:>6}  blockjump={blockjump:>4}{flag}"
        );
    }
    Ok(())
}
