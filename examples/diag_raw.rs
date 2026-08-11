//! Dumps the raw MIDI event stream using midly directly (bypassing our
//! parser) to verify running-status / controller handling.
//! Usage: diag_raw <file.mid>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/C4-C5.mid".into());
    let raw = std::fs::read(&path)?;
    let smf = lumino_midly::Smf::parse(&raw)?;
    println!(
        "tracks={}, timing={:?}",
        smf.tracks.len(),
        smf.header.timing
    );
    let mut total = 0usize;
    for (ti, track) in smf.tracks.iter().enumerate() {
        let mut tick: u64 = 0;
        let mut n = 0usize;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            n += 1;
            use lumino_midly::TrackEventKind::*;
            match &ev.kind {
                Meta(m) => {
                    if n <= 4 {
                        println!("t{ti} @{tick} META {m:?}");
                    }
                }
                Midi { channel, message } => {
                    if n <= 30 || n % 13 == 0 {
                        println!("t{ti} @{tick} ch{} {:?}", channel.as_int(), message);
                    }
                }
                _ => {}
            }
        }
        total += n;
        println!("track {ti}: {n} events");
    }
    println!("total events: {total}");
    Ok(())
}
