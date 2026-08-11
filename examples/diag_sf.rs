//! Diagnostic: prints the envelope parameters of the soundfont zones for a
//! given key, to verify release lengths.

use lumino_gpu_synth::soundfont::SoundFont;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let sf = SoundFont::load("assets/test.sf2", 0, 0, false)?;
    println!("samples: {}", sf.sample_count());

    for key in [60u8, 61, 64, 72] {
        for vel in [100u8] {
            let ids = sf.zones_at(key, vel);
            println!("key={key} vel={vel}: {} zones", ids.len());
            for &zid in ids.iter().take(4) {
                let z = sf.zone(zid);
                let e = &z.envelope;
                println!(
                    "  zone {zid}: attack={:.3}s decay={:.3}s sustain={:.3} release={:.3}s delay={:.3}s hold={:.3}s start={:.3}",
                    e.attack,
                    e.decay,
                    e.sustain_percent,
                    e.release,
                    e.delay,
                    e.hold,
                    e.start_percent
                );
                println!(
                    "    sample_id={} native_rate={} loop={:?} cutoff={:?}",
                    z.sample_id, z.native_rate, z.loop_mode, z.cutoff
                );
                println!(
                    "    volume={:.6} pan={:.4} speed_mult={:.6} offset={} sample_end={}",
                    z.volume, z.pan, z.speed_mult, z.offset, z.sample_end
                );
            }
        }
    }
    Ok(())
}
