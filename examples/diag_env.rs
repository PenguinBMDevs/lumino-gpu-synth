//! Prints the peak amplitude envelope of a wav over a time range, to inspect
//! attack/release shapes. Usage: diag_env <file.wav> <start_s> <end_s>
//! Prints one peak per ~5 ms.

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let w = read_wav(&args[1])?;
    let s0 = args[2].parse::<f64>()?;
    let s1 = args[3].parse::<f64>()?;
    let sr = w.sample_rate as usize;
    let ch = w.channels as usize;
    let i0 = (s0 * sr as f64) as usize;
    let i1 = (s1 * sr as f64) as usize;
    let step = sr / 200; // 5 ms
    let mut i = i0;
    while i < i1.min(w.samples.len() / ch) {
        let e = (i + step).min(i1).min(w.samples.len() / ch);
        let mut peak = 0.0f32;
        for j in i..e {
            peak = peak.max(w.samples[j * ch].abs());
        }
        println!("{:.4}s: peak={:.6}", i as f64 / sr as f64, peak);
        i = e;
    }
    Ok(())
}
