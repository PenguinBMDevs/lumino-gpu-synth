//! Streaming offline render — zero MIDI event Vec, zero full-sample Vec.
//!
//! The MIDI file is never loaded as a `Vec<TimedEvent>` (heap-merged
//! `MidiStream` yields one event at a time) and audio is flushed
//! block-by-block through `WavStreamWriter`. Peak memory is
//! `O(tracks + block)` instead of `O(events + samples)`.
//!
//! Usage:
//! ```text
//! cargo run --release --example render_example -- [midi] [seconds] [wav_out]
//! ```
//! `seconds` limits the render to the first N seconds (0 = whole file).
//! `wav_out` defaults to `render-output.wav`.

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
    let wav_path = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "render-output.wav".to_string());

    let config = SynthConfig {
        block_size: std::env::var("LUMINO_BLOCK")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2048),
        max_voices: std::env::var("LUMINO_VOICES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(SynthConfig::default().max_voices),
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
        "initializing GPU synth: {} Hz, {} voices, block {} — streaming mode",
        config.sample_rate, config.max_voices, config.block_size
    );
    println!("midi: {midi_path} -> wav: {wav_path} (streaming, no MIDI Vec)");

    let mut synth = GpuSynth::new(config.clone())?;
    println!("adapter: {}", synth.adapter_info().name);

    let t_sf = std::time::Instant::now();
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    println!(
        "soundfont loaded (bank 0 preset 0) in {:.2?}",
        t_sf.elapsed()
    );

    let start = std::time::Instant::now();
    let result = if max_seconds > 0.0 {
        let frames = (max_seconds * config.sample_rate as f64) as u64;
        println!("streaming first {frames} frames ({max_seconds}s) ...");
        synth.render_midi_to_wav_streaming(&midi_path, &wav_path, Some(frames))?
    } else {
        println!(
            "streaming whole file ... (MidiStream heap-merge, WavStreamWriter flush per block)"
        );
        synth.render_midi_file_to_wav_streaming(&midi_path, &wav_path)?
    };
    let elapsed = start.elapsed();

    println!(
        "streamed {} frames ({} s) in {:.2?} -> {:.2}x realtime",
        result.frames,
        result.frames as f64 / result.sample_rate as f64,
        elapsed,
        result.frames as f64 / (elapsed.as_secs_f64() * result.sample_rate as f64)
    );
    println!("wrote {wav_path} (streaming, peak O(block) not O(samples))");

    Ok(())
}
