//! Bindless maps: every texture the renderer has in one array the shader
//! indexes, so a draw is not split from the next by its textures.
//!
//! Without it a material's four maps — base, normal, mask, emission — are
//! a bind group of their own, and the frame's draws are batched by mesh,
//! look *and* maps: forty crates of forty pictures are forty draws. With
//! it, where the device has texture binding arrays indexed per fragment
//! (Metal, Vulkan and DX12 on current hardware; `Gpu::bindless`), group 1
//! is one array of every uploaded texture and the sampler, each instance
//! carries its four handles (an eighteenth vertex attribute), and the
//! shader reads `textures[handle]`: forty crates of one mesh are one draw.
//!
//! The shader is one: `render.wgsl` reads its maps through `surface_at`,
//! `normal_at`, `mask_at` and `emission_at`, which read the bound maps;
//! [`prepared`] puts the array and the indexed versions in their place.
//! The array is made again only when a texture is added or replaced.
//! Not with the terrain drawn by mesh shaders, whose stage carries no
//! handles.

use crate::gpu::Gpu;
use crate::render::TextureHandle;

/// The maps every batch is keyed by, bindless: one set for all.
pub(crate) const KEY: [TextureHandle; 4] = [
    TextureHandle::WHITE,
    TextureHandle::FLAT_NORMAL,
    TextureHandle::WHITE,
    TextureHandle::WHITE,
];

/// Textures the array holds at most.
pub const MOST: u32 = 4096;

const STUB_BEGIN: &str = "// maps: begin";
const STUB_END: &str = "// maps: end";

const SHADER: &str = r#"// maps: bindless (bindless.rs)
// Every texture, one array; an instance names its four by their handles.
@group(1) @binding(0) var map_textures: binding_array<texture_2d<f32>>;
@group(1) @binding(1) var surface_sampler: sampler;

fn surface_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(map_textures[maps.x], surface_sampler, uv);
}
fn normal_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(map_textures[maps.y], surface_sampler, uv);
}
fn mask_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(map_textures[maps.z], surface_sampler, uv);
}
fn emission_at(maps: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    return textureSample(map_textures[maps.w], surface_sampler, uv);
}
"#;

/// The renderer's shader, bindless where `on`: the bound maps' stub
/// replaced by the array's.
pub(crate) fn prepared(source: &str, on: bool) -> String {
    let (Some(begin), Some(end), true) = (source.find(STUB_BEGIN), source.find(STUB_END), on) else {
        return source.to_string();
    };
    // The extension's directive before everything else.
    format!(
        "enable wgpu_binding_array;\n{}{}{}",
        &source[..begin],
        SHADER,
        &source[end + STUB_END.len()..]
    )
}

/// Group 1, bindless: the array, partly bound, and the sampler.
pub(crate) fn layout(gpu: &Gpu) -> wgpu::BindGroupLayout {
    gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("surface maps (bindless)"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: std::num::NonZeroU32::new(MOST),
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// The one group of every texture, made again when they change.
pub(crate) struct Bindless {
    /// Textures the group was made with; `None` to make it again.
    made_with: Option<usize>,
}

impl Bindless {
    pub(crate) fn new() -> Self {
        Self { made_with: None }
    }

    /// A texture was replaced: the group points at the old one.
    pub(crate) fn invalidate(&mut self) {
        self.made_with = None;
    }

    /// The group again, when the textures have changed since it was made.
    pub(crate) fn group(
        &mut self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        views: &[&wgpu::TextureView],
        sampler: &wgpu::Sampler,
    ) -> Option<wgpu::BindGroup> {
        if self.made_with == Some(views.len()) {
            return None;
        }
        self.made_with = Some(views.len());
        let views = &views[..views.len().min(MOST as usize)];
        Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface maps (bindless)"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(views),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_bound_maps_give_way_to_the_array_only_when_asked() {
        let source = "a\n// maps: begin\nold\n// maps: end\nb";
        assert_eq!(super::prepared(source, false), source);
        let bindless = super::prepared(source, true);
        assert!(bindless.contains("binding_array") && !bindless.contains("old"));
        assert!(bindless.starts_with("enable wgpu_binding_array;\na\n") && bindless.ends_with("\nb"));
    }
}
