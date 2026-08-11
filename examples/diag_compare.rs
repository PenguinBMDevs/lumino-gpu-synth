//! Diagnostic: compares our rendered output against the reference in time
//! slices to locate where the signals diverge (timebase error, content
//! error, or both). Temporary tool.

use lumino_gpu_synth::audio::wav::read_wav;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let ours = read_wav("compare-output.wav")?;
    let refr = read_wav("assets/[Zackson_Y]Kentite.wav")?;
    let sr = 64_000usize;
    let frames = ours.samples.len() / 2;
    let rframes = refr.samples.len() / 2;
    println!(
        "ours: {frames} frames ({:.3}s), ref: {rframes} frames ({:.3}s)",
        frames as f64 / sr as f64,
        rframes as f64 / sr as f64
    );

    // Peak per second (L+R combined), first 30 s.
    println!("--- per-second peak (ours | ref) ---");
    for s in 0..30 {
        let o = slice_peak(&ours.samples, s * sr, sr);
        let r = slice_peak(&refr.samples, s * sr, sr);
        println!("{s:3}s: ours={o:.4} ref={r:.4}");
    }

    // Normalized cross-correlation over sliding 1s windows at a few fixed
    // offsets, to find where alignment holds.
    println!("--- correlation per 2s window (offset: -0.5s -0.25s 0 +0.25s +0.5s) ---");
    let win = sr * 2;
    let step = sr / 2;
    let offsets = [
        -(sr as i64) / 2,
        -(sr as i64) / 4,
        0i64,
        (sr as i64) / 4,
        (sr as i64) / 2,
    ];
    let mut w = 0usize;
    while w + win <= frames && w + win <= rframes {
        let cs: Vec<f32> = offsets
            .iter()
            .map(|&off| corr_at(&ours.samples, &refr.samples, w, win, off))
            .collect();
        println!(
            "{:6.2}s: corr={:.3} {:.3} {:.3} {:.3} {:.3}",
            w as f64 / sr as f64,
            cs[0],
            cs[1],
            cs[2],
            cs[3],
            cs[4]
        );
        w += step;
        if w > 30 * sr {
            break;
        }
    }
    Ok(())
}

fn slice_peak(samples: &[f32], start: usize, len: usize) -> f32 {
    samples[start * 2..(start + len).min(samples.len() / 2) * 2]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

fn corr_at(a: &[f32], b: &[f32], start: usize, win: usize, off: i64) -> f32 {
    let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut n = 0usize;
    for i in 0..win {
        let ai = start + i;
        let bi = (start as i64 + i as i64 + off) as usize;
        if ai * 2 + 1 >= a.len() || bi * 2 + 1 >= b.len() {
            continue;
        }
        let (xa, ya) = (a[ai * 2] as f64, a[ai * 2 + 1] as f64);
        let (xb, yb) = (b[bi * 2] as f64, b[bi * 2 + 1] as f64);
        let (x, y) = (xa + ya, xb + yb);
        sa += x;
        sb += y;
        saa += x * x;
        sbb += y * y;
        sab += x * y;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    let denom = ((n as f64 * saa - sa * sa) * (n as f64 * sbb - sb * sb)).sqrt();
    if denom > 1e-12 {
        ((n as f64 * sab - sa * sb) / denom) as f32
    } else {
        0.0
    }
}
