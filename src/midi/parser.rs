//! MIDI file parsing on top of `lumino-midly`.
//!
//! [`MidiFile`] converts a standard MIDI file into a [`MidiSequence`] of
//! sample-accurate events, honoring tempo changes and merging all tracks.

use lumino_midly::num::u24;
use lumino_midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

use crate::SynthError;
use crate::midi::{MidiEvent, MidiSequence, TimedEvent};

/// A parsed MIDI file.
///
/// # Example
///
/// ```
/// use lumino_gpu_synth::MidiFile;
///
/// let midi = MidiFile::load("assets/right-example.mid", 64_000).unwrap();
/// assert_eq!(midi.sample_rate, 64_000);
/// assert!(midi.sequence.events.len() >= 5);
/// ```
#[derive(Debug, Clone)]
pub struct MidiFile {
    /// The parsed, sample-accurate event sequence.
    pub sequence: MidiSequence,
    /// The sample rate the sequence was computed for.
    pub sample_rate: u32,
    /// Tempo events (tick position, microseconds per quarter note).
    pub tempos: Vec<(u64, u32)>,
    /// Total length of the song in ticks.
    pub length_ticks: u64,
}

impl MidiFile {
    /// Parses a MIDI file from disk and builds a sample-accurate event
    /// sequence at `sample_rate` Hz.
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Midi`] if the file is not a valid MIDI file, and
    /// [`SynthError::Io`] on read failures.
    pub fn load(path: impl AsRef<std::path::Path>, sample_rate: u32) -> Result<Self, SynthError> {
        let raw = std::fs::read(path)?;
        Self::parse(&raw, sample_rate)
    }

    /// Parses a MIDI file from raw bytes at `sample_rate` Hz.
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Midi`] if the data is not a valid MIDI file.
    pub fn parse(raw: &[u8], sample_rate: u32) -> Result<Self, SynthError> {
        let smf = Smf::parse(raw)
            .map_err(|e| SynthError::Midi(format!("lumino-midly failed to parse: {e}")))?;

        let ticks_per_beat = match smf.header.timing {
            Timing::Metrical(ppq) => ppq.as_int() as u64,
            Timing::Timecode(_, _) => {
                return Err(SynthError::Midi(
                    "SMPTE timecode timing is not supported".into(),
                ));
            }
        };
        if ticks_per_beat == 0 {
            return Err(SynthError::Midi("zero ticks per beat".into()));
        }

        // Collect per-track event lists with absolute tick positions.
        let mut raw_events: Vec<(u64, u8, MidiEvent)> = Vec::new();
        let mut tempos: Vec<(u64, u32)> = Vec::new();
        let mut length_ticks: u64 = 0;

        for track in &smf.tracks {
            let mut tick: u64 = 0;
            for ev in track {
                tick += ev.delta.as_int() as u64;
                length_ticks = length_ticks.max(tick);
                match &ev.kind {
                    TrackEventKind::Meta(MetaMessage::Tempo(us_per_beat)) => {
                        tempos.push((tick, u24_to_u32(*us_per_beat)));
                    }
                    TrackEventKind::Meta(MetaMessage::EndOfTrack) => {}
                    TrackEventKind::Midi { channel, message } => {
                        let channel = channel.as_int();
                        let event = match *message {
                            MidiMessage::NoteOn { key, vel } => {
                                let vel = vel.as_int();
                                if vel == 0 {
                                    MidiEvent::NoteOff { key }
                                } else {
                                    MidiEvent::NoteOn { key, vel }
                                }
                            }
                            MidiMessage::NoteOff { key, .. } => MidiEvent::NoteOff { key },
                            MidiMessage::Controller { controller, value } => {
                                MidiEvent::ControlChange {
                                    controller: controller.as_int(),
                                    value: value.as_int(),
                                }
                            }
                            MidiMessage::ProgramChange { program } => MidiEvent::ProgramChange {
                                program: program.as_int(),
                            },
                            MidiMessage::PitchBend { bend } => MidiEvent::PitchBend {
                                value: bend.0.as_int(),
                            },
                            _ => continue,
                        };
                        raw_events.push((tick, channel, event));
                    }
                    _ => {}
                }
            }
        }

        // Tempo map: for each interval between tempo changes, seconds per tick.
        // The default tempo is 500_000 us/beat (120 BPM).
        let mut tempo_map: Vec<(u64, f64)> = Vec::new();
        let mut prev_tick = 0u64;
        let mut prev_tempo = 500_000.0;
        for &(tick, us) in tempos.iter() {
            tempo_map.push((prev_tick, prev_tempo));
            prev_tick = tick;
            prev_tempo = us as f64;
        }
        tempo_map.push((prev_tick, prev_tempo));

        let ticks_to_sec = |tick: u64| -> f64 {
            // Find the tempo segment containing `tick`.
            let mut sec = 0.0;
            let mut seg_start_tick = 0u64;
            for &(start_tick, us) in tempo_map.iter() {
                if tick <= start_tick {
                    break;
                }
                sec +=
                    (start_tick - seg_start_tick) as f64 * us / 1_000_000.0 / ticks_per_beat as f64;
                seg_start_tick = start_tick;
            }
            sec + (tick - seg_start_tick) as f64 * prev_tempo / 1_000_000.0 / ticks_per_beat as f64
        };

        let mut events: Vec<TimedEvent> = raw_events
            .into_iter()
            .map(|(tick, channel, event)| {
                let sec = ticks_to_sec(tick);
                let sample = (sec * sample_rate as f64).round() as u64;
                TimedEvent {
                    sample,
                    channel,
                    event,
                }
            })
            .collect();
        events.sort_by_key(|e| (e.sample, e.channel));

        let end_sample = (ticks_to_sec(length_ticks) * sample_rate as f64).round() as u64;

        Ok(Self {
            sequence: MidiSequence { events, end_sample },
            sample_rate,
            tempos,
            length_ticks,
        })
    }

    /// Returns the sequence length in seconds.
    pub fn duration_secs(&self) -> f64 {
        self.sequence.end_sample as f64 / self.sample_rate as f64
    }

    /// Writes this MIDI back to a file (mostly useful for debugging).
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), SynthError> {
        let _ = (path, self);
        // Note: full SMF re-serialization is intentionally not implemented;
        // this method exists as a placeholder for tooling.
        Err(SynthError::Config(
            "MidiFile::save is not implemented; use lumino-midly directly".into(),
        ))
    }
}

fn u24_to_u32(v: u24) -> u32 {
    v.as_int()
}
