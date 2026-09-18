//! The runity software renderer: a full triangle pipeline on the CPU.
//!
//! ```text
//! Mesh -> Shader::vertex -> near-plane clip -> perspective divide ->
//! viewport map -> backface cull -> scan conversion (top-left fill rule) ->
//! perspective-correct interpolation -> depth test -> Shader::fragment -> blend
//! ```
//!
//! Everything here is plain safe Rust with no dependencies, and it never talks
//! to the OS: [`Framebuffer`] is just memory. Getting that memory onto a screen
//! is `runity-platform`'s job.

#![forbid(unsafe_code)]

pub mod bloom;
pub mod color;
pub mod debug;
pub mod font;
pub mod framebuffer;
pub mod gbuffer;
pub mod geometry;
pub mod golden;
pub mod inflate;
pub mod mesh;
pub mod pbr;
pub mod pipeline;
pub mod png;
pub mod post;
pub mod raster;
pub mod shader;
pub mod shadow;
pub mod sky;
pub mod ssao;
pub mod ssr;
pub mod texture;
pub mod tonemap;
pub mod view;

pub use bloom::{Bloom, BloomSettings};
pub use color::Color;
pub use framebuffer::Framebuffer;
pub use gbuffer::{GBuffer, Surface};
pub use geometry::{GeometryShader, GeometryVarying};
pub use mesh::{Mesh, ObjError};
pub use pbr::{Light, LightKind, Material};
pub use pipeline::{RenderSettings, Renderer};
pub use png::{decode_png, encode_png, load_png, save_png, save_ppm, Image, PngError};
pub use post::PostSettings;
pub use raster::{Blend, CullMode, DrawStats, PolygonMode, Rasterizer};
pub use shader::{Shader, UnlitShader, Varying, Vertex, VertexOutput};
pub use shadow::{ShadowMap, ShadowSettings};
pub use sky::{Sky, SkyParams};
pub use ssao::{OcclusionBuffer, SsaoSettings};
pub use ssr::SsrSettings;
pub use texture::{Filter, Texture, Wrap};
pub use tonemap::ToneMap;
pub use view::CameraView;
