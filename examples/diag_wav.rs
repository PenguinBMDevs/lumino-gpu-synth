//! Compares two wav files (reference vs candidate) frame-by-frame.
//! Usage: cargo run --release --example diag_wav -- <ref.wav> <cand.wav>

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (ref_path, cand_path) = if args.len() >= 3 {
        (args[1].clone(), args[2].clone())
    } else {
        ("assets/ref_default.wav".into(), "compare-output.wav".into())
    };

    let r = read_wav(&ref_path)?;
    let c = read_wav(&cand_path)?;
    let r_ch = r.channels as usize;
    let c_ch = c.channels as usize;
    let r_frames = r.samples.len() / r_ch;
    let c_frames = c.samples.len() / c_ch;
    println!(
        "ref: {} frames x {} ch ({}s), cand: {} frames x {} ch ({}s)",
        r_frames,
        r_ch,
        r_frames as f64 / r.sample_rate as f64,
        c_frames,
        c_ch,
        c_frames as f64 / c.sample_rate as f64,
    );

    let n = r_frames.min(c_frames);
    let sr = r.sample_rate.min(c.sample_rate) as usize;
    let mut seg = 0;
    while seg * sr < n.min(20 * sr) {
        let s = seg * sr;
        let e = (s + sr).min(n);
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
            "{:>6.1}s: corr={:+.4} rms(r={:.4} c={:.4}) peak_diff={:.4}",
            seg as f64, corr, rms_r, rms_c, peak_diff
        );
        seg += 1;
    }

    let mut rs = 0.0f64;
    let mut cs = 0.0f64;
    let mut rss = 0.0f64;
    let mut css = 0.0f64;
    let mut cross = 0.0f64;
    for i in 0..n {
        let rv = r.samples[i * r_ch] as f64;
        let cv = c.samples[i * c_ch] as f64;
        rs += rv;
        cs += cv;
        rss += rv * rv;
        css += cv * cv;
        cross += rv * cv;
    }
    let len = n as f64;
    let denom = ((rss - rs * rs / len) * (css - cs * cs / len))
        .sqrt()
        .max(1e-9);
    let corr = (cross - rs * cs / len) / denom;
    println!("GLOBAL corr={:+.6} (first {}s)", corr, n as f64 / 64000.0);
    Ok(())
}
