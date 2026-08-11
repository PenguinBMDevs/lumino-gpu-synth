//! Realtime playback of a MIDI file on the GPU synth through the default
//! audio device (cpal).
//!
//! The parsed event stream is replayed against a wall-clock timeline at the
//! engine's sample rate, so the audio plays in real time.
//!
//! Usage:
//! ```text
//! cargo run --release --example realtime_midi -- <midi file> [seconds]
//! ```
//! `seconds` (optional) stops after that many seconds of playback.

use std::time::{Duration, Instant};

use lumino_gpu_synth::audio::playback::AudioPlayback;
use lumino_gpu_synth::{GpuSynth, MidiFile, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<String> = std::env::args().collect();
    let midi_path = args.get(1).cloned().ok_or_else(|| {
        lumino_gpu_synth::SynthError::Config("usage: realtime_midi <midi file> [seconds]".into())
    })?;
    let max_seconds = args
        .get(2)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    // The engine renders at 48 kHz; `AudioPlayback::start` negotiates with
    // the device and resamples if needed.
    let sample_rate = 48_000;
    let config = SynthConfig {
        sample_rate,
        block_size: 512,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    // Parse the MIDI: the sequence is sample-accurate at the engine rate.
    let midi = MidiFile::load(&midi_path, sample_rate)?;
    let max_channel = midi
        .sequence
        .events
        .iter()
        .fold(0u16, |m, e| m.max((e.channel + 1) as u16));
    let end_sample = midi.sequence.events.last().map_or(0, |e| e.sample);
    let events = midi.sequence.events;
    println!(
        "loaded {} events, {:.2}s, {} channel(s) from {midi_path}",
        events.len(),
        end_sample as f64 / sample_rate as f64,
        max_channel
    );

    let mut playback = AudioPlayback::start(synth)?;
    let engine_rate = playback.engine_sample_rate();
    println!(
        "playing on device @ {} Hz (engine renders @ {} Hz)",
        playback.sample_rate(),
        engine_rate
    );

    // Realtime scheduler: send events whose sample time has been reached,
    // paced by a wall clock so the audio runs in real time.
    let stop_at = if max_seconds > 0.0 {
        Some(max_seconds * engine_rate as f64)
    } else {
        None
    };
    let t0 = Instant::now();
    let mut cursor = 0usize;
    let mut sent = 0u64;
    while cursor < events.len() {
        let elapsed = t0.elapsed().as_secs_f64();
        let elapsed_frames = elapsed * engine_rate as f64;
        if let Some(limit) = stop_at
            && elapsed_frames > limit
        {
            break;
        }
        while cursor < events.len() {
            let ev = events[cursor];
            if (ev.sample as f64) > elapsed_frames {
                break;
            }
            playback.send_event(ev.channel, ev.event);
            sent += 1;
            cursor += 1;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    match stop_at {
        Some(_) => println!("stopped after {max_seconds}s: {sent} events sent"),
        None => println!("playback finished: {sent} events sent"),
    }
    // Release every channel's notes, then let the tails decay.
    for ch in 0..16 {
        playback.all_notes_off(ch);
    }
    std::thread::sleep(Duration::from_millis(600));
    playback.stop();
    Ok(())
}
