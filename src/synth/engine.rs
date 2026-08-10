//! The `GpuSynth` engine: MIDI event scheduling, voice management and
//! block-wise GPU rendering.

use std::collections::VecDeque;
use std::sync::Arc;

use bytemuck::Zeroable;

use crate::config::{ChannelMode, SynthConfig};
use crate::error::SynthError;
use crate::gpu::{
    ChannelMix, EnvStageGpu, GpuResources, GrowableBuffer, MixParams, VoiceParams, VoiceState,
    create_gpu_context,
};
use crate::midi::{MidiEvent, MidiFile, TimedEvent};
use crate::soundfont::SoundFont;
use crate::synth::voices::{Voice, build_voice, refresh_env_stages};

/// The result of an offline render.
#[derive(Debug, Clone)]
pub struct RenderResult {
    /// Interleaved samples (L/R/L/R...) for the whole render.
    pub samples: Vec<f32>,
    /// Output sample rate.
    pub sample_rate: u32,
    /// Number of channels (1 or 2).
    pub channels: u32,
    /// Total rendered frames.
    pub frames: u64,
}

/// A 10 ms linear-smoothed controller value (mirror of XSynth's `ValueLerp`).
#[derive(Debug, Clone, Copy)]
struct LerpState {
    current: f32,
    end: f32,
    step: f32,
}

impl LerpState {
    fn new(initial: f32) -> Self {
        Self {
            current: initial,
            end: initial,
            step: 0.0,
        }
    }

    fn set_end(&mut self, end: f32, sample_rate: u32) {
        self.step = (end - self.current) / (sample_rate as f32 * 0.01);
        self.end = end;
    }

    /// Advances the lerp by `n` samples (CPU-side pre-roll to the block
    /// boundary), returning the value at that point.
    fn advance(&mut self, n: u32) -> f32 {
        if self.end > self.current {
            self.current = (self.current + self.step * n as f32).min(self.end);
        } else if self.end < self.current {
            self.current = (self.current + self.step * n as f32).max(self.end);
        }
        self.current
    }
}

/// Per-channel MIDI state.
#[derive(Debug)]
struct ChannelState {
    program: u8,
    volume: LerpState,
    expression: LerpState,
    pan: LerpState,
    damper: bool,
    pitch_multiplier: f32,
    /// CC73 (attack) value affecting voices of this channel.
    env_attack: Option<u8>,
    /// CC72 (release) value affecting voices of this channel.
    env_release: Option<u8>,
}

impl ChannelState {
    fn new() -> Self {
        Self {
            program: 0,
            volume: LerpState::new(1.0),
            expression: LerpState::new(1.0),
            pan: LerpState::new(0.5),
            damper: false,
            pitch_multiplier: 1.0,
            env_attack: None,
            env_release: None,
        }
    }
}

/// The GPU-accelerated MIDI synthesizer.
///
/// # Example
///
/// ```no_run
/// use lumino_gpu_synth::{GpuSynth, SynthConfig};
///
/// let mut synth = GpuSynth::new(SynthConfig::default())?;
/// synth.load_soundfont("assets/test.sf2", 0, 0)?;
/// let result = synth.render_midi_file("assets/right-example.mid")?;
/// # Ok::<(), lumino_gpu_synth::SynthError>(())
/// ```
pub struct GpuSynth {
    config: SynthConfig,
    res: GpuResources,
    sf: Option<SoundFont>,

    // GPU buffers
    params_buf: GrowableBuffer,
    samples_buf: GrowableBuffer,
    sinc_buf: wgpu::Buffer,
    env_buf: GrowableBuffer,
    states_buf: GrowableBuffer,
    voice_out_buf: wgpu::Buffer,
    out_storage_buf: wgpu::Buffer,
    out_readback_buf: wgpu::Buffer,
    states_readback_buf: wgpu::Buffer,
    voice_chans_buf: wgpu::Buffer,
    channel_mix_buf: wgpu::Buffer,
    mix_params_buf: wgpu::Buffer,

    render_bg: Option<wgpu::BindGroup>,
    mix_bg: Option<wgpu::BindGroup>,
    render_bg_dirty: bool,

    // State
    channels: Vec<ChannelState>,
    voices: Vec<Voice>,
    sample_offsets: std::collections::HashMap<usize, (u32, u32)>, // sample_id -> (offset, len)
    samples_next_offset: u32,
    global_frame: u64,
    pending_events: VecDeque<TimedEvent>,
    offline_events: Vec<TimedEvent>,
    offline_cursor: usize,
    active_voice_count: u32,
    // Readback staging (filled by dispatch, consumed by readback/sync).
    last_out: Option<Vec<u8>>,
    last_states: Option<Vec<u8>>,
}

impl GpuSynth {
    /// Creates a new synthesizer with the given configuration, initializing
    /// the GPU device (a wgpu adapter is picked automatically).
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::GpuInit`] if no GPU device can be created.
    pub fn new(config: SynthConfig) -> Result<Self, SynthError> {
        config.validate()?;
        let ctx = Arc::new(create_gpu_context()?);
        let res = GpuResources::new(ctx, config.block_size, config.max_voices)?;
        Self::with_resources(config, res)
    }

    /// Creates a synthesizer reusing an existing [`GpuResources`] (advanced
    /// use: multiple engines sharing one device).
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Config`] if the configuration is invalid.
    pub fn with_resources(config: SynthConfig, res: GpuResources) -> Result<Self, SynthError> {
        config.validate()?;
        let device = &res.ctx.device;
        let block = config.block_size;
        let max_voices = config.max_voices;

        let params_buf = GrowableBuffer::new(
            device,
            "voice params",
            (VoiceParams::SIZE * max_voices) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let samples_buf =
            GrowableBuffer::new(device, "samples", 1 << 20, wgpu::BufferUsages::STORAGE);
        let sinc = crate::synth::dsp::build_sinc_table();
        let sinc_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sinc table"),
            size: (sinc.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        res.ctx
            .queue
            .write_buffer(&sinc_buf, 0, bytemuck::cast_slice(&sinc));

        let env_buf = GrowableBuffer::new(
            device,
            "env stages",
            (EnvStageGpu::SIZE * max_voices * 8) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let states_buf = GrowableBuffer::new(
            device,
            "voice states",
            (VoiceState::SIZE * max_voices) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let voice_out_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voice out"),
            size: (max_voices * block * 2 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let out_storage_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out storage"),
            size: (block * 2 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out readback"),
            size: (block * 2 * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let states_readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("states readback"),
            size: (VoiceState::SIZE * max_voices) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let voice_chans_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("voice channels"),
            size: (max_voices * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let channel_mix_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("channel mix"),
            size: (ChannelMix::SIZE * ChannelMix::CHANNELS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mix_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mix params"),
            size: MixParams::SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Zero the dynamic storage buffers that are read every dispatch.
        let zero = vec![0u8; VoiceParams::SIZE * max_voices];
        res.ctx.queue.write_buffer(params_buf.buffer(), 0, &zero);
        let zero = vec![0u8; VoiceState::SIZE * max_voices];
        res.ctx.queue.write_buffer(states_buf.buffer(), 0, &zero);
        let zero = vec![0u8; EnvStageGpu::SIZE * max_voices * 8];
        res.ctx.queue.write_buffer(env_buf.buffer(), 0, &zero);
        let zero = vec![0u8; max_voices * 4];
        res.ctx.queue.write_buffer(&voice_chans_buf, 0, &zero);

        let mut engine = Self {
            config,
            res,
            sf: None,
            params_buf,
            samples_buf,
            sinc_buf,
            env_buf,
            states_buf,
            voice_out_buf,
            out_storage_buf,
            out_readback_buf,
            states_readback_buf,
            voice_chans_buf,
            channel_mix_buf,
            mix_params_buf,
            render_bg: None,
            mix_bg: None,
            render_bg_dirty: true,
            channels: (0..16).map(|_| ChannelState::new()).collect(),
            voices: Vec::new(),
            sample_offsets: std::collections::HashMap::new(),
            samples_next_offset: 0,
            global_frame: 0,
            pending_events: VecDeque::new(),
            offline_events: Vec::new(),
            offline_cursor: 0,
            active_voice_count: 0,
            last_out: None,
            last_states: None,
        };
        engine.rebuild_bind_groups();
        Ok(engine)
    }

    /// Returns the engine configuration.
    pub fn config(&self) -> &SynthConfig {
        &self.config
    }

    /// Returns the adapter info (for diagnostics).
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.res.ctx.adapter_info
    }

    /// Loads a soundfont and selects `bank`/`preset`.
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::SoundFont`] if parsing fails or the preset is
    /// missing.
    pub fn load_soundfont(
        &mut self,
        path: impl AsRef<std::path::Path>,
        bank: u16,
        preset: u16,
    ) -> Result<(), SynthError> {
        let sf = SoundFont::load(path, bank, preset, self.config.use_effects)?;
        self.sf = Some(sf);
        Ok(())
    }

    /// Unloads the current soundfont.
    pub fn unload_soundfont(&mut self) {
        self.sf = None;
    }

    /// Returns the number of currently active voices.
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// The number of frames rendered so far.
    pub fn rendered_frames(&self) -> u64 {
        self.global_frame
    }

    // ------------------------------------------------------------------
    // Real-time event injection
    // ------------------------------------------------------------------

    /// Queues a MIDI event (applied at the next block boundary).
    pub fn send_event(&mut self, channel: u8, event: MidiEvent) {
        self.pending_events.push_back(TimedEvent {
            sample: self.global_frame,
            channel: channel.min(15),
            event,
        });
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

    // ------------------------------------------------------------------
    // Block rendering
    // ------------------------------------------------------------------

    /// Renders one block of `block_size` frames into `out` (interleaved
    /// L/R, length `block_size * channels`).
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Gpu`] on dispatch/readback failures.
    pub fn render_block(&mut self, out: &mut [f32]) -> Result<(), SynthError> {
        let block = self.config.block_size;
        let chs = self.output_channels();
        if out.len() < block * chs {
            return Err(SynthError::Config("output buffer too small".into()));
        }

        let base = self.global_frame;
        self.apply_events(base, base + block as u64)?;
        self.upload_voices(base)?;
        self.upload_new_samples()?;
        self.update_mix_params()?;
        self.dispatch(base)?;
        self.readback(out)?;
        self.sync_voice_states();

        self.global_frame += block as u64;
        Ok(())
    }

    /// Renders a full MIDI file to memory, stopping once all voices have
    /// decayed below the silence threshold (mirroring XSynth's offline
    /// renderer).
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Midi`] if the file cannot be parsed, or
    /// [`SynthError::Gpu`] on GPU failures.
    pub fn render_midi_file(
        &mut self,
        midi_path: impl AsRef<std::path::Path>,
    ) -> Result<RenderResult, SynthError> {
        self.offline_cursor = 0;
        self.offline_events = Vec::new();
        self.voices.clear();
        self.global_frame = 0;
        self.active_voice_count = 0;

        let midi = MidiFile::load(midi_path, self.config.sample_rate)?;
        self.offline_events = midi.sequence.events;

        let block = self.config.block_size;
        let chs = self.output_channels();
        let mut samples: Vec<f32> = Vec::new();
        let mut block_buf = vec![0.0f32; block * chs];

        // Phase 1: process all events and render until no voices remain.
        loop {
            let events_done = self.offline_cursor >= self.offline_events.len();
            if events_done && self.voices.is_empty() {
                break;
            }
            self.render_block(&mut block_buf)?;
            samples.extend_from_slice(&block_buf);
        }

        // Phase 2: decay tail - render blocks until one is entirely silent.
        loop {
            self.render_block(&mut block_buf)?;
            let silent = block_buf
                .iter()
                .all(|s| s.abs() <= self.config.render_silence_threshold);
            if silent {
                break;
            }
            samples.extend_from_slice(&block_buf);
        }

        let frames = (samples.len() / chs) as u64;
        Ok(RenderResult {
            samples,
            sample_rate: self.config.sample_rate,
            channels: chs as u32,
            frames,
        })
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    fn output_channels(&self) -> usize {
        if self.config.channels == ChannelMode::Stereo {
            2
        } else {
            1
        }
    }

    fn apply_events(&mut self, _base: u64, end: u64) -> Result<(), SynthError> {
        // Real-time queue first.
        while let Some(ev) = self.pending_events.pop_front() {
            self.handle_event(ev)?;
        }
        // Offline event stream (events with sample < end belong to this block).
        while self.offline_cursor < self.offline_events.len() {
            let ev = self.offline_events[self.offline_cursor];
            if ev.sample >= end {
                break;
            }
            self.offline_cursor += 1;
            self.handle_event(ev)?;
        }
        Ok(())
    }

    fn handle_event(&mut self, ev: TimedEvent) -> Result<(), SynthError> {
        let ch = ev.channel as usize;
        match ev.event {
            MidiEvent::NoteOn { key, vel } => {
                if vel == 0 {
                    self.release_key(ch, key, ev.sample)
                } else {
                    self.spawn_voices(ch, key, vel, ev.sample)
                }
            }
            MidiEvent::NoteOff { key } => self.release_key(ch, key, ev.sample),
            MidiEvent::ControlChange { controller, value } => {
                self.apply_cc(ch, controller, value);
                Ok(())
            }
            MidiEvent::ProgramChange { program } => {
                self.channels[ch].program = program.min(127);
                Ok(())
            }
            MidiEvent::PitchBend { value } => {
                // 14-bit value: 0..16383, center 8192; sensitivity 2 semitones.
                let bend_semitones = (value as f32 - 8192.0) / 8192.0 * 2.0;
                let pitch_mult = 2.0f32.powf(bend_semitones / 12.0);
                self.channels[ch].pitch_multiplier = pitch_mult;
                // Propagate to active voices of this channel.
                if let Some(sf) = self.sf.as_ref() {
                    for v in &mut self.voices {
                        if v.channel as usize == ch {
                            let zone = sf.zone(v.zone_id);
                            v.speed = zone.speed_mult * pitch_mult;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn release_key(&mut self, ch: usize, key: u8, at: u64) -> Result<(), SynthError> {
        let damper = self.channels[ch].damper;
        for v in &mut self.voices {
            if v.channel as usize == ch && v.key == key && !v.released && !damper {
                v.release_at = at;
            }
            // When the damper is down, the voice stays sustained until
            // the damper is lifted (release_at stays u64::MAX).
        }
        Ok(())
    }

    fn spawn_voices(&mut self, ch: usize, key: u8, vel: u8, at: u64) -> Result<(), SynthError> {
        let sf = self.sf.as_ref().ok_or_else(|| {
            SynthError::Config("no soundfont loaded; call load_soundfont first".into())
        })?;
        let zone_ids = sf.zones_at(key, vel).to_vec();
        if zone_ids.is_empty() {
            return Ok(());
        }
        let pitch_mult = self.channels[ch].pitch_multiplier;

        for zone_id in zone_ids {
            let voice = build_voice(
                sf,
                zone_id,
                key,
                vel,
                ch as u8,
                at,
                self.config.sample_rate,
                pitch_mult,
                self.channels[ch].env_attack,
                self.channels[ch].env_release,
                self.config.envelope_curves,
            );
            let Some(mut voice) = voice else { continue };

            // Exclusive class: kill previous voices with the same class.
            if let Some(class) = voice.exclusive_class {
                self.voices.retain(|v| v.exclusive_class != Some(class));
            }

            // Voice limit: kill the oldest voice if we are over.
            if self.voices.len() >= self.config.max_voices {
                if self.voices.is_empty() {
                    return Err(SynthError::VoiceLimit(self.config.max_voices));
                }
                self.voices.remove(0);
            }

            voice.id = self.voices.len() as u32;
            self.voices.push(voice);
        }
        Ok(())
    }

    fn apply_cc(&mut self, ch: usize, controller: u8, value: u8) {
        let sr = self.config.sample_rate;
        match controller {
            0x07 => self.channels[ch].volume.set_end(value as f32 / 128.0, sr),
            0x0B => self.channels[ch]
                .expression
                .set_end(value as f32 / 128.0, sr),
            0x0A | 0x08 => self.channels[ch].pan.set_end(value as f32 / 128.0, sr),
            0x47 => {
                // Resonance (CC71): unused by the SF2 voice path in XSynth
                // (voice resonance comes from the soundfont), but tracked for
                // completeness.
                let _ = value;
            }
            0x48 => {
                // Release time (CC72): modifies the release envelope stage.
                self.channels[ch].env_release = Some(value);
                for v in &mut self.voices {
                    if v.channel as usize == ch {
                        v.env_release = Some(value);
                        refresh_env_stages(v);
                    }
                }
            }
            0x49 => {
                // Attack time (CC73): modifies the attack envelope stage.
                self.channels[ch].env_attack = Some(value);
                for v in &mut self.voices {
                    if v.channel as usize == ch {
                        v.env_attack = Some(value);
                        refresh_env_stages(v);
                    }
                }
            }
            0x40 => {
                let was_damper = self.channels[ch].damper;
                let damper = value >= 64;
                self.channels[ch].damper = damper;
                // Releasing the damper frees all voices that were sustained.
                if was_damper && !damper {
                    for v in &mut self.voices {
                        if v.channel as usize == ch
                            && !v.released
                            && v.release_at == u64::MAX
                            && v.state.ended == 0
                        {
                            v.release_at = self.global_frame;
                        }
                    }
                }
            }
            0x79 => {
                // Reset all controllers.
                self.channels[ch] = ChannelState::new();
            }
            0x7B => {
                // All notes off.
                for v in &mut self.voices {
                    if v.channel as usize == ch {
                        v.release_at = self.global_frame;
                    }
                }
            }
            0x78 => {
                // All sounds off: kill immediately.
                self.voices.retain(|v| v.channel as usize != ch);
            }
            _ => {}
        }
    }

    fn upload_voices(&mut self, base: u64) -> Result<(), SynthError> {
        let device = &self.res.ctx.device;
        let queue = &self.res.ctx.queue;

        // Drop voices that ended (state refreshed by the previous readback).
        self.voices.retain(|v| v.state.ended == 0);

        let n = self.voices.len();
        let mut params = vec![VoiceParams::zeroed(); n.max(1)];
        let mut states = vec![VoiceState::zeroed(); n.max(1)];
        let mut env_stages: Vec<EnvStageGpu> = Vec::new();
        let mut voice_chans = vec![0u32; n.max(1)];

        for (i, v) in self.voices.iter_mut().enumerate() {
            let sample_offset = self
                .sample_offsets
                .get(&v.sample_id)
                .map(|(off, _)| *off)
                .unwrap_or(0);
            let sample_offset_r = self
                .sample_offsets
                .get(&v.sample_id_r)
                .map(|(off, _)| *off)
                .unwrap_or(sample_offset);
            v.sample_offset_r = sample_offset_r;
            let env_base = env_stages.len() as u32;
            let stages = v.gpu_env_stages();
            env_stages.extend_from_slice(&stages);
            params[i] = v.gpu_params(
                sample_offset,
                sample_offset_r,
                env_base,
                base,
                self.config.interpolation,
            );
            states[i] = v.state;
            voice_chans[i] = v.channel as u32;
        }

        self.params_buf
            .write(device, queue, 0, bytemuck::cast_slice(&params));
        self.states_buf
            .write(device, queue, 0, bytemuck::cast_slice(&states));
        self.env_buf
            .write(device, queue, 0, bytemuck::cast_slice(&env_stages));
        queue.write_buffer(&self.voice_chans_buf, 0, bytemuck::cast_slice(&voice_chans));

        self.active_voice_count = n as u32;
        self.render_bg_dirty = true;
        Ok(())
    }

    fn upload_new_samples(&mut self) -> Result<(), SynthError> {
        let sf = match self.sf.as_mut() {
            Some(sf) => sf,
            None => return Ok(()),
        };

        // Find samples that need uploading (both channels).
        let mut needed: Vec<usize> = Vec::new();
        for v in &self.voices {
            if !self.sample_offsets.contains_key(&v.sample_id) {
                needed.push(v.sample_id);
            }
            if !self.sample_offsets.contains_key(&v.sample_id_r) {
                needed.push(v.sample_id_r);
            }
        }
        if needed.is_empty() {
            return Ok(());
        }
        needed.sort_unstable();
        needed.dedup();

        let device = &self.res.ctx.device;
        let queue = &self.res.ctx.queue;

        for sample_id in needed {
            let data = sf.resample(sample_id, self.config.sample_rate);
            let len = data.len() as u32;
            let offset = self.samples_next_offset;
            let grown = self.samples_buf.write(
                device,
                queue,
                offset as u64 * 4,
                bytemuck::cast_slice(&data),
            );
            if grown {
                self.render_bg_dirty = true;
            }
            self.sample_offsets.insert(sample_id, (offset, len));
            self.samples_next_offset = offset + len;
        }
        Ok(())
    }

    fn update_mix_params(&mut self) -> Result<(), SynthError> {
        let queue = &self.res.ctx.queue;
        let block = self.config.block_size as u32;

        // Build per-channel mix curves. The lerp states are advanced by the
        // block length so `start` is the value at the first frame and
        // `delta` interpolates towards `end` (10 ms smoothing).
        let mut mixes = Vec::with_capacity(ChannelMix::CHANNELS);
        for ch in &mut self.channels {
            let vol_start = ch.volume.advance(block);
            let vol_end = ch.volume.end;
            let vol_delta = ch.volume.step;
            let expr_start = ch.expression.advance(block);
            let expr_end = ch.expression.end;
            let expr_delta = ch.expression.step;
            let pan_start = ch.pan.advance(block);
            let pan_end = ch.pan.end;
            let pan_delta = ch.pan.step;
            mixes.push(ChannelMix {
                vol_start,
                vol_delta,
                vol_end,
                expr_start,
                expr_delta,
                expr_end,
                pan_start,
                pan_delta,
                pan_end,
            });
        }
        while mixes.len() < ChannelMix::CHANNELS {
            mixes.push(ChannelMix::zeroed());
        }

        queue.write_buffer(&self.channel_mix_buf, 0, bytemuck::cast_slice(&mixes));
        queue.write_buffer(
            &self.mix_params_buf,
            0,
            bytemuck::cast_slice(&[MixParams {
                voice_count: self.active_voice_count,
                block_size: block,
                channel_count: 16,
                reserved: 0,
            }]),
        );
        Ok(())
    }

    fn rebuild_bind_groups(&mut self) {
        let device = &self.res.ctx.device;
        self.render_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render bind group"),
            layout: &self.res.render_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.samples_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.sinc_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.env_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.states_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.voice_out_buf.as_entire_binding(),
                },
            ],
        }));
        self.mix_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mix bind group"),
            layout: &self.res.mix_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.voice_out_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.out_storage_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.voice_chans_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.channel_mix_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.mix_params_buf.as_entire_binding(),
                },
            ],
        }));
        self.render_bg_dirty = false;
    }

    fn dispatch(&mut self, _base: u64) -> Result<(), SynthError> {
        if self.render_bg_dirty {
            self.rebuild_bind_groups();
        }

        let device = &self.res.ctx.device;
        let queue = &self.res.ctx.queue;
        let block = self.config.block_size as u32;
        let voices = self.active_voice_count;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lumino block encoder"),
        });

        let render_bg = self
            .render_bg
            .as_ref()
            .ok_or_else(|| SynthError::Gpu("render bind group missing".into()))?;
        let mix_bg = self
            .mix_bg
            .as_ref()
            .ok_or_else(|| SynthError::Gpu("mix bind group missing".into()))?;

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("render pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.res.render_pipeline);
            pass.set_bind_group(0, render_bg, &[]);
            pass.dispatch_workgroups(voices.div_ceil(128).max(1), 1, 1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mix pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.res.mix_pipeline);
            pass.set_bind_group(0, mix_bg, &[]);
            pass.dispatch_workgroups(block.div_ceil(128).max(1), 1, 1);
        }

        // Readbacks.
        encoder.copy_buffer_to_buffer(
            &self.out_storage_buf,
            0,
            &self.out_readback_buf,
            0,
            (self.config.block_size * 2 * 4) as u64,
        );
        encoder.copy_buffer_to_buffer(
            self.states_buf.buffer(),
            0,
            &self.states_readback_buf,
            0,
            (VoiceState::SIZE * self.config.max_voices) as u64,
        );

        queue.submit(Some(encoder.finish()));

        // Poll and map both readback buffers.
        self.last_out = Some(map_readback(device, &self.out_readback_buf)?);
        self.last_states = Some(map_readback(device, &self.states_readback_buf)?);
        Ok(())
    }

    fn readback(&mut self, out: &mut [f32]) -> Result<(), SynthError> {
        let Some(data) = self.last_out.take() else {
            return Err(SynthError::Gpu("no output data".into()));
        };
        let count = (data.len() / 4).min(out.len());
        out[..count].copy_from_slice(bytemuck::cast_slice(&data[..count * 4]));
        Ok(())
    }

    /// Applies the read-back voice states to the CPU mirror.
    fn sync_voice_states(&mut self) {
        let Some(states) = self.last_states.take() else {
            return;
        };
        let count = states.len() / VoiceState::SIZE;
        for (i, v) in self.voices.iter_mut().enumerate() {
            if i < count {
                let off = i * VoiceState::SIZE;
                let st: &VoiceState = bytemuck::from_bytes(&states[off..off + VoiceState::SIZE]);
                v.state = *st;
                if st.ended != 0 {
                    v.released = true;
                }
            }
        }
    }
}

fn map_readback(device: &wgpu::Device, buf: &wgpu::Buffer) -> Result<Vec<u8>, SynthError> {
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r.is_ok());
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| SynthError::Gpu(format!("poll failed: {e:?}")))?;
    if rx.recv().unwrap_or(false) {
        let data = slice.get_mapped_range().to_vec();
        buf.unmap();
        Ok(data)
    } else {
        Err(SynthError::Gpu("buffer map failed".into()))
    }
}
