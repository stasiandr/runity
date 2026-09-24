//! The GPU (DNA, "Видеокарта — модуль `gpu`"): the device and its queue,
//! the surface a host hands the engine, and offscreen targets. The render
//! module draws the 3D scene on it and the UI module draws the interface;
//! the editor always has it, a headless server never does.

pub mod gpu;
pub mod surface;

pub use gpu::{Gpu, GpuError, OffscreenTarget};
pub use surface::{AcquiredFrame, SurfaceError};
