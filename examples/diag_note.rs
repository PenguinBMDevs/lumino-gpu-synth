//! Diagnostic: render a single held note and inspect the waveform. A
//! healthy render shows a stable, periodic sample (clean tone); sample-data
//! corruption shows up as noise or wildly varying zero-crossing rates.

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let sr = 64_000usize;
    let config = SynthConfig {
        block_size: 512,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    synth.note_on(0, 60, 100);
    let mut out = vec![0.0f32; 512 * 2];
    let mut prev = 0.0f32;
    let mut crossings = 0usize;
    let mut samples_since_crossing = 0usize;
    let mut interval_sum = 0usize;
    let mut interval_count = 0usize;

    for block in 0..200 {
        synth.render_block(&mut out)?;
        if block < 20 {
            continue; // skip attack
        }
        for (_i, &s) in out.iter().enumerate().step_by(2) {
            if prev <= 0.0 && s > 0.0 {
                if samples_since_crossing > 0 {
                    interval_sum += samples_since_crossing;
                    interval_count += 1;
                }
                samples_since_crossing = 0;
                crossings += 1;
            }
            samples_since_crossing += 1;
            prev = s;
        }
    }

    let n = 180 * 512;
    let avg_interval = if interval_count > 0 {
        interval_sum as f64 / interval_count as f64
    } else {
        0.0
    };
    println!(
        "blocks 20..200 ({n} frames): crossings={crossings}, avg zero-crossing interval={avg_interval:.2} samples (=> {:.1} Hz)",
        sr as f64 / (2.0 * avg_interval)
    );
    println!("  (expect ~a few hundred Hz for a musical note; noise would be erratic)");

    // Print the first 64 samples of block 30 (after attack) for inspection.
    let mut synth2 = GpuSynth::new(SynthConfig {
        block_size: 512,
        ..SynthConfig::default()
    })?;
    synth2.load_soundfont("assets/test.sf2", 0, 0)?;
    synth2.note_on(0, 60, 100);
    let mut o2 = vec![0.0f32; 512 * 2];
    for _ in 0..30 {
        synth2.render_block(&mut o2)?;
    }
    println!("first 32 L samples of block 30:");
    for i in 0..32 {
        print!("{:.4} ", o2[i * 2]);
    }
    println!();
    Ok(())
}
