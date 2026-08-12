//! Compares two WAVs and reports correlation (for audio-regression checks).
//! Usage: cargo run --release --example cmp_wav -- <a.wav> <b.wav>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let a = read_wav(&args[1]).expect("read a");
    let b = read_wav(&args[2]).expect("read b");
    println!("a: {} frames @ {}Hz", a.samples.len(), a.sample_rate);
    println!("b: {} frames @ {}Hz", b.samples.len(), b.sample_rate);
    let n = a.samples.len().min(b.samples.len());
    let (mut sa, mut sb, mut sa2, mut sb2, mut sab) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let stride = (n / 100_000).max(1);
    let mut count = 0usize;
    for i in (0..n).step_by(stride) {
        let x = a.samples[i] as f64;
        let y = b.samples[i] as f64;
        sa += x;
        sb += y;
        sa2 += x * x;
        sb2 += y * y;
        sab += x * y;
        count += 1;
    }
    if count == 0 {
        println!("empty");
        return;
    }
    let denom = ((count as f64 * sa2 - sa * sa) * (count as f64 * sb2 - sb * sb)).sqrt();
    let corr = if denom > 0.0 {
        (count as f64 * sab - sa * sb) / denom
    } else {
        0.0
    };
    let mut maxdiff = 0.0f32;
    for i in 0..n {
        maxdiff = maxdiff.max((a.samples[i] - b.samples[i]).abs());
    }
    println!("samples compared: {count} (stride {stride})");
    println!("corr: {corr:.6}");
    println!("max abs diff (full {n} samples): {maxdiff:.6}");
}
