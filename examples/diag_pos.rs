//! Correlates a wav segment against a resampled sample to find the playback
//! position. Usage: diag_pos <wav> <sample_id> <start_s> <dur_s>
//! Prints the sample position with max correlation (assumes amp=1, no env).

use lumino_gpu_synth::audio::wav::read_wav;
use lumino_gpu_synth::soundfont::SoundFont;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let w = read_wav(&args[1])?;
    let sid: usize = args[2].parse()?;
    let s0 = (args[3].parse::<f64>()? * 64000.0) as usize;
    let dur = (args[4].parse::<f64>()? * 64000.0) as usize;
    let ch = w.channels as usize;
    let mut sf = SoundFont::load("assets/test.sf2", 0, 0, false)?;
    let data = sf.resample(sid, 64_000);

    let n = dur.min(data.len()).min(w.samples.len() / ch - s0);
    // Try positions in the sample that could correspond to s0*speed.
    let center = (s0 as f64) as i64;
    let mut best = -1.0f64;
    let mut best_pos = 0usize;
    for pos in (center - 2000).max(0)..(center + 2000).min(data.len() as i64 - n as i64) {
        let mut cross = 0.0f64;
        let mut asq = 0.0f64;
        let mut bsq = 0.0f64;
        for i in 0..n {
            let a = w.samples[(s0 + i) * ch] as f64;
            let b = data[pos as usize + i] as f64;
            cross += a * b;
            asq += a * a;
            bsq += b * b;
        }
        let c = cross / (asq * bsq).sqrt().max(1e-9);
        if c > best {
            best = c;
            best_pos = pos as usize;
        }
    }
    println!(
        "best position = {best_pos} ({:.4}s) corr={best:.4} (expected {s0})",
        best_pos as f64 / 64000.0
    );
    Ok(())
}
