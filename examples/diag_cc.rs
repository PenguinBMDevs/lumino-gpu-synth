//! Dumps the ControlChange controller histogram of a MIDI file.
//! Usage: diag_cc <file.mid>

use std::collections::BTreeMap;

use lumino_gpu_synth::MidiFile;
use lumino_gpu_synth::midi::MidiEvent;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/right-example.mid".into());
    let midi = MidiFile::load(&path, 64_000)?;
    let mut hist: BTreeMap<u8, (u64, BTreeMap<u8, u64>)> = BTreeMap::new();
    for ev in &midi.sequence.events {
        if let MidiEvent::ControlChange { controller, value } = ev.event {
            let e = hist.entry(controller).or_default();
            e.0 += 1;
            *e.1.entry(value).or_default() += 1;
        }
    }
    for (c, (n, values)) in &hist {
        let top: Vec<String> = values
            .iter()
            .take(6)
            .map(|(v, c)| format!("{v}x{c}"))
            .collect();
        println!("CC{c:3}: {n} events, values: {}", top.join(", "));
    }
    Ok(())
}
