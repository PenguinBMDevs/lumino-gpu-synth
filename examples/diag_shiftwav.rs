//! Shifts a wav by `lag` samples (positive = later) and writes shifted.wav.
//! Usage: diag_shiftwav <in.wav> <lag> <out.wav>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let w = read_wav(&args[1])?;
    let lag: i64 = args[2].parse()?;
    let ch = w.channels as usize;
    let frames = w.samples.len() / ch;
    let mut out = vec![0.0f32; w.samples.len()];
    for f in 0..frames {
        let src = (f as i64) - lag; // shift output later by lag
        for c in 0..ch {
            let v = if src >= 0 && src < frames as i64 {
                w.samples[src as usize * ch + c]
            } else {
                0.0
            };
            out[f * ch + c] = v;
        }
    }
    lumino_gpu_synth::audio::wav::write_f32_wav(&args[3], &out, w.sample_rate)?;
    println!("wrote {} (lag={lag})", args[3]);
    Ok(())
}
