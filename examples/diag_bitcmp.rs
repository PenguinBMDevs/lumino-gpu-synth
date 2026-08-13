//! Bit-identical check between two WAV files.
//! Usage: diag_bitcmp <a.wav> <b.wav>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a = read_wav(std::env::args().nth(1).unwrap())?;
    let b = read_wav(std::env::args().nth(2).unwrap())?;
    let n = a.samples.len().min(b.samples.len());
    let mut diffs = 0usize;
    let mut max_err = 0.0f32;
    for i in 0..n {
        let e = (a.samples[i] - b.samples[i]).abs();
        if e > 0.0 {
            diffs += 1;
            max_err = max_err.max(e);
        }
    }
    println!(
        "len a={} b={} compared={} bit-diffs={} max_err={:.3e} {}",
        a.samples.len(),
        b.samples.len(),
        n,
        diffs,
        max_err,
        if diffs == 0 {
            "BIT-IDENTICAL"
        } else {
            "DIFFERS"
        }
    );
    Ok(())
}
