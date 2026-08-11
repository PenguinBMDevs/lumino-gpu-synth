//! Quick event-profile analyzer: counts note/CC/pitch events per second and
//! per channel, to find CPU hotspots on dense black-MIDI files.
//!
//! Usage: cargo run --release --example analyze_midi -- <midi file>

use lumino_gpu_synth::MidiFile;
use lumino_gpu_synth::midi::MidiEvent;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .ok_or_else(|| lumino_gpu_synth::SynthError::Config("usage: analyze_midi <file>".into()))?;
    let sr = 48_000u32;
    let midi = MidiFile::load(&path, sr)?;
    let evs = &midi.sequence.events;
    let dur = evs.last().map_or(0.0, |e| e.sample as f64 / sr as f64);
    println!(
        "events: {}  duration: {:.2}s  ev/s: {:.0}",
        evs.len(),
        dur,
        evs.len() as f64 / dur.max(0.001)
    );

    let mut notes = 0u64;
    let mut offs = 0u64;
    let mut ccs = 0u64;
    let mut pb = 0u64;
    let mut pc = 0u64;
    let mut note_cc: std::collections::HashMap<u8, u64> = std::collections::HashMap::new();
    let mut per_chan = [0u64; 16];
    for e in evs {
        per_chan[(e.channel as usize) % 16] += 1;
        match e.event {
            MidiEvent::NoteOn { .. } => {
                notes += 1;
                *note_cc.entry(e.channel).or_default() += 1;
            }
            MidiEvent::NoteOff { .. } => offs += 1,
            MidiEvent::ControlChange { .. } => ccs += 1,
            MidiEvent::PitchBend { .. } => pb += 1,
            MidiEvent::ProgramChange { .. } => pc += 1,
        }
    }
    println!("note_on: {notes}  note_off: {offs}  cc: {ccs}  pitchbend: {pb}  program: {pc}");
    println!("per-channel:");
    for (ch, n) in per_chan.iter().enumerate() {
        if *n > 0 {
            println!(
                "  ch{ch}: {n}  (note_on: {})",
                note_cc.get(&(ch as u8)).unwrap_or(&0)
            );
        }
    }
    // Peak simultaneous voices estimate: count overlapping note-ons at the
    // densest point (coarse 1s buckets).
    let buckets = dur as usize + 1;
    let mut active = vec![0i64; buckets.max(1)];
    for e in evs {
        let b = (e.sample as f64 / sr as f64) as usize;
        match e.event {
            MidiEvent::NoteOn { .. } if b < buckets => active[b] += 1,
            MidiEvent::NoteOff { .. } if b < buckets => active[b] -= 1,
            _ => {}
        }
    }
    let mut peak = 0i64;
    let mut cur = 0i64;
    for &a in &active {
        cur += a;
        peak = peak.max(cur);
    }
    println!("peak overlapping notes (1s buckets): {peak}");
    Ok(())
}
