//! Gain ratio of a vs b over fixed-size windows.
//! Usage: diag_gainwin <a.wav> <b.wav> <win_s>
//! ratio = a/b using window rms (sign-insensitive).

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let a = read_wav(&args[1])?;
    let b = read_wav(&args[2])?;
    let win = (args[3].parse::<f64>()? * 64000.0) as usize;
    let ach = a.channels as usize;
    let bch = b.channels as usize;
    let n = (a.samples.len() / ach).min(b.samples.len() / bch);
    let mut w = 0;
    while w * win < n {
        let s = w * win;
        let e = (s + win).min(n);
        let mut asq = 0.0f64;
        let mut bsq = 0.0f64;
        for i in s..e {
            let av = a.samples[i * ach] as f64;
            let bv = b.samples[i * bch] as f64;
            asq += av * av;
            bsq += bv * bv;
        }
        let len = (e - s) as f64;
        let arms = (asq / len).sqrt();
        let brms = (bsq / len).sqrt();
        let ratio = if brms > 1e-6 { arms / brms } else { 0.0 };
        println!(
            "{:>5.3}s: rms_a={:.5} rms_b={:.5} ratio={:.4}",
            s as f64 / 64000.0,
            arms,
            brms,
            ratio
        );
        w += 1;
    }
    Ok(())
}
