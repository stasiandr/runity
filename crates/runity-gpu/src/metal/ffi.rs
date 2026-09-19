//! Metal's enums, structs and the message signatures they travel in.
//!
//! Everything here is a plain transcription of a public header. The constants
//! are the values `MTLPixelFormat` and friends actually have; the structs are
//! their C layouts; the `msg_*` functions are `objc_msgSend` cast to one
//! particular signature, exactly as in [`super::objc`].

#![allow(non_snake_case, non_upper_case_globals)]

use super::objc::{Bool, Id, Sel};
use std::ffi::c_void;

#[link(name = "Metal", kind = "framework")]
extern "C" {
    pub fn MTLCreateSystemDefaultDevice() -> Id;
}

pub type CGColorSpaceRef = *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
    pub fn CGColorSpaceRelease(space: CGColorSpaceRef);
}

// --- MTLPixelFormat --------------------------------------------------------
/// The window's format and the offscreen target's: not `_sRGB`, because the
/// CPU rasterizer writes linear 8-bit values and the present path hands them
/// to a `CGColorSpaceCreateDeviceRGB` image without a transfer curve either.
pub const MTLPixelFormatBGRA8Unorm: u64 = 80;
pub const MTLPixelFormatRGBA16Float: u64 = 115;
pub const MTLPixelFormatDepth32Float: u64 = 252;

// --- MTLResourceOptions / MTLStorageMode -----------------------------------
pub const MTLResourceStorageModeShared: u64 = 0 << 4;
pub const MTLStorageModeShared: u64 = 0;
pub const MTLStorageModePrivate: u64 = 2;

// --- MTLTextureUsage -------------------------------------------------------
pub const MTLTextureUsageShaderRead: u64 = 1;
pub const MTLTextureUsageRenderTarget: u64 = 4;

// --- MTLLoadAction / MTLStoreAction ----------------------------------------
pub const MTLLoadActionClear: u64 = 2;
pub const MTLStoreActionStore: u64 = 1;
pub const MTLStoreActionDontCare: u64 = 0;

// --- MTLCompareFunction ----------------------------------------------------
pub const MTLCompareFunctionAlways: u64 = 7;
/// The rasterizer's test is `z < depth[i]`, so this is the matching one.
pub const MTLCompareFunctionLess: u64 = 1;

// --- MTLWinding / MTLCullMode ----------------------------------------------
pub const MTLWindingClockwise: u64 = 0;
pub const MTLWindingCounterClockwise: u64 = 1;
pub const MTLCullModeNone: u64 = 0;
pub const MTLCullModeFront: u64 = 1;
pub const MTLCullModeBack: u64 = 2;

// --- MTLPrimitiveType / MTLIndexType ---------------------------------------
pub const MTLPrimitiveTypeTriangle: u64 = 3;
pub const MTLIndexTypeUInt32: u64 = 1;

// --- MTLBlendFactor / MTLBlendOperation ------------------------------------
pub const MTLBlendFactorOne: u64 = 1;
pub const MTLBlendFactorSourceAlpha: u64 = 4;
pub const MTLBlendFactorOneMinusSourceAlpha: u64 = 5;
pub const MTLBlendFactorOneMinusDestinationAlpha: u64 = 9;
pub const MTLBlendOperationAdd: u64 = 0;

// --- MTLSamplerMinMagFilter / MTLSamplerAddressMode ------------------------
pub const MTLSamplerMinMagFilterNearest: u64 = 0;
pub const MTLSamplerMinMagFilterLinear: u64 = 1;
/// The whole enum, because the three we use are not the first three:
/// `ClampToEdge`, `MirrorClampToEdge`, `Repeat`, `MirrorRepeat`, ...
pub const MTLSamplerAddressModeClampToEdge: u64 = 0;
pub const MTLSamplerAddressModeRepeat: u64 = 2;
pub const MTLSamplerAddressModeMirrorRepeat: u64 = 3;

// --- CALayerContentsRedrawPolicy -------------------------------------------
/// `kCALayerContentsRedrawNever`: we draw when we say so, not when AppKit
/// decides the layer is dirty.
pub const CALayerContentsRedrawNever: u64 = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MTLClearColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MTLOrigin {
    pub x: usize,
    pub y: usize,
    pub z: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MTLSize {
    pub width: usize,
    pub height: usize,
    pub depth: usize,
}

/// 48 bytes, so it travels by hidden pointer on arm64 — which is what
/// `extern "C"` already does for it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MTLRegion {
    pub origin: MTLOrigin,
    pub size: MTLSize,
}

impl MTLRegion {
    pub fn two_d(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            origin: MTLOrigin { x, y, z: 0 },
            size: MTLSize {
                width,
                height,
                depth: 1,
            },
        }
    }
}

macro_rules! msg_send_fn {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        /// # Safety
        /// `receiver` must respond to `selector` with exactly this signature.
        #[allow(clippy::too_many_arguments)]
        pub unsafe fn $name(receiver: Id, selector: Sel $(, $arg: $ty)*) -> $ret {
            let send: unsafe extern "C" fn(Id, Sel $(, $ty)*) -> $ret =
                std::mem::transmute(super::objc::objc_msgSend_addr());
            send(receiver, selector $(, $arg)*)
        }
    };
}

msg_send_fn!(msg_with_size(a: CGSize) -> ());
msg_send_fn!(msg_with_clear_color(a: MTLClearColor) -> ());
msg_send_fn!(msg_with_ptr(a: *const c_void) -> ());
msg_send_fn!(msg_id_with_error(a: Id, err: *mut Id) -> Id);
msg_send_fn!(msg_id_with_options_error(a: Id, opts: Id, err: *mut Id) -> Id);
msg_send_fn!(msg_new_buffer(length: usize, options: u64) -> Id);
msg_send_fn!(msg_texture_descriptor(format: u64, width: usize, height: usize, mipmapped: Bool) -> Id);
msg_send_fn!(msg_set_buffer(buffer: Id, offset: usize, index: u64) -> ());
msg_send_fn!(msg_set_indexed(object: Id, index: u64) -> ());
msg_send_fn!(msg_draw_indexed(primitive: u64, count: usize, index_type: u64, buffer: Id, offset: usize) -> ());
msg_send_fn!(msg_replace_region(region: MTLRegion, level: usize, bytes: *const c_void, row: usize) -> ());
msg_send_fn!(msg_blit_texture_to_buffer(
    texture: Id,
    slice: usize,
    level: usize,
    origin: MTLOrigin,
    size: MTLSize,
    buffer: Id,
    offset: usize,
    bytes_per_row: usize,
    bytes_per_image: usize
) -> ());

#[cfg(test)]
mod tests {
    use super::*;

    /// These layouts are the ABI contract with Metal. A wrong size means an
    /// argument travels in registers where the framework expects a pointer,
    /// and the failure is silent corruption rather than a crash.
    #[test]
    fn the_struct_layouts_match_the_metal_headers() {
        assert_eq!(std::mem::size_of::<CGSize>(), 16);
        assert_eq!(std::mem::size_of::<MTLClearColor>(), 32);
        assert_eq!(std::mem::size_of::<MTLOrigin>(), 24);
        assert_eq!(std::mem::size_of::<MTLSize>(), 24);
        assert_eq!(std::mem::size_of::<MTLRegion>(), 48);
    }

    #[test]
    fn a_two_d_region_covers_one_slice() {
        let r = MTLRegion::two_d(0, 0, 8, 4);
        assert_eq!(r.size.depth, 1, "a 2-D region is one slice deep");
        assert_eq!((r.size.width, r.size.height), (8, 4));
    }
}
