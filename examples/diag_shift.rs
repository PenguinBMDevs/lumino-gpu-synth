//! Corr of a vs b after shifting a by `lag` samples.
//! Usage: diag_shift <a.wav> <b.wav> <start_s> <dur_s> <lag>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let a = read_wav(&args[1])?;
    let b = read_wav(&args[2])?;
    let s0 = (args[3].parse::<f64>()? * 64000.0) as usize;
    let dur = (args[4].parse::<f64>()? * 64000.0) as usize;
    let lag: i64 = args[5].parse()?;
    let ach = a.channels as usize;
    let bch = b.channels as usize;
    let end = (s0 + dur)
        .min(a.samples.len() / ach)
        .min(b.samples.len() / bch);
    let mut rs = 0.0f64;
    let mut cs = 0.0f64;
    let mut rss = 0.0f64;
    let mut css = 0.0f64;
    let mut cross = 0.0f64;
    let mut n = 0f64;
    for i in s0..end {
        let ai = i as i64 + lag;
        if ai < 0 || ai >= end as i64 {
            continue;
        }
        let av = a.samples[ai as usize * ach] as f64;
        let bv = b.samples[i * bch] as f64;
        rs += av;
        cs += bv;
        rss += av * av;
        css += bv * bv;
        cross += av * bv;
        n += 1.0;
    }
    let denom = ((rss - rs * rs / n) * (css - cs * cs / n)).sqrt().max(1e-9);
    let corr = (cross - rs * cs / n) / denom;
    println!("lag={lag}: corr={corr:.6} (n={n})");
    Ok(())
}
