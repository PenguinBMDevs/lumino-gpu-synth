//! Dumps every CC7/CC11/CC10/CC64 event with absolute tick across all tracks.
//! Usage: diag_cc7 <file.mid>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/C4-C5.mid".into());
    let raw = std::fs::read(&path)?;
    let smf = lumino_midly::Smf::parse(&raw)?;
    for (ti, track) in smf.tracks.iter().enumerate() {
        let mut tick: u64 = 0;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            if let lumino_midly::TrackEventKind::Midi { channel, message } = &ev.kind {
                if let lumino_midly::MidiMessage::Controller { controller, value } = message {
                    let c = controller.as_int();
                    if c == 7 || c == 11 || c == 10 || c == 64 || c == 1 || c == 6 {
                        let secs = tick as f64 * 500000.0 / 1e6 / 480.0;
                        println!(
                            "t{ti} ch{} CC{c}={} @tick {tick} ({secs:.3}s)",
                            channel.as_int(),
                            value.as_int()
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
