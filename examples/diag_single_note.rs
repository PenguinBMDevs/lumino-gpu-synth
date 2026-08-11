//! Creates a single-note MIDI (first note of right-example.mid:
//! key=21 vel=70 ch=3, 0.7305s at tempo 117186 us/beat) and renders it with
//! both engines, then compares the waveforms.
//!
//! Usage: cargo run --release --example diag_single_note
//! Then: xsynth-render single-note.mid test.sf2 -o ref_single.wav -s 64000
//!       cargo run --release --example diag_single_note  (renders ours)

use lumino_gpu_synth::audio::wav::read_wav;
use lumino_gpu_synth::compare::{compare, format_report};
use lumino_gpu_synth::{GpuSynth, SynthConfig};
fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--gen" {
        // Write single-note.mid via lumino-midly (correct varlen encoding).
        let key: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
        let vel: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
        let off_tick: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(5984);
        use lumino_midly::num::u28;
        use lumino_midly::{
            Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
        };
        let smf = Smf {
            header: Header::new(Format::SingleTrack, Timing::Metrical(960.into())),
            tracks: vec![vec![
                TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Midi {
                        channel: 0u8.into(),
                        message: MidiMessage::NoteOn {
                            key,
                            vel: vel.into(),
                        },
                    },
                },
                TrackEvent {
                    delta: u28::new(off_tick),
                    kind: TrackEventKind::Midi {
                        channel: 0u8.into(),
                        message: MidiMessage::NoteOff { key, vel: 0.into() },
                    },
                },
                TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                },
            ]],
        };
        smf.save("single-note.mid").expect("save");
        println!("wrote single-note.mid (key={key} vel={vel} off_tick={off_tick})");
        return Ok(());
    }

    // Render our version.
    let config = SynthConfig {
        use_effects: false,
        max_voices: 16384,
        envelope_curves: lumino_gpu_synth::synth::dsp::EnvelopeCurveConfig {
            attack_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
            decay_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
            release_curve: lumino_gpu_synth::synth::dsp::CurveKind::Exponential,
        },
        ..SynthConfig::default()
    };
    let mut synth = GpuSynth::new(config)?;
    synth.load_soundfont("assets/test.sf2", 0, 0)?;
    let result = synth.render_midi_file("single-note.mid")?;
    println!(
        "ours: {} frames ({:.4}s)",
        result.samples.len() / 2,
        result.samples.len() as f64 / 64000.0 / 2.0
    );
    let reference = read_wav("ref_single.wav")?;
    println!(
        "ref: {} frames ({:.4}s)",
        reference.samples.len() / reference.channels as usize,
        reference.samples.len() as f64 / reference.sample_rate as f64 / reference.channels as f64
    );
    let report = compare(
        &reference.samples,
        &result.samples,
        reference.channels as usize,
        &[],
    );
    println!("{}", format_report(&report));
    lumino_gpu_synth::audio::wav::write_f32_wav(
        "single-ours.wav",
        &result.samples,
        result.sample_rate,
    )?;
    println!("wrote single-ours.wav");
    Ok(())
}
