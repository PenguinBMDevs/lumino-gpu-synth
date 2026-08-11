//! Computes the per-sample gain ratio between two wavs over a window.
//! Usage: diag_gain <a.wav> <b.wav> <start_s> <end_s>
//! Prints min/median/max ratio (a/b) for |b| above a noise floor.

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let a = read_wav(&args[1])?;
    let b = read_wav(&args[2])?;
    let s0 = (args[3].parse::<f64>()? * 64000.0) as usize;
    let s1 = (args[4].parse::<f64>()? * 64000.0) as usize;
    let ach = a.channels as usize;
    let bch = b.channels as usize;
    let mut ratios: Vec<f64> = Vec::new();
    let mut n = 0usize;
    for i in s0..s1.min(a.samples.len() / ach).min(b.samples.len() / bch) {
        let av = a.samples[i * ach] as f64;
        let bv = b.samples[i * bch] as f64;
        if bv.abs() > 1e-4 {
            ratios.push(av / bv);
        }
        n += 1;
    }
    ratios.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let med = ratios[ratios.len() / 2];
    let p10 = ratios[ratios.len() / 10];
    let p90 = ratios[ratios.len() * 9 / 10];
    println!("frames={n} ratio(a/b): p10={p10:.5} median={med:.5} p90={p90:.5}");
    Ok(())
}
