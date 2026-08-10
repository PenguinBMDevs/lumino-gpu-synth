//! wgpu device management and the two compute pipelines (render + mix).

mod layout;

pub use layout::*;

use std::sync::Arc;

use crate::SynthError;

/// A ready-to-use GPU device/queue pair.
#[derive(Debug)]
pub struct GpuContext {
    /// The wgpu device.
    pub device: wgpu::Device,
    /// The wgpu queue.
    pub queue: wgpu::Queue,
    /// The adapter used (exposed for diagnostics).
    pub adapter_info: wgpu::AdapterInfo,
}

/// Creates a [`GpuContext`] using the default high-performance adapter.
///
/// Falls back from any backend failure to the fallback adapter, then errors.
///
/// # Errors
///
/// Returns [`SynthError::GpuInit`] when no usable adapter/device exists.
pub fn create_gpu_context() -> Result<GpuContext, SynthError> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .map_err(|e| SynthError::GpuInit(format!("request_adapter failed: {e:?}")))?;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("lumino-gpu-synth"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        experimental_features: Default::default(),
        trace: wgpu::Trace::Off,
    }))
    .map_err(|e| SynthError::GpuInit(format!("request_device failed: {e:?}")))?;

    let adapter_info = adapter.get_info();
    Ok(GpuContext {
        device,
        queue,
        adapter_info,
    })
}

/// A storage buffer with dynamic (grow-on-demand) capacity.
#[derive(Debug)]
pub struct GrowableBuffer {
    buffer: wgpu::Buffer,
    size: u64,
    usage: wgpu::BufferUsages,
    label: String,
}

impl GrowableBuffer {
    /// Creates a growable storage buffer with an initial `capacity` bytes.
    pub fn new(
        device: &wgpu::Device,
        label: &str,
        capacity: u64,
        usage: wgpu::BufferUsages,
    ) -> Self {
        let size = capacity.max(16);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            size,
            usage,
            label: label.to_string(),
        }
    }

    /// Returns the current backing buffer.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Returns the allocated size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Writes `data` at `offset`, growing the buffer first if needed.
    ///
    /// Growing creates a new buffer, copies the old contents into it, and
    /// returns `true` so callers can rebuild bind groups that reference it.
    pub fn write(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        offset: u64,
        data: &[u8],
    ) -> bool {
        let end = offset + data.len() as u64;
        if end > self.size {
            let new_size = (self.size * 2).max(end.max(1024));
            let new_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&self.label),
                size: new_size,
                usage: self.usage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            // Copy old contents (if any) into the new buffer.
            if self.size > 0 {
                let mut encoder =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                encoder.copy_buffer_to_buffer(&self.buffer, 0, &new_buf, 0, self.size);
                queue.submit(Some(encoder.finish()));
            }
            self.buffer = new_buf;
            self.size = new_size;
            queue.write_buffer(&self.buffer, offset, data);
            return true;
        }
        queue.write_buffer(&self.buffer, offset, data);
        false
    }

    /// Clears the buffer contents to zero.
    pub fn clear(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Zero via write of a small zero chunk (buffers are re-uploaded every
        // block anyway for the fixed-size ones).
        let zeros = vec![0u8; self.size.min(4096) as usize];
        let mut off = 0u64;
        while off < self.size {
            let n = (self.size - off).min(4096);
            queue.write_buffer(&self.buffer, off, &zeros[..n as usize]);
            off += n;
        }
        let _ = device; // device unused; kept for signature symmetry
    }
}

/// Reference-counted GPU resources shared by the engine.
#[derive(Debug)]
pub struct GpuResources {
    /// Device/queue.
    pub ctx: Arc<GpuContext>,
    /// Render pipeline (pass 1).
    pub render_pipeline: wgpu::ComputePipeline,
    /// Mix pipeline (pass 2).
    pub mix_pipeline: wgpu::ComputePipeline,
    /// Render bind group layout.
    pub render_layout: wgpu::BindGroupLayout,
    /// Mix bind group layout.
    pub mix_layout: wgpu::BindGroupLayout,
    /// Block size compiled into the shaders.
    pub block_size: usize,
    /// Max voices compiled into the shaders.
    pub max_voices: usize,
}

impl GpuResources {
    /// Creates the pipelines for a given configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SynthError::Gpu`] when shader compilation fails.
    pub fn new(
        ctx: Arc<GpuContext>,
        block_size: usize,
        max_voices: usize,
    ) -> Result<Self, SynthError> {
        let device = &ctx.device;

        // --- render bind group layout ---
        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render bind group layout"),
            entries: &[
                bind_entry(
                    0,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                bind_entry(
                    1,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                bind_entry(
                    2,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                bind_entry(
                    3,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                bind_entry(
                    4,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ),
                bind_entry(
                    5,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ),
            ],
        });

        // --- mix bind group layout ---
        let mix_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mix bind group layout"),
            entries: &[
                bind_entry(
                    0,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                bind_entry(
                    1,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ),
                bind_entry(
                    2,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                bind_entry(
                    3,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                bind_entry(
                    4,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Uniform,
                ),
            ],
        });

        let render_pipeline = create_render_pipeline(device, &render_layout, block_size)?;
        let mix_pipeline = create_mix_pipeline(device, &mix_layout)?;

        Ok(Self {
            ctx,
            render_pipeline,
            mix_pipeline,
            render_layout,
            mix_layout,
            block_size,
            max_voices,
        })
    }
}

fn bind_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn create_render_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    block_size: usize,
) -> Result<wgpu::ComputePipeline, SynthError> {
    let source = include_str!("shaders/render.wgsl").replace(
        "const BLOCK: u32 = 512u;",
        &format!("const BLOCK: u32 = {block_size}u;"),
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("render.wgsl"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("lumino render"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("render layout"),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            }),
        ),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    // Shader compilation errors surface on the device; validate eagerly by
    // checking the shader module info.
    Ok(pipeline)
}

fn create_mix_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> Result<wgpu::ComputePipeline, SynthError> {
    let source = include_str!("shaders/mix.wgsl");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mix.wgsl"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("lumino mix"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mix layout"),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            }),
        ),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    Ok(pipeline)
}
