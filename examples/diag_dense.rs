//! Stress test: dense repeated notes across several keys, checking BOTH
//! dropped notes (self-steal bug) AND note duration (each note must sound
//! for its full length, not be truncated by an over-aggressive steal).
//!
//! For each key, sends N rapid note_on/note_off pairs and measures the
//! sounding duration of every note against the expected length.

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        max_voices_per_key: 4,
        max_voices: 1024,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    // A low key, a mid key and a high key: all should behave identically.
    let keys = [36u8, 60, 88];
    let note_len = 0.048f64; // sounding length requested
    let gap = 0.096f64;
    let notes = 40;
    let block = 2048u64;

    for &key in &keys {
        let frames_per_note = (gap * 64_000.0) as u64;
        let frames_per_off = (note_len * 64_000.0) as u64;
        let total = frames_per_note * notes + 64_000;
        let out_frames = total + block;
        let mut out = vec![0.0f32; (out_frames as usize) * 2];

        let mut frame = 0u64;
        let mut rendered = 0usize;
        for i in 0..notes {
            let at = i as u64 * frames_per_note;
            while frame < at {
                let mut buf = vec![0.0f32; block as usize * 2];
                synth.render_block(&mut buf)?;
                let start = rendered * block as usize;
                out[start * 2..start * 2 + buf.len()].copy_from_slice(&buf);
                rendered += 1;
                frame += block;
            }
            synth.note_on(0, key, 100);
            while frame < at + frames_per_off {
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

        // Measure per-note: audible frames vs expected length.
        let mut sounding = 0usize;
        let mut short = 0usize;
        for w in 0..notes as usize {
            let start = (w as u64 * frames_per_note) as usize * 2;
            let end = ((w as u64 * frames_per_note + frames_per_off) as usize * 2).min(out.len());
            let mut frames_on = 0usize;
            for i in (start..end).step_by(32) {
                let e = out[i] * out[i] + out[i + 1] * out[i + 1];
                if e > 1e-5 {
                    frames_on += 1;
                }
            }
            let expected = (frames_per_off as usize) / 32;
            if frames_on > 0 {
                sounding += 1;
            }
            if frames_on < expected * 7 / 10 {
                short += 1;
            }
        }
        let full = sounding == notes as usize && short == 0;
        println!(
            "key={key:3}: notes={notes} sounding={sounding} short(<70% len)={short} {}",
            if full { "PASS" } else { "FAIL" }
        );
    }
    Ok(())
}
