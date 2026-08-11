//! Prints the first note-on samples as computed by our parser.
//! Usage: diag_first_notes <midi>

use lumino_gpu_synth::MidiFile;
use lumino_gpu_synth::midi::MidiEvent;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/right-example.mid".into());
    let midi = MidiFile::load(&path, 64_000)?;
    let mut shown = 0;
    for ev in &midi.sequence.events {
        if let MidiEvent::NoteOn { key, vel } = ev.event {
            println!(
                "note_on key={key} vel={vel} sample={} ({:.4}s) ch={}",
                ev.sample,
                ev.sample as f64 / 64000.0,
                ev.channel
            );
            shown += 1;
            if shown >= 5 {
                break;
            }
        }
    }
    Ok(())
}
