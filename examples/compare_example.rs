//! Renders `assets/right-example.mid` with `assets/test.sf2` on the GPU and
//! compares the waveform against the reference `assets/ref_xsynth_default.wav`
//! (rendered with the XSynth offline renderer using its default settings).
//!
//! Prints overall correlation / RMS error / peak error plus per-note segment
//! metrics. The acceptance target is a correlation >= 0.999 and a normalized
//! RMS error < 1%.
//!
//! Usage:
//! ```text
//! cargo run --release --example compare_example
//! ```

use lumino_gpu_synth::audio::wav::read_wav;
use lumino_gpu_synth::compare::{compare, format_report};
use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let config = SynthConfig {
        // Same parameters as the XSynth renderer defaults: envelope curves
        // attack=Exponential, decay/release=Linear, effects off for the
        // synthetic comparison.
        use_effects: false,
        max_voices: 16384,
        ..SynthConfig::default()
    };

    println!("rendering...");
    // Hard 60 s deadline for the whole render; the process panics if the
    // GPU render does not finish in time.
    let result = {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut synth = GpuSynth::new(config).expect("gpu synth");
            synth
                .load_soundfont("assets/test.sf2", 0, 0)
                .expect("soundfont");
            let r = synth.render_midi_file("assets/right-example.mid");
            let _ = tx.send(r);
        });
        match rx.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(e),
            Err(_) => panic!("RENDER TIMED OUT after 60s"),
        }
    };

    println!("reading reference...");
    let reference = read_wav("assets/ref_xsynth_default.wav")?;
    println!(
        "reference: {} Hz, {} ch, {:.3} s",
        reference.sample_rate,
        reference.channels,
        reference.samples.len() as f64 / reference.sample_rate as f64 / 2.0
    );

    if reference.sample_rate != result.sample_rate {
        println!(
            "WARNING: sample rate mismatch (reference {}, rendered {})",
            reference.sample_rate, result.sample_rate
        );
    }

    // Whole-file comparison (no fixed note segments for this large MIDI).
    let segments: Vec<(usize, usize)> = Vec::new();

    let report = compare(
        &reference.samples,
        &result.samples,
        reference.channels as usize,
        &segments,
    );
    println!("{}", format_report(&report));

    // Acceptance check.
    let ok = report.correlation >= 0.999 && report.rms_error < 0.01;
    println!(
        "acceptance (corr >= 0.999 && rms < 0.01): {}",
        if ok { "PASS" } else { "FAIL" }
    );

    // Also write the rendered audio for inspection.
    lumino_gpu_synth::audio::wav::write_f32_wav(
        "compare-output.wav",
        &result.samples,
        result.sample_rate,
    )?;
    println!("wrote compare-output.wav");

    Ok(())
}
