//! Renders `assets/right-example.mid` with `assets/test.sf2` on the GPU and
//! compares the waveform against the reference `assets/right-example.wav`.
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
        use_effects: false,
        envelope_curves: lumino_gpu_synth::synth::dsp::EnvelopeCurveConfig {
            attack_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
            decay_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
            release_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
        },
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;

    println!("rendering...");
    let result = synth.render_midi_file("assets/right-example.mid")?;

    println!("reading reference...");
    let reference = read_wav("assets/right-example.wav")?;
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

    // Per-note segments: notes start at 2.0 s, one every 0.5 s, 0.5 s long.
    let sr = result.sample_rate;
    let segments: Vec<(usize, usize)> = (0..5)
        .map(|i| {
            let start = ((2.0 + i as f32 * 0.5) * sr as f32) as usize;
            let end = start + (sr / 2) as usize;
            (start, end)
        })
        .collect();

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
