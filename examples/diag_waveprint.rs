//! Prints the raw output waveform around discontinuity points.
//! Usage: diag_waveprint <voices> <pool>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let voices: usize = std::env::args()
        .nth(1)
        .unwrap_or("16000".into())
        .parse()
        .unwrap();
    let pool: usize = std::env::args()
        .nth(2)
        .unwrap_or("16384".into())
        .parse()
        .unwrap();
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        max_voices: pool,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    let mut buf = vec![0.0f32; 2048 * 2];

    for i in 0..voices / 3 {
        let ch = (i / 128) as u8;
        let key = (i % 128) as u8;
        let vel = 90 + ((i * 7) % 37) as u8;
        synth.note_on(ch, key, vel);
    }
    for _ in 0..6 {
        synth.render_block(&mut buf)?;
    }
    for i in 0..voices {
        let ch = (i / 128) as u8;
        let key = (i % 128) as u8;
        let vel = 90 + ((i * 7) % 37) as u8;
        synth.note_on(ch, key, vel);
    }
    // Block 0 after phase 2; its audio surfaces in block 1 (pipeline lag).
    for block in 0..3 {
        buf.fill(0.0);
        synth.render_block(&mut buf)?;
        if block == 1 {
            println!("--- block {block} waveform (frames 250..420) ---");
            for (i, chunk) in buf.chunks_exact(2).enumerate().take(420).skip(250) {
                println!("f={i:>4} L={:+.4} R={:+.4}", chunk[0], chunk[1]);
            }
        }
    }
    Ok(())
}
