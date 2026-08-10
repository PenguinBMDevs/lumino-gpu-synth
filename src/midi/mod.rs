//! MIDI event types shared by the parser, the scheduler and the engine.

pub mod parser;

pub use parser::MidiFile;

/// A MIDI event as understood by the synthesizer (channel-scoped).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiEvent {
    /// A note starts. `vel` is the MIDI velocity (1-127).
    NoteOn { key: u8, vel: u8 },
    /// A note stops.
    NoteOff { key: u8 },
    /// A control change. `controller` is the CC number (0-127).
    ControlChange { controller: u8, value: u8 },
    /// A program change (instrument selection).
    ProgramChange { program: u8 },
    /// A pitch bend. `value` is the raw 14-bit value (0-16383, 8192 = center).
    PitchBend { value: u16 },
}

impl MidiEvent {
    /// Returns `true` if this event is a note-on with zero velocity, which
    /// by the MIDI convention is equivalent to a note-off.
    pub fn is_zero_velocity_note_on(&self) -> bool {
        matches!(self, MidiEvent::NoteOn { vel, .. } if *vel == 0)
    }
}

/// A MIDI event bound to an absolute sample position in the output stream.
///
/// The `sample` field is computed from the MIDI tempo map, so events are
/// sample-accurate regardless of tempo changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedEvent {
    /// Absolute output sample index at which this event is applied.
    pub sample: u64,
    /// The MIDI channel (0-15) this event belongs to.
    pub channel: u8,
    /// The event payload.
    pub event: MidiEvent,
}

/// A parsed MIDI sequence: the full event stream with sample-accurate
/// timestamps, ready to be consumed by the engine.
///
/// Obtained via [`crate::MidiFile::load`].
#[derive(Debug, Clone, PartialEq)]
pub struct MidiSequence {
    /// All events in ascending sample order (events from every track and
    /// channel are merged).
    pub events: Vec<TimedEvent>,
    /// The output sample position of the last event (the MIDI's end).
    pub end_sample: u64,
}
