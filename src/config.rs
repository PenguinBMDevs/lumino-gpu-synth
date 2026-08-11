//! Configuration types for the synthesizer.

use crate::synth::dsp::EnvelopeCurveConfig;

/// The sample interpolation algorithm used inside the GPU render kernels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpolationMode {
    /// Linear interpolation between adjacent samples.
    ///
    /// This matches the XSynth engine used to produce the reference audio
    /// and is therefore the default.
    #[default]
    Linear,
    /// High quality 64-point windowed sinc interpolation.
    ///
    /// This uses a precomputed 64-tap Blackman-Harris windowed sinc table and
    /// is the highest quality mode; it is slightly more expensive on the GPU.
    Point64Sinc,
}

/// Output channel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelMode {
    /// Stereo output (interleaved L/R samples).
    #[default]
    Stereo,
    /// Mono output (sum of both channels is down-mixed).
    Mono,
}

/// Configuration for a [`crate::GpuSynth`] instance.
///
/// # Example
///
/// ```
/// use lumino_gpu_synth::{InterpolationMode, SynthConfig};
///
/// let config = SynthConfig {
///     sample_rate: 64_000,
///     ..SynthConfig::default()
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SynthConfig {
    /// Output sample rate in Hz.
    ///
    /// Default: `64_000` (matches the reference render).
    pub sample_rate: u32,

    /// Maximum number of concurrently active voices.
    ///
    /// This is the size of the GPU voice pool (buffers and dispatch width),
    /// **not** a musical polyphony limit: voices are never killed because
    /// of it. It must be large enough to hold the peak number of voices the
    /// MIDI can produce; for pathological files with tens of thousands of
    /// simultaneous notes, raise it accordingly.
    ///
    /// Default: `16384`.
    pub max_voices: usize,

    /// Maximum number of simultaneous voices for the *same key* on the same
    /// channel (XSynth-style per-key polyphony limit).
    ///
    /// When a note-on would exceed this, the oldest voice of that key is
    /// replaced, so a repeated note always steals from its own key rather
    /// than from unrelated notes. `0` disables the limit entirely.
    ///
    /// Default: `4`.
    pub max_voices_per_key: usize,

    /// Number of audio frames rendered per GPU dispatch (per channel).
    ///
    /// Must be a power of two and at least 16. Smaller blocks keep the
    /// per-block voice population (and therefore upload/GPU cost) low for
    /// dense MIDI; larger blocks amortize dispatch overhead. Default: `1024`.
    pub block_size: usize,

    /// Sample interpolation mode used by the GPU render kernel.
    ///
    /// Default: [`InterpolationMode::Linear`].
    pub interpolation: InterpolationMode,

    /// Whether voice processing effects (the resonant low-pass filter) are
    /// enabled, mirroring XSynth's `use_effects` option.
    ///
    /// Default: `true`.
    pub use_effects: bool,

    /// Envelope curve selection for the volume envelope stages, mirroring
    /// XSynth's `EnvelopeOptions`. Set `decay_curve`/`release_curve` to
    /// `CurveKind::Exponential` to match OmniConverter's "LinearEnvelope"
    /// mode.
    ///
    /// Default: XSynth defaults (attack Exponential, decay/release Linear).
    pub envelope_curves: EnvelopeCurveConfig,

    /// Output channel layout. Default: [`ChannelMode::Stereo`].
    pub channels: ChannelMode,

    /// The absolute silence threshold (per sample) used by offline rendering
    /// to decide when the tail has decayed and rendering can stop. Mirrors
    /// XSynth's offline renderer (`0.0001`).
    ///
    /// Default: `0.0001`.
    pub render_silence_threshold: f32,

    /// Maximum number of seconds to keep rendering *after* the last MIDI
    /// event before aborting with [`crate::SynthError::RenderTimeout`].
    ///
    /// This is a safety valve against infinite offline renders caused by
    /// voices that can never finish (a held damper pedal, a missing note-off
    /// at the end of the file, a zero-duration envelope stage...). It does
    /// not limit legitimate files: a normal render ends as soon as the
    /// output goes silent, which is always well before this budget.
    ///
    /// Default: `120.0` seconds.
    pub max_tail_seconds: f32,
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            sample_rate: 64_000,
            max_voices: 16_384,
            max_voices_per_key: 32,
            block_size: 512,
            interpolation: InterpolationMode::Linear,
            use_effects: true,
            envelope_curves: EnvelopeCurveConfig::default(),
            channels: ChannelMode::Stereo,
            render_silence_threshold: 0.0001,
            max_tail_seconds: 120.0,
        }
    }
}

impl SynthConfig {
    /// Validates the configuration and returns a descriptive error if it is
    /// unusable.
    pub fn validate(&self) -> Result<(), crate::SynthError> {
        if self.sample_rate == 0 {
            return Err(crate::SynthError::Config(
                "sample_rate must be non-zero".into(),
            ));
        }
        if self.max_voices == 0 || self.max_voices > 65536 {
            return Err(crate::SynthError::Config(format!(
                "max_voices must be within 1..=65536, got {}",
                self.max_voices
            )));
        }
        if !self.block_size.is_power_of_two() || self.block_size < 16 {
            return Err(crate::SynthError::Config(format!(
                "block_size must be a power of two >= 16, got {}",
                self.block_size
            )));
        }
        if !self.render_silence_threshold.is_finite() || self.render_silence_threshold <= 0.0 {
            return Err(crate::SynthError::Config(
                "render_silence_threshold must be positive".into(),
            ));
        }
        if !self.max_tail_seconds.is_finite() || self.max_tail_seconds <= 0.0 {
            return Err(crate::SynthError::Config(
                "max_tail_seconds must be positive".into(),
            ));
        }
        Ok(())
    }
}
