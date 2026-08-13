//! Locates samples with absurd magnitude in a WAV and prints their
//! positions (frame, channel, value) to find which blocks escaped the
//! limiter.
//! Usage: diag_wavloc <file.wav> <threshold>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap();
    let thresh: f32 = std::env::args().nth(2).unwrap_or("100.0".into()).parse()?;
    let wav = read_wav(&path)?;
    let mut hits = 0usize;
    for (i, &s) in wav.samples.iter().enumerate() {
        if s.abs() > thresh {
            let frame = i / wav.channels as usize;
            let ch = i % wav.channels as usize;
            if hits < 60 {
                println!("sample {} frame {} ch {} = {:.3e}", i, frame, ch, s);
            }
            hits += 1;
        }
    }
    println!("total > {thresh}: {hits}");
    Ok(())
}
