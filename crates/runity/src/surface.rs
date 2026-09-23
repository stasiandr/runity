//! Drawing into a window someone else owns.
//!
//! The engine never creates a window (see the crate docs). What it accepts is
//! a surface: from winit on a desktop, from a `UIViewController` or an
//! `Activity` on a phone. All this module does is hold the swapchain and
//! keep its size honest.

use crate::gpu::Gpu;

/// A window's drawable surface, configured and ready.
pub struct Surface {
    inner: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

/// Why a frame could not be acquired.
#[derive(Debug)]
pub enum SurfaceError {
    /// No frame this time: the window is being dragged, is minimised, or
    /// moved between monitors. Reconfigure and try again next frame; it is
    /// routine, not a failure.
    Outdated,
    /// The GPU is gone: a driver reset, or the device was removed.
    Lost,
    /// Out of memory, or something the driver would not explain.
    Other(String),
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurfaceError::Outdated => write!(f, "surface out of date; reconfigure"),
            SurfaceError::Lost => write!(f, "surface lost"),
            SurfaceError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SurfaceError {}

impl Surface {
    // Used by every constructor; on a build with no shell feature there is
    // not yet one, and the native entry points land here too.
    #[allow(dead_code)]
    pub(crate) fn configure(
        gpu: &Gpu,
        inner: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Self {
        let capabilities = inner.get_capabilities(&gpu.adapter);
        // An sRGB format, so the GPU encodes on write and shading stays
        // linear all the way to the end. Picking the first available format
        // instead is how a whole renderer ends up looking washed out.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(capabilities.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: capabilities.alpha_modes[0],
            // The same bytes seen as plain RGBA, for a UI that blends as a
            // browser does (`AcquiredFrame::ui_view`).
            view_formats: vec![format.remove_srgb_suffix()],
            // The default: the format already says sRGB, and naming a wider
            // space here would change what the encode step means.
            color_space: Default::default(),
            // One frame in flight past the one being shown. More adds
            // latency, which on a first-person camera is felt immediately.
            desired_maximum_frame_latency: 2,
        };
        inner.configure(&gpu.device, &config);
        Self { inner, config }
    }

    pub fn width(&self) -> u32 {
        self.config.width
    }

    pub fn height(&self) -> u32 {
        self.config.height
    }

    /// Rebuild the swapchain at a new size.
    ///
    /// A zero in either axis means the window was minimised. Configuring a
    /// zero-sized surface is an error on every backend, so the old size is
    /// kept and drawing simply carries on into a surface nobody can see.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (self.config.width, self.config.height) == (width, height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.inner.configure(&gpu.device, &self.config);
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub(crate) fn acquire(&self) -> Result<wgpu::SurfaceTexture, SurfaceError> {
        use wgpu::CurrentSurfaceTexture as Acquired;
        match self.inner.get_current_texture() {
            Acquired::Success(texture) => Ok(texture),
            // Suboptimal still draws. It means the surface wants
            // reconfiguring, which the next resize will do anyway, and
            // throwing the frame away over it would drop frames during every
            // window drag.
            Acquired::Suboptimal(texture) => Ok(texture),
            // Timeout, occluded and outdated are all "not this frame, try
            // again": a window being dragged, minimised, or moved to another
            // monitor. None of them is a failure worth surfacing to a game.
            Acquired::Timeout | Acquired::Occluded | Acquired::Outdated => {
                Err(SurfaceError::Outdated)
            }
            Acquired::Lost => Err(SurfaceError::Lost),
            Acquired::Validation => Err(SurfaceError::Other(
                "validation error acquiring a frame".into(),
            )),
        }
    }

    /// A frame that has been acquired but not yet presented.
    ///
    /// It exists because the overlay draws over the scene: both passes need
    /// the same texture, and acquiring twice would present an empty frame
    /// over a full one.
    pub fn begin_frame(&self) -> Result<AcquiredFrame, SurfaceError> {
        let texture = self.acquire()?;
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(AcquiredFrame {
            texture,
            view,
            width: self.config.width,
            height: self.config.height,
        })
    }

    /// Reconfigure with the size already held — the answer to
    /// [`SurfaceError::Outdated`].
    pub fn reconfigure(&self, gpu: &Gpu) {
        self.inner.configure(&gpu.device, &self.config);
    }
}

/// A frame in progress: acquired, drawn into, not yet shown.
pub struct AcquiredFrame {
    pub(crate) texture: wgpu::SurfaceTexture,
    pub(crate) view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl AcquiredFrame {
    /// The frame as plain bytes rather than sRGB: what a UI draws into,
    /// the same as `OffscreenTarget::ui_view`.
    pub fn ui_view(&self) -> wgpu::TextureView {
        let format = self.texture.texture.format().remove_srgb_suffix();
        self.texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                ..Default::default()
            })
    }

    /// The frame as it is: what the 3D renderer draws into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Hand it to the compositor. Everything drawn into it must already be
    /// submitted.
    pub fn present(self, gpu: &Gpu) {
        gpu.queue.present(self.texture);
    }
}

#[cfg(target_os = "macos")]
impl Surface {
    /// Take a `CAMetalLayer` a native host already owns.
    ///
    /// The path a native Apple host takes: it makes the view, the view has a
    /// layer, and the engine draws into it. No window is created here, which
    /// is the rule the whole crate is arranged around. Kept for iOS, where a
    /// view controller will hand over exactly this; the editor's viewport
    /// goes another way, which is DNA's open question 1.
    ///
    /// # Safety
    /// `layer` must be a live `CAMetalLayer` that outlives the returned
    /// surface. Nothing in Rust can check that — the host owns the view, and
    /// releasing it while the engine still holds a swapchain is a use after
    /// free the type system never sees.
    pub unsafe fn from_metal_layer(
        gpu: &Gpu,
        layer: *mut core::ffi::c_void,
        width: u32,
        height: u32,
    ) -> Result<Self, SurfaceError> {
        if layer.is_null() {
            return Err(SurfaceError::Other("null CAMetalLayer".into()));
        }
        let inner = unsafe {
            gpu.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
        }
        .map_err(|e| SurfaceError::Other(e.to_string()))?;
        Ok(Self::configure(gpu, inner, width, height))
    }
}

#[cfg(feature = "desktop-shell")]
impl Surface {
    /// Take a winit window's surface.
    ///
    /// The window is an `Arc` because the surface borrows it for as long as
    /// it lives, and a surface outliving its window is a crash rather than a
    /// compile error.
    pub fn from_window(
        gpu: &Gpu,
        window: std::sync::Arc<winit::window::Window>,
    ) -> Result<Self, SurfaceError> {
        let size = window.inner_size();
        let inner = gpu
            .instance
            .create_surface(window)
            .map_err(|e| SurfaceError::Other(e.to_string()))?;
        Ok(Self::configure(gpu, inner, size.width, size.height))
    }
}
