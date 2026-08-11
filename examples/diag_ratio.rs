//! Prints the sample ratio of a vs b at a window (a shifted by lag).
//! Usage: diag_ratio <a.wav> <b.wav> <start_s> <dur_s> <lag>

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
    let mut ratios: Vec<f64> = Vec::new();
    for i in s0..end {
        let ai = i as i64 + lag;
        if ai < 0 || ai >= end as i64 {
            continue;
        }
        let av = a.samples[ai as usize * ach] as f64;
        let bv = b.samples[i * bch] as f64;
        if bv.abs() > 0.005 {
            ratios.push(av / bv);
        }
    }
    ratios.sort_by(|x, y| x.partial_cmp(y).unwrap());
    if ratios.is_empty() {
        println!("no samples above floor");
        return Ok(());
    }
    let med = ratios[ratios.len() / 2];
    let p10 = ratios[ratios.len() / 10];
    let p90 = ratios[ratios.len() * 9 / 10];
    println!(
        "window {s0}-{end}: ratio a/b (lag={lag}): p10={p10:.4} median={med:.4} p90={p90:.4} n={}",
        ratios.len()
    );
    Ok(())
}
