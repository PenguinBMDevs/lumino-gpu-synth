//! Dumps sf2 region params for the zones of a key: root_key vs pitch_keycenter.
//! Usage: diag_region [key]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key: i8 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    let key = key as u8;
    let presets =
        xsynth_soundfonts::sf2::load_soundfont(std::path::Path::new("assets/test.sf2"), 44_100)?;
    for preset in presets.iter() {
        for (reg_idx, region) in preset.regions.iter().enumerate() {
            if region.keyrange.contains(&key) {
                println!(
                    "preset{} region[{reg_idx}] keyrange={:?} root_key={} loop_start={} loop_end={} loop_mode={:?} sample_rate={} sample_len={}",
                    preset.preset,
                    region.keyrange,
                    region.root_key,
                    region.scale_tuning,
                    region.fine_tune,
                    region.coarse_tune,
                    region.sample_rate,
                    region.sample[0].len()
                );
            }
        }
    }
    Ok(())
}
