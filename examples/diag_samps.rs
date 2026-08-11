//! Dumps raw mono samples from two wav files over a window for visual
//! comparison. Usage: diag_samps <a.wav> <b.wav> <start_s> <end_s>
//! Prints up to 96 samples per file (16 per line).

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let a = read_wav(&args[1])?;
    let b = read_wav(&args[2])?;
    let s0 = (args[3].parse::<f64>()? * 64000.0) as usize;
    let s1 = (args[4].parse::<f64>()? * 64000.0) as usize;
    let ach = a.channels as usize;
    let bch = b.channels as usize;
    let n = (s1 - s0).min(96);
    println!("=== {} (ch0) ===", args[1]);
    for i in 0..n {
        let v = a.samples[(s0 + i) * ach];
        if i % 16 == 0 {
            print!("\n{:>6}: ", (s0 + i) as f64 / 64000.0);
        }
        print!("{:.4} ", v);
    }
    println!("\n=== {} (ch0) ===", args[2]);
    for i in 0..n {
        let v = b.samples[(s0 + i) * bch];
        if i % 16 == 0 {
            print!("\n{:>6}: ", (s0 + i) as f64 / 64000.0);
        }
        print!("{:.4} ", v);
    }
    println!();
    Ok(())
}
