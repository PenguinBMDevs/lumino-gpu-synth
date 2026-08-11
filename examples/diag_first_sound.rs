//! Finds the first frame where |sample| exceeds a threshold.
//! Usage: diag_first_sound <wav> <start_s> <end_s> <threshold>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let w = read_wav(&args[1])?;
    let s0 = (args[2].parse::<f64>()? * 64000.0) as usize;
    let s1 = (args[3].parse::<f64>()? * 64000.0) as usize;
    let th: f32 = args[4].parse().unwrap_or(1e-4);
    let ch = w.channels as usize;
    let end = s1.min(w.samples.len() / ch);
    for i in s0..end {
        let v = w.samples[i * ch];
        let v2 = w.samples[i * ch + 1];
        if v.abs() > th || v2.abs() > th {
            println!(
                "first sound at frame {i} ({:.6}s) L={:.6} R={:.6}",
                i as f64 / 64000.0,
                v,
                v2
            );
            return Ok(());
        }
    }
    println!("no sound in range");
    Ok(())
}
