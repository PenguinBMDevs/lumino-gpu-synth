//! Removes controllers matching a predicate from a MIDI track.
//! Usage: diag_rmcc <in.mid> <out.mid> <keep_cc_csv|all>
//!   keep_cc_csv: comma list of controllers to KEEP (e.g. "7,10,11").
//!   "all": remove every controller.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&args[1])?;
    let smf = lumino_midly::Smf::parse(&raw)?;
    let keep: Option<Vec<u8>> = if args[3] == "all" {
        None
    } else {
        Some(
            args[3]
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect(),
        )
    };

    let mut tracks = Vec::new();
    for track in &smf.tracks {
        let mut out: Vec<lumino_midly::TrackEvent> = Vec::new();
        let mut carry = 0u64;
        for ev in track {
            let mut ev = ev.clone();
            let is_cc = matches!(
                &ev.kind,
                lumino_midly::TrackEventKind::Midi {
                    message: lumino_midly::MidiMessage::Controller { .. },
                    ..
                }
            );
            if is_cc {
                let cc = if let lumino_midly::TrackEventKind::Midi { message, .. } = &ev.kind {
                    match message {
                        lumino_midly::MidiMessage::Controller { controller, .. } => {
                            Some(*controller)
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let keep_this = keep
                    .as_ref()
                    .map(|k| k.contains(&cc.map(|c| c.as_int()).unwrap_or(0)))
                    .unwrap_or(false);
                if !keep_this {
                    carry += ev.delta.as_int() as u64;
                    continue;
                }
            }
            ev.delta = lumino_midly::num::u28::new(ev.delta.as_int() as u32 + carry as u32);
            carry = 0;
            out.push(ev);
        }
        if carry > 0 {
            if let Some(last) = out.last_mut() {
                last.delta = lumino_midly::num::u28::new(last.delta.as_int() as u32 + carry as u32);
            }
        }
        tracks.push(out);
    }
    let out = lumino_midly::Smf {
        header: smf.header.clone(),
        tracks,
    };
    out.save(&args[2])?;
    println!("wrote {}", args[2]);
    Ok(())
}
