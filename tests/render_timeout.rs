//! Offline-render timeout guards (requires a GPU + `assets/test.sf2`).
//!
//! A voice that can never finish - a note-on without note-off, a held damper
//! pedal - must terminate the render with `SynthError::RenderTimeout`
//! instead of looping forever.
//!
//! Run with: `cargo test --test render_timeout -- --ignored`

use std::io::Write;

use lumino_gpu_synth::{GpuSynth, SynthConfig, SynthError};

/// Runs `f` in a helper thread and fails the test if it does not finish
/// within `secs` seconds (hard timeout). The orphan thread dies with the
/// process once the test runner exits.
fn with_timeout<T>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T
where
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(r) => r,
        Err(_) => panic!("TEST TIMED OUT after {secs}s"),
    }
}

/// Encodes a tiny Standard MIDI File with the given channel messages.
///
/// `events` is a list of `(delta_ticks, message)`; the track is closed with
/// an end-of-track meta event. Division is 480 and no tempo event is
/// emitted, so the parser assumes the default 120 BPM (1 s = 960 ticks).
fn make_midi(events: &[(u32, [u8; 3])]) -> Vec<u8> {
    let mut track = Vec::new();
    for (delta, msg) in events {
        push_varint(&mut track, *delta);
        track.extend_from_slice(msg);
    }
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]); // end of track

    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&[0, 0, 0, 6, 0, 0, 0, 1, 0x01, 0xE0]); // format 0, 1 track, div 480
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(track.len() as u32).to_be_bytes());
    out.extend_from_slice(&track);
    out
}

fn push_varint(out: &mut Vec<u8>, mut v: u32) {
    let mut buf = [0u8; 4];
    let mut i = 3;
    buf[i] = (v & 0x7F) as u8;
    v >>= 7;
    while v > 0 {
        i -= 1;
        buf[i] = ((v & 0x7F) as u8) | 0x80;
        v >>= 7;
    }
    out.extend_from_slice(&buf[i..]);
}

fn write_tmp(bytes: &[u8], name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("lumino-gpu-synth-tests");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).expect("create temp midi");
    f.write_all(bytes).expect("write temp midi");
    path
}

/// A note-on with no matching note-off: the voice sits in sustain forever.
/// The render must abort with `RenderTimeout` (with a tiny tail budget so
/// the test finishes fast).
#[test]
#[ignore = "requires a GPU and the assets/test.sf2 soundfont"]
fn missing_note_off_times_out() {
    with_timeout(60, || {
        let path = write_tmp(&make_midi(&[(0, [0x90, 60, 100])]), "missing-note-off.mid");
        let config = SynthConfig {
            max_tail_seconds: 0.5,
            ..SynthConfig::default()
        };
        let mut synth = GpuSynth::new(config).expect("gpu synth");
        synth
            .load_soundfont("assets/test.sf2", 0, 0)
            .expect("soundfont");
        match synth.render_midi_file(&path) {
            Err(SynthError::RenderTimeout {
                active_voices,
                frames,
                ..
            }) => {
                assert!(active_voices > 0, "expected a stuck voice, got none");
                assert!(frames < 1_000_000, "timeout fired too late: {frames}");
            }
            other => panic!("expected RenderTimeout, got {other:?}"),
        }
    });
}

/// The damper pedal (CC64) swallows note-offs; the voice stays in sustain
/// until the pedal is lifted. A file that never lifts it must time out.
#[test]
#[ignore = "requires a GPU and the assets/test.sf2 soundfont"]
fn held_damper_times_out() {
    with_timeout(60, || {
        let path = write_tmp(
            &make_midi(&[
                (0, [0xB0, 0x40, 0x7F]), // damper on
                (0, [0x90, 60, 100]),    // note-on
                (960, [0x80, 60, 0]),    // note-off (swallowed by the damper)
            ]),
            "held-damper.mid",
        );
        let config = SynthConfig {
            max_tail_seconds: 0.5,
            ..SynthConfig::default()
        };
        let mut synth = GpuSynth::new(config).expect("gpu synth");
        synth
            .load_soundfont("assets/test.sf2", 0, 0)
            .expect("soundfont");
        match synth.render_midi_file(&path) {
            Err(SynthError::RenderTimeout { active_voices, .. }) => {
                assert!(active_voices > 0, "expected a stuck voice, got none");
            }
            other => panic!("expected RenderTimeout, got {other:?}"),
        }
    });
}

/// Sanity regression: a well-formed file must still render to the end and
/// produce sound. The current asset is a large MIDI (~224 s of events).
#[test]
#[ignore = "requires a GPU and the assets/test.sf2 soundfont"]
fn well_formed_midi_finishes() {
    with_timeout(60, || {
        let mut synth = GpuSynth::new(SynthConfig::default()).expect("gpu synth");
        synth
            .load_soundfont("assets/test.sf2", 0, 0)
            .expect("soundfont");
        let start = std::time::Instant::now();
        let result = synth
            .render_midi_file("assets/right-example.mid")
            .expect("render must finish");
        let elapsed = start.elapsed();
        let seconds = result.frames as f64 / result.sample_rate as f64;
        println!(
            "rendered {:.1} s of audio in {:.2?} ({:.2}x realtime)",
            seconds,
            elapsed,
            seconds / elapsed.as_secs_f64()
        );
        assert!(result.frames > 13_000_000, "too short: {}", result.frames);
        assert!(result.frames < 16_000_000, "too long: {}", result.frames);
        assert!(
            result.samples.iter().any(|s| s.abs() > 0.1),
            "output should contain audible material"
        );
    });
}

/// The block size must never change the audio output: it only affects how
/// many frames are dispatched per GPU submission. Renders the file with
/// block sizes 512 and 2048 and compares every sample.
#[test]
#[ignore = "requires a GPU and the assets/test.sf2 soundfont (slow: ~2 renders)"]
fn block_size_does_not_change_output() {
    with_timeout(60, || {
        let render = |block_size: usize| -> Vec<f32> {
            let config = SynthConfig {
                block_size,
                ..SynthConfig::default()
            };
            let mut synth = GpuSynth::new(config).expect("gpu synth");
            synth
                .load_soundfont("assets/test.sf2", 0, 0)
                .expect("soundfont");
            synth
                .render_midi_file("assets/right-example.mid")
                .expect("render must finish")
                .samples
        };

        let a = render(512);
        let b = render(2048);
        // The tail cutoff granularity follows the block size: the larger block
        // may keep a few extra *silent* frames at the end. All audible content
        // must be bit-identical.
        let n = a.len().min(b.len());
        // Diagnostics: locate the first genuinely different frame.
        if let Some((i, (x, y))) = a[..n]
            .iter()
            .zip(&b[..n])
            .enumerate()
            .find(|(_, (x, y))| (**x - **y).abs() > 1e-4)
        {
            let f = i / 2;
            println!("first diff at frame {f}: 512={x} 2048={y}");
            for k in i.saturating_sub(4)..(i + 8).min(n) {
                println!(
                    "  sample {k}: 512={} 2048={} diff={}",
                    a[k],
                    b[k],
                    a[k] - b[k]
                );
            }
        }
        let max_diff = a[..n]
            .iter()
            .zip(&b[..n])
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        // Locate the largest difference.
        let max_at = a[..n]
            .iter()
            .zip(&b[..n])
            .enumerate()
            .map(|(i, (x, y))| ((*x - *y).abs(), i, *x, *y))
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        if let Some((_, i, x, y)) = max_at {
            println!(
                "max diff {} at sample {i} (frame {}): 512={x} 2048={y}",
                (x - y).abs(),
                i / 2
            );
            for k in i.saturating_sub(4)..(i + 8).min(n) {
                println!(
                    "  sample {k}: 512={} 2048={} diff={}",
                    a[k],
                    b[k],
                    a[k] - b[k]
                );
            }
        }
        println!(
            "block 512: {} samples, block 2048: {} samples, common prefix max diff = {max_diff}",
            a.len(),
            b.len()
        );
        assert!(
            max_diff < 1e-6,
            "output differs with block size: {max_diff}"
        );
        // The longer render's tail must be pure silence.
        let (long, short) = if a.len() > b.len() { (a, b) } else { (b, a) };
        let tail = &long[short.len()..];
        assert!(
            tail.iter().all(|s| s.abs() <= 0.0001),
            "tail beyond the shorter render is not silent ({} non-silent)",
            tail.iter().filter(|s| s.abs() > 0.0001).count()
        );
    });
}
