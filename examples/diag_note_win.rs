//! Per-note window comparison: renders nothing itself, compares an already
//! rendered candidate wav against a reference, walking note windows of the
//! given size (default 0.5s) and printing corr + envelope energy per window.
//! Usage: diag_note_win <ref.wav> <cand.wav> [win_seconds]

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (r, c) = (read_wav(&args[1])?, read_wav(&args[2])?);
    let win = args
        .get(3)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(32_000);
    let r_ch = r.channels as usize;
    let c_ch = c.channels as usize;
    let r_frames = r.samples.len() / r_ch;
    let c_frames = c.samples.len() / c_ch;
    let n = r_frames.min(c_frames);
    println!(
        "ref {} frames, cand {} frames, win={}",
        r_frames, c_frames, win
    );

    let mut w = 0;
    while w * win < n {
        let s = w * win;
        let e = (s + win).min(n);
        let mut rs = 0.0f64;
        let mut cs = 0.0f64;
        let mut rss = 0.0f64;
        let mut css = 0.0f64;
        let mut cross = 0.0f64;
        let mut peak_diff = 0.0f64;
        for i in s..e {
            let rv = r.samples[i * r_ch] as f64;
            let cv = c.samples[i * c_ch] as f64;
            rs += rv;
            cs += cv;
            rss += rv * rv;
            css += cv * cv;
            cross += rv * cv;
            peak_diff = peak_diff.max((rv - cv).abs());
        }
        let len = (e - s) as f64;
        let denom = ((rss - rs * rs / len) * (css - cs * cs / len))
            .sqrt()
            .max(1e-9);
        let corr = (cross - rs * cs / len) / denom;
        let rms_r = (rss / len).sqrt();
        let rms_c = (css / len).sqrt();
        println!(
            "{:>5.2}s: corr={:+.4} rms(r={:.4} c={:.4}) peak_diff={:.4}",
            s as f64 / 64000.0,
            corr,
            rms_r,
            rms_c,
            peak_diff
        );
        w += 1;
    }
    Ok(())
}
