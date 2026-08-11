//! Estimates pitch via zero crossings over a window.
//! Usage: diag_freq <wav> <start_s> <dur_s>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let w = read_wav(&args[1])?;
    let s0 = (args[2].parse::<f64>()? * 64000.0) as usize;
    let dur = (args[3].parse::<f64>()? * 64000.0) as usize;
    let ch = w.channels as usize;
    let end = (s0 + dur).min(w.samples.len() / ch);
    let mut crossings = 0usize;
    let mut prev = w.samples[s0 * ch];
    for i in s0..end {
        let v = w.samples[i * ch];
        if prev < 0.0 && v >= 0.0 {
            crossings += 1;
        }
        prev = v;
    }
    let secs = (end - s0) as f64 / 64000.0;
    let freq = crossings as f64 / secs;
    println!("crossings={crossings} secs={secs:.4} freq={freq:.2} Hz");
    Ok(())
}
