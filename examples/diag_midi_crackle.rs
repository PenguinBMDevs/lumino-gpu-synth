//! Crackle detection on a real dense MIDI (the user's scenario: ~800-1000
//! voices sustained). Renders the first N seconds of the given MIDI and
//! measures peak / clip / sample jumps (graded) / block-boundary jumps.
//!
//! Usage: diag_midi_crackle <file.mid> [seconds=30] [per_key=8] [max_voices=4096]

use lumino_gpu_synth::{GpuSynth, MidiFile, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "assets/right-example.mid".into());
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let per_key: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let max_voices: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4096);
    let sr = 64_000u32;
    let block = 2048usize;

    let config = SynthConfig {
        sample_rate: sr,
        block_size: block,
        max_voices,
        max_voices_per_key: per_key,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    let midi = MidiFile::load(&path, sr)?;
    // Prewarm every sample the MIDI uses: if the garbage disappears, the
    // crackle source is the incremental sample upload leaving holes.
    synth.prewarm_midi_file(&path)?;
    synth.set_events(midi.sequence.events);

    let blocks = (secs as u64 * sr as u64 / block as u64) as usize;
    let mut buf = vec![0.0f32; block as usize * 2];
    let mut peak = 0.0f32;
    let mut clip = 0usize;
    let mut jumps = [0usize; 4]; // >0.25 / >0.5 / >0.75 / >1.0
    let mut jump_pos = [0usize; 3]; // head(<256) / mid / tail(>1792)
    let mut blockjump = 0usize;
    let mut blockjump_big = 0usize; // block-boundary jump > 0.5
    let mut prev = 0.0f32;
    let mut max_voices = 0usize;
    let mut sum_voices = 0usize;
    let mut pool_blocks = 0usize; // blocks where voices >= pool (6144)
    let mut dumps = 0usize;
    let mut bigs = 0usize;
    for b in 0..blocks {
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
        let vc = synth.voice_count();
        let bpeak = buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        if bpeak > 100.0 {
            let pi = buf.iter().position(|&s| s.abs() > 100.0).unwrap_or(0);
            let lo = pi.saturating_sub(6);
            let hi = (pi + 6).min(buf.len());
            let win: Vec<String> = buf[lo..hi].iter().map(|&s| format!("{:.0}", s)).collect();
            println!(
                "  BIGBLOCK block={b} peak={bpeak:.0} at={} voices={vc} win=[{}]",
                pi / 2,
                win.join(",")
            );
        }
        let vc = synth.voice_count();
        max_voices = max_voices.max(vc);
        sum_voices += vc;
        if vc >= 6144 {
            pool_blocks += 1;
        }
        let bj = (buf[0] - prev).abs();
        if bj > 0.25 {
            blockjump += 1;
            if bj > 0.5 {
                blockjump_big += 1;
            }
        }
        prev = buf[buf.len() - 2];
        for pair in buf.chunks_exact(2) {
            let a = pair[0].abs();
            let c = pair[1].abs();
            peak = peak.max(a).max(c);
            if a > 100.0 && bigs < 6 {
                println!(
                    "  BIG#{bigs} t={:.3}s block={b} voices={vc}",
                    (b as f64 * block as f64) / sr as f64
                );
                bigs += 1;
            }
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
                } else if idx < block - 256 {
                    jump_pos[1] += 1;
                } else {
                    jump_pos[2] += 1;
                }
                if d > 0.75 && dumps < 8 {
                    let t = (b as f64 * block as f64 + idx as f64) / sr as f64;
                    let p4 = if i >= 4 { buf[i - 4] } else { 0.0 };
                    println!(
                        "  jump#{dumps} t={t:.3}s block={b} frame={idx} d={d:.3} prev={:.3} cur={:.3} voices={vc}",
                        p4, buf[i]
                    );
                    dumps += 1;
                }
            }
        }
    }
    println!(
        "midi={path} secs={secs}  peak={peak:.3}  clip={clip}  jumps(.25/.5/.75/1.0)={jumps:?}  pos(h/m/t)={jump_pos:?}  blockjump={blockjump}(big={blockjump_big})  voices(avg={} max={max_voices}) pool_blocks={pool_blocks}",
        sum_voices / blocks.max(1)
    );
    let total: usize = jumps.iter().sum();
    if total == 0 && clip == 0 && blockjump == 0 {
        println!("PASS: no crackle detected");
    } else {
        println!("CHECK: {} candidate jumps", total);
    }
    Ok(())
}
