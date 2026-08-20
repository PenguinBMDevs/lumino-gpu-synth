//! Detects output discontinuities (sample-to-sample jumps) - the signature
//! of voices being killed/trimmed without fade, which sounds like clicks.
//! Usage: diag_discontinuity <voices> <max_voices>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let voices: usize = std::env::args()
        .nth(1)
        .unwrap_or("12000".into())
        .parse()
        .unwrap();
    let max_voices: usize = std::env::args()
        .nth(2)
        .unwrap_or("4096".into())
        .parse()
        .unwrap();
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        max_voices,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    let mut buf = vec![0.0f32; 2048 * 2];

    // Phase 1: fill the pool to capacity and let the voices sound.
    for i in 0..voices / 3 {
        let ch = (i / 128) as u8;
        let key = (i % 128) as u8;
        let vel = 90 + ((i * 7) % 37) as u8;
        synth.note_on(ch, key, vel);
    }
    for _ in 0..6 {
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
    }
    // Phase 2: blow way past the pool - the over-capacity notes force the
    // engine to trim voices that are ALREADY SOUNDING (the click scenario).
    for i in 0..voices {
        let ch = (i / 128) as u8;
        let key = (i % 128) as u8;
        let vel = 90 + ((i * 7) % 37) as u8;
        synth.note_on(ch, key, vel);
    }
    // Render and look for sample-to-sample jumps.
    let mut peak_jump = 0.0f32;
    let mut big_jumps = 0usize;
    let mut prev_l = 0.0f32;
    let mut prev_r = 0.0f32;
    for block in 0..24 {
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
        for (i, chunk) in buf.chunks_exact(2).enumerate() {
            let (l, r) = (chunk[0], chunk[1]);
            if i > 0 {
                let dl = (l - prev_l).abs();
                let dr = (r - prev_r).abs();
                peak_jump = peak_jump.max(dl.max(dr));
                if dl > 0.25 || dr > 0.25 {
                    big_jumps += 1;
                    if big_jumps <= 10 {
                        let d = if dl > dr { dl } else { dr };
                        println!(
                            "  jump @ block={block} frame={i} prev=({prev_l:.3},{prev_r:.3}) now=({l:.3},{r:.3}) d={d:.3}"
                        );
                    }
                }
            }
            prev_l = l;
            prev_r = r;
        }
    }
    println!(
        "voices={voices} pool={max_voices} peak_jump={peak_jump:.3} big_jumps={big_jumps} {}",
        if big_jumps > 0 {
            "DISCONTINUITIES (voice kills without fade)"
        } else {
            "continuous"
        }
    );
    Ok(())
}
