//! The `GpuSynth` engine: MIDI event scheduling, voice management and
//! block-wise GPU rendering.

use std::collections::VecDeque;
use std::sync::Arc;

use bytemuck::Zeroable;
use rayon::prelude::*;

use crate::config::{ChannelMode, SynthConfig};
use crate::error::SynthError;
use crate::gpu::{
    EnvStageGpu, GpuResources, GrowableBuffer, MIX_CHANNELS, MixEvent, MixParams, MixStart,
    SAMPLES_CHUNK_BINDING_BASE, SAMPLES_CHUNK_BYTES, SAMPLES_CHUNKS, VoiceParams, VoiceState,
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

/// Hard ceiling for a single offline render (≈ 13.6 h @ 64 kHz). Keeps the
/// guard well below the 2^32 frame range of the u32 GPU timestamps.
const MAX_RENDER_FRAMES: u64 = 1 << 31;

/// Voice states are read back every N-th block (see
/// `GpuSynth::states_sync_counter`).
///
/// This must be small: ended voices are only pruned after a readback, and
/// with a large block size a lag of a few blocks lets thousands of dead
/// voices accumulate (dense MIDI adds thousands per block), bloating the
/// GPU voice pool to the physical buffer-size wall. One block of lag is the
/// right trade-off (a single extra map per block is cheap).
const STATES_SYNC_EVERY: u32 = 1;

/// Hard cap for the voice output buffer (per-voice output for one block).
/// The wgpu/D3D12-style maximum buffer size is 2 GiB - 1; staying well
/// below it keeps headroom. `max_voices` must be chosen so the *peak*
/// active voice count stays under this (a dense MIDI may need 32k+).
const MAX_VOICE_OUT_BYTES: u64 = (1 << 30) + (1 << 29); // 1.5 GiB

/// A 10 ms linear-smoothed controller value (mirror of XSynth's `ValueLerp`).
#[derive(Debug, Clone, Copy)]
struct LerpState {
    /// Absolute frame of the last `advance_to` call.
    frame: u64,
    current: f32,
    end: f32,
    step: f32,
}

impl LerpState {
    fn new(initial: f32) -> Self {
        Self {
            frame: 0,
            current: initial,
            end: initial,
            step: 0.0,
        }
    }

    fn set_end(&mut self, end: f32, sample_rate: u32) {
        self.step = (end - self.current) / (sample_rate as f32 * 0.01);
        self.end = end;
    }

    /// Advances the lerp to the absolute `target` frame, returning the value
    /// at that point. Frame-exact, so the result does not depend on the
    /// block size.
    fn advance_to(&mut self, target: u64) -> f32 {
        let n = target.saturating_sub(self.frame);
        if n > 0 {
            self.frame = target;
            if self.end > self.current {
                self.current = (self.current + self.step * n as f32).min(self.end);
            } else if self.end < self.current {
                self.current = (self.current + self.step * n as f32).max(self.end);
            }
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
    /// Resampled sample data, split across several capped chunks so no
    /// single storage binding exceeds the 128 MiB limit (D3D12).
    samples_chunks: Vec<GrowableBuffer>,
    sinc_buf: wgpu::Buffer,
    env_buf: GrowableBuffer,
    states_buf: GrowableBuffer,
    /// Per-voice output, grown on demand so dense MIDI never runs out of
    /// voice slots (the pool is a *physical* limit, not a polyphony one).
    voice_out_buf: GrowableBuffer,
    out_storage_buf: wgpu::Buffer,
    /// Double-buffered readback so the CPU can wait for the *previous*
    /// submission while the current one is still running on the GPU
    /// (CPU/GPU pipelining).
    out_readback: [wgpu::Buffer; 2],
    out_readback_cur: usize,
    /// Double-buffered voice-state readback: the copy lands in one buffer,
    /// the map reads the other (states from several blocks ago), so the
    /// wait only ever needs to cover already-completed work.
    states_readback: [GrowableBuffer; 2],
    states_readback_cur: usize,
    /// Per-voice channel ids, grown like `voice_out_buf`.
    voice_chans_buf: GrowableBuffer,
    /// Per-block controller events (frame-exact, replayed by the mix pass).
    mix_events_buf: GrowableBuffer,
    mix_params_buf: wgpu::Buffer,

    render_bg: Option<wgpu::BindGroup>,
    mix_bg: Option<wgpu::BindGroup>,
    render_bg_dirty: bool,
    mix_bg_dirty: bool,

    // State
    channels: Vec<ChannelState>,
    voices: Vec<Voice>,
    /// Per-(channel,key) positions of active voices, rebuilt after every
    /// voice-list mutation (`retain`) so note-on/note-off handling is O(1)
    /// instead of scanning the whole voice list (dense MIDI can hold tens
    /// of thousands of voices and millions of note events).
    key_voices: std::collections::HashMap<(u8, u8), VecDeque<usize>>,
    sample_offsets: std::collections::HashMap<usize, (u32, u32)>, // sample_id -> (offset, len)
    samples_next_offset: u32,
    global_frame: u64,
    pending_events: VecDeque<TimedEvent>,
    offline_events: Vec<TimedEvent>,
    offline_cursor: usize,
    /// Volume/expression/pan CC events deferred to the mix stage so they are
    /// applied at their exact frame (not at the block boundary): a tuple of
    /// `(sample, channel, controller, value)`.
    pending_mix_events: Vec<(u64, u8, u8, u8)>,
    active_voice_count: u32,
    // Readback staging (filled by dispatch, consumed by readback/sync).
    last_out: Option<Vec<u8>>,
    last_states: Option<Vec<u8>>,
    /// Voice ids of the last uploaded voice list, in upload order; used to
    /// map the read-back states onto the current (possibly shrunk) list.
    prev_voice_ids: Vec<u32>,
    /// Monotonic note-on counter; every zone voice of one note-on shares
    /// the current value as its `note_id`.
    note_counter: u64,
    /// Monotonic voice id counter. Voice ids must be unique for the lifetime
    /// of the engine: `upload_voices` maps read-back GPU states back to
    /// voices by id, and reusing ids (e.g. the array position) would apply a
    /// stale state to the wrong voice and roll its envelope back.
    voice_id_counter: u32,
    /// Voice states are only read back every `STATES_SYNC_EVERY` blocks:
    /// a voice ending late does not change any audio sample, and skipping
    /// the extra map/poll round trip per block is a large CPU win.
    states_sync_counter: u32,
    /// Submission index of the previously dispatched block; the CPU waits
    /// for this one (not the current) so GPU work pipelines with CPU work.
    prev_submission: Option<wgpu::SubmissionIndex>,
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
        let samples_chunks = (0..SAMPLES_CHUNKS)
            .map(|i| {
                GrowableBuffer::with_max_capacity(
                    device,
                    &format!("samples chunk {i}"),
                    1 << 20,
                    SAMPLES_CHUNK_BYTES,
                    wgpu::BufferUsages::STORAGE,
                )
            })
            .collect::<Vec<_>>();
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
        let voice_out_buf = GrowableBuffer::with_max_capacity(
            device,
            "voice out",
            (max_voices * block * 2 * 4) as u64,
            MAX_VOICE_OUT_BYTES,
            wgpu::BufferUsages::STORAGE,
        );
        let out_storage_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out storage"),
            size: (block * 2 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_readback = [
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("out readback 0"),
                size: (block * 2 * 4) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("out readback 1"),
                size: (block * 2 * 4) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ];
        let states_readback = [
            GrowableBuffer::new(
                device,
                "states readback 0",
                (VoiceState::SIZE * max_voices) as u64,
                wgpu::BufferUsages::MAP_READ,
            ),
            GrowableBuffer::new(
                device,
                "states readback 1",
                (VoiceState::SIZE * max_voices) as u64,
                wgpu::BufferUsages::MAP_READ,
            ),
        ];
        let voice_chans_buf = GrowableBuffer::new(
            device,
            "voice channels",
            (max_voices * 4) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let mix_events_buf =
            GrowableBuffer::new(device, "mix events", 16 << 10, wgpu::BufferUsages::STORAGE);
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
        let mut voice_chans_buf = voice_chans_buf;
        let _ = voice_chans_buf.write(&res.ctx.device, &res.ctx.queue, 0, &zero);

        let mut engine = Self {
            config,
            res,
            sf: None,
            params_buf,
            samples_chunks,
            sinc_buf,
            env_buf,
            states_buf,
            voice_out_buf,
            out_storage_buf,
            out_readback,
            out_readback_cur: 0,
            states_readback,
            states_readback_cur: 0,
            voice_chans_buf,
            mix_events_buf,
            mix_params_buf,
            render_bg: None,
            mix_bg: None,
            render_bg_dirty: true,
            mix_bg_dirty: true,
            channels: (0..16).map(|_| ChannelState::new()).collect(),
            voices: Vec::new(),
            key_voices: std::collections::HashMap::new(),
            sample_offsets: std::collections::HashMap::new(),
            samples_next_offset: 0,
            global_frame: 0,
            pending_events: VecDeque::new(),
            offline_events: Vec::new(),
            offline_cursor: 0,
            pending_mix_events: Vec::new(),
            active_voice_count: 0,
            last_out: None,
            last_states: None,
            prev_voice_ids: Vec::new(),
            note_counter: 0,
            voice_id_counter: 0,
            states_sync_counter: 0,
            prev_submission: None,
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

    /// Diagnostics: `(voices, released, ended)` - how many voices exist,
    /// how many have a release scheduled, and how many the GPU marked ended.
    #[doc(hidden)]
    pub fn debug_voice_lifecycle(&self) -> (usize, usize, usize) {
        let released = self
            .voices
            .iter()
            .filter(|v| v.released || v.release_at != u64::MAX)
            .count();
        let ended = self.voices.iter().filter(|v| v.state.ended != 0).count();
        (self.voices.len(), released, ended)
    }

    /// Diagnostics: details of the first voice's GPU state.
    #[doc(hidden)]
    pub fn debug_voice_state(&self) -> Option<(u32, u32, u32, u32, u64, u64)> {
        let v = self.voices.first()?;
        Some((
            v.state.is_released,
            v.state.ended,
            v.state.env_stage,
            v.state.env_t,
            v.release_at,
            v.start_at,
        ))
    }

    /// Diagnostics: per-voice `(key, vel, speed, amp, released, ended,
    /// env_stage, env_t, release_at, gpu_is_released, env_from)`.
    #[doc(hidden)]
    pub fn debug_voices(&self) -> Vec<(u8, u8, f32, f32, bool, bool, u32, u32, u64, u32, f32)> {
        self.voices
            .iter()
            .map(|v| {
                (
                    v.key,
                    v.vel,
                    v.speed,
                    v.amp,
                    v.released || v.release_at != u64::MAX,
                    v.state.ended != 0,
                    v.state.env_stage,
                    v.state.env_t,
                    v.release_at,
                    v.state.is_released,
                    v.state.env_from,
                )
            })
            .collect()
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

        let prof = std::env::var("LUMINO_PROFILE").is_ok();
        let base = self.global_frame;

        let t0 = std::time::Instant::now();
        self.apply_events(base, base + block as u64)?;
        let t1 = std::time::Instant::now();

        // Fast path: no voices at all - the block is pure silence (the mix
        // pass would sum nothing). Advance the controller states so CC
        // smoothing stays continuous and skip the whole GPU round trip.
        // Dense-but-sparse MIDI spends a large fraction of its timeline in
        // note gaps, so this is a significant win.
        if self.voices.is_empty() {
            self.update_mix_params(base)?;
            out[..block * chs].fill(0.0);
            self.global_frame += block as u64;
            // No GPU work happened; drop any stale read-back states so a
            // later block cannot resume from a mismatched voice list.
            self.last_states = None;
            self.prev_voice_ids.clear();
            if prof {
                eprintln!(
                    "[profile] block {}: silent skip",
                    self.global_frame / block as u64 - 1
                );
            }
            return Ok(());
        }

        self.upload_voices(base)?;
        let t2 = std::time::Instant::now();
        self.upload_new_samples()?;
        let t3 = std::time::Instant::now();
        self.update_mix_params(base)?;
        let t4 = std::time::Instant::now();
        self.dispatch(base)?;
        let t5 = std::time::Instant::now();
        self.readback(out)?;
        let t6 = std::time::Instant::now();
        // States are read back every block (STATES_SYNC_EVERY = 1 keeps the
        // counter at 0, so every dispatch maps them); apply the readback
        // unconditionally. sync_voice_states is a no-op when no states are
        // pending, which keeps this correct if the cadence is ever changed.
        self.sync_voice_states();

        if prof && self.global_frame.is_multiple_of(block as u64 * 25) {
            let block_no = self.global_frame / block as u64;
            eprintln!(
                "[profile] block {block_no}: apply={}us upload={}us samples={}us mix={}us dispatch={}us readback={}us voices={}",
                (t1 - t0).as_micros(),
                (t2 - t1).as_micros(),
                (t3 - t2).as_micros(),
                (t4 - t3).as_micros(),
                (t5 - t4).as_micros(),
                (t6 - t5).as_micros(),
                self.voices.len()
            );
        }

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
        self.render_midi_inner(midi_path, None)
    }

    /// Pre-warms the GPU sample cache with every sample the MIDI file will
    /// use (resampled and uploaded up front).
    ///
    /// Use this before realtime playback so the render loop never stalls on
    /// a lazily-resampled sample during dense sections — otherwise a single
    /// large sample can take hundreds of milliseconds to resample+upload in
    /// the middle of a block, emptying the audio queue and causing crackle.
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Midi`] if the file cannot be parsed, or
    /// [`SynthError::Gpu`] on GPU failures.
    pub fn prewarm_midi_file(
        &mut self,
        midi_path: impl AsRef<std::path::Path>,
    ) -> Result<(), SynthError> {
        let midi = MidiFile::load(midi_path, self.config.sample_rate)?;
        let events = &midi.sequence.events;
        if let Some(sf) = self.sf.as_ref() {
            let mut wanted: Vec<usize> = Vec::new();
            for ev in events {
                if let MidiEvent::NoteOn { key, vel } = ev.event {
                    for &zid in sf.zones_at(key, vel) {
                        let zone = sf.zone(zid);
                        wanted.push(zone.sample_id);
                        wanted.push(zone.sample_id_r);
                    }
                }
            }
            wanted.sort_unstable();
            wanted.dedup();
            let rate = self.config.sample_rate;
            let pre: Vec<(usize, Arc<[f32]>)> = wanted
                .par_iter()
                .map(|&id| (id, sf.resample_uncached(id, rate)))
                .collect();
            let sf = self.sf.as_mut().expect("soundfont present");
            let device = &self.res.ctx.device;
            let queue = &self.res.ctx.queue;
            let mut grown = false;
            for (id, data) in pre {
                sf.cache_resampled(id, rate, data.clone());
                let len = data.len() as u32;
                let offset = self.samples_next_offset;
                grown |= write_samples(
                    &mut self.samples_chunks,
                    device,
                    queue,
                    offset as u64 * 4,
                    bytemuck::cast_slice(&data),
                )?;
                self.sample_offsets.insert(id, (offset, len));
                self.samples_next_offset = offset + len;
            }
            if grown {
                self.render_bg_dirty = true;
            }
        }
        Ok(())
    }

    /// Renders the first `frames` frames of a MIDI file (used to compare the
    /// beginning of long MIDIs without rendering the whole piece).
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Midi`] if the file cannot be parsed, or
    /// [`SynthError::Gpu`] on GPU failures.
    pub fn render_midi_frames(
        &mut self,
        midi_path: impl AsRef<std::path::Path>,
        frames: u64,
    ) -> Result<RenderResult, SynthError> {
        self.render_midi_inner(midi_path, Some(frames))
    }

    fn render_midi_inner(
        &mut self,
        midi_path: impl AsRef<std::path::Path>,
        limit_frames: Option<u64>,
    ) -> Result<RenderResult, SynthError> {
        self.offline_cursor = 0;
        self.offline_events = Vec::new();
        self.voices.clear();
        self.global_frame = 0;
        self.active_voice_count = 0;
        self.last_states = None;
        self.last_out = None;
        self.prev_voice_ids.clear();

        let prof = std::env::var("LUMINO_PROFILE").is_ok();
        let t0 = std::time::Instant::now();
        let midi = MidiFile::load(midi_path, self.config.sample_rate)?;
        let t1 = std::time::Instant::now();
        self.offline_events = midi.sequence.events;

        // Pre-warm resampling AND upload: resolve every sample the MIDI will
        // use, resample it in parallel and upload it to the GPU up front, so
        // the render loop never stalls on a lazily-resampled sample or pays
        // per-block sample uploads during the dense sections.
        if let Some(sf) = self.sf.as_ref() {
            let mut wanted: Vec<usize> = Vec::new();
            for ev in &self.offline_events {
                if let MidiEvent::NoteOn { key, vel } = ev.event {
                    for &zid in sf.zones_at(key, vel) {
                        let zone = sf.zone(zid);
                        wanted.push(zone.sample_id);
                        wanted.push(zone.sample_id_r);
                    }
                }
            }
            wanted.sort_unstable();
            wanted.dedup();
            let rate = self.config.sample_rate;
            let pre: Vec<(usize, Arc<[f32]>)> = wanted
                .par_iter()
                .map(|&id| (id, sf.resample_uncached(id, rate)))
                .collect();
            let sf = self.sf.as_mut().expect("soundfont present");
            let device = &self.res.ctx.device;
            let queue = &self.res.ctx.queue;
            let mut grown = false;
            for (id, data) in pre {
                sf.cache_resampled(id, rate, data.clone());
                let len = data.len() as u32;
                let offset = self.samples_next_offset;
                grown |= write_samples(
                    &mut self.samples_chunks,
                    device,
                    queue,
                    offset as u64 * 4,
                    bytemuck::cast_slice(&data),
                )?;
                self.sample_offsets.insert(id, (offset, len));
                self.samples_next_offset = offset + len;
            }
            if grown {
                self.render_bg_dirty = true;
            }
        }

        // Render timeout guard: the offline loops must terminate on their own
        // (events consumed + silence / no voices). A voice that can never
        // finish - held damper, missing note-off, pathological envelope -
        // would otherwise loop forever. Abort once the last event is behind
        // us by `max_tail_seconds`; the hard cap keeps the guard well inside
        // the u32 frame range used by the GPU parameters.
        let events_end = self.offline_events.last().map_or(0, |e| e.sample);
        let tail_budget =
            (self.config.max_tail_seconds as f64 * self.config.sample_rate as f64) as u64;
        let max_frames = match limit_frames {
            Some(n) => n.min(MAX_RENDER_FRAMES),
            None => events_end
                .saturating_add(tail_budget)
                .min(MAX_RENDER_FRAMES),
        };
        let limited = limit_frames.is_some();

        let block = self.config.block_size;
        let chs = self.output_channels();
        let threshold = self.config.render_silence_threshold;
        let mut samples: Vec<f32> = Vec::new();
        let mut block_buf = vec![0.0f32; block * chs];

        // Progress reporting: the total is the render horizon (`max_frames`).
        // Phase 1 walks the event stream; the tail phase renders past the
        // last event, so the bar is allowed to exceed 100% there.
        let mut progress = ProgressBar::new(max_frames, self.config.show_progress);

        // Phase 1: process all events and render until no voices remain. If
        // the events are exhausted and the block went silent, we stop even
        // when voices linger (they are stuck in sustain and contribute
        // nothing; the tail below would be silent too).
        loop {
            let events_done = self.offline_cursor >= self.offline_events.len();
            if events_done && self.voices.is_empty() {
                if prof {
                    eprintln!(
                        "[render] break: events_done+empty at frame {}",
                        self.global_frame
                    );
                }
                break;
            }
            self.render_block(&mut block_buf)?;
            progress.tick(self.global_frame);
            let silent = block_buf.iter().all(|s| s.abs() <= threshold);
            if events_done && silent {
                if prof {
                    eprintln!(
                        "[render] break: events_done+silent at frame {} cursor={}/{}",
                        self.global_frame,
                        self.offline_cursor,
                        self.offline_events.len()
                    );
                }
                break;
            }
            if self.global_frame >= max_frames {
                if limited {
                    break;
                }
                return Err(self.render_timeout(&block_buf));
            }
            samples.extend_from_slice(&block_buf);
        }

        // Phase 2: decay tail - render blocks until one is entirely silent.
        loop {
            self.render_block(&mut block_buf)?;
            progress.tick(self.global_frame);
            let silent = block_buf.iter().all(|s| s.abs() <= threshold);
            if silent || (limited && self.global_frame >= max_frames) {
                break;
            }
            if self.global_frame >= max_frames {
                return Err(self.render_timeout(&block_buf));
            }
            samples.extend_from_slice(&block_buf);
        }

        // Drain the pipeline: the data of the last submitted block is only
        // read back by one more render. Append it if it is not silence
        // (the loops above already consumed every non-silent block).
        self.render_block(&mut block_buf)?;
        if block_buf.iter().any(|s| s.abs() > threshold) {
            samples.extend_from_slice(&block_buf);
        }
        progress.finish();

        if prof {
            let t2 = std::time::Instant::now();
            eprintln!(
                "[profile] midi load: {:?}, render loops: {:?}, flush: {:?}",
                t1 - t0,
                t2 - t1,
                t2.elapsed()
            );
        }

        let frames = (samples.len() / chs) as u64;
        Ok(RenderResult {
            samples,
            sample_rate: self.config.sample_rate,
            channels: chs as u32,
            frames,
        })
    }

    /// Builds the error reported when offline rendering exceeds its frame
    /// budget (a voice never finished).
    fn render_timeout(&self, last_block: &[f32]) -> SynthError {
        let last_peak = last_block.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        SynthError::RenderTimeout {
            frames: self.global_frame,
            active_voices: self.voices.len(),
            last_peak,
        }
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
                // Velocity 0 is converted to a note-off by the parser; a
                // velocity of 1 is a barely-audible note that XSynth does
                // not render. Dropping it saves a voice slot without any
                // audible change.
                if vel <= 1 {
                    Ok(())
                } else {
                    self.spawn_voices(ch, key, vel, ev.sample)
                }
            }
            MidiEvent::NoteOff { key } => self.release_key(ch, key, ev.sample),
            MidiEvent::ControlChange { controller, value } => {
                match controller {
                    // Channel mix controllers: deferred for frame-exact
                    // application at the mix stage.
                    0x07 | 0x0B | 0x0A | 0x08 => {
                        self.pending_mix_events
                            .push((ev.sample, ch as u8, controller, value));
                    }
                    _ => self.apply_cc(ch, controller, value),
                }
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
        // Indexed by (channel, key): only the voices of this key are touched.
        //
        // XSynth releases exactly one note per NoteOff - the *oldest* note
        // not yet releasing (FIFO, `release_next_voice`), and it releases
        // the whole note *group* (all zone voices spawned by that note-on)
        // at once. Releasing every voice of the key would cut newer notes
        // early; releasing a single zone would split a stereo pair.
        if let Some(positions) = self.key_voices.get(&(ch as u8, key)) {
            let mut note_id: Option<u64> = None;
            for &pos in positions {
                let Some(v) = self.voices.get(pos) else {
                    continue;
                };
                if v.released || v.release_at != u64::MAX {
                    continue;
                }
                // First not-yet-releasing note wins; release all of its
                // zone voices together.
                note_id = Some(v.note_id);
                break;
            }
            if let Some(nid) = note_id {
                for &pos in positions {
                    if let Some(v) = self.voices.get_mut(pos)
                        && v.note_id == nid
                        && !damper
                    {
                        v.release_at = at;
                    }
                    // When the damper is down, the voice stays sustained
                    // until the damper is lifted (release_at stays MAX).
                }
            }
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
        let zone_count = zone_ids.len();
        let pitch_mult = self.channels[ch].pitch_multiplier;

        // Build every zone voice of this note up front so that polyphony
        // management (exclusive class + per-key stealing) runs *once per
        // note* instead of once per zone. The old code ran the steal logic
        // inside the zone loop, so the second zone of a stereo pair could
        // steal the first zone's voice pushed milliseconds earlier - the
        // cause of dropped notes on dense repeated single keys.
        let mut built: Vec<Voice> = Vec::with_capacity(zone_count);
        for zone_id in zone_ids {
            if let Some(v) = build_voice(
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
            ) {
                built.push(v);
            }
        }
        if built.is_empty() {
            return Ok(());
        }

        self.note_counter += 1;
        let note_id = self.note_counter;

        // Exclusive class: kill previous voices with the same class and
        // rebuild the per-key index. One class check per note (the zones of
        // a note share their class).
        if let Some(class) = built.iter().find_map(|v| v.exclusive_class) {
            self.voices.retain(|v| v.exclusive_class != Some(class));
            self.rebuild_key_voices();
        }

        // XSynth-style per-key polyphony limit: a new note-on may only
        // steal from the voices of the *same key* on this channel, keeping
        // at most `max_voices_per_key` layers per key (counting all zones).
        // There is no global voice cap - dense MIDI never drops unrelated
        // notes.
        //
        // Mirror XSynth's `pop_quietest_voice_group`: steal the *quietest*
        // voices of the key (smallest velocity), not the oldest. Killing a
        // barely-audible layer produces no audible click and preserves the
        // loud melodic line, so repeated notes sound like XSynth instead of
        // a chopped, clicky mess.
        //
        // XSynth steals whole note *groups* (all zones spawned by one
        // note-on), so a stereo pair is never split. We do the same: voices
        // of one note share `note_id` and are killed together.
        //
        // The stolen voices end *immediately* (no release tail): the layer
        // limit counts voices of the key, so a repeated note replaces its
        // own predecessors instead of stacking inaudible release tails.
        // This keeps the voice pool bounded by `keys x max_voices_per_key`
        // even for pathological note densities.
        //
        // Voices already marked `ended` (killed by a previous steal, still
        // in the array until the next block upload) occupy a slot without
        // any audible content - they are released for free first, so the
        // quota only counts voices that can actually be heard.
        let limit = self.config.max_voices_per_key;
        if limit > 0 {
            let mut kept: VecDeque<usize> = VecDeque::new();
            let mut killed = 0usize;
            if let Some(positions) = self.key_voices.get(&(ch as u8, key)) {
                // Group consecutive same-note voices (spawn order keeps one
                // note's zones adjacent); ended voices are freed first.
                let mut groups: Vec<(u8, Vec<usize>)> = Vec::new();
                let mut stale: Vec<usize> = Vec::new();
                for &pos in positions {
                    let Some(v) = self.voices.get(pos) else {
                        continue;
                    };
                    if v.state.ended != 0 {
                        stale.push(pos);
                        continue;
                    }
                    let (vel, note) = (v.vel, v.note_id);
                    match groups.last_mut() {
                        Some((_, g)) if self.voices[g[0]].note_id == note => g.push(pos),
                        _ => groups.push((vel, vec![pos])),
                    }
                }
                // Free already-ended voices for free, then kill whole
                // quietest groups until the new note (zone_count voices)
                // keeps the key at or under `limit`.
                let active = groups.iter().map(|(_, g)| g.len()).sum::<usize>();
                let need_free = active.saturating_sub(limit.saturating_sub(zone_count));
                groups.sort_by_key(|&(vel, _)| vel);
                let mut freed = 0usize;
                let mut kill_groups = 0usize;
                for (_, g) in &groups {
                    if freed >= need_free {
                        break;
                    }
                    freed += g.len();
                    kill_groups += 1;
                }
                let mut kill_set: Vec<usize> = Vec::new();
                for (_, g) in groups.iter().take(kill_groups) {
                    kill_set.extend(g.iter().copied());
                }
                for &pos in positions {
                    if kill_set.contains(&pos) || stale.contains(&pos) {
                        if let Some(v) = self.voices.get_mut(pos) {
                            v.state.ended = 1;
                            killed += 1;
                        }
                    } else {
                        kept.push_back(pos);
                    }
                }
            }
            if killed > 0 {
                self.key_voices.insert((ch as u8, key), kept);
            }
        }

        for mut voice in built {
            voice.id = self.voice_id_counter;
            self.voice_id_counter += 1;
            voice.note_id = note_id;
            let pos = self.voices.len();
            self.voices.push(voice);
            self.key_voices
                .entry((ch as u8, key))
                .or_default()
                .push_back(pos);
        }
        Ok(())
    }

    /// Rebuilds the per-key voice index after any mutation of `voices`
    /// (retain-based removal changes all positions).
    fn rebuild_key_voices(&mut self) {
        self.key_voices.clear();
        for (i, v) in self.voices.iter().enumerate() {
            self.key_voices
                .entry((v.channel, v.key))
                .or_default()
                .push_back(i);
        }
    }

    fn apply_cc(&mut self, ch: usize, controller: u8, value: u8) {
        let sr = self.config.sample_rate;
        match controller {
            // CC7 (volume), CC11 (expression), CC10/CC8 (pan) are handled by
            // `handle_event` -> `defer_mix_cc` for frame-exact application;
            // they never reach this function.
            0x07 | 0x0B | 0x0A | 0x08 => {
                debug_assert!(false, "CC7/11/10/8 must go through defer_mix_cc");
                let _ = sr;
            }
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
                self.rebuild_key_voices();
            }
            _ => {}
        }
    }

    fn upload_voices(&mut self, base: u64) -> Result<(), SynthError> {
        // Drop voices that ended (state refreshed by the previous readback)
        // and rebuild the per-key index before borrowing the GPU device.
        self.voices.retain(|v| v.state.ended == 0);
        self.rebuild_key_voices();

        let device = &self.res.ctx.device;
        let queue = &self.res.ctx.queue;

        let n = self.voices.len();
        let mut params = vec![VoiceParams::zeroed(); n.max(1)];
        let mut states = vec![VoiceState::zeroed(); n.max(1)];
        let mut env_stages: Vec<EnvStageGpu> = Vec::new();
        let mut voice_chans = vec![0u32; n.max(1)];

        // The states readback holds the GPU state at the end of the
        // *previous* block, which is exactly where this block must resume.
        // The CPU mirror (`v.state`) is one block older, so uploading it
        // would replay the previous block's audio (every other block
        // repeats).
        //
        // The read-back states are stored in the *previous* upload's voice
        // order; `retain` may have removed ended voices since then, so
        // index-aligned lookup would apply the wrong state to a surviving
        // voice (wrong int_time -> instant "ended" -> lost notes). Map by
        // voice id instead.
        let resumed: std::collections::HashMap<u32, VoiceState> = self
            .last_states
            .as_ref()
            .map(|st| {
                let count = st.len() / VoiceState::SIZE;
                let mut by_id: std::collections::HashMap<u32, VoiceState> =
                    std::collections::HashMap::with_capacity(count);
                for (i, v) in self.prev_voice_ids.iter().enumerate() {
                    if i < count {
                        let off = i * VoiceState::SIZE;
                        let s: &VoiceState = bytemuck::from_bytes(&st[off..off + VoiceState::SIZE]);
                        by_id.insert(*v, *s);
                    }
                }
                by_id
            })
            .unwrap_or_default();

        self.prev_voice_ids = self.voices.iter().map(|v| v.id).collect();

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
            // Inline the env stage copy (avoids one small Vec allocation per
            // voice; dense MIDI creates tens of thousands of voices per
            // block).
            for s in &v.env_stages {
                env_stages.push(EnvStageGpu {
                    kind: s.kind,
                    target_val: s.target,
                    duration: s.duration,
                });
            }
            params[i] = v.gpu_params(
                sample_offset,
                sample_offset_r,
                env_base,
                base,
                self.config.interpolation,
            );
            // A voice killed since the last upload (ended=1 set on the CPU
            // side) must stay ended: the resumed state from the previous
            // block predates the kill and would otherwise resurrect it.
            states[i] = if v.state.ended != 0 {
                v.state
            } else {
                resumed.get(&v.id).copied().unwrap_or(v.state)
            };
            voice_chans[i] = v.channel as u32
                | ((if v.released || v.release_at != u64::MAX {
                    1u32
                } else {
                    0u32
                }) << 7);
        }

        if self
            .params_buf
            .write(device, queue, 0, bytemuck::cast_slice(&params))?
        {
            self.render_bg_dirty = true;
        }
        if self
            .states_buf
            .write(device, queue, 0, bytemuck::cast_slice(&states))?
        {
            self.render_bg_dirty = true;
        }
        if self
            .env_buf
            .write(device, queue, 0, bytemuck::cast_slice(&env_stages))?
        {
            self.render_bg_dirty = true;
        }
        if self
            .voice_chans_buf
            .write(device, queue, 0, bytemuck::cast_slice(&voice_chans))?
        {
            self.render_bg_dirty = true;
            self.mix_bg_dirty = true;
        }

        self.active_voice_count = n as u32;
        // NOTE: no unconditional `render_bg_dirty` here. The four `write()`
        // calls above already flag dirty on actual buffer *growth*; flagging
        // unconditionally rebuilt both bind groups every block (pure CPU+GPU
        // waste - bind groups only depend on the buffers, not their
        // contents).
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
        let rate = self.config.sample_rate;

        // Resampling is the dominant CPU cost for large soundfonts; run it
        // in parallel (each sample is independent), then upload sequentially.
        // `resample_read` hits the pre-warmed cache when available, so the
        // render loop only pays for samples that were never used before.
        let resampled: Vec<(usize, Arc<[f32]>)> = needed
            .par_iter()
            .map(|&sample_id| {
                let data = sf.resample_read(sample_id, rate);
                (sample_id, data)
            })
            .collect();

        for (sample_id, data) in resampled {
            sf.cache_resampled(sample_id, rate, data.clone());
            let len = data.len() as u32;
            let offset = self.samples_next_offset;
            let grown = write_samples(
                &mut self.samples_chunks,
                device,
                queue,
                offset as u64 * 4,
                bytemuck::cast_slice(&data),
            )?;
            if grown {
                self.render_bg_dirty = true;
            }
            self.sample_offsets.insert(sample_id, (offset, len));
            self.samples_next_offset = offset + len;
        }
        Ok(())
    }
    fn update_mix_params(&mut self, base: u64) -> Result<(), SynthError> {
        let queue = &self.res.ctx.queue;
        let block = self.config.block_size as u32;
        let end = base + block as u64;
        let sr = self.config.sample_rate;

        // Take this block's deferred controller events; keep the rest for
        // the blocks that follow.
        let mut in_block: Vec<(u64, u8, u8, u8)> = Vec::new();
        let mut rest: Vec<(u64, u8, u8, u8)> = Vec::new();
        for ev in std::mem::take(&mut self.pending_mix_events) {
            if ev.0 < end {
                in_block.push(ev);
            } else {
                rest.push(ev);
            }
        }
        self.pending_mix_events = rest;
        in_block.sort_by_key(|e| e.0);

        // Frame-exact controller curve: the mix kernel replays this block's
        // events against the block-start lerp states, so the output does not
        // depend on the block size or on how many events a block contains.
        let events: Vec<MixEvent> = in_block
            .iter()
            .map(|e| MixEvent {
                frame: (e.0 - base) as u32,
                channel: e.1 as u32,
                cc: e.2 as u32,
                value: e.3 as f32 / 128.0,
            })
            .collect();

        // Per-channel block-start states, then advance the CPU-side lerp
        // state machines through this block (all events + the block end) so
        // the next block starts from the right values.
        let mut starts: Vec<MixStart> = Vec::with_capacity(MIX_CHANNELS);
        for ch_idx in 0..MIX_CHANNELS {
            let st = &mut self.channels[ch_idx];
            starts.push(MixStart {
                vol: st.volume.current,
                vol_step: st.volume.step,
                vol_end: st.volume.end,
                expr: st.expression.current,
                expr_step: st.expression.step,
                expr_end: st.expression.end,
                pan: st.pan.current,
                pan_step: st.pan.step,
                pan_end: st.pan.end,
                _pad: [0.0; 3],
            });
            for ev in in_block.iter().filter(|e| e.1 as usize == ch_idx) {
                let (s, cc, value) = (ev.0, ev.2, ev.3);
                match cc {
                    0x07 => {
                        st.volume.advance_to(s);
                        st.volume.set_end(value as f32 / 128.0, sr);
                    }
                    0x0B => {
                        st.expression.advance_to(s);
                        st.expression.set_end(value as f32 / 128.0, sr);
                    }
                    0x0A | 0x08 => {
                        st.pan.advance_to(s);
                        st.pan.set_end(value as f32 / 128.0, sr);
                    }
                    _ => {}
                }
            }
            st.volume.advance_to(end);
            st.expression.advance_to(end);
            st.pan.advance_to(end);
        }

        let device = &self.res.ctx.device;
        if self
            .mix_events_buf
            .write(device, queue, 0, bytemuck::cast_slice(&events))?
        {
            self.mix_bg_dirty = true;
        }
        let params = MixParams {
            voice_count: self.active_voice_count,
            block_size: block,
            channel_count: MIX_CHANNELS as u32,
            event_count: events.len() as u32,
            lerp_len: sr as f32 * 0.01,
            _pad: [0.0; 3],
            starts: starts
                .try_into()
                .map_err(|_| SynthError::Gpu("channel count mismatch".into()))?,
        };
        if std::env::var("LUMINO_VOICEDUMP").is_ok() && base > 415_000 && base < 420_000 {
            let s = &self.channels[0];
            eprintln!(
                "[mix] base={base} ch0 vol={:.4} expr={:.4} pan={:.4}",
                s.volume.current, s.expression.current, s.pan.current
            );
        }
        queue.write_buffer(&self.mix_params_buf, 0, bytemuck::cast_slice(&[params]));
        Ok(())
    }

    fn rebuild_bind_groups(&mut self) {
        let device = &self.res.ctx.device;
        let mut entries: Vec<wgpu::BindGroupEntry> = Vec::with_capacity(13);
        entries.push(wgpu::BindGroupEntry {
            binding: 0,
            resource: self.params_buf.buffer().as_entire_binding(),
        });
        for (i, chunk) in self.samples_chunks.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: SAMPLES_CHUNK_BINDING_BASE + i as u32,
                resource: chunk.buffer().as_entire_binding(),
            });
        }
        entries.extend([
            wgpu::BindGroupEntry {
                binding: crate::gpu::SINC_BINDING,
                resource: self.sinc_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: crate::gpu::ENV_BINDING,
                resource: self.env_buf.buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: crate::gpu::STATES_BINDING,
                resource: self.states_buf.buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: crate::gpu::VOICE_OUT_BINDING,
                resource: self.voice_out_buf.buffer().as_entire_binding(),
            },
        ]);
        self.render_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render bind group"),
            layout: &self.res.render_layout,
            entries: &entries,
        }));
        self.mix_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mix bind group"),
            layout: &self.res.mix_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.voice_out_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.out_storage_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.voice_chans_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.mix_events_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.mix_params_buf.as_entire_binding(),
                },
            ],
        }));
        self.render_bg_dirty = false;
        self.mix_bg_dirty = false;
    }

    fn rebuild_mix_bind_group(&mut self) {
        let device = &self.res.ctx.device;
        self.mix_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mix bind group"),
            layout: &self.res.mix_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.voice_out_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.out_storage_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.voice_chans_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.mix_events_buf.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.mix_params_buf.as_entire_binding(),
                },
            ],
        }));
        self.mix_bg_dirty = false;
    }

    #[allow(clippy::modulo_one)] // STATES_SYNC_EVERY is 1; the cadence is configurable
    fn dispatch(&mut self, _base: u64) -> Result<(), SynthError> {
        let voices = self.active_voice_count;
        let block = self.config.block_size as u32;

        // Physical ceiling: the voice output buffer cannot exceed the
        // device's maximum buffer size. Report a clear error instead of
        // crashing (the pool is huge - 1.5 GiB - so this only triggers for
        // genuinely pathological polyphony).
        if (voices as u64) * (block as u64) * 8 > MAX_VOICE_OUT_BYTES {
            return Err(SynthError::VoiceLimit(voices as usize));
        }

        // Grow the per-voice buffers if the active voice count exceeds the
        // current pool (dense MIDI may hold tens of thousands of voices).
        // Growing replaces the backing buffers, so bind groups are rebuilt
        // right below.
        if self.voice_out_buf.ensure(
            &self.res.ctx.device,
            &self.res.ctx.queue,
            (voices * block * 2 * 4) as u64,
        ) {
            self.render_bg_dirty = true;
            self.mix_bg_dirty = true;
        }
        if self.voice_chans_buf.ensure(
            &self.res.ctx.device,
            &self.res.ctx.queue,
            (voices * 4) as u64,
        ) {
            self.render_bg_dirty = true;
            self.mix_bg_dirty = true;
        }
        if self.render_bg_dirty {
            self.rebuild_bind_groups();
        }
        if self.mix_bg_dirty {
            self.rebuild_mix_bind_group();
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
            // Each voice is split across RENDER_SEGMENTS threads (gid.y);
            // the shader fast-forwards to its segment start, so the GPU
            // parallelism is voices x segments.
            pass.dispatch_workgroups(voices.div_ceil(128).max(1), crate::gpu::RENDER_SEGMENTS, 1);
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

        // Readbacks. The voice states are only copied back every
        // STATES_SYNC_EVERY blocks (see `states_sync_counter`); the output
        // must come back every block.
        let cur = self.out_readback_cur;
        encoder.copy_buffer_to_buffer(
            &self.out_storage_buf,
            0,
            &self.out_readback[cur],
            0,
            (self.config.block_size * 2 * 4) as u64,
        );
        let states_cur = self.states_readback_cur;
        if self.states_sync_counter == 0 {
            let states_bytes = (VoiceState::SIZE * self.voices.len()) as u64;
            let grew = self.states_readback[states_cur].ensure(device, queue, states_bytes);
            if grew {
                // The readback buffer was replaced; nothing else references
                // it (it is mapped below by value), so no rebind is needed.
            }
            encoder.copy_buffer_to_buffer(
                self.states_buf.buffer(),
                0,
                self.states_readback[states_cur].buffer(),
                0,
                states_bytes,
            );
        }

        let idx = queue.submit(Some(encoder.finish()));

        // Out pipelining: read back the data of the *previous* submission
        // (on the first block, of this one). The current submission may
        // still be in flight, so the GPU stays busy while the CPU prepares
        // the next block. The map callbacks fire inside the poll below.
        //
        // NOTE: the previous-block scheme would replay block 0's audio at
        // block 1 (the output would lag one block and the first block would
        // be doubled), so both output and voice states are read back from
        // the CURRENT submission. This makes the readbacks synchronous, but
        // correct; the submission index (`idx`) is waited on below.
        let read_cur = cur;
        self.prev_submission = Some(idx.clone());

        let out_slice = self.out_readback[read_cur].slice(..);
        let (otx, orx) = std::sync::mpsc::channel();
        out_slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = otx.send(r.is_ok());
        });
        // The states map reads the copy made for THIS block, so its data is
        // ready only after the current submission completes (`wait` below).
        let states_map = if self.states_sync_counter == 0 {
            let rb = self.states_readback[cur].buffer();
            let s = rb.slice(..);
            let (stx, srx) = std::sync::mpsc::channel();
            s.map_async(wgpu::MapMode::Read, move |r| {
                let _ = stx.send(r.is_ok());
            });
            Some((s, srx))
        } else {
            None
        };

        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(idx),
                // Bounded wait: a wedged GPU must surface as an error instead
                // of stalling the realtime render thread forever.
                timeout: Some(std::time::Duration::from_millis(100)),
            })
            .map_err(|e| SynthError::Gpu(format!("poll failed: {e:?}")))?;

        if orx
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap_or(false)
        {
            self.last_out = Some(out_slice.get_mapped_range().to_vec());
            self.out_readback[read_cur].unmap();
        } else {
            return Err(SynthError::Gpu("output readback map failed".into()));
        }
        if let Some((s, srx)) = states_map {
            if srx
                .recv_timeout(std::time::Duration::from_millis(100))
                .unwrap_or(false)
            {
                self.last_states = Some(s.get_mapped_range().to_vec());
                self.states_readback[cur].buffer().unmap();
            } else {
                return Err(SynthError::Gpu("states readback map failed".into()));
            }
        }

        self.out_readback_cur ^= 1;
        if self.states_sync_counter == 0 {
            self.states_readback_cur ^= 1;
        }
        // `STATES_SYNC_EVERY = 1` makes the counter always 0 (every block
        // maps its states); the modulo is intentional and kept for the
        // configurable cadence.
        self.states_sync_counter = if STATES_SYNC_EVERY > 1 {
            (self.states_sync_counter + 1) % STATES_SYNC_EVERY
        } else {
            0
        };
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
    ///
    /// Does *not* consume `last_states`: the next block's `upload_voices`
    /// resumes the GPU from these exact states (the readback lags one block
    /// by design, so the CPU mirror alone is one block stale and would
    /// replay the previous block). `upload_voices` clears it after use.
    ///
    /// Maps by voice id (`prev_voice_ids` records the upload order): the
    /// list may have shrunk since, so positional lookup would apply a
    /// stale state (and miss `ended`) on the wrong voice.
    fn sync_voice_states(&mut self) {
        let Some(states) = self.last_states.as_ref() else {
            return;
        };
        let count = states.len() / VoiceState::SIZE;
        if count == 0 {
            return;
        }
        let ids = std::mem::take(&mut self.prev_voice_ids);
        let mut pos_by_id: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::with_capacity(ids.len());
        for (i, id) in ids.into_iter().enumerate() {
            pos_by_id.insert(id, i);
        }
        for v in self.voices.iter_mut() {
            if let Some(&i) = pos_by_id.get(&v.id) {
                if i >= count {
                    continue;
                }
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

/// Writes resampled sample bytes across the fixed-size sample chunks,
/// splitting at chunk boundaries. Returns `true` when any chunk grew
/// (bind groups must be rebuilt).
///
/// A free function so the caller can borrow `sf` (soundfont) and
/// `samples_chunks` as disjoint fields of the engine.
fn write_samples(
    chunks: &mut [GrowableBuffer],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    byte_offset: u64,
    data: &[u8],
) -> Result<bool, SynthError> {
    let mut off = byte_offset;
    let mut remaining = data;
    let mut grown = false;
    while !remaining.is_empty() {
        let chunk = (off / SAMPLES_CHUNK_BYTES) as usize;
        let Some(buf) = chunks.get_mut(chunk) else {
            return Err(SynthError::Gpu(format!(
                "sample data exceeds the chunked samples buffer capacity \
                 ({} chunks of {} MiB)",
                SAMPLES_CHUNKS,
                SAMPLES_CHUNK_BYTES / (1024 * 1024)
            )));
        };
        let in_chunk = off % SAMPLES_CHUNK_BYTES;
        let take = ((SAMPLES_CHUNK_BYTES - in_chunk) as usize).min(remaining.len());
        grown |= buf.write(device, queue, in_chunk, &remaining[..take])?;
        off += take as u64;
        remaining = &remaining[take..];
    }
    Ok(grown)
}

/// A single-line `\r`-rewritten progress bar for offline rendering.
///
/// The bar shows the fraction of the render horizon that is complete. It is
/// a no-op when disabled (library callers), and rewrites one line on stderr
/// so long exports stay visibly alive without flooding the log.
struct ProgressBar {
    /// Total frame count the bar is measured against.
    total: u64,
    /// Progress bar width in characters.
    width: usize,
    /// Last reported percent, so the bar only repaints when it changes.
    last_pct: i32,
    /// Whether output is enabled at all.
    enabled: bool,
}

impl ProgressBar {
    fn new(total: u64, enabled: bool) -> Self {
        Self {
            total: total.max(1),
            width: 24,
            last_pct: -1,
            enabled,
        }
    }

    /// Advances the bar to `done` frames and repaints when the percent
    /// crossed a whole-number boundary.
    fn tick(&mut self, done: u64) {
        if !self.enabled {
            return;
        }
        let pct = ((done as f64 / self.total as f64) * 100.0) as i32;
        if pct <= self.last_pct {
            return;
        }
        self.last_pct = pct;
        self.paint(pct);
    }

    /// Ends the bar on its own line (100% or the last painted value).
    fn finish(&mut self) {
        if !self.enabled {
            return;
        }
        self.last_pct = 100;
        self.paint(100);
        eprintln!();
    }

    fn paint(&self, pct: i32) {
        let pct = pct.clamp(0, 100);
        let filled = (pct as usize * self.width) / 100;
        let bar: String = std::iter::repeat_n('=', filled)
            .chain(std::iter::repeat_n(' ', self.width - filled))
            .collect();
        eprint!("\r[render] [{}] {pct:3}%", bar);
    }
}
