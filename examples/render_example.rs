//! Renders a MIDI with `assets/test.sf2` on the GPU and writes the result to
//! `render-output.wav` (32-bit float, 64 kHz stereo).
//!
//! Usage:
//! ```text
//! cargo run --release --example render_example -- [midi] [seconds]
//! ```
//! `seconds` limits the render to the first N seconds (0 = whole file).

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<String> = std::env::args().collect();
    let midi_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "assets/right-example.mid".to_string());
    let max_seconds = args
        .get(2)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let config = SynthConfig {
        block_size: 2048,
        show_progress: true,
        envelope_curves: if std::env::var("LUMINO_LERP_ENV").is_ok() {
            lumino_gpu_synth::synth::dsp::EnvelopeCurveConfig {
                attack_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
                decay_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
                release_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
            }
        } else {
            lumino_gpu_synth::synth::dsp::EnvelopeCurveConfig::default()
        },
        ..SynthConfig::default()
    };
    println!(
        "initializing GPU synth: {} Hz, {} voices, block {}",
        config.sample_rate, config.max_voices, config.block_size
    );

    let mut synth = GpuSynth::new(config)?;
    println!("adapter: {}", synth.adapter_info().name);

    let t_sf = std::time::Instant::now();
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    println!(
        "soundfont loaded (bank 0 preset 0) in {:.2?}",
        t_sf.elapsed()
    );

    let start = std::time::Instant::now();
    let result = if max_seconds > 0.0 {
        let frames = (max_seconds * 64_000.0) as u64;
        println!("rendering first {frames} frames ({max_seconds}s)...");
        synth.render_midi_frames(&midi_path, frames)?
    } else {
        println!("rendering whole file...");
        synth.render_midi_file(&midi_path)?
    };
    let elapsed = start.elapsed();

    println!(
        "rendered {} frames ({} s) in {:.2?} -> {:.2}x realtime",
        result.frames,
        result.frames as f64 / result.sample_rate as f64,
        elapsed,
        result.frames as f64 / (elapsed.as_secs_f64() * result.sample_rate as f64)
    );

    lumino_gpu_synth::audio::wav::write_f32_wav(
        "render-output.wav",
        &result.samples,
        result.sample_rate,
    )?;
    println!("wrote render-output.wav");

    Ok(())
}
