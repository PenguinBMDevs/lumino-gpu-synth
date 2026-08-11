//! Prints the exact speed_mult for a key (all zones) and the underlying cents.
//! Usage: diag_speed <key> <vel>

use lumino_gpu_synth::soundfont::SoundFont;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key: u8 = std::env::args().nth(1).unwrap_or("64".into()).parse()?;
    let vel: u8 = std::env::args().nth(2).unwrap_or("100".into()).parse()?;
    let sf = SoundFont::load("assets/test.sf2", 0, 0, false)?;
    for &zid in sf.zones_at(key, vel) {
        let z = sf.zone(zid);
        println!(
            "key={key} vel={vel} zone={zid} speed_mult={:.7} pan={} sample_id={}",
            z.speed_mult, z.pan, z.sample_id
        );
    }
    Ok(())
}
