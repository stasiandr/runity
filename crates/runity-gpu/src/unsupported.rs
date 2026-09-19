//! The same surface, on a platform with no Metal.
//!
//! Everything above this crate — `Engine::draw`, the renderer switch, the
//! differential tests — is written once and compiles everywhere. On Linux and
//! Windows the device simply never opens, so the only method here that does
//! anything is [`Gpu::new`], and it says why.

use crate::shader::GpuShader;
use crate::GpuError;
use runity_render::{Blend, Color, CullMode, DrawStats, Framebuffer, Mesh};
use std::ffi::c_void;

/// Where a frame is being drawn. Kept identical to the Metal backend's so the
/// callers do not have to be written twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Window,
    Offscreen { width: usize, height: usize },
}

/// A GPU that is never there.
pub struct Gpu {
    _private: (),
}

impl Gpu {
    /// Always [`GpuError::Unsupported`]: the backend is Metal.
    pub fn new() -> Result<Self, GpuError> {
        Err(GpuError::Unsupported)
    }

    pub fn device_name(&self) -> String {
        String::new()
    }

    pub fn attach(&mut self, _view: *mut c_void) -> Result<(), GpuError> {
        Err(GpuError::Unsupported)
    }

    pub fn drawable_size(&self) -> Option<(usize, usize)> {
        None
    }

    pub fn force_scale(&mut self, _scale: Option<f64>) {}

    pub fn paces_frames(&self) -> bool {
        false
    }

    pub fn set_vsync(&mut self, _enabled: bool) {}

    pub fn set_depth_readback(&mut self, _wanted: bool) {}

    pub fn depth_snapshot(&self) -> Option<&Framebuffer> {
        None
    }

    pub fn last_gpu_seconds(&self) -> f32 {
        0.0
    }

    pub fn last_download_seconds(&self) -> f32 {
        0.0
    }

    pub fn frame_stats(&self) -> DrawStats {
        DrawStats::default()
    }

    pub fn begin_frame(&mut self, _target: Target, _clear: Color) -> bool {
        false
    }

    pub fn end_frame(&mut self, _into: Option<&mut Framebuffer>) {}

    pub fn is_encoding(&self) -> bool {
        false
    }

    pub fn frame_size(&self) -> Option<(usize, usize)> {
        None
    }

    pub fn draw<S: GpuShader>(
        &mut self,
        _mesh: &Mesh,
        _shader: &S,
        _cull: CullMode,
        _blend: Blend,
        _depth_test: bool,
        _depth_write: bool,
    ) -> DrawStats {
        DrawStats::default()
    }

    pub fn draw_lines(&mut self, _segments: &[crate::lines::Segment], _width: f32) {}

    pub fn draw_fullscreen_image(&mut self, _image: &Framebuffer) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_never_opens_and_says_why() {
        assert_eq!(Gpu::new().err(), Some(GpuError::Unsupported));
        assert!(!crate::available());
    }
}
