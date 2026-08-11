//! Extreme single-key stress: one key hammered at very high rate (200
//! notes, overlapping), the exact scenario the user reported ("dense
//! single-key high-rate -> some keys go silent").

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

    // Single key 60, 200 notes, 50 ms long, starts every 25 ms -> up to 4
    // simultaneous layers (limit=4), maximal steal pressure.
    let key = 60u8;
    let note_len = 0.050f64;
    let gap = 0.025f64;
    let reps = 200u64;
    let block = 2048u64;
    let sr = 64_000f64;

    let total = (gap * reps as f64 * sr) as u64 + 128_000;
    let out_frames = total + block;
    let mut out = vec![0.0f32; (out_frames as usize) * 2];

    let mut frame = 0u64;
    let mut rendered = 0usize;
    for r in 0..reps {
        let at = r * (gap * sr) as u64;
        while frame < at {
            let mut buf = vec![0.0f32; block as usize * 2];
            synth.render_block(&mut buf)?;
            let start = rendered * block as usize;
            out[start * 2..start * 2 + buf.len()].copy_from_slice(&buf);
            rendered += 1;
            frame += block;
        }
        synth.note_on(0, key, 110);
    }
    for r in 0..reps {
        let at = r * (gap * sr) as u64 + (note_len * sr) as u64;
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
    while frame < total {
        let mut buf = vec![0.0f32; block as usize * 2];
        synth.render_block(&mut buf)?;
        let start = rendered * block as usize;
        out[start * 2..start * 2 + buf.len()].copy_from_slice(&buf);
        rendered += 1;
        frame += block;
    }

    let frames_per_rep = (gap * sr) as usize;
    let mut sounding = 0usize;
    let mut short = 0usize;
    for r in 0..reps as usize {
        let start = (r * frames_per_rep) as usize * 2;
        let end = (r * frames_per_rep + frames_per_rep * 2) as usize * 2;
        let end = end.min(out.len());
        let mut on = 0usize;
        for i in (start..end).step_by(64) {
            let e = out[i] * out[i] + out[i + 1] * out[i + 1];
            if e > 1e-5 {
                on += 1;
            }
        }
        if on > 0 {
            sounding += 1;
        }
        if on < (frames_per_rep as usize / 64) * 5 / 10 {
            short += 1;
        }
    }
    println!("single-key high-rate: notes={reps} sounding={sounding} short={short}");
    if sounding == reps as usize && short == 0 {
        println!("PASS: no dropped notes, no truncation at extreme density");
    } else {
        println!(
            "FAIL: {} silent / {} short",
            reps as usize - sounding,
            short
        );
    }
    Ok(())
}
