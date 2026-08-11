//! Counts NaN/Inf/huge samples in a wav. Usage: diag_nan <wav>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let w = read_wav(&std::env::args().nth(1).unwrap())?;
    let mut nan = 0usize;
    let mut inf = 0usize;
    let mut huge = 0usize;
    let mut first: Option<usize> = None;
    for (i, &s) in w.samples.iter().enumerate() {
        if s.is_nan() {
            nan += 1;
            first.get_or_insert(i);
        } else if s.is_infinite() {
            inf += 1;
            first.get_or_insert(i);
        } else if s.abs() > 10.0 {
            huge += 1;
            first.get_or_insert(i);
        }
    }
    println!(
        "nan={nan} inf={inf} huge={huge} first={:?}",
        first.map(|i| i as f64 / 64000.0)
    );
    Ok(())
}
