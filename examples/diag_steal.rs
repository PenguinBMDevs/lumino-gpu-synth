//! Minimal check: 20 note-ons on the same key without note-offs. With the
//! per-key layer limit (4) and immediate steal, voices must stay <= 8
//! (2 zones per key x 4 layers).

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let mut synth = GpuSynth::new(SynthConfig::default())?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    let mut out = vec![0.0f32; 512 * 2];

    for i in 0..20 {
        synth.note_on(0, 60, 100);
        for _ in 0..4 {
            synth.render_block(&mut out)?;
        }
        println!(
            "after note {i}: voices={} lifecycle={:?}",
            synth.voice_count(),
            synth.debug_voice_lifecycle()
        );
    }
    Ok(())
}
