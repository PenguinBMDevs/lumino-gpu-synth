//! Diagnostic: render a burst of very short notes and watch the voice
//! population. If voices accumulate instead of ending, the voice lifecycle
//! is broken (release never fires, or the GPU never marks ended).

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let config = SynthConfig::default();
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    // 200 notes of 4 ms each, 1 ms apart, on key 60.
    for i in 0..200u64 {
        let on = i * 64; // 1 ms @ 64k
        synth.note_on(0, 60, 100);
        // note_off is scheduled via the event queue at the right sample...
        // (send_event timestamps are "next block", so emulate with manual
        //  blocks instead: queue both events with explicit times below.)
        let _ = on;
    }

    // Rebuild: push timed events directly through render_block timing.
    // Simpler: alternate note on/off by rendering in small blocks.
    let mut out = vec![0.0f32; 1024 * 2];
    let mut voices: Vec<usize> = Vec::new();
    for i in 0..200u32 {
        // block 0..: note on; block i+1: note off (4 ms later = 4 blocks @1024)
        if i % 2 == 0 {
            synth.note_on(0, 60, 100);
        }
        for _ in 0..2 {
            synth.render_block(&mut out)?;
            voices.push(synth.voice_count());
        }
        if i % 2 == 1 {
            synth.note_off(0, 60);
        }
    }
    // Drain remaining voices.
    for _ in 0..40 {
        synth.render_block(&mut out)?;
        voices.push(synth.voice_count());
    }
    println!("voice population over time (every 2 blocks):");
    for (i, v) in voices.iter().enumerate().step_by(8) {
        println!("  step {i}: voices={v}");
    }
    let max = voices.iter().max().copied().unwrap_or(0);
    let last = voices.last().copied().unwrap_or(0);
    println!("max voices={max}, final voices={last} (expect small, e.g. <= 16)");

    // Lifecycle diagnostics on the final state.
    let (n, released, ended) = synth.debug_voice_lifecycle();
    println!("final lifecycle: voices={n}, released={released}, gpu_ended={ended}");
    if let Some((is_rel, gpu_end, env_stage, env_t, rel_at, start_at)) = synth.debug_voice_state() {
        println!(
            "first voice: gpu_is_released={is_rel} gpu_ended={gpu_end} env_stage={env_stage} env_t={env_t} release_at={rel_at} start_at={start_at}"
        );
    }

    // Early check: render 12 blocks with one held note and inspect state.
    let mut synth2 = GpuSynth::new(SynthConfig::default())?;
    synth2.load_soundfont("assets/test.sf2", 0, 0)?;
    synth2.note_on(0, 60, 100);
    let mut out2 = vec![0.0f32; 512 * 2];
    for _ in 0..12 {
        synth2.render_block(&mut out2)?;
    }
    println!(
        "after 12 blocks, 1 held note: lifecycle={:?}",
        synth2.debug_voice_lifecycle()
    );
    if let Some((is_rel, gpu_end, env_stage, env_t, rel_at, start_at)) = synth2.debug_voice_state()
    {
        println!(
            "held note state: gpu_is_released={is_rel} gpu_ended={gpu_end} env_stage={env_stage} env_t={env_t} release_at={rel_at} start_at={start_at}"
        );
    }
    Ok(())
}
