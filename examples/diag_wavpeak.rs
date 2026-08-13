//! Reads a WAV and reports its peak level, to check whether reference
//! renders clip too.
//! Usage: cargo run --release --example diag_wavpeak <file.wav>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/render-output-baseline.wav".into());
    let wav = read_wav(&path)?;
    let mut peak = 0.0f32;
    let mut over1 = 0usize;
    let mut sum_sq = 0.0f64;
    for &s in &wav.samples {
        let a = s.abs();
        peak = peak.max(a);
        if a > 1.0 {
            over1 += 1;
        }
        sum_sq += s as f64 * s as f64;
    }
    let rms = (sum_sq / wav.samples.len().max(1) as f64).sqrt();
    println!(
        "{path}: {} Hz {} ch {} samples peak={:.3} over1.0={} rms={:.4}",
        wav.sample_rate,
        wav.channels,
        wav.samples.len(),
        peak,
        over1,
        rms
    );
    Ok(())
}
