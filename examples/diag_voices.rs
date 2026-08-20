//! Diagnostic: renders a MIDI block-by-block with the SAME engine path used by
//! realtime playback (`set_events` + `render_block`), and logs per-block:
//!   - instantaneous voice count (`synth.voice_count()`)
//!   - block RMS (silence detector)
//!   - `render_block` wall time (stall detector)
//!   - event cursor position (whether the stream still has data)
//!
//! This reproduces the realtime "high polyphony -> silence" behavior without an
//! audio device, because `render_block` is identical in both paths.
//!
//! Usage:
//! ```text
//! cargo run --release --example diag_voices -- [midi] [max_voices] [block_size]
//! ```

use lumino_gpu_synth::{ChannelMode, GpuSynth, MidiFile, SynthConfig};
use std::time::Instant;

fn rms(buf: &[f32]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    let mut s = 0.0f64;
    for &x in buf {
        s += (x as f64) * (x as f64);
    }
    (s / buf.len() as f64).sqrt() as f32
}

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<String> = std::env::args().collect();
    let midi_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "assets/test.mid".to_string());
    let max_voices: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2048);
    let block_size: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2048);

    let config = SynthConfig {
        block_size,
        max_voices,
        ..SynthConfig::default()
    };
    let chs = match config.channels {
        ChannelMode::Mono => 1,
        ChannelMode::Stereo => 2,
    };

    let sr = config.sample_rate;
    let mut synth = GpuSynth::new(config)?;
    println!("adapter: {}", synth.adapter_info().name);
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    let midi = MidiFile::load(&midi_path, sr)?;
    let total_events = midi.sequence.events.len();
    println!(
        "midi: {} events, rate {} Hz, block {}, max_voices {}, channels {}",
        total_events, sr, block_size, max_voices, chs
    );
    synth.set_events(midi.sequence.events);

    let mut out = vec![0.0f32; block_size * chs];
    let mut block = 0u64;
    let mut max_vc = 0usize;
    let mut silent_blocks = 0u64;
    let mut collapse_events = 0u64;
    let mut last_collapse_block = None;
    let mut stalled_blocks = 0u64;
    let mut errors = 0u64;
    let block_dur = block_size as f64 / sr as f64;

    loop {
        let _vc_before = synth.voice_count();
        let t0 = Instant::now();
        let res = synth.render_block(&mut out);
        let dt = t0.elapsed();
        let dt_ms = dt.as_secs_f64() * 1000.0;
        let vc = synth.voice_count();
        let r = rms(&out);
        let exhausted = synth.stream_exhausted();

        max_vc = max_vc.max(vc);

        if vc == 0 && !exhausted {
            if last_collapse_block.map_or(true, |b| block - b > 4) {
                println!(
                    "[COLLAPSE] block {block} t={t:.2}s vc=0 (cursor not exhausted) rms={r:.4e} dt={dt_ms:.1}ms",
                    t = block as f64 * block_dur,
                    r = r,
                );
                collapse_events += 1;
            }
            last_collapse_block = Some(block);
        }
        if r < 1e-5 {
            silent_blocks += 1;
        }
        if dt_ms > 150.0 {
            println!(
                "[STALL] block {block} t={t:.2}s vc={vc} dt={dt_ms:.1}ms rms={r:.4e}",
                t = block as f64 * block_dur,
            );
            stalled_blocks += 1;
        }

        if let Err(e) = res {
            println!(
                "[RENDER ERROR] block {block} t={t:.2}s: {e:?}",
                t = block as f64 * block_dur,
            );
            errors += 1;
            if errors > 20 {
                break;
            }
        }

        if block % 200 == 0 {
            println!(
                "block {block} t={t:.2}s vc={vc} (max {max_vc}) rms={r:.4e} dt={dt_ms:.1}ms exhausted={exhausted}",
                t = block as f64 * block_dur,
            );
        }

        block += 1;
        if exhausted {
            let _ = synth.render_block(&mut out);
            break;
        }
    }

    println!("---- summary ----");
    println!(
        "blocks={block} max_vc={max_vc} silent_blocks={silent_blocks} collapses={collapse_events} stalls(>150ms)={stalled_blocks} render_errors={errors}"
    );
    Ok(())
}
