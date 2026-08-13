//! Peak-level probe: with N simultaneous voices, what is the output peak?
//! Verifies whether high instantaneous polyphony clips (> 1.0) at the mix.

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        max_voices: 16384,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };

    // A single note-on per key; keys/channels span the whole range so we can
    // push 128..=16384 simultaneous voices.
    for &voices in &[1usize, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 12288] {
        let mut synth = GpuSynth::new(config.clone())?;
        synth.load_soundfont("assets/test.sf2", 0, 0)?;
        let mut buf = vec![0.0f32; 2048 * 2];

        // Spread across keys 0..127 and channels so `max_voices_per_key`
        // does not trim them.
        for i in 0..voices {
            let ch = (i / 128) as u8;
            let key = (i % 128) as u8;
            let vel = 90 + ((i * 7) % 37) as u8;
            synth.note_on(ch, key, vel);
        }

        // Render a few blocks so all voices are sounding at peak attack.
        let mut peak = 0.0f32;
        for _ in 0..4 {
            buf.fill(0.0);
            synth.render_block(&mut buf)?;
            for &s in &buf {
                peak = peak.max(s.abs());
            }
        }
        let over = buf.iter().filter(|&&s| s.abs() > 1.0).count();
        println!(
            "voices={voices:>6}  peak={peak:.3}  over1.0={over:>6}  ({})",
            if peak > 1.0 { "CLIPPED" } else { "ok" }
        );
    }

    // Also probe the realtime example's typical config (block 2048) via the
    // event-stream path? Not needed: same mix kernel.
    Ok(())
}
