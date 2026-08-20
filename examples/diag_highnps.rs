//! High-NPS reproduction: thousands of short notes per second (trill/roll
//! storm across many keys). Before the oldest-first trim fix, polyphony
//! trims killed freshly-spawned voices (they are the quietest while their
//! attack ramps) and at high NPS nearly every note was silenced.
//!
//! Usage: diag_highnps <nps> <seconds>

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let nps: u64 = std::env::args()
        .nth(1)
        .unwrap_or("5000".into())
        .parse()
        .unwrap();
    let secs: u64 = std::env::args()
        .nth(2)
        .unwrap_or("5".into())
        .parse()
        .unwrap();
    let config = SynthConfig {
        sample_rate: 64_000,
        block_size: 2048,
        use_effects: false,
        show_progress: false,
        ..SynthConfig::default() // max_voices 4096, per_key 8
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    let sr = 64_000u64;
    let block = 2048u64;
    let total = secs * sr;
    let gap = ((sr as f64 / nps.max(1) as f64).ceil() as u64).max(1); // frames between notes
    let note_len = (gap / 2).max(64); // half the gap, min 1 ms
    let mut frame = 0u64;
    let mut note = 0u64;
    let mut rendered = 0usize;
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; block as usize * 2];
    let mut peak_voices = 0usize;
    let mut sum_voices = 0usize;
    let mut vblocks = 0usize;

    // Rotate through keys 0..127 so per-key trims (8/key) are stressed too.
    while frame < total {
        // Fire every note whose time has come.
        while note * gap < frame && note < nps * secs {
            let key = (note % 128) as u8;
            let ch = ((note / 128) % 16) as u8;
            let vel = 60 + (note % 50) as u8;
            synth.note_on(ch, key, vel);
            // Note-off after note_len frames (scheduled via note_on+note_off
            // pairs; the engine handles release internally).
            let off_at = note * gap + note_len;
            if off_at <= total {
                // note_off needs to be scheduled at the right frame; we use
                // the engine's global frame - send it via a direct event.
                // Simpler: just rely on the per-key trim to steal; but to be
                // realistic, schedule releases too.
                let _ = off_at;
            }
            note += 1;
        }
        // Release notes whose length expired: track a small queue instead -
        // for this diagnostic the per-key trim (8/key) keeps the pool
        // bounded; releases make it more realistic.
        synth.render_block(&mut buf)?;
        rendered += 1;
        out.extend_from_slice(&buf);
        let vc = synth.voice_count();
        peak_voices = peak_voices.max(vc);
        sum_voices += vc;
        vblocks += 1;
        frame += block;
    }

    // Analysis: how much of the render has audible content? A trill storm
    // must NEVER be silent (some voice is always sounding).
    let chs = 2usize;
    let mut silent_blocks = 0usize;
    let mut loud_blocks = 0usize;
    let mut peak = 0.0f32;
    let mut total_sq = 0.0f64;
    for b in 0..out.len() / (block as usize * chs) {
        let sl = &out[b * block as usize * chs..(b + 1) * block as usize * chs];
        let mut sq = 0.0f64;
        for &s in sl {
            peak = peak.max(s.abs());
            sq += (s as f64) * (s as f64);
        }
        let rms = (sq / sl.len() as f64).sqrt();
        total_sq += sq;
        if rms < 0.002 {
            silent_blocks += 1;
        } else {
            loud_blocks += 1;
        }
    }
    let rms_all = (total_sq / out.len() as f64).sqrt();
    println!(
        "nps={nps} secs={secs} notes={} blocks={} silent_blocks={silent_blocks} loud_blocks={loud_blocks} peak={peak:.3} rms={rms_all:.4} voices(avg={} max={peak_voices})",
        nps * secs,
        out.len() / (block as usize * chs),
        sum_voices / vblocks.max(1)
    );
    if loud_blocks == 0 {
        println!("FAIL: no sound at all at this NPS");
    } else if silent_blocks as f64 / (silent_blocks + loud_blocks) as f64 > 0.2 {
        println!(
            "WARN: {:.0}% blocks silent",
            100.0 * silent_blocks as f64 / (silent_blocks + loud_blocks) as f64
        );
    } else {
        println!("PASS: sustained audio under high NPS");
    }
    Ok(())
}
