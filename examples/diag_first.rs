//! Renders only the first `N` seconds of the MIDI and compares against the
//! reference window. Usage: cargo run --release --example diag_first -- <secs>

use lumino_gpu_synth::audio::wav::read_wav;
use lumino_gpu_synth::compare::{compare, format_report};
use lumino_gpu_synth::midi::MidiEvent;
use lumino_gpu_synth::{GpuSynth, MidiFile, SynthConfig};

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let secs: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);
    let cutoff = (secs * 64_000.0) as u64;

    let midi = MidiFile::load("assets/right-example.mid", 64_000)?;
    let mut synth = GpuSynth::new(SynthConfig::default())?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    for ev in &midi.sequence.events {
        if (ev.sample as u64) >= cutoff {
            break;
        }
        match ev.event() {
            MidiEvent::NoteOn { key, vel } => synth.note_on(ev.channel(), key, vel),
            MidiEvent::NoteOff { key } => synth.note_off(ev.channel(), key),
            MidiEvent::ControlChange { controller, value } => {
                synth.control_change(ev.channel(), controller, value)
            }
            _ => {}
        }
    }
    // Render up to cutoff + tail.
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 512 * 2];
    let tail = (1.0f64 * 64_000.0) as u64;
    let end = cutoff + tail;
    let mut frame = 0u64;

    loop {
        synth.render_block(&mut buf)?;
        frame += 512;
        out.extend_from_slice(&buf);
        if buf.iter().all(|s| s.abs() <= 0.0001) && frame > end {
            break;
        }
        if frame > end + 64_000 {
            break;
        }
    }
    println!("rendered {frame} frames ({:.2}s)", frame as f64 / 64000.0);

    let reference = read_wav("assets/ref_xsynth_default.wav")?;
    let ref_samples = &reference.samples[..(cutoff as usize * 2).min(reference.samples.len())];
    let n = ref_samples.len().min(out.len());
    let report = compare(&ref_samples[..n], &out[..n], 2, &[]);
    println!("{}", format_report(&report));
    Ok(())
}
