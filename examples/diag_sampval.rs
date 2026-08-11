//! Prints resampled sample values around a given frame position, to compare
//! with rendered output. Usage: diag_sampval <sample_id> <native_pos_s>
//! Prints sample values at output-rate positions around native_pos_s.

use lumino_gpu_synth::soundfont::SoundFont;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(86);
    let at_s: f64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.05);

    let mut sf = SoundFont::load("assets/test.sf2", 0, 0, false)?;
    let data = sf.resample(id, 64_000);
    let pos = (at_s * 64000.0) as usize;
    println!("sample {id} resampled len={} pos={pos}:", data.len());
    for p in (pos.saturating_sub(4))..(pos + 5) {
        println!("  data[{p}] = {:.6}", data[p]);
    }
    Ok(())
}
