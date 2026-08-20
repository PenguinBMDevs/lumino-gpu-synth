//! Quick 30-second render profile of a dense MIDI: renders only the first
//! 30 s of frames and prints per-phase cost summaries, to find the hidden
//! per-block overhead (spawn/trim/upload/GPU-wait) without a full render.

use lumino_gpu_synth::{GpuSynth, MidiFile, SynthConfig};
use std::time::Instant;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    let midi = MidiFile::load("assets/right-example.mid", 64_000)?;
    synth.set_events(midi.sequence.events);

    // 30 s of frames = 30 * 64000 / 2048 = 937 blocks.
    let blocks = 937usize;
    let mut buf = vec![0.0f32; 2048 * 2];
    let mut sums = [0.0f64; 6]; // apply, collect, sync, upload, samples, dispatch
    let mut max_collect = 0.0f64;
    let mut max_total = 0.0f64;
    let mut peak = 0.0f32;
    let mut max_voices = 0usize;
    let mut slow_blocks: Vec<(usize, f64, usize)> = Vec::new();

    for b in 0..blocks {
        let t0 = Instant::now();
        synth.render_block(&mut buf)?;
        // Mirror the playback render thread: lookahead sample pre-upload.
        let _ = synth.prefetch_samples(6 << 20);
        let t1 = Instant::now();
        let dt = t1.duration_since(t0).as_secs_f64();
        max_total = max_total.max(dt);
        max_voices = max_voices.max(synth.voice_count() as usize);
        // Track slow blocks (>= 50 ms) with their block index.
        if dt >= 0.05 {
            slow_blocks.push((b, dt, synth.voice_count() as usize));
        }
        for &s in &buf {
            peak = peak.max(s.abs());
        }
        sums[5] += dt;
    }
    let _ = sums;
    slow_blocks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    println!(
        "30s profile: blocks={blocks} avg_total={:.1}ms max_total={:.0}ms max_voices={max_voices} peak={peak:.3}",
        sums[5] / blocks as f64 * 1000.0,
        max_total * 1000.0
    );
    for (b, dt, vc) in slow_blocks.iter().take(6) {
        println!("  slow block {b}: {:.0}ms voices={vc}", dt * 1000.0);
    }
    Ok(())
}
