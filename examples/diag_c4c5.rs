//! Renders assets/C4-C5.mid with a configurable envelope curve and compares
//! against assets/C4-C5.wav (the user's XSynth reference).
//! Usage: diag_c4c5 [lerp|concave]

use lumino_gpu_synth::audio::wav::read_wav;
use lumino_gpu_synth::compare::{compare, format_report};
use lumino_gpu_synth::synth::dsp::{CurveKind, EnvelopeCurveConfig};
use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "lerp".into());
    let envelope_curves = match mode.as_str() {
        "concave" => EnvelopeCurveConfig::default(), // attack Exp, decay/rel Linear
        _ => EnvelopeCurveConfig {
            attack_curve: CurveKind::Exponential,
            decay_curve: CurveKind::Exponential, // LERP in amp
            release_curve: CurveKind::Exponential,
        },
    };
    println!("mode={mode} envelope={envelope_curves:?}");

    let config = SynthConfig {
        use_effects: false,
        max_voices: 16384,
        envelope_curves,
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    let result = synth.render_midi_file("assets/C4-C5.mid")?;
    println!(
        "rendered {} frames ({:.3}s)",
        result.samples.len() / 2,
        result.samples.len() as f64 / 64000.0 / 2.0
    );

    let reference = read_wav("assets/C4-C5.wav")?;
    let report = compare(
        &reference.samples,
        &result.samples,
        reference.channels as usize,
        &[],
    );
    println!("{}", format_report(&report));

    let out = format!("c4c5-{mode}.wav");
    lumino_gpu_synth::audio::wav::write_f32_wav(&out, &result.samples, result.sample_rate)?;
    println!("wrote {out}");
    Ok(())
}
