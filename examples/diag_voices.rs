//! Renders assets/C4-C5.mid and dumps per-voice details around a frame.
//! Usage: diag_voices <frame_s>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let target = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(3.05)
        * 64000.0;

    let config = SynthConfig {
        use_effects: false,
        max_voices: 16384,
        block_size: 2048,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    synth.render_midi_frames("assets/C4-C5.mid", target as u64)?;
    let mut buf = vec![0.0f32; 2048 * 2];
    let _ = synth.render_block(&mut buf);

    println!("frame={} ({:.3}s) voices:", target as u64, target / 64000.0);
    for (key, vel, speed, amp, released, ended, stage, t, rel_at, gpu_rel, env_from) in
        synth.debug_voices()
    {
        println!(
            "  key={key} vel={vel} speed={speed:.5} amp={amp:.5} released={released} ended={ended} stage={stage} t={t} rel_at={rel_at} gpu_rel={gpu_rel} env_from={env_from}"
        );
    }
    Ok(())
}
