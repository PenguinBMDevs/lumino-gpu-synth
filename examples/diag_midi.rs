//! Diagnostic: prints tempo event statistics of the MIDI file.
use lumino_gpu_synth::MidiFile;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/right-example.mid".into());
    let midi = MidiFile::load(&path, 64_000)?;
    println!(
        "events: {}, length_ticks: {}, duration: {:.3}s",
        midi.sequence.events.len(),
        midi.length_ticks,
        midi.duration_secs()
    );
    // Derive ppq directly from the raw file.
    let raw = std::fs::read(&path)?;
    let smf = lumino_midly::Smf::parse(&raw).expect("parse");
    if let lumino_midly::Timing::Metrical(ppq) = smf.header.timing {
        println!("ppq: {}", ppq.as_int());
    }
    println!("tempos: {}", midi.tempos.len());
    let n = midi.tempos.len();
    for (i, &(tick, us)) in midi.tempos.iter().enumerate() {
        if i < 15 || i + 5 >= n {
            println!(
                "  tempo[{i}]: tick={tick} us={us} ({:.0} BPM)",
                60e6 / us as f64
            );
        }
    }
    // Event type histogram.
    let (mut note_on, mut note_off, mut cc, mut pc, mut pb) = (0, 0, 0, 0, 0);
    for ev in &midi.sequence.events {
        use lumino_gpu_synth::midi::MidiEvent::*;
        match ev.event {
            NoteOn { .. } => note_on += 1,
            NoteOff { .. } => note_off += 1,
            ControlChange { .. } => cc += 1,
            ProgramChange { .. } => pc += 1,
            PitchBend { .. } => pb += 1,
        }
    }
    println!("note_on={note_on} note_off={note_off} cc={cc} pc={pc} pb={pb}");
    // First/last event samples.
    if let (Some(f), Some(l)) = (midi.sequence.events.first(), midi.sequence.events.last()) {
        println!("first event: sample={} channel={}", f.sample, f.channel);
        println!(
            "last event: sample={} ({:.3}s) channel={}",
            l.sample,
            l.sample as f64 / 64000.0,
            l.channel
        );
    }
    Ok(())
}
