//! Finds the lag (in samples) with maximum cross-correlation between two
//! wavs over a window. Usage: diag_lag <a.wav> <b.wav> <start_s> <dur_s>
//! Positive lag means a is delayed relative to b.

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let a = read_wav(&args[1])?;
    let b = read_wav(&args[2])?;
    let s0 = (args[3].parse::<f64>()? * 64000.0) as usize;
    let dur = (args[4].parse::<f64>()? * 64000.0) as usize;
    let ach = a.channels as usize;
    let bch = b.channels as usize;
    let end = (s0 + dur)
        .min(a.samples.len() / ach)
        .min(b.samples.len() / bch);
    let n = end - s0;
    let mut best_lag = 0i64;
    let mut best = -1.0f64;
    for lag in -64..=64i64 {
        let mut cross = 0.0f64;
        let mut asq = 0.0f64;
        let mut bsq = 0.0f64;
        for i in 0..n {
            let ai = (i as i64) + lag;
            if ai < 0 || ai >= n as i64 {
                continue;
            }
            let av = a.samples[(s0 + i) * ach] as f64;
            let bv = b.samples[(s0 + ai as usize) * bch] as f64;
            cross += av * bv;
            asq += av * av;
            bsq += bv * bv;
        }
        let c = cross / (asq * bsq).sqrt().max(1e-9);
        if c > best {
            best = c;
            best_lag = lag;
        }
    }
    println!("best lag = {best_lag} samples, corr = {best:.6}");
    Ok(())
}
