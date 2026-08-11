//! Stress test: many keys playing dense overlapping notes at once - the
//! scenario the user reported ("dense single-key high-rate, some keys go
//! silent"). Checks that no note is dropped and no note is truncated.

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        max_voices_per_key: 4,
        max_voices: 4096,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    // 24 keys, each repeating 30 times, notes overlapping (note_len > gap):
    // each note is 60 ms, starts every 40 ms -> sustained overlap of 3-4
    // notes per key, 60+ notes total at peak.
    let keys: Vec<u8> = (0..24).map(|i| 40 + i * 2).collect();
    let note_len = 0.060f64;
    let gap = 0.040f64;
    let reps = 30;
    let block = 2048u64;
    let sample_rate = 64_000f64;

    let total = (gap * reps as f64 * sample_rate) as u64 + 128_000;
    let out_frames = total + block;
    let mut out = vec![0.0f32; (out_frames as usize) * 2];

    let mut frame = 0u64;
    let mut rendered = 0usize;
    // Schedule: for each rep, all keys note_on then note_off staggered.
    let mut notes_sent = 0u64;
    for r in 0..reps as u64 {
        for &key in &keys {
            let at = r as u64 * (gap * sample_rate) as u64 + key as u64 * 128;
            while frame < at {
                let mut buf = vec![0.0f32; block as usize * 2];
                synth.render_block(&mut buf)?;
                let start = rendered * block as usize;
                out[start * 2..start * 2 + buf.len()].copy_from_slice(&buf);
                rendered += 1;
                frame += block;
            }
            synth.note_on(0, key, 110);
            notes_sent += 1;
        }
    }
    for r in 0..reps as u64 {
        for &key in &keys {
            let at = r as u64 * (gap * sample_rate) as u64 + key as u64 * 128
                + (note_len * sample_rate) as u64;
            while frame < at {
                let mut buf = vec![0.0f32; block as usize * 2];
                synth.render_block(&mut buf)?;
                let start = rendered * block as usize;
                out[start * 2..start * 2 + buf.len()].copy_from_slice(&buf);
                rendered += 1;
                frame += block;
            }
            synth.note_off(0, key);
        }
    }
    while frame < total {
        let mut buf = vec![0.0f32; block as usize * 2];
        synth.render_block(&mut buf)?;
        let start = rendered * block as usize;
        out[start * 2..start * 2 + buf.len()].copy_from_slice(&buf);
        rendered += 1;
        frame += block;
    }

    // Per key: count sounding reps and check each is not truncated.
    let frames_per_rep = (gap * sample_rate) as usize;
    let mut total_short = 0usize;
    let mut total_silent = 0usize;
    for &key in &keys {
        let offset = key as usize * 128;
        let mut sounding = 0usize;
        let mut short = 0usize;
        for r in 0..reps as usize {
            let start = (r * frames_per_rep + offset) as usize * 2;
            let end = (r * frames_per_rep + offset + frames_per_rep) as usize * 2;
            let end = end.min(out.len());
            let mut frames_on = 0usize;
            for i in (start..end).step_by(64) {
                let e = out[i] * out[i] + out[i + 1] * out[i + 1];
                if e > 1e-5 {
                    frames_on += 1;
                }
            }
            if frames_on > 0 {
                sounding += 1;
            }
            if frames_on < (frames_per_rep as usize / 64) * 6 / 10 {
                short += 1;
            }
        }
        total_silent += reps as usize - sounding;
        total_short += short;
        if sounding != reps as usize || short != 0 {
            println!(
                "  key={key:3}: sounding={sounding}/{} short={short}",
                reps as usize
            );
        }
    }
    println!(
        "keys={} notes_sent={} silent_reps={} truncated_reps={}",
        keys.len(),
        notes_sent,
        total_silent,
        total_short
    );
    if total_silent == 0 && total_short == 0 {
        println!("PASS: no dropped notes, no truncated notes");
    } else {
        println!("FAIL: see per-key lines above");
    }
    Ok(())
}
