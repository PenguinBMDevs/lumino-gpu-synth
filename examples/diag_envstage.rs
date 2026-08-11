//! Renders a single-note midi to a frame and prints the first voice's env
//! stage/t. Usage: diag_envstage <midi> <frame_s>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let midi = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "single-note.mid".into());
    let at = std::env::args()
        .nth(2)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.3)
        * 64000.0;

    let config = SynthConfig {
        use_effects: false,
        max_voices: 16384,
        block_size: 512,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    synth.render_midi_frames(&midi, at as u64)?;
    let mut buf = vec![0.0f32; 512 * 2];
    let _ = synth.render_block(&mut buf);
    if let Some((is_rel, ended, stage, t, rel_at, start_at)) = synth.debug_voice_state() {
        println!(
            "frame={at}: is_released={is_rel} ended={ended} env_stage={stage} env_t={t} release_at={rel_at} start_at={start_at}"
        );
    } else {
        println!("no voices");
    }
    Ok(())
}
