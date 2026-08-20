//! Renders a single sustained note and finds periodic sample-to-sample
//! jumps - the signature of a discontinuous loop point in the soundfont.
//! Usage: diag_loopseam <key>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let key: u8 = std::env::args()
        .nth(1)
        .unwrap_or("60".into())
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
    let mut jumps: Vec<(usize, f32)> = Vec::new();
    let mut prev_l = 0.0f32;
    let mut prev_r = 0.0f32;
    for block in 0..32 {
        // 32 blocks * 2048 = 65536 frames = 1.024 s
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
        for (i, chunk) in buf.chunks_exact(2).enumerate() {
            if i == 0 {
                continue;
            }
            let d = (chunk[0] - prev_l).abs().max((chunk[1] - prev_r).abs());
            if d > 0.02 {
                jumps.push((block * 2048 + i, d));
            }
            prev_l = chunk[0];
            prev_r = chunk[1];
        }
    }
    println!("key={key} jumps({}>0.02): {}", jumps.len(), jumps.len());
    let mut last = 0usize;
    for (i, (f, d)) in jumps.iter().enumerate().take(40) {
        let gap = if i == 0 { 0 } else { f - last };
        println!("  jump at frame {f:>6} d={d:.3} gap={gap}");
        last = *f;
    }
    Ok(())
}
