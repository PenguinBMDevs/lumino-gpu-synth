//! Dumps a sample range. Usage: diag_srange <sample_id> <start_pos> <n>

use lumino_gpu_synth::soundfont::SoundFont;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id: usize = std::env::args().nth(1).unwrap().parse()?;
    let start: usize = std::env::args().nth(2).unwrap().parse()?;
    let n: usize = std::env::args().nth(3).unwrap().parse()?;
    let mut sf = SoundFont::load("assets/test.sf2", 0, 0, false)?;
    let data = sf.resample(id, 64_000);
    let mut peak = 0.0f32;
    for i in start..(start + n).min(data.len()) {
        peak = peak.max(data[i].abs());
        print!("{:.5} ", data[i]);
    }
    println!("\npeak in range = {peak:.5}");
    Ok(())
}
