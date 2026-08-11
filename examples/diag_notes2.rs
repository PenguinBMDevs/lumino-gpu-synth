//! Dumps every note-on/note-off with absolute time across all tracks.
//! Usage: diag_notes2 <file.mid>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/C4-C5.mid".into());
    let raw = std::fs::read(&path)?;
    let smf = lumino_midly::Smf::parse(&raw)?;
    let ppq = match smf.header.timing {
        lumino_midly::Timing::Metrical(p) => p.as_int() as f64,
        _ => 480.0,
    };
    let mut notes = 0usize;
    for (ti, track) in smf.tracks.iter().enumerate() {
        let mut tick: u64 = 0;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            if let lumino_midly::TrackEventKind::Midi { channel, message } = &ev.kind {
                let secs = tick as f64 * 500000.0 / 1e6 / ppq;
                match message {
                    lumino_midly::MidiMessage::NoteOn { key, vel } => {
                        notes += 1;
                        println!(
                            "t{ti} ch{} NoteOn k{} v{} @tick {tick} ({secs:.3}s)",
                            channel.as_int(),
                            *key,
                            *vel
                        );
                    }
                    lumino_midly::MidiMessage::NoteOff { key, .. } => {
                        notes += 1;
                        println!(
                            "t{ti} ch{} NoteOff k{} @tick {tick} ({secs:.3}s)",
                            channel.as_int(),
                            *key
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    println!("total note events: {notes}");
    Ok(())
}
