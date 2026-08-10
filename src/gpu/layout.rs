//! GPU buffer layouts (bytemuck POD structs matching the WGSL kernels).

use bytemuck::{Pod, Zeroable};

/// Mirror of `VoiceParams` in `render.wgsl`.
///
/// All fields are 4-byte scalars so the CPU layout matches WGSL's default
/// storage-buffer layout without padding.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct VoiceParams {
    pub is_active: u32,
    pub sample_offset: u32,
    pub sample_offset_r: u32,
    pub sample_len: u32,
    pub offset: u32,
    pub sample_end: u32,
    pub loop_mode: u32,
    pub loop_start: u32,
    pub loop_end: u32,
    pub speed: f32,
    pub amp: f32,
    pub pan_l: f32,
    pub pan_r: f32,
    pub filter_on: u32,
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
    pub env_base: u32,
    pub env_count: u32,
    pub release_idx: u32,
    pub finished_idx: u32,
    pub release_at: u32,
    pub base_frame: u32,
    pub interp: u32,
    pub channels: u32,
    pub start_at: u32,
    pub channel: u32,
}

impl VoiceParams {
    pub const SIZE: usize = std::mem::size_of::<Self>();
    pub const RELEASE_AT_NONE: u32 = u32::MAX;
}

/// Mirror of `VoiceState` in `render.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct VoiceState {
    pub int_time: u32,
    pub frac: f32,
    pub env_stage: u32,
    pub env_t: u32,
    pub env_from: f32,
    pub lx1: f32,
    pub lx2: f32,
    pub ly1: f32,
    pub ly2: f32,
    pub rx1: f32,
    pub rx2: f32,
    pub ry1: f32,
    pub ry2: f32,
    pub last_loop_pos: u32,
    pub is_released: u32,
    pub ended: u32,
}

impl VoiceState {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

impl Default for VoiceState {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Mirror of `EnvStageGpu` in `render.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EnvStageGpu {
    pub kind: u32,
    pub target_val: f32,
    pub duration: u32,
}

impl EnvStageGpu {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// Mirror of `MixParams` in `mix.wgsl` (uniform buffer, 16-byte padded).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MixParams {
    pub voice_count: u32,
    pub block_size: u32,
    pub channel_count: u32,
    pub reserved: u32,
}

impl MixParams {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// Mirror of `ChannelMix` in `mix.wgsl` (per-channel controller curves).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ChannelMix {
    pub vol_start: f32,
    pub vol_delta: f32,
    pub vol_end: f32,
    pub expr_start: f32,
    pub expr_delta: f32,
    pub expr_end: f32,
    pub pan_start: f32,
    pub pan_delta: f32,
    pub pan_end: f32,
}

impl ChannelMix {
    pub const SIZE: usize = std::mem::size_of::<Self>();
    pub const CHANNELS: usize = 16;
}

impl Default for MixParams {
    fn default() -> Self {
        Self::zeroed()
    }
}
