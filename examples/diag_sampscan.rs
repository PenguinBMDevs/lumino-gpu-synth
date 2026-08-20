//! Scans all resampled sample data for huge values (>100) - the suspected
//! source of the single-frame garbage in the GPU mix output.
//! Usage: diag_sampscan

use lumino_gpu_synth::soundfont::SoundFont;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sf = SoundFont::load("assets/test.sf2", 0, 0, false)?;
    let n = sf.sample_count();
    let mut found = 0usize;
    for id in 0..n {
        let data = sf.resample(id, 64_000);
        for (i, &v) in data.iter().enumerate() {
            if !v.is_finite() || v.abs() > 100.0 {
                println!("sample {id} pos {i}: {v}");
                found += 1;
                if found > 10 {
                    println!("...");
                    return Ok(());
                }
            }
        }
    }
    println!("scanned {n} samples, huge/nonfinite values: {found}");
    Ok(())
}
