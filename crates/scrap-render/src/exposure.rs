//! Auto exposure: the eye getting used to the light. HDRP's Exposure,
//! Automatic Histogram.
//!
//! Out of a dark passage into the desert sun, the picture is white for a
//! moment and settles; back into the shade, it is black and slowly comes
//! up. Each frame a histogram of the picture's brightness is taken on the
//! GPU (in stops, a sample every fourth pixel each way), the darkest part
//! and the brightest few are left out — a black doorway or the sun's disc
//! should not decide it — and the rest's average is what the exposure
//! aims to show as middle grey. It moves toward that over time, faster
//! into the light than out of it, as eyes do; a picture with nothing
//! before it (the first frame, a still shot) starts where it is aimed.
//!
//! The exposure lives on the GPU and the post shader reads it there: no
//! waiting on a read back, no frame behind. It is applied before bloom, so
//! what glows is what is bright for the eye now, not what was bright in
//! the file.

use serde::{Deserialize, Serialize};

use crate::gpu::Gpu;

/// How the exposure follows the light: `post: (auto_exposure: (...))`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoExposure {
    pub enabled: bool,
    /// Stops added to what the meter says: + brighter, − darker.
    pub compensation: f32,
    /// How much of the way to middle grey it goes, 0 to 1: at 1 every
    /// place ends up as bright as every other, the noon desert as the
    /// overcast day; less keeps a bright place bright and a dim one dim,
    /// while a dark passage still blinds on the way out.
    pub adaptation: f32,
    /// The least and most it will do, in stops: a night stays dark, the
    /// sun stays bright.
    pub min_ev: f32,
    pub max_ev: f32,
    /// How fast it settles after the light rises (out into the sun) and
    /// after it falls (into the shade), per second: the eye takes the first
    /// in a moment and the second slowly. HDRP's Speed Dark to Light and
    /// Speed Light to Dark.
    pub speed_dark_to_light: f32,
    pub speed_light_to_dark: f32,
    /// The share of the picture, darkest first, left out; and where the
    /// brightest left out begin. HDRP's histogram percentages.
    pub low_percent: f32,
    pub high_percent: f32,
}

impl Default for AutoExposure {
    fn default() -> Self {
        Self {
            enabled: true,
            compensation: 0.0,
            adaptation: 0.8,
            min_ev: -3.0,
            max_ev: 4.0,
            speed_dark_to_light: 0.4,
            speed_light_to_dark: 1.0,
            low_percent: 10.0,
            high_percent: 95.0,
        }
    }
}

impl AutoExposure {
    pub const OFF: AutoExposure = AutoExposure {
        enabled: false,
        compensation: 0.0,
        adaptation: 0.8,
        min_ev: -3.0,
        max_ev: 4.0,
        speed_dark_to_light: 0.4,
        speed_light_to_dark: 1.0,
        low_percent: 10.0,
        high_percent: 95.0,
    };
}

/// The brightness a picture is aimed to show as the middle of its range:
/// what the engine's scenes were graded at by hand, so a daylit valley
/// looks as it did before the exposure moved by itself.
pub const KEY: f32 = 0.4;
/// The histogram: this many bins over these stops of luminance.
const BINS: u32 = 128;
const LOW_STOP: f32 = -12.0;
const HIGH_STOP: f32 = 12.0;

pub const SHADER: &str = r#"
struct Metering {
    // width, height, the dt of this frame, 1 to start where it is aimed
    size: vec4<f32>,
    // compensation, min ev, max ev, key
    aim: vec4<f32>,
    // speed dark to light, light to dark, low share, high share
    speed: vec4<f32>,
    // low stop, stops per bin, bins, adaptation
    bins: vec4<f32>,
};

@group(0) @binding(0) var<uniform> metering: Metering;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>, 128>;
// the exposure now, in stops; 1 once there is one
@group(0) @binding(3) var<storage, read_write> state: array<f32, 4>;

var<workgroup> local: array<atomic<u32>, 128>;

@compute @workgroup_size(16, 16, 1)
fn cs_histogram(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    if li < 128u {
        atomicStore(&local[li], 0u);
    }
    workgroupBarrier();
    let at = id.xy * 4u + 2u;
    if f32(at.x) < metering.size.x && f32(at.y) < metering.size.y {
        let c = textureLoad(picture, vec2<i32>(at), 0).rgb;
        let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        if l > 1e-6 {
            let stop = log2(l);
            let bin = u32(clamp((stop - metering.bins.x) / metering.bins.y, 0.0, metering.bins.z - 1.0));
            atomicAdd(&local[bin], 1u);
        }
    }
    workgroupBarrier();
    if li < 128u {
        let n = atomicLoad(&local[li]);
        if n > 0u {
            atomicAdd(&histogram[li], n);
        }
    }
}

var<workgroup> counts: array<u32, 128>;

@compute @workgroup_size(128, 1, 1)
fn cs_adapt(@builtin(local_invocation_index) li: u32) {
    counts[li] = atomicLoad(&histogram[li]);
    atomicStore(&histogram[li], 0u);
    workgroupBarrier();
    if li != 0u {
        return;
    }
    var total = 0.0;
    for (var i = 0u; i < 128u; i = i + 1u) {
        total += f32(counts[i]);
    }
    if total < 1.0 {
        return;
    }
    // The middle of the picture by brightness: what is between the low and
    // the high share, each bin counted for as much of it as falls there.
    let low = total * metering.speed.z;
    let high = total * metering.speed.w;
    var seen = 0.0;
    var sum = 0.0;
    var weight = 0.0;
    for (var i = 0u; i < 128u; i = i + 1u) {
        let n = f32(counts[i]);
        let from_ = max(seen, low);
        let to = min(seen + n, high);
        let inside = max(to - from_, 0.0);
        let stop = metering.bins.x + (f32(i) + 0.5) * metering.bins.y;
        sum += stop * inside;
        weight += inside;
        seen += n;
    }
    let average = sum / max(weight, 1.0);
    let aimed = clamp((log2(metering.aim.w) - average) * metering.bins.w + metering.aim.x, metering.aim.y, metering.aim.z);
    var ev = aimed;
    if state[1] > 0.5 && metering.size.w < 0.5 {
        let was = state[0];
        // Brighter light to take in: the exposure falls, quickly.
        let speed = select(metering.speed.y, metering.speed.x, aimed < was);
        ev = was + (aimed - was) * (1.0 - exp(-metering.size.z * speed));
    }
    state[0] = ev;
    state[1] = 1.0;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MeteringUniform {
    size: [f32; 4],
    aim: [f32; 4],
    speed: [f32; 4],
    bins: [f32; 4],
}

/// The meter and the exposure it keeps.
pub(crate) struct Metering {
    histogram: wgpu::Buffer,
    /// The exposure now, in stops: what the post shader reads.
    pub(crate) state: wgpu::Buffer,
    uniform: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    histogram_pass: wgpu::ComputePipeline,
    adapt_pass: wgpu::ComputePipeline,
    /// Frames metered since it last started over.
    frames: u32,
}

impl Metering {
    pub(crate) fn new(gpu: &Gpu) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scrap::exposure"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("exposure"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    storage(2),
                    storage(3),
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scrap::exposure"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pass = |entry: &str| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(entry),
                    // Both passes clear what they share before reading it:
                    // no zeroing of their own asked for, which on a device
                    // without it natively is a loop over the group's size
                    // that some translations (the Android emulator's
                    // MoltenVK) cannot compile.
                    compilation_options: wgpu::PipelineCompilationOptions {
                        zero_initialize_workgroup_memory: false,
                        ..Default::default()
                    },
                    cache: None,
                })
        };
        let buffer = |label, size| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let histogram = buffer("exposure histogram", BINS as u64 * 4);
        let state = buffer("exposure", 16);
        gpu.queue
            .write_buffer(&histogram, 0, &vec![0u8; BINS as usize * 4]);
        gpu.queue
            .write_buffer(&state, 0, bytemuck::cast_slice(&[0.0f32; 4]));
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure"),
            size: std::mem::size_of::<MeteringUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            histogram,
            state,
            uniform,
            layout,
            histogram_pass: pass("cs_histogram"),
            adapt_pass: pass("cs_adapt"),
            frames: 0,
        }
    }

    /// No exposure of its own: the picture as graded.
    pub(crate) fn hold(&mut self, gpu: &Gpu) {
        gpu.queue
            .write_buffer(&self.state, 0, bytemuck::cast_slice(&[0.0f32; 4]));
        self.frames = 0;
    }

    /// Meter `picture` and move the exposure on by `dt` seconds.
    pub(crate) fn run(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        picture: &wgpu::TextureView,
        size: (u32, u32),
        settings: &AutoExposure,
        dt: f32,
    ) {
        let uniform = MeteringUniform {
            size: [
                size.0 as f32,
                size.1 as f32,
                dt.clamp(0.0, 0.5),
                if self.frames == 0 { 1.0 } else { 0.0 },
            ],
            aim: [
                settings.compensation,
                settings.min_ev.min(settings.max_ev),
                settings.max_ev.max(settings.min_ev),
                KEY,
            ],
            speed: [
                settings.speed_dark_to_light.max(0.0),
                settings.speed_light_to_dark.max(0.0),
                (settings.low_percent / 100.0).clamp(0.0, 0.99),
                (settings.high_percent / 100.0).clamp(0.01, 1.0),
            ],
            bins: [
                LOW_STOP,
                (HIGH_STOP - LOW_STOP) / BINS as f32,
                BINS as f32,
                settings.adaptation.clamp(0.0, 1.0),
            ],
        };
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("exposure"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(picture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.histogram.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.state.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("scrap::exposure"),
            timestamp_writes: crate::gpu_timer::compute("exposure"),
        });
        pass.set_bind_group(0, &group, &[]);
        pass.set_pipeline(&self.histogram_pass);
        pass.dispatch_workgroups(size.0.div_ceil(64).max(1), size.1.div_ceil(64).max(1), 1);
        pass.set_pipeline(&self.adapt_pass);
        pass.dispatch_workgroups(1, 1, 1);
        self.frames = self.frames.saturating_add(1);
    }
}
