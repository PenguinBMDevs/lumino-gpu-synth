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
    /// Each MIDI note can spawn one or more voices (one per matching SF2
    /// region). Voices beyond this limit are killed immediately.
    ///
    /// Default: `128`.
    pub max_voices: usize,

    /// Number of audio frames rendered per GPU dispatch (per channel).
    ///
    /// Must be a power of two and at least 16. Default: `512`.
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
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            sample_rate: 64_000,
            max_voices: 128,
            block_size: 512,
            interpolation: InterpolationMode::Linear,
            use_effects: true,
            envelope_curves: EnvelopeCurveConfig::default(),
            channels: ChannelMode::Stereo,
            render_silence_threshold: 0.0001,
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
        if self.max_voices == 0 || self.max_voices > 4096 {
            return Err(crate::SynthError::Config(format!(
                "max_voices must be within 1..=4096, got {}",
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
        Ok(())
    }
}
