//! Realtime playback of a MIDI file on the GPU synth through the default
//! audio device (cpal), with a live status line mirroring XSynth's
//! `realtime/examples/midi.rs`:
//!
//! ```text
//! Voice Count: 24   Buffer: 8192   Render time: 0.42 (underruns: 0)
//! ```
//!
//! The parsed event stream is replayed against a wall-clock timeline at the
//! engine's sample rate, so the audio plays in real time.
//!
//! Usage:
//! ```text
//! cargo run --release --example realtime_midi -- <midi file> [seconds]
//! ```
//! `seconds` (optional) stops after that many seconds of playback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    //
    // A larger block amortizes the per-block fixed costs (GPU dispatch,
    // readback poll, CPU upload) and widens the realtime budget: 2048 frames
    // at 48 kHz is 42.7 ms per block, so even a heavy 4000-voice dense
    // section (a few ms of GPU per block) leaves ample headroom and the
    // queue never starves.
    let sample_rate = 48_000;
    let config = SynthConfig {
        sample_rate,
        block_size: 2048,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    // Pre-warm the GPU sample cache with everything this MIDI will use, so
    // the render loop never stalls on a lazily-resampled sample mid-play
    // (that would empty the audio queue and crackle).
    let prewarm_t0 = std::time::Instant::now();
    synth.prewarm_midi_file(&midi_path)?;
    println!(
        "prewarmed samples in {:.1}s",
        prewarm_t0.elapsed().as_secs_f64()
    );

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

    // Live status thread: same stats as XSynth's realtime example.
    // `print!` buffers, so flush every tick: otherwise the `\r` lines
    // accumulate and burst out all at once (and a full stdout pipe blocks).
    let running = Arc::new(AtomicBool::new(true));
    let stats = playback.stats();
    let stats_running = running.clone();
    let stats_thread = std::thread::spawn(move || {
        use std::io::Write;
        let mut last_underruns = 0u64;
        while stats_running.load(Ordering::Relaxed) {
            let underruns = stats.underruns();
            let delta = underruns.saturating_sub(last_underruns);
            last_underruns = underruns;
            print!(
                "\rVoice Count: {}\tBuffer: {}\tRender time: {:.2} (underruns this window: {delta})",
                stats.voice_count(),
                stats.last_samples_after_read(),
                stats.average_renderer_load(),
            );
            let _ = std::io::stdout().flush();
            std::thread::sleep(Duration::from_millis(100));
        }
    });

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

    running.store(false, Ordering::Relaxed);
    let _ = stats_thread.join();
    println!();

    match stop_at {
        Some(_) => println!("stopped after {max_seconds}s: {sent} events sent"),
        None => println!("playback finished: {sent} events sent"),
    }
    let final_stats = playback.stats();
    println!(
        "final stats: avg render load {:.2}, total underruns {}",
        final_stats.average_renderer_load(),
        final_stats.underruns()
    );
    // Release every channel's notes, then let the tails decay.
    for ch in 0..16 {
        playback.all_notes_off(ch);
    }
    std::thread::sleep(Duration::from_millis(600));
    playback.stop();
    Ok(())
}
