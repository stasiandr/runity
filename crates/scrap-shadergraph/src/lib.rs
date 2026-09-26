//! Content graphs of numbers: a material's shader graph
//! (`shaders/<name>.graph.ron`) and, on the same nodes, a particle
//! effect's (`shaders/<name>.vfx.ron`). The text is the graph — nodes by name, each with its inputs by
//! name, no positions (DNA, postulate 5) — and it compiles to WGSL here,
//! without a GPU, so an agent or `scrap check` can say what is wrong with
//! it before anything draws.

pub mod effect;
pub mod expr;
pub mod fullscreen;
pub mod subgraph;
pub mod surface;
pub mod text;

pub use effect::{effect_name, EffectGraph};
pub use fullscreen::{fullscreen_name, FullscreenGraph};
pub use expr::{Input, Node, Ty};
pub use surface::{shader_name, Kind, Param, ShaderGraph, SurfaceOut, VertexOut};
