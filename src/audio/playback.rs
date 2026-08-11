//! Realtime playback of a [`crate::GpuSynth`] through `cpal`.
//!
//! # Architecture
//!
//! The engine is owned by a background **render thread**; MIDI events are
//! sent through a channel and rendered blocks are pushed into the audio
//! callback via a bounded sync channel. The audio callback runs on the
//! OS audio thread and must never block - it only copies from the queue
//! and writes silence on underrun.
//!
//! # Sample-rate negotiation
//!
//! The engine renders at its configured sample rate (e.g. 64 kHz). Most
//! output devices do not run at 64 kHz, so the playback layer picks the
//! device's default configuration first and falls back to any supported
//! config whose sample rate matches the engine; if none matches, it
//! resamples the engine output to the device rate with a small linear
//! interpolator. Use [`AudioPlayback::device_sample_rates`] to list what
//! a device supports before constructing the engine.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::resample::LinearResampler;
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
    stop_flag: Arc<AtomicBool>,
    stop_tx: Option<mpsc::Sender<()>>,
    event_tx: Option<mpsc::Sender<(u8, MidiEvent)>>,
    thread: Option<JoinHandle<()>>,
    sample_rate: u32,
    engine_rate: u32,
    _stream: Option<cpal::Stream>,
}

impl AudioPlayback {
    /// Opens the default output device and starts the render/playback
    /// thread.
    ///
    /// The device is opened with the engine's configured sample rate if the
    /// device supports it, otherwise with the device default sample rate and
    /// the engine output is linearly resampled to match.
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Gpu`] if no audio output device is available or
    /// the stream cannot be opened.
    pub fn start(synth: GpuSynth) -> Result<Self, SynthError> {
        let engine_rate = synth.config().sample_rate;
        let channels = synth.config().channels.channel_count();
        let block = synth.config().block_size;

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| SynthError::Gpu("no default audio output device".into()))?;

        // Negotiate: prefer the engine rate, else the device default, and
        // remember which we got so the render thread can resample.
        let (stream_config, resample_needed) =
            negotiate_config(&device, engine_rate, channels, block)?;
        let device_rate = stream_config.sample_rate;
        let needs_resample = resample_needed || device_rate != engine_rate;

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (event_tx, event_rx) = mpsc::channel::<(u8, MidiEvent)>();
        let (sample_tx, sample_rx) = mpsc::sync_channel::<Vec<f32>>(4);
        let stop_flag = Arc::new(AtomicBool::new(false));

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
        // blocks (optionally resampling to the device rate) and forwards
        // them to the audio callback. Non-blocking pushes bound the wait so
        // a stalled consumer can never deadlock `stop()`.
        let thread_stop = stop_flag.clone();
        let thread = thread::Builder::new()
            .name("lumino-gpu-synth-render".into())
            .spawn(move || {
                let mut synth = synth;
                let mut buf = vec![0.0f32; block * channels];
                let mut resampler = LinearResampler::new(engine_rate, device_rate, channels);
                let drain_timeout = std::time::Duration::from_millis(5);
                loop {
                    // Drain pending MIDI events (non-blocking).
                    while let Ok((ch, ev)) = event_rx.try_recv() {
                        synth.send_event(ch, ev);
                    }
                    if thread_stop.load(Ordering::Relaxed) || stop_rx.try_recv().is_ok() {
                        break;
                    }
                    // Wait a short while for more events before the next
                    // block, so burst input is batched into fewer renders.
                    if let Ok((ch, ev)) = event_rx.recv_timeout(drain_timeout) {
                        synth.send_event(ch, ev);
                        // Also drain anything else queued behind it.
                        while let Ok((c, e)) = event_rx.try_recv() {
                            synth.send_event(c, e);
                        }
                    }
                    if thread_stop.load(Ordering::Relaxed) || stop_rx.try_recv().is_ok() {
                        break;
                    }
                    if synth.render_block(&mut buf).is_err() {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    let out = if needs_resample {
                        resampler.process(&buf)
                    } else {
                        buf.clone()
                    };
                    // Non-blocking push: when the consumer is slow the queue
                    // stays full and we simply skip this block instead of
                    // blocking forever (which would deadlock `stop()`).
                    if sample_tx.try_send(out).is_err() {
                        std::thread::sleep(drain_timeout);
                    }
                }
            })
            .map_err(SynthError::Io)?;

        Ok(Self {
            stop_flag,
            stop_tx: Some(stop_tx),
            event_tx: Some(event_tx),
            thread: Some(thread),
            sample_rate: device_rate,
            engine_rate,
            _stream: Some(stream),
        })
    }

    /// Lists the sample rates the default output device supports (empty if
    /// the device cannot be queried).
    pub fn device_sample_rates() -> Vec<u32> {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return Vec::new();
        };
        let mut rates = Vec::new();
        if let Ok(iter) = device.supported_output_configs() {
            for cfg in iter {
                rates.push(cfg.min_sample_rate());
                rates.push(cfg.max_sample_rate());
            }
        }
        rates.sort_unstable();
        rates.dedup();
        rates
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

    /// Convenience: sends a program change (instrument selection).
    pub fn program_change(&mut self, channel: u8, program: u8) {
        self.send_event(channel, MidiEvent::ProgramChange { program });
    }

    /// Convenience: sends a pitch bend. `value` is the raw 14-bit value
    /// (0-16383, 8192 = center).
    pub fn pitch_bend(&mut self, channel: u8, value: u16) {
        self.send_event(channel, MidiEvent::PitchBend { value });
    }

    /// Sends a control change to a channel with 14-bit MSB/LSB splitting
    /// (e.g. CC1/CC33 for vibrato depth).
    pub fn control_change_14bit(&mut self, channel: u8, msb: u8, lsb: u8, value: u16) {
        let hi = (value >> 7) as u8 & 0x7F;
        let lo = (value & 0x7F) as u8;
        self.control_change(channel, msb, hi);
        self.control_change(channel, lsb, lo);
    }

    /// Damper pedal (CC64): `down` holds all released notes until lifted.
    pub fn damper(&mut self, channel: u8, down: bool) {
        self.control_change(channel, 0x40, if down { 127 } else { 0 });
    }

    /// All notes off (CC123): releases every note on the channel.
    pub fn all_notes_off(&mut self, channel: u8) {
        self.control_change(channel, 0x7B, 0);
    }

    /// All sounds off (CC120): kills every voice on the channel instantly.
    pub fn all_sounds_off(&mut self, channel: u8) {
        self.control_change(channel, 0x78, 0);
    }

    /// Reset all controllers (CC121).
    pub fn reset_controllers(&mut self, channel: u8) {
        self.control_change(channel, 0x79, 0);
    }

    /// The device sample rate in use.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The engine's render sample rate (events scheduled against this).
    pub fn engine_sample_rate(&self) -> u32 {
        self.engine_rate
    }

    /// Stops the render thread (and closes the audio stream).
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
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

/// Negotiates the `StreamConfig` for `device`.
///
/// Strategy (verified against WASAPI shared mode): the *enumerated* sample
/// rates are a broad claim and opening a stream at a non-default rate often
/// fails with "not supported in shared mode". The robust choice is the
/// device's *default* configuration: it is guaranteed to open. When the
/// device default rate differs from the engine rate, the render thread
/// resamples (see [`LinearResampler`]). `resampled` reports that case.
fn negotiate_config(
    device: &cpal::Device,
    engine_rate: u32,
    channels: usize,
    block: usize,
) -> Result<(cpal::StreamConfig, bool), SynthError> {
    let default = device
        .default_output_config()
        .map_err(|e| SynthError::Gpu(format!("default output config: {e}")))?;
    let mut stream: cpal::StreamConfig = default.into();
    // The device default is authoritative; keep the engine's channel count
    // only when the device has at least that many (mono on a stereo device
    // is up-mixed by the callback writing L/R, so we keep stereo).
    if (stream.channels as usize) < channels {
        stream.channels = channels as u16;
    }
    // Fixed buffer size is not guaranteed either; prefer the device default
    // unless the device explicitly supports our block size.
    if let Ok(mut iter) = device.supported_output_configs() {
        let compatible = iter.any(|cfg| {
            matches!(
                cfg.buffer_size(),
                cpal::SupportedBufferSize::Range { min, max }
                    if block as u32 >= *min && block as u32 <= *max
            )
        });
        if compatible {
            stream.buffer_size = cpal::BufferSize::Fixed(block as u32);
        }
    }
    let resampled = stream.sample_rate != engine_rate;
    Ok((stream, resampled))
}
