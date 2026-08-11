//! Generates: CC7=100 at tick 0, note on 0, off tick 480.
//! Usage: diag_ccrel

use lumino_midly::num::u28;
use lumino_midly::{Format, Header, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(480.into())),
        tracks: vec![vec![
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel: 0u8.into(),
                    message: MidiMessage::Controller {
                        controller: 7.into(),
                        value: 100.into(),
                    },
                },
            },
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Midi {
                    channel: 0u8.into(),
                    message: MidiMessage::NoteOn {
                        key: 64,
                        vel: 100.into(),
                    },
                },
            },
            TrackEvent {
                delta: u28::new(480),
                kind: TrackEventKind::Midi {
                    channel: 0u8.into(),
                    message: MidiMessage::NoteOff {
                        key: 64,
                        vel: 0.into(),
                    },
                },
            },
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(lumino_midly::MetaMessage::EndOfTrack),
            },
        ]],
    };
    smf.save("ccrel.mid")?;
    println!("wrote ccrel.mid");
    Ok(())
}
