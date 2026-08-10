//! Renders `assets/right-example.mid` with `assets/test.sf2` on the GPU and
//! writes the result to `render-output.wav` (32-bit float, 64 kHz stereo).
//!
//! Usage:
//! ```text
//! cargo run --release --example render_example
//! ```

use lumino_gpu_synth::{GpuSynth, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let config = SynthConfig::default();
    println!(
        "initializing GPU synth: {} Hz, {} voices, block {}",
        config.sample_rate, config.max_voices, config.block_size
    );

    let mut synth = GpuSynth::new(config)?;
    println!("adapter: {}", synth.adapter_info().name);

    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    println!("soundfont loaded (bank 0 preset 0)");

    let start = std::time::Instant::now();
    let result = synth.render_midi_file("assets/right-example.mid")?;
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
