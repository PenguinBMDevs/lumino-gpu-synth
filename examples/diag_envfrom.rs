//! Renders C4-C5.mid to a frame and dumps every voice's env_from/stage.
//! Usage: diag_envfrom <frame_s>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let at = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(6.55)
        * 64000.0;
    let config = SynthConfig {
        use_effects: false,
        block_size: 512,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    synth.render_midi_frames("notes12.mid", at as u64)?;
    let mut buf = vec![0.0f32; 512 * 2];
    let _ = synth.render_block(&mut buf);
    println!("frame={at}:");
    for (key, vel, speed, amp, released, ended, stage, t, rel_at, gpu_rel, env_from) in
        synth.debug_voices()
    {
        println!(
            "  key={key} stage={stage} t={t} rel_at={rel_at} gpu_rel={gpu_rel} env_from={env_from}"
        );
    }
    Ok(())
}
