//! Prints sf2 zone vel2release / envelope params. Usage: diag_velrel <key>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key: u8 = std::env::args().nth(1).unwrap_or("71".into()).parse()?;
    let presets =
        xsynth_soundfonts::sf2::load_soundfont(std::path::Path::new("assets/test.sf2"), 64_000)?;
    for preset in presets.iter() {
        for (reg_idx, region) in preset.regions.iter().enumerate() {
            if region.keyrange.contains(&key) {
                println!(
                    "region[{reg_idx}] key={key} vel2release={} ampeg_release={}",
                    region.ampeg_envelope.ampeg_vel2release, region.ampeg_envelope.ampeg_release
                );
            }
        }
    }
    Ok(())
}
