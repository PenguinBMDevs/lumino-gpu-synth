//! Prints the GPU envelope stages for a key. Usage: diag_envstages <key> <vel>

use lumino_gpu_synth::soundfont::SoundFont;
use lumino_gpu_synth::synth::dsp::to_gpu_stages;
use lumino_gpu_synth::synth::voices::build_voice;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key: u8 = std::env::args().nth(1).unwrap_or("64".into()).parse()?;
    let vel: u8 = std::env::args().nth(2).unwrap_or("100".into()).parse()?;
    let sf = SoundFont::load("assets/test.sf2", 0, 0, false)?;
    for &zid in sf.zones_at(key, vel) {
        let z = sf.zone(zid);
        println!("zone {zid}: env={:?}", z.envelope);
        let v = build_voice(
            &sf,
            zid,
            key,
            vel,
            0,
            0,
            64_000,
            1.0,
            None,
            None,
            lumino_gpu_synth::synth::dsp::EnvelopeCurveConfig {
                attack_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
                decay_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
                release_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
            },
        );
        if let Some(v) = v {
            println!("  stages:");
            for (i, s) in v.env_stages.iter().enumerate() {
                println!(
                    "    [{i}] kind={} target={} duration={} ({}s)",
                    s.kind,
                    s.target,
                    s.duration,
                    s.duration as f64 / 64000.0
                );
            }
            println!(
                "  release_idx={} finished_idx={}",
                v.release_idx, v.finished_idx
            );
        }
    }
    Ok(())
}
