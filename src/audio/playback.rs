//! Realtime playback of a [`crate::GpuSynth`] through `cpal`.
//!
//! The engine is owned by a background render thread; MIDI events are sent
//! through a channel and rendered blocks are pushed into the audio callback
//! via a sync channel. This is a thin convenience layer; the actual
//! rendering is still performed by the engine on the GPU.

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::GpuSynth;
use crate::SynthError;
use crate::midi::MidiEvent;

/// A running realtime playback session.
///
/// # Example
///
/// ```no_run
/// use lumino_gpu_synth::{GpuSynth, SynthConfig};
/// use lumino_gpu_synth::audio::playback::AudioPlayback;
///
/// let mut synth = GpuSynth::new(SynthConfig::default())?;
/// synth.load_soundfont("assets/test.sf2", 0, 0)?;
/// let mut playback = AudioPlayback::start(synth)?;
/// playback.note_on(0, 60, 100);
/// std::thread::sleep(std::time::Duration::from_millis(500));
/// playback.note_off(0, 60);
/// playback.stop();
/// # Ok::<(), lumino_gpu_synth::SynthError>(())
/// ```
pub struct AudioPlayback {
    stop_tx: Option<mpsc::Sender<()>>,
    event_tx: Option<mpsc::Sender<(u8, MidiEvent)>>,
    thread: Option<JoinHandle<()>>,
    sample_rate: u32,
    _stream: Option<cpal::Stream>,
}

impl AudioPlayback {
    /// Opens the default output device at the engine's sample rate and
    /// starts the render/playback thread.
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Gpu`] if no audio output device supports the
    /// configured sample rate.
    pub fn start(synth: GpuSynth) -> Result<Self, SynthError> {
        let sample_rate = synth.config().sample_rate;
        let channels = if synth.config().channels == crate::ChannelMode::Stereo {
            2usize
        } else {
            1usize
        };
        let block = synth.config().block_size;

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| SynthError::Gpu("no default audio output device".into()))?;

        let stream_config = cpal::StreamConfig {
            channels: channels as u16,
            sample_rate,
            buffer_size: cpal::BufferSize::Fixed(block as u32),
        };

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (event_tx, event_rx) = mpsc::channel::<(u8, MidiEvent)>();
        let (sample_tx, sample_rx) = mpsc::sync_channel::<Vec<f32>>(4);

        // Audio callback: pull blocks from the queue into the device buffer.
        let err_fn = |e| eprintln!("lumino-gpu-synth playback error: {e}");
        let mut next_block: Vec<f32> = Vec::new();
        let mut next_pos = 0usize;
        let stream = device
            .build_output_stream(
                stream_config,
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    let mut i = 0;
                    while i < data.len() {
                        if next_pos >= next_block.len() {
                            match sample_rx.try_recv() {
                                Ok(b) => {
                                    next_block = b;
                                    next_pos = 0;
                                }
                                Err(_) => {
                                    // Underrun: silence the remainder.
                                    data[i..].fill(0.0);
                                    break;
                                }
                            }
                        } else {
                            let n = (next_block.len() - next_pos).min(data.len() - i);
                            data[i..i + n].copy_from_slice(&next_block[next_pos..next_pos + n]);
                            i += n;
                            next_pos += n;
                        }
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| SynthError::Gpu(format!("audio stream: {e}")))?;
        stream
            .play()
            .map_err(|e| SynthError::Gpu(format!("audio stream play: {e}")))?;

        // Render thread: owns the engine, drains incoming events, renders
        // blocks and forwards them to the audio callback.
        let thread = thread::Builder::new()
            .name("lumino-gpu-synth-render".into())
            .spawn(move || {
                let mut synth = synth;
                let mut buf = vec![0.0f32; block * channels];
                while stop_rx.try_recv().is_err() {
                    while let Ok((ch, ev)) = event_rx.try_recv() {
                        synth.send_event(ch, ev);
                    }
                    if synth.render_block(&mut buf).is_ok() {
                        let _ = sample_tx.send(buf.clone());
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            })
            .map_err(SynthError::Io)?;

        Ok(Self {
            stop_tx: Some(stop_tx),
            event_tx: Some(event_tx),
            thread: Some(thread),
            sample_rate,
            _stream: Some(stream),
        })
    }

    /// Sends a MIDI event to the engine (applied at the next block).
    pub fn send_event(&mut self, channel: u8, event: MidiEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send((channel, event));
        }
    }

    /// Convenience: sends a note-on.
    pub fn note_on(&mut self, channel: u8, key: u8, vel: u8) {
        self.send_event(channel, MidiEvent::NoteOn { key, vel });
    }

    /// Convenience: sends a note-off.
    pub fn note_off(&mut self, channel: u8, key: u8) {
        self.send_event(channel, MidiEvent::NoteOff { key });
    }

    /// Convenience: sends a control change.
    pub fn control_change(&mut self, channel: u8, controller: u8, value: u8) {
        self.send_event(channel, MidiEvent::ControlChange { controller, value });
    }

    /// The engine's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Stops the render thread (and closes the audio stream).
    pub fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
        self.event_tx = None;
        self._stream = None;
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        self.stop();
    }
}
