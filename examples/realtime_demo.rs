//! Realtime playback demo: plays a short melody on the GPU synth through
//! the default audio device (cpal) and exercises the public playback API.
//!
//! Usage:
//! ```text
//! cargo run --release --example realtime_demo
//! ```

use std::time::Duration;

use lumino_gpu_synth::audio::playback::AudioPlayback;
use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    // Query the device first so we can pick a rate the hardware supports.
    let rates = AudioPlayback::device_sample_rates();
    println!("device sample rates: {rates:?}");

    // The engine renders at 48 kHz; `AudioPlayback::start` negotiates with
    // the device and resamples if needed, so the engine rate does not have
    // to match the hardware exactly.
    let sample_rate = 48_000;
    println!("engine sample rate: {sample_rate}");

    let config = SynthConfig {
        sample_rate,
        block_size: 512,
        show_progress: false,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    let mut playback = AudioPlayback::start(synth)?;
    println!(
        "playing on device @ {} Hz (engine renders @ {} Hz)",
        playback.sample_rate(),
        sample_rate
    );

    // A little melody: C4 E4 G4 C5 (quarter-ish notes).
    let melody: [(u8, Duration); 8] = [
        (60, Duration::from_millis(400)),
        (64, Duration::from_millis(400)),
        (67, Duration::from_millis(400)),
        (72, Duration::from_millis(700)),
        (67, Duration::from_millis(400)),
        (64, Duration::from_millis(400)),
        (60, Duration::from_millis(700)),
        (0, Duration::from_millis(200)), // rest
    ];

    // Demonstrate program change + damper on channel 0.
    playback.program_change(0, 0);
    playback.damper(0, true);

    for (key, dur) in &melody {
        if *key > 0 {
            playback.note_on(0, *key, 110);
            println!("note_on  key={key}");
        }
        std::thread::sleep(*dur);
        if *key > 0 {
            playback.note_off(0, *key);
        }
    }

    // Pitch bend demo: bend up a full tone on a sustained note.
    playback.note_on(0, 60, 110);
    for v in (8192..=9000).step_by(64) {
        playback.pitch_bend(0, v);
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(300));
    playback.pitch_bend(0, 8192); // reset
    playback.note_off(0, 60);

    // All notes off + reset, then stop cleanly.
    playback.all_notes_off(0);
    playback.damper(0, false);
    std::thread::sleep(Duration::from_millis(300));
    playback.stop();
    println!("done");
    Ok(())
}
