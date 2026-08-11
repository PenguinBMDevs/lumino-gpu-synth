//! Truncates C4-C5.mid to the first N notes and writes notesN.mid.
//! Usage: diag_trunc <n_notes>

use lumino_midly::num::{u14, u28};
use lumino_midly::{Format, Header, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let n: u8 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    // C4-C5: keys 60..=72, one note each, 0.5s apart (480 ticks at 960ppq).
    let mut track: Vec<TrackEvent> = Vec::new();
    // CC7 = 100 at tick 0 (before any notes), like the C4-C5 reference MIDI.
    if std::env::var("LUMINO_ADD_CC7").is_ok() {
        track.push(TrackEvent {
            delta: u28::new(720),
            kind: TrackEventKind::Midi {
                channel: 0u8.into(),
                message: MidiMessage::Controller {
                    controller: 7.into(),
                    value: 100.into(),
                },
            },
        });
    }
    // CC72/CC73 = 64 (neutral) around tick 1620, like C4-C5.
    if std::env::var("LUMINO_ADD_CC7273").is_ok() {
        for (cc, tick) in [(72u8, 1620u32), (73u8, 1560u32)] {
            let mut delta = tick;
            if let Some(last) = track.last() {
                delta = delta.saturating_sub(last.delta.as_int() as u32);
            }
            track.push(TrackEvent {
                delta: u28::new(delta),
                kind: TrackEventKind::Midi {
                    channel: 0u8.into(),
                    message: MidiMessage::Controller {
                        controller: cc.into(),
                        value: 64.into(),
                    },
                },
            });
        }
    }
    // CC11 = 127 (expression) at tick 1200, like C4-C5.
    if std::env::var("LUMINO_ADD_CC11").is_ok() {
        let mut delta = 1200u32;
        if let Some(last) = track.last() {
            delta = delta.saturating_sub(last.delta.as_int() as u32);
        }
        track.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: 0u8.into(),
                message: MidiMessage::Controller {
                    controller: 11.into(),
                    value: 127.into(),
                },
            },
        });
    }
    // CC10 = 64 (pan center) at tick 780, like C4-C5.
    if std::env::var("LUMINO_ADD_CC10").is_ok() {
        let mut delta = 780u32;
        if let Some(last) = track.last() {
            delta = delta.saturating_sub(last.delta.as_int() as u32);
        }
        track.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: 0u8.into(),
                message: MidiMessage::Controller {
                    controller: 10.into(),
                    value: 64.into(),
                },
            },
        });
    }
    // NRPN: CC99=1, CC98=8, CC6=64 at tick 1260 (like C4-C5 RPN setup).
    if std::env::var("LUMINO_ADD_NRPN").is_ok() {
        for (cc, val) in [(99u8, 1u8), (98u8, 8u8), (6u8, 64u8)] {
            let mut delta = 1260u32;
            if let Some(last) = track.last() {
                delta = delta.saturating_sub(last.delta.as_int() as u32);
            }
            track.push(TrackEvent {
                delta: u28::new(delta),
                kind: TrackEventKind::Midi {
                    channel: 0u8.into(),
                    message: MidiMessage::Controller {
                        controller: cc.into(),
                        value: val.into(),
                    },
                },
            });
        }
    }
    // PitchBend = 8192 (center) at tick 1080, like C4-C5.
    if std::env::var("LUMINO_ADD_PB").is_ok() {
        let mut delta = 1080u32;
        if let Some(last) = track.last() {
            delta = delta.saturating_sub(last.delta.as_int() as u32);
        }
        track.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: 0u8.into(),
                message: MidiMessage::PitchBend {
                    bend: lumino_midly::PitchBend(u14::new(8192)),
                },
            },
        });
    }
    // ProgramChange = 0 at tick 840, like C4-C5.
    if std::env::var("LUMINO_ADD_PC").is_ok() {
        let mut delta = 840u32;
        if let Some(last) = track.last() {
            delta = delta.saturating_sub(last.delta.as_int() as u32);
        }
        track.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi {
                channel: 0u8.into(),
                message: MidiMessage::ProgramChange { program: 0.into() },
            },
        });
    }
    for i in 0..n {
        let key = 60u8 + i;
        // C4-C5 structure: every note is 480 ticks; the next note's on lands
        // on the same tick as the previous note's off (on delta 0 for i>0).
        track.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Midi {
                channel: 0u8.into(),
                message: MidiMessage::NoteOn {
                    key,
                    vel: 100.into(),
                },
            },
        });
        track.push(TrackEvent {
            delta: u28::new(480),
            kind: TrackEventKind::Midi {
                channel: 0u8.into(),
                message: MidiMessage::NoteOff { key, vel: 0.into() },
            },
        });
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(lumino_midly::MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(480.into())),
        tracks: vec![track],
    };
    let path = format!("notes{n}.mid");
    smf.save(&path)?;
    println!("wrote {path}");
    Ok(())
}
