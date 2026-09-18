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

pub mod color;
pub mod debug;
pub mod font;
pub mod framebuffer;
pub mod golden;
pub mod inflate;
pub mod mesh;
pub mod png;
pub mod raster;
pub mod shader;
pub mod texture;

pub use color::Color;
pub use font::{Align, Font, FontError, GlyphBitmap, GlyphMetrics, TextSize, TextStyle};
pub use framebuffer::Framebuffer;
pub use mesh::{Mesh, ObjError};
pub use png::{decode_png, encode_png, load_png, save_png, save_ppm, Image, PngError};
pub use raster::{Blend, CullMode, DrawStats, PolygonMode, Rasterizer};
pub use shader::{
    BasicShader, BasicVarying, DirectionalLight, Shader, UnlitShader, Varying, Vertex, VertexOutput,
};
pub use texture::{Filter, Texture, Wrap};
