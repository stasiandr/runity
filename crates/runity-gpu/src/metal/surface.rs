//! Putting a `CAMetalLayer` where the window's own layer was.
//!
//! The CPU path presents by handing the content view's layer a fresh
//! `CGImage`. The GPU path cannot: it needs a layer that hands *out* textures.
//! So the view's layer is replaced, and the order that happens in is fussier
//! than it looks:
//!
//! 1. `setWantsLayer:NO` — while a view is layer-backed, AppKit owns its layer
//!    and quietly ignores a replacement.
//! 2. `setLayer:` — hand over ours.
//! 3. `setWantsLayer:YES` — layer-backed again, now around our layer.
//! 4. `setLayerContentsRedrawPolicy:` — only meaningful once the view is
//!    layer-backed, and it has to be `Never`: we draw when the loop says so,
//!    not when AppKit decides the view is dirty.
//!
//! Getting steps 1 and 3 the wrong way round leaves an `NSView`-owned
//! `CALayer` on screen and a `CAMetalLayer` nothing ever presents, which looks
//! exactly like a renderer that draws nothing.

use super::ffi::*;
use super::objc::*;
use crate::GpuError;
use std::ffi::c_void;

/// The window's layer, once it is ours.
pub struct Surface {
    view: Id,
    layer: Id,
    color_space: CGColorSpaceRef,
    width: usize,
    height: usize,
    scale: f64,
    forced_scale: Option<f64>,
    vsync: bool,
}

/// `-[NSView bounds]` returns a `CGRect`, which is where the two architectures'
/// calling conventions differ — the same split `runity-platform` makes.
///
/// # Safety
/// `receiver` must respond to `selector` by returning a `CGRect`.
unsafe fn msg_rect(receiver: Id, selector: Sel) -> CGRect {
    #[cfg(target_arch = "x86_64")]
    {
        #[link(name = "objc", kind = "dylib")]
        extern "C" {
            fn objc_msgSend_stret();
        }
        let mut out = CGRect::default();
        let send: unsafe extern "C" fn(*mut CGRect, Id, Sel) =
            std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
        send(&mut out, receiver, selector);
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let send: unsafe extern "C" fn(Id, Sel) -> CGRect =
            std::mem::transmute(objc_msgSend_addr());
        send(receiver, selector)
    }
}

impl Surface {
    /// Replace `view`'s layer with a `CAMetalLayer` bound to `device`.
    ///
    /// # Safety
    ///
    /// `view` must be null or a live `NSView`, and `device` a live `MTLDevice`.
    /// Must be called on the main thread, like every other AppKit call.
    pub unsafe fn attach(device: Id, view: *mut c_void) -> Result<Self, GpuError> {
        if view.is_null() {
            return Err(GpuError::NoSurface);
        }
        let view = view as Id;
        // SAFETY: AppKit and Core Animation calls on the main thread, each
        // with its declared signature; the layer is retained before the
        // autorelease pool that made it pops.
        unsafe {
            let _pool = Pool::push();
            let layer = msg_id(class(b"CAMetalLayer\0"), sel(b"layer\0"));
            if layer.is_null() {
                return Err(GpuError::NoSurface);
            }
            msg(layer, sel(b"retain\0"));

            msg_with_id(layer, sel(b"setDevice:\0"), device);
            msg_with_u64(layer, sel(b"setPixelFormat:\0"), MTLPixelFormatBGRA8Unorm);
            // Nothing reads the drawable back, so let the driver keep it in
            // whatever form is cheapest to scan out.
            msg_with_bool(layer, sel(b"setFramebufferOnly:\0"), YES);
            // The same colour space the CPU path's CGImage is built in, so the
            // two renderers put identical bytes on identical pixels.
            let color_space = CGColorSpaceCreateDeviceRGB();
            msg_with_ptr(layer, sel(b"setColorspace:\0"), color_space);

            msg_with_bool(view, sel(b"setWantsLayer:\0"), NO);
            msg_with_id(view, sel(b"setLayer:\0"), layer);
            msg_with_bool(view, sel(b"setWantsLayer:\0"), YES);
            msg_with_u64(
                view,
                sel(b"setLayerContentsRedrawPolicy:\0"),
                CALayerContentsRedrawNever,
            );

            let mut surface = Self {
                view,
                layer,
                color_space,
                width: 0,
                height: 0,
                scale: 0.0,
                forced_scale: None,
                vsync: true,
            };
            surface.begin();
            Ok(surface)
        }
    }

    /// Re-read the backing scale and resize the drawable if it moved.
    ///
    /// Asked every frame rather than subscribed to: `backingScaleFactor`
    /// changes the moment a window is dragged onto another display, and a
    /// notification would arrive after a frame has already been drawn at the
    /// old size.
    pub fn begin(&mut self) {
        // SAFETY: `bounds`, `window` and `backingScaleFactor` on a live view.
        unsafe {
            let bounds = msg_rect(self.view, sel(b"bounds\0"));
            let window = msg_id(self.view, sel(b"window\0"));
            let scale = if window.is_null() {
                1.0
            } else {
                msg_f64(window, sel(b"backingScaleFactor\0")).max(1.0)
            };
            let scale = self.forced_scale.unwrap_or(scale);
            let width = (bounds.size.width * scale).round().max(1.0);
            let height = (bounds.size.height * scale).round().max(1.0);

            if scale != self.scale {
                self.scale = scale;
                msg_with_f64(self.layer, sel(b"setContentsScale:\0"), scale);
            }
            if width as usize != self.width || height as usize != self.height {
                self.width = width as usize;
                self.height = height as usize;
                msg_with_size(
                    self.layer,
                    sel(b"setDrawableSize:\0"),
                    CGSize { width, height },
                );
            }
        }
    }

    /// The next drawable, or nil.
    ///
    /// Nil is not a failure: the layer has a small pool of drawables and
    /// `nextDrawable` times out rather than blocking forever when they are all
    /// in flight. The frame is skipped and the next one asks again.
    pub fn next_drawable(&mut self) -> Id {
        // SAFETY: `-[CAMetalLayer nextDrawable]` on a layer we own.
        unsafe { msg_id(self.layer, sel(b"nextDrawable\0")) }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// The backing scale in effect, which is 2 on a Retina display.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn force_scale(&mut self, scale: Option<f64>) {
        self.forced_scale = scale;
        self.begin();
    }

    /// Whether the layer is presenting in step with the display.
    pub fn vsync(&self) -> bool {
        self.vsync
    }

    pub fn set_vsync(&mut self, enabled: bool) {
        if enabled == self.vsync {
            return;
        }
        self.vsync = enabled;
        // SAFETY: `setDisplaySyncEnabled:` takes a BOOL.
        unsafe {
            msg_with_bool(
                self.layer,
                sel(b"setDisplaySyncEnabled:\0"),
                if enabled { YES } else { NO },
            );
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the layer was retained on the way in and the colour space
        // was created here; both are released once.
        unsafe {
            release(&mut self.layer);
            if !self.color_space.is_null() {
                CGColorSpaceRelease(self.color_space);
                self.color_space = std::ptr::null_mut();
            }
        }
    }
}

/// The compiled shader library shipped inside an application bundle.
///
/// The runtime compiles `shader.metal` from source first, because that is the
/// single source of truth and it works wherever a Metal toolchain does. This
/// is the belt to that pair of braces: `tools/package-macos.sh` runs
/// `xcrun metal` at packaging time and drops the result in `Contents/
/// Resources`, so a machine that cannot compile MSL at runtime still starts.
pub fn load_bundled_library(device: Id) -> Option<Id> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let candidates = [
        // Contents/MacOS/Binary -> Contents/Resources/runity.metallib
        directory.join("../Resources/runity.metallib"),
        directory.join("runity.metallib"),
    ];
    for path in candidates {
        if !path.exists() {
            continue;
        }
        let text = path.to_string_lossy().into_owned();
        // SAFETY: an NSString path handed to `newLibraryWithFile:error:`,
        // which returns an owned library or nil with an error.
        let library = unsafe {
            let _pool = Pool::push();
            let mut string = nsstring(&text);
            let mut error: Id = NIL;
            let library = msg_id_with_error(
                device,
                sel(b"newLibraryWithFile:error:\0"),
                string,
                &mut error,
            );
            release(&mut string);
            library
        };
        if !library.is_null() {
            eprintln!("runity: loaded the bundled shader library from {text}");
            return Some(library);
        }
    }
    None
}
