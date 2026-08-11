//! Compares the same sample loaded at two different `load_soundfont` sample
//! rates (44100 vs 64000), to see whether the loader resamples the data.
//! Usage: diag_sampload <sample_id>

use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(86);

    let a =
        xsynth_soundfonts::sf2::load_soundfont(std::path::Path::new("assets/test.sf2"), 44_100)?;
    let b =
        xsynth_soundfonts::sf2::load_soundfont(std::path::Path::new("assets/test.sf2"), 64_000)?;

    // Collect samples in preset order by walking all regions.
    let mut find = |presets: &[xsynth_soundfonts::sf2::Sf2Preset], label: &str| {
        let mut found: Option<(u32, usize)> = None;
        let mut n = 0usize;
        'outer: for p in presets {
            for r in &p.regions {
                for s in r.sample.iter() {
                    if n == id {
                        found = Some((r.sample_rate, s.len()));
                        println!(
                            "{label}: region sample_rate={} len={}",
                            r.sample_rate,
                            s.len()
                        );
                        for i in 0..24 {
                            print!("{:.5} ", s[i]);
                        }
                        println!();
                        break 'outer;
                    }
                    n += 1;
                }
            }
        }
        if found.is_none() {
            println!("{label}: sample id {id} not found (max {n})");
        }
    };
    find(&a, "load@44100");
    find(&b, "load@64000");

    // Re-resample the 44100 data to 64000 and compare with the loader's own.
    let mut data441: Option<Arc<[f32]>> = None;
    let mut data640: Option<Arc<[f32]>> = None;
    let mut n = 0usize;
    for p in &a {
        for r in &p.regions {
            for s in r.sample.iter() {
                if n == id {
                    data441 = Some(s.clone());
                }
                n += 1;
            }
        }
    }
    let mut n = 0usize;
    for p in &b {
        for r in &p.regions {
            for s in r.sample.iter() {
                if n == id {
                    data640 = Some(s.clone());
                }
                n += 1;
            }
        }
    }
    if let (Some(raw), Some(loader)) = (data441, data640) {
        let mine = xsynth_soundfonts::resample::resample_vec(raw.to_vec(), 44_100.0, 64_000.0);
        println!("resample_vec(44100->64000): len={}", mine.len());
        let n = mine.len().min(loader.len()).min(24);
        let mut max_diff = 0.0f32;
        for i in 0..n {
            print!("{:.5} ", mine[i]);
            max_diff = max_diff.max((mine[i] - loader[i]).abs());
        }
        println!();
        println!("max_diff (first {n}) = {max_diff:.6}");
        let cmp = mine.len() == loader.len();
        println!(
            "lengths match: {cmp} (mine {}, loader {})",
            mine.len(),
            loader.len()
        );
    }
    Ok(())
}
