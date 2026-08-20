//! Crackle detection under sustained spawn/trim turnover (the user's
//! scenario: ~800-1000 live voices with continuous note-on/off churn).
//!
//! Keeps ~`target` voices alive while spawning/releasing notes every block
//! and measures clip / sample jumps / block-boundary jumps.
//!
//! Usage: diag_crackle2 [max_voices] [target_voices ...]

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let (max_voices, targets): (usize, Vec<usize>) = match args.split_first() {
        Some((&mv, rest)) if !rest.is_empty() => (mv, rest.to_vec()),
        Some((&mv, _)) => (mv, vec![mv / 2, mv * 3 / 4, mv]),
        None => (4096, vec![512, 800, 1024, 2048, 3072]),
    };

    for &target in &targets {
        if target > max_voices {
            continue;
        }
        let config = SynthConfig {
            sample_rate: 64_000,
            block_size: 2048,
            max_voices,
            max_voices_per_key: 0, // no per-key trim: engine pool handles it
            use_effects: false,
            show_progress: false,
            ..SynthConfig::default()
        };
        let mut synth = GpuSynth::new(config)?;
        synth.load_soundfont("assets/test.sf2", 0, 0)?;

        // Pre-fill exactly `target` voices: round-robin over keys so each
        // note is a distinct voice (mono sample = 1 zone per note).
        let mut buf = vec![0.0f32; 2048 * 2];
        for i in 0..target {
            let ch = (i / 128) as u8;
            let key = (i % 128) as u8;
            let vel = 90 + ((i * 7) % 37) as u8;
            synth.note_on(ch, key, vel);
        }
        for _ in 0..4 {
            synth.render_block(&mut buf)?;
        }
        let vc0 = synth.voice_count();

        let mut peak = 0.0f32;
        let mut clip = 0usize;
        let mut jumps = [0usize; 4]; // >0.25 / >0.5 / >0.75 / >1.0
        let mut jump_pos = [0usize; 4]; // head/mid/tail of block
        let mut blockjump = 0usize;
        let mut prev = 0.0f32;
        let mut note_id = 0u64;
        let mut live_notes: Vec<u64> = (0..target as u64).collect();
        for b in 0..40 {
            // Churn: release the oldest third of live notes, spawn the same
            // number of fresh ones.
            let churn = (live_notes.len() / 3).max(1);
            for _ in 0..churn {
                let note = live_notes.remove(0);
                synth.note_off((note / 128) as u8, (note % 128) as u8);
            }
            for _ in 0..churn {
                let key = (note_id % 128) as u8;
                let vel = 90 + ((note_id * 7) % 37) as u8;
                synth.note_on((note_id / 128) as u8, key, vel);
                live_notes.push(note_id);
                note_id += 1;
            }

            buf.fill(0.0);
            synth.render_block(&mut buf)?;
            if b > 0 && (buf[0] - prev).abs() > 0.25 {
                blockjump += 1;
            }
            prev = buf[buf.len() - 2];
            for pair in buf.chunks_exact(2) {
                let a = pair[0].abs();
                let c = pair[1].abs();
                peak = peak.max(a).max(c);
                if a >= 0.999 || c >= 0.999 {
                    clip += 1;
                }
            }
            for i in 2..buf.len() {
                let d = (buf[i] - buf[i - 2]).abs();
                if d > 0.25 {
                    if d > 0.5 {
                        if d > 0.75 {
                            if d > 1.0 {
                                jumps[3] += 1;
                            } else {
                                jumps[2] += 1;
                            }
                        } else {
                            jumps[1] += 1;
                        }
                    } else {
                        jumps[0] += 1;
                    }
                    let idx = i / 2;
                    if idx < 256 {
                        jump_pos[0] += 1;
                    } else if idx < 2048 - 256 {
                        jump_pos[1] += 1;
                    } else {
                        jump_pos[2] += 1;
                    }
                }
            }
        }
        let total_jumps: usize = jumps.iter().sum();
        let flag = if clip > 0 || total_jumps > 0 || blockjump > 0 {
            "  <<< CRACKLE"
        } else {
            ""
        };
        println!(
            "max_voices={max_voices} target={target:>4} voices={vc0:>4}  peak={peak:.3}  clip={clip:>6}  jumps(.25/.5/.75/1.0)={:?}  pos(h/m/t)={:?}  blockjump={blockjump:>4}{flag}",
            jumps, jump_pos
        );
    }
    Ok(())
}
