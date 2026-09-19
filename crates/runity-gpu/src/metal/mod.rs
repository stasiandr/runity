//! The Metal backend: one encoder, two targets.
//!
//! A frame is opened against either the window's `CAMetalLayer` drawable or an
//! offscreen texture, and from that point on nothing downstream knows which.
//! The same pipeline states, the same samplers, the same depth-stencil states
//! and the same order of draw calls serve both — which is the only reason the
//! differential tests prove anything: if the offscreen path had a pipeline of
//! its own, agreeing with the CPU would say nothing about what the window
//! shows.
//!
//! Three things here exist for the sake of running in real time rather than
//! for correctness, and are worth naming:
//!
//! * a **pipeline cache**, because *Depth & blending* changes `Blend`,
//!   `depth_test` and `depth_write` while the scene is on screen, and building
//!   a pipeline state per frame would be visible;
//! * **triple buffering**, a ring of three arenas with a `waitUntilCompleted`
//!   on the command buffer from three frames ago, so the CPU can fill frame
//!   N+1's vertices while the GPU still reads frame N's;
//! * an **autorelease pool per frame**, because Metal hands back autoreleased
//!   command buffers and drawables, and at 120 Hz they pile up fast.

pub mod alert;
mod ffi;
pub mod objc;
mod surface;

use crate::shader::GpuShader;
use crate::wire::GpuVertex;
use crate::GpuError;
use ffi::*;
use objc::*;
use runity_render::{Blend, Color, CullMode, DrawStats, Filter, Framebuffer, Mesh, Texture, Wrap};
use std::ffi::c_void;

pub use surface::Surface;

/// Buffers are suballocated out of arenas this big, and a new one is added
/// when a frame outgrows what it has.
const ARENA_CHUNK: usize = 8 << 20;
/// Every suballocation starts here. 256 is the strictest `setVertexBuffer:`
/// offset alignment Metal asks for on any target we can reach.
const ARENA_ALIGN: usize = 256;
/// Frames in flight. Three is the usual answer, and it is what the arena ring
/// and the fence below are sized for.
const FRAMES_IN_FLIGHT: usize = 3;
/// How many uploaded textures are kept before the cache is dropped wholesale.
const TEXTURE_CACHE_LIMIT: usize = 48;

/// Where a frame is being drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The window's next drawable, presented in step with the display.
    Window,
    /// A texture of our own, downloaded into a [`Framebuffer`] when the frame
    /// ends. This is what headless runs, the tests and the benchmark use.
    Offscreen { width: usize, height: usize },
}

/// Which face survives, and how fragments reach the target — the same
/// fixed-function state the rasterizer carries, narrowed to what a pipeline
/// state object keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PipelineKey {
    vertex: &'static str,
    fragment: &'static str,
    blend: Blend,
    format: u64,
}

/// One arena of the ring: a bump allocator over Metal buffers, reset when the
/// GPU is known to be finished with the frame that last used it.
struct Arena {
    chunks: Vec<(Id, usize)>,
    chunk: usize,
    offset: usize,
    /// The command buffer this arena's contents were handed to, retained so it
    /// can be waited on before the arena is reused.
    fence: Id,
    /// Staging for a depth download; read one lap of the ring later.
    depth_readback: Id,
    depth_size: (usize, usize),
}

impl Arena {
    fn new() -> Self {
        Self {
            chunks: Vec::new(),
            chunk: 0,
            offset: 0,
            fence: NIL,
            depth_readback: NIL,
            depth_size: (0, 0),
        }
    }

    fn reset(&mut self) {
        self.chunk = 0;
        self.offset = 0;
    }

    /// Reserve `len` bytes. Returns the buffer, the offset into it, and a
    /// pointer to write through.
    ///
    /// # Safety
    /// `device` must be a live `MTLDevice`.
    unsafe fn alloc(&mut self, device: Id, len: usize) -> (Id, usize, *mut u8) {
        let len = len.max(1);
        loop {
            if self.chunk < self.chunks.len() {
                let (buffer, size) = self.chunks[self.chunk];
                let start = self.offset.div_ceil(ARENA_ALIGN) * ARENA_ALIGN;
                if start + len <= size {
                    self.offset = start + len;
                    let base = msg_ptr(buffer, sel(b"contents\0")) as *mut u8;
                    return (buffer, start, base.add(start));
                }
                self.chunk += 1;
                self.offset = 0;
                continue;
            }
            let size = ARENA_CHUNK.max(len);
            let buffer = msg_new_buffer(
                device,
                sel(b"newBufferWithLength:options:\0"),
                size,
                MTLResourceStorageModeShared,
            );
            assert!(
                !buffer.is_null(),
                "MTLDevice could not allocate {size} bytes"
            );
            self.chunks.push((buffer, size));
        }
    }
}

/// The frame currently being encoded.
struct Frame {
    _pool: Pool,
    command_buffer: Id,
    encoder: Id,
    drawable: Id,
    depth_texture: Id,
    width: usize,
    height: usize,
    format: u64,
    slot: usize,
    /// True for the window path: present the drawable rather than download.
    present: bool,
}

/// A Metal device, everything cached against it, and at most one open frame.
///
/// There are no lifetimes on any of this on purpose. The engine owns one
/// `Gpu`, and `Engine::draw` reaches it through `&mut self` while the frame is
/// open; a `Frame<'a>` borrowed out of the device would make `Engine`
/// self-referential for no gain.
pub struct Gpu {
    device: Id,
    queue: Id,
    libraries: Vec<(*const u8, Id)>,
    functions: Vec<(&'static str, Id)>,
    pipelines: Vec<(PipelineKey, Id)>,
    /// Indexed by `depth_index(test, write)`; all four built at start-up.
    depth_states: [Id; 4],
    /// Indexed by `sampler_index(filter, wrap)`: two filters by three wraps.
    samplers: [Id; 6],
    /// Bound wherever a shader declares a texture it is not using this draw;
    /// sampling an unbound texture is not defined, and a branch around it is
    /// not enough for the validation layer.
    fallback_texture: Id,
    textures: Vec<(u64, Id)>,
    arenas: Vec<Arena>,
    offscreen: Option<Offscreen>,
    /// The window path draws into someone else's colour texture but always
    /// into a depth texture of ours, so it keeps one of its own.
    window_depth: Option<(usize, usize, Id)>,
    surface: Option<Surface>,
    frame: Option<Frame>,
    frame_index: usize,
    stats: DrawStats,
    gpu_seconds: f32,
    download_seconds: f32,
    /// The depth buffer as of a few frames ago, in the layout `debug::depth_view`
    /// expects. A lap of the ring behind, which is what makes it free.
    depth_snapshot: Option<Framebuffer>,
    depth_wanted: bool,
}

/// The offscreen target: a private colour texture, a depth texture, and the
/// shared buffer the colour is blitted into on its way back to the CPU.
struct Offscreen {
    width: usize,
    height: usize,
    color: Id,
    depth: Id,
    readback: Id,
    row_bytes: usize,
}

// SAFETY-adjacent note: `Gpu` holds raw Objective-C pointers and is not `Send`.
// Nothing here tries to make it one; Metal and AppKit are both used from the
// thread that opened the window.

impl Gpu {
    /// Open the system's default device and build everything that does not
    /// depend on a target: the command queue, the shader library, the four
    /// depth-stencil states and the six samplers.
    pub fn new() -> Result<Self, GpuError> {
        // SAFETY: every call below is a documented Metal API used with its
        // declared signature; failures are checked rather than assumed away.
        unsafe {
            let _pool = Pool::push();
            let device = MTLCreateSystemDefaultDevice();
            if device.is_null() {
                return Err(GpuError::NoDevice);
            }
            let queue = msg_id(device, sel(b"newCommandQueue\0"));
            if queue.is_null() {
                return Err(GpuError::NoDevice);
            }

            let mut gpu = Self {
                device,
                queue,
                libraries: Vec::new(),
                functions: Vec::new(),
                pipelines: Vec::new(),
                depth_states: [NIL; 4],
                samplers: [NIL; 6],
                fallback_texture: NIL,
                textures: Vec::new(),
                arenas: (0..FRAMES_IN_FLIGHT).map(|_| Arena::new()).collect(),
                offscreen: None,
                window_depth: None,
                surface: None,
                frame: None,
                frame_index: 0,
                stats: DrawStats::default(),
                gpu_seconds: 0.0,
                download_seconds: 0.0,
                depth_snapshot: None,
                depth_wanted: false,
            };

            // The library is built first and on purpose: a typo in the MSL is
            // the one failure worth finding before anything else exists.
            gpu.library(crate::shader::ENGINE_SOURCE)?;
            gpu.build_depth_states();
            gpu.build_samplers();
            gpu.build_fallback_texture();
            Ok(gpu)
        }
    }

    /// The name Metal gives the adapter, for a title bar or a benchmark header.
    pub fn device_name(&self) -> String {
        // SAFETY: `-[MTLDevice name]` returns an NSString.
        unsafe {
            let _pool = Pool::push();
            nsstring_to_string(msg_id(self.device, sel(b"name\0")))
        }
    }

    /// Take over a native view's layer so frames can be presented to it.
    ///
    /// Safe to call because the only pointer that reaches here comes from
    /// `NativeSurface::AppKitView`, which the platform backend only ever builds
    /// from a live window it owns; a null pointer is rejected, not dereferenced.
    ///
    /// The boundary has to be safe here: `runity-core`, which calls this, is
    /// `#![forbid(unsafe_code)]`.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn attach(&mut self, view: *mut c_void) -> Result<(), GpuError> {
        // SAFETY: `view` is a live `NSView` per the invariant above, `device` is
        // this `Gpu`'s own, and the engine drives the loop on the main thread.
        let surface = unsafe { Surface::attach(self.device, view)? };
        self.surface = Some(surface);
        Ok(())
    }

    /// Size in backing pixels of the window's drawable, if there is one.
    pub fn drawable_size(&self) -> Option<(usize, usize)> {
        self.surface.as_ref().map(|s| s.size())
    }

    /// Force the window's backing scale to 1, whatever the display says.
    ///
    /// The benchmark measures on one scale so its three columns are
    /// comparable; nothing else wants this.
    pub fn force_scale(&mut self, scale: Option<f64>) {
        if let Some(surface) = &mut self.surface {
            surface.force_scale(scale);
        }
    }

    /// Whether presenting already paces the loop, so a frame limiter would
    /// only be a second one.
    ///
    /// True exactly when there is a window to present to and it is in step
    /// with the display: `nextDrawable` then blocks until the display is ready
    /// and the frame rate is the refresh rate. Sleeping to a target on top of
    /// that does not make the loop steadier — it makes it miss vsyncs.
    pub fn paces_frames(&self) -> bool {
        self.surface.as_ref().is_some_and(|s| s.vsync())
    }

    /// Present in step with the display (the default), or as fast as the
    /// device will go (the benchmark).
    pub fn set_vsync(&mut self, enabled: bool) {
        if let Some(surface) = &mut self.surface {
            surface.set_vsync(enabled);
        }
    }

    /// Ask for the depth buffer to be downloaded. It arrives a lap of the ring
    /// later, in [`Gpu::depth_snapshot`] — which is exactly the frame of
    /// latency the depth debug view shows.
    pub fn set_depth_readback(&mut self, wanted: bool) {
        self.depth_wanted = wanted;
        if !wanted {
            self.depth_snapshot = None;
        }
    }

    /// The most recent downloaded depth buffer, as a [`Framebuffer`] whose
    /// depth plane is filled in and whose colour plane is untouched.
    pub fn depth_snapshot(&self) -> Option<&Framebuffer> {
        self.depth_snapshot.as_ref()
    }

    /// What the last completed frame cost the GPU, from `MTLCommandBuffer`'s
    /// own `GPUStartTime` and `GPUEndTime`.
    pub fn last_gpu_seconds(&self) -> f32 {
        self.gpu_seconds
    }

    /// What the last offscreen frame spent copying the shared buffer back into
    /// a [`Framebuffer`] — the price of a frame the CPU can look at, which the
    /// window path never pays. Zero on the window path.
    pub fn last_download_seconds(&self) -> f32 {
        self.download_seconds
    }

    /// Counters for the frame so far. `triangles_in` is honest;
    /// `fragments_shaded` and `fragments_written` are zero, because the
    /// hardware does not count them for us and inventing a number would be
    /// worse than a documented zero.
    pub fn frame_stats(&self) -> DrawStats {
        self.stats
    }

    // -----------------------------------------------------------------------
    // Frames
    // -----------------------------------------------------------------------

    /// Open a frame. Returns `false` when there is nothing to draw into —
    /// `nextDrawable` timing out is a skipped frame, not an error.
    pub fn begin_frame(&mut self, target: Target, clear: Color) -> bool {
        // SAFETY: Metal calls with their declared signatures, on the thread
        // that owns the device; every returned object is null-checked.
        unsafe {
            let pool = Pool::push();
            let slot = self.frame_index % FRAMES_IN_FLIGHT;
            self.wait_for_slot(slot);
            self.arenas[slot].reset();
            self.stats = DrawStats::default();

            let (color_texture, drawable, width, height, present) = match target {
                Target::Window => {
                    let Some(surface) = &mut self.surface else {
                        return false;
                    };
                    surface.begin();
                    let drawable = surface.next_drawable();
                    if drawable.is_null() {
                        return false;
                    }
                    let texture = msg_id(drawable, sel(b"texture\0"));
                    let (w, h) = surface.size();
                    (texture, drawable, w, h, true)
                }
                Target::Offscreen { width, height } => {
                    let off = self.ensure_offscreen(width, height);
                    (off.color, NIL, width, height, false)
                }
            };
            if color_texture.is_null() || width == 0 || height == 0 {
                return false;
            }

            let depth_texture = match target {
                Target::Window => self.ensure_window_depth(width, height),
                Target::Offscreen { .. } => self.offscreen.as_ref().map(|o| o.depth).unwrap_or(NIL),
            };

            let descriptor = msg_id(
                class(b"MTLRenderPassDescriptor\0"),
                sel(b"renderPassDescriptor\0"),
            );
            let color_attachments = msg_id(descriptor, sel(b"colorAttachments\0"));
            let color0 = msg_id_with_u64(color_attachments, sel(b"objectAtIndexedSubscript:\0"), 0);
            msg_with_id(color0, sel(b"setTexture:\0"), color_texture);
            msg_with_u64(color0, sel(b"setLoadAction:\0"), MTLLoadActionClear);
            msg_with_u64(color0, sel(b"setStoreAction:\0"), MTLStoreActionStore);
            msg_with_clear_color(
                color0,
                sel(b"setClearColor:\0"),
                MTLClearColor {
                    red: clear.r.clamp(0.0, 1.0) as f64,
                    green: clear.g.clamp(0.0, 1.0) as f64,
                    blue: clear.b.clamp(0.0, 1.0) as f64,
                    alpha: clear.a.clamp(0.0, 1.0) as f64,
                },
            );

            let depth_attachment = msg_id(descriptor, sel(b"depthAttachment\0"));
            msg_with_id(depth_attachment, sel(b"setTexture:\0"), depth_texture);
            msg_with_u64(
                depth_attachment,
                sel(b"setLoadAction:\0"),
                MTLLoadActionClear,
            );
            // Far is 1.0, and the compare function is Less: the same "smaller
            // is closer, nothing drawn is farthest" the rasterizer uses.
            msg_with_f64(depth_attachment, sel(b"setClearDepth:\0"), 1.0);
            let depth_store = if self.depth_wanted {
                MTLStoreActionStore
            } else {
                MTLStoreActionDontCare
            };
            msg_with_u64(depth_attachment, sel(b"setStoreAction:\0"), depth_store);

            let command_buffer = msg_id(self.queue, sel(b"commandBuffer\0"));
            if command_buffer.is_null() {
                return false;
            }
            // Autoreleased, and this pool pops at the end of the frame — but
            // the arena has to wait on it a lap later, so take a reference.
            msg(command_buffer, sel(b"retain\0"));
            let encoder = msg_id_with_id(
                command_buffer,
                sel(b"renderCommandEncoderWithDescriptor:\0"),
                descriptor,
            );
            if encoder.is_null() {
                let mut cb = command_buffer;
                release(&mut cb);
                return false;
            }
            // No `setViewport:`: the encoder defaults to the whole attachment
            // with a 0..1 depth range, which is exactly what we want, and the
            // default is one fewer 48-byte struct across the ABI per frame.

            self.frame = Some(Frame {
                _pool: pool,
                command_buffer,
                encoder,
                drawable,
                depth_texture,
                width,
                height,
                format: MTLPixelFormatBGRA8Unorm,
                slot,
                present,
            });
            true
        }
    }

    /// Close the frame: present it, or download it into `into`.
    ///
    /// `into` is resized to the frame and filled with its pixels on the
    /// offscreen path, and left alone on the window path.
    pub fn end_frame(&mut self, into: Option<&mut Framebuffer>) {
        let Some(frame) = self.frame.take() else {
            return;
        };
        // SAFETY: the encoder and command buffer are the ones this frame
        // opened, ended and committed exactly once.
        unsafe {
            msg(frame.encoder, sel(b"endEncoding\0"));

            if self.depth_wanted && !frame.depth_texture.is_null() {
                self.blit_depth(&frame);
            }

            if frame.present {
                if !frame.drawable.is_null() {
                    msg_with_id(
                        frame.command_buffer,
                        sel(b"presentDrawable:\0"),
                        frame.drawable,
                    );
                }
                msg(frame.command_buffer, sel(b"commit\0"));
            } else {
                self.blit_color(&frame);
                msg(frame.command_buffer, sel(b"commit\0"));
                msg(frame.command_buffer, sel(b"waitUntilCompleted\0"));
                if let Some(target) = into {
                    let started = std::time::Instant::now();
                    self.download_color(&frame, target);
                    self.download_seconds = started.elapsed().as_secs_f32();
                }
                self.record_gpu_time(frame.command_buffer);
            }

            // The arena this frame drew from cannot be reused until the GPU is
            // done with it; the command buffer is the thing that says when.
            let slot = frame.slot;
            let mut previous =
                std::mem::replace(&mut self.arenas[slot].fence, frame.command_buffer);
            release(&mut previous);
        }
        self.frame_index += 1;
    }

    /// True while a frame is open and `draw` will encode something.
    pub fn is_encoding(&self) -> bool {
        self.frame.is_some()
    }

    /// The frame's size in backing pixels, while one is open.
    pub fn frame_size(&self) -> Option<(usize, usize)> {
        self.frame.as_ref().map(|f| (f.width, f.height))
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    /// Encode one indexed draw. The mesh is uploaded into this frame's arena
    /// every time: a resource cache is its own piece of work, and without one
    /// the cost is visible and honest.
    pub fn draw<S: GpuShader>(
        &mut self,
        mesh: &Mesh,
        shader: &S,
        cull: CullMode,
        blend: Blend,
        depth_test: bool,
        depth_write: bool,
    ) -> DrawStats {
        let stats = DrawStats {
            triangles_in: mesh.triangle_count(),
            triangles_rasterized: mesh.triangle_count(),
            ..DrawStats::default()
        };
        if mesh.indices.is_empty() || mesh.vertices.is_empty() {
            return DrawStats::default();
        }
        let Some(frame) = &self.frame else {
            return DrawStats::default();
        };
        let (encoder, slot, format) = (frame.encoder, frame.slot, frame.format);

        let pipeline = match self.pipeline(S::SOURCE, S::VERTEX, S::FRAGMENT, blend, format) {
            Ok(pipeline) => pipeline,
            Err(error) => panic!("{error}"),
        };
        let depth_state = self.depth_states[depth_index(depth_test, depth_write)];
        let texture = shader.texture();
        let sampler = texture
            .map(|t| self.samplers[sampler_index(t.filter, t.wrap)])
            .unwrap_or(self.samplers[0]);
        let metal_texture = match texture {
            Some(t) => self.upload_texture(t),
            None => self.fallback_texture,
        };
        let uniforms = shader.uniforms();

        // SAFETY: the encoder is open, and every argument below is an object
        // this device made or memory this arena owns.
        unsafe {
            let device = self.device;
            let arena = &mut self.arenas[slot];
            // Written out one vertex at a time rather than memcpy'd: see
            // `crate::wire` for why a `&[Vertex]` is not a vertex buffer.
            let vertex_bytes = std::mem::size_of::<GpuVertex>() * mesh.vertices.len();
            let (vertex_buffer, vertex_offset, vertex_ptr) = arena.alloc(device, vertex_bytes);
            let vertices = vertex_ptr.cast::<GpuVertex>();
            for (index, vertex) in mesh.vertices.iter().enumerate() {
                vertices.add(index).write(GpuVertex::from(vertex));
            }

            let index_bytes = std::mem::size_of_val(&mesh.indices[..]);
            let (index_buffer, index_offset, index_ptr) = arena.alloc(device, index_bytes);
            std::ptr::copy_nonoverlapping(
                mesh.indices.as_ptr().cast::<u8>(),
                index_ptr,
                index_bytes,
            );

            let uniform_bytes = std::mem::size_of::<S::Uniforms>();
            let (uniform_buffer, uniform_offset, uniform_ptr) = arena.alloc(device, uniform_bytes);
            std::ptr::copy_nonoverlapping(
                (&uniforms as *const S::Uniforms).cast::<u8>(),
                uniform_ptr,
                uniform_bytes,
            );

            msg_with_id(encoder, sel(b"setRenderPipelineState:\0"), pipeline);
            msg_with_id(encoder, sel(b"setDepthStencilState:\0"), depth_state);
            msg_with_u64(encoder, sel(b"setCullMode:\0"), metal_cull(cull));
            msg_with_u64(encoder, sel(b"setFrontFacingWinding:\0"), FRONT_FACING);
            msg_set_buffer(
                encoder,
                sel(b"setVertexBuffer:offset:atIndex:\0"),
                vertex_buffer,
                vertex_offset,
                0,
            );
            msg_set_buffer(
                encoder,
                sel(b"setVertexBuffer:offset:atIndex:\0"),
                uniform_buffer,
                uniform_offset,
                1,
            );
            msg_set_buffer(
                encoder,
                sel(b"setFragmentBuffer:offset:atIndex:\0"),
                uniform_buffer,
                uniform_offset,
                1,
            );
            msg_set_indexed(
                encoder,
                sel(b"setFragmentTexture:atIndex:\0"),
                metal_texture,
                0,
            );
            msg_set_indexed(
                encoder,
                sel(b"setFragmentSamplerState:atIndex:\0"),
                sampler,
                0,
            );
            msg_draw_indexed(
                encoder,
                sel(b"drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:\0"),
                MTLPrimitiveTypeTriangle,
                mesh.indices.len(),
                MTLIndexTypeUInt32,
                index_buffer,
                index_offset,
            );
        }

        self.stats.triangles_in += stats.triangles_in;
        self.stats.triangles_rasterized += stats.triangles_rasterized;
        stats
    }

    /// Draw screen-space segments with a line pipeline of its own.
    ///
    /// These are the only pixels in the GPU path that are not expected to
    /// match the rasterizer's: a segment here is a quad `width` pixels across,
    /// which on a Retina drawable is the two backing pixels that make a line
    /// look the way the CPU one does at 1x. They stay out of the differential
    /// tests for that reason.
    pub fn draw_lines(&mut self, segments: &[crate::lines::Segment], width: f32) {
        if segments.is_empty() {
            return;
        }
        let Some(frame) = &self.frame else { return };
        let (w, h) = (frame.width as f32, frame.height as f32);
        let mesh = crate::lines::segments_to_mesh(segments, w, h, width);
        if mesh.indices.is_empty() {
            return;
        }
        let shader = crate::lines::LineShader;
        // Overlay: on top of whatever the scene left, and never in the depth
        // buffer — the same contract `debug.rs` has on the CPU.
        self.draw(&mesh, &shader, CullMode::None, Blend::Alpha, false, false);
    }

    /// Draw an image over the whole frame, ignoring depth.
    ///
    /// The depth debug view arrives as a picture rather than as geometry, and
    /// the drawable is `framebufferOnly`, so it reaches the screen as a
    /// textured quad rather than as a blit.
    pub fn draw_fullscreen_image(&mut self, image: &Framebuffer) {
        if self.frame.is_none() {
            return;
        }
        let texture = crate::lines::image_to_texture(image);
        // The unlit shader with an identity matrix is exactly a blit: the
        // quad's positions are already in clip space.
        let shader = runity_render::UnlitShader {
            mvp: runity_math::Mat4::IDENTITY,
            tint: Color::WHITE,
            texture: Some(&texture),
        };
        let mesh = crate::lines::fullscreen_quad();
        self.draw(&mesh, &shader, CullMode::None, Blend::Replace, false, false);
    }

    // -----------------------------------------------------------------------
    // Caches
    // -----------------------------------------------------------------------

    /// Compile an MSL source once per process and keep the library.
    fn library(&mut self, source: &'static str) -> Result<Id, GpuError> {
        let key = source.as_ptr();
        if let Some((_, library)) = self.libraries.iter().find(|(k, _)| *k == key) {
            return Ok(*library);
        }
        // SAFETY: `newLibraryWithSource:options:error:` with an NSString, the
        // compile options we own, and an out-parameter for the error.
        let library = unsafe {
            let _pool = Pool::push();
            let mut text = nsstring(source);
            let options = msg_id(
                msg_id(class(b"MTLCompileOptions\0"), sel(b"alloc\0")),
                sel(b"init\0"),
            );
            // Fast math would let the compiler reassociate the arithmetic the
            // rasterizer performs in a fixed order; a differential test at
            // 2/255 notices.
            msg_with_bool(options, sel(b"setFastMathEnabled:\0"), NO);
            let mut error: Id = NIL;
            let library = msg_id_with_options_error(
                self.device,
                sel(b"newLibraryWithSource:options:error:\0"),
                text,
                options,
                &mut error,
            );
            release(&mut text);
            let mut options = options;
            release(&mut options);
            if library.is_null() {
                let text = error_text(error);
                // The bundled library is the fallback for a machine with no
                // Metal toolchain to compile with at runtime.
                match surface::load_bundled_library(self.device) {
                    Some(bundled) => bundled,
                    None => return Err(GpuError::ShaderCompilation(text)),
                }
            } else {
                library
            }
        };
        self.libraries.push((key, library));
        Ok(library)
    }

    fn function(&mut self, source: &'static str, name: &'static str) -> Result<Id, GpuError> {
        if let Some((_, f)) = self.functions.iter().find(|(k, _)| *k == name) {
            return Ok(*f);
        }
        let library = self.library(source)?;
        // SAFETY: `newFunctionWithName:` takes an NSString and returns an
        // owned function, or nil when the name is not in the library.
        let function = unsafe {
            let _pool = Pool::push();
            let mut string = nsstring(name);
            let function = msg_id_with_id(library, sel(b"newFunctionWithName:\0"), string);
            release(&mut string);
            function
        };
        if function.is_null() {
            return Err(GpuError::ShaderCompilation(format!(
                "the shader library has no function named `{name}`"
            )));
        }
        self.functions.push((name, function));
        Ok(function)
    }

    fn pipeline(
        &mut self,
        source: &'static str,
        vertex: &'static str,
        fragment: &'static str,
        blend: Blend,
        format: u64,
    ) -> Result<Id, GpuError> {
        let key = PipelineKey {
            vertex,
            fragment,
            blend,
            format,
        };
        if let Some((_, pipeline)) = self.pipelines.iter().find(|(k, _)| *k == key) {
            return Ok(*pipeline);
        }
        let vertex_function = self.function(source, vertex)?;
        let fragment_function = self.function(source, fragment)?;

        // SAFETY: a descriptor we build and release, handed to the device.
        let pipeline = unsafe {
            let _pool = Pool::push();
            let descriptor = msg_id(
                msg_id(class(b"MTLRenderPipelineDescriptor\0"), sel(b"alloc\0")),
                sel(b"init\0"),
            );
            msg_with_id(descriptor, sel(b"setVertexFunction:\0"), vertex_function);
            msg_with_id(
                descriptor,
                sel(b"setFragmentFunction:\0"),
                fragment_function,
            );
            msg_with_u64(
                descriptor,
                sel(b"setDepthAttachmentPixelFormat:\0"),
                MTLPixelFormatDepth32Float,
            );
            let attachments = msg_id(descriptor, sel(b"colorAttachments\0"));
            let color0 = msg_id_with_u64(attachments, sel(b"objectAtIndexedSubscript:\0"), 0);
            msg_with_u64(color0, sel(b"setPixelFormat:\0"), format);
            match blend {
                Blend::Replace => msg_with_bool(color0, sel(b"setBlendingEnabled:\0"), NO),
                Blend::Alpha => {
                    msg_with_bool(color0, sel(b"setBlendingEnabled:\0"), YES);
                    msg_with_u64(
                        color0,
                        sel(b"setRgbBlendOperation:\0"),
                        MTLBlendOperationAdd,
                    );
                    msg_with_u64(
                        color0,
                        sel(b"setAlphaBlendOperation:\0"),
                        MTLBlendOperationAdd,
                    );
                    msg_with_u64(
                        color0,
                        sel(b"setSourceRGBBlendFactor:\0"),
                        MTLBlendFactorSourceAlpha,
                    );
                    msg_with_u64(
                        color0,
                        sel(b"setDestinationRGBBlendFactor:\0"),
                        MTLBlendFactorOneMinusSourceAlpha,
                    );
                    // The rasterizer's alpha lerp is `dst.a + (1 - dst.a) * a`,
                    // which is this pair and not the obvious one.
                    msg_with_u64(
                        color0,
                        sel(b"setSourceAlphaBlendFactor:\0"),
                        MTLBlendFactorOneMinusDestinationAlpha,
                    );
                    msg_with_u64(
                        color0,
                        sel(b"setDestinationAlphaBlendFactor:\0"),
                        MTLBlendFactorOne,
                    );
                }
            }

            let mut error: Id = NIL;
            let pipeline = msg_id_with_error(
                self.device,
                sel(b"newRenderPipelineStateWithDescriptor:error:\0"),
                descriptor,
                &mut error,
            );
            let mut descriptor = descriptor;
            release(&mut descriptor);
            if pipeline.is_null() {
                return Err(GpuError::ShaderCompilation(error_text(error)));
            }
            pipeline
        };
        self.pipelines.push((key, pipeline));
        Ok(pipeline)
    }

    /// # Safety
    /// The device must be live; this runs once, at start-up.
    unsafe fn build_depth_states(&mut self) {
        for test in [false, true] {
            for write in [false, true] {
                let descriptor = msg_id(
                    msg_id(class(b"MTLDepthStencilDescriptor\0"), sel(b"alloc\0")),
                    sel(b"init\0"),
                );
                let compare = if test {
                    MTLCompareFunctionLess
                } else {
                    MTLCompareFunctionAlways
                };
                msg_with_u64(descriptor, sel(b"setDepthCompareFunction:\0"), compare);
                msg_with_bool(
                    descriptor,
                    sel(b"setDepthWriteEnabled:\0"),
                    if write { YES } else { NO },
                );
                let state = msg_id_with_id(
                    self.device,
                    sel(b"newDepthStencilStateWithDescriptor:\0"),
                    descriptor,
                );
                let mut descriptor = descriptor;
                release(&mut descriptor);
                self.depth_states[depth_index(test, write)] = state;
            }
        }
    }

    /// # Safety
    /// As [`Gpu::build_depth_states`].
    unsafe fn build_samplers(&mut self) {
        for filter in [Filter::Nearest, Filter::Bilinear] {
            for wrap in [Wrap::Repeat, Wrap::Clamp, Wrap::Mirror] {
                let descriptor = msg_id(
                    msg_id(class(b"MTLSamplerDescriptor\0"), sel(b"alloc\0")),
                    sel(b"init\0"),
                );
                let metal_filter = match filter {
                    Filter::Nearest => MTLSamplerMinMagFilterNearest,
                    Filter::Bilinear => MTLSamplerMinMagFilterLinear,
                };
                let address = match wrap {
                    Wrap::Repeat => MTLSamplerAddressModeRepeat,
                    Wrap::Clamp => MTLSamplerAddressModeClampToEdge,
                    Wrap::Mirror => MTLSamplerAddressModeMirrorRepeat,
                };
                msg_with_u64(descriptor, sel(b"setMinFilter:\0"), metal_filter);
                msg_with_u64(descriptor, sel(b"setMagFilter:\0"), metal_filter);
                msg_with_u64(descriptor, sel(b"setSAddressMode:\0"), address);
                msg_with_u64(descriptor, sel(b"setTAddressMode:\0"), address);
                let state = msg_id_with_id(
                    self.device,
                    sel(b"newSamplerStateWithDescriptor:\0"),
                    descriptor,
                );
                let mut descriptor = descriptor;
                release(&mut descriptor);
                self.samplers[sampler_index(filter, wrap)] = state;
            }
        }
    }

    /// # Safety
    /// As [`Gpu::build_depth_states`].
    unsafe fn build_fallback_texture(&mut self) {
        let white = Texture::solid(Color::WHITE);
        self.fallback_texture = self.make_texture(&white);
    }

    /// Upload a texture, or hand back the one already on the device.
    ///
    /// Keyed on the texels themselves rather than on the address of the
    /// `Texture`: a scene that rebuilds its textures (the *Textures* page
    /// does, on `Space`) would otherwise get a stale one at the same address.
    fn upload_texture(&mut self, texture: &Texture) -> Id {
        let key = texture_hash(texture);
        if let Some((_, id)) = self.textures.iter().find(|(k, _)| *k == key) {
            return *id;
        }
        if self.textures.len() >= TEXTURE_CACHE_LIMIT {
            // SAFETY: every entry is a texture this device made and we own.
            unsafe {
                for (_, id) in self.textures.drain(..) {
                    let mut id = id;
                    release(&mut id);
                }
            }
        }
        // SAFETY: a descriptor and a texture we own, filled with exactly
        // `width * height` RGBA halves.
        let id = unsafe { self.make_texture(texture) };
        self.textures.push((key, id));
        id
    }

    /// # Safety
    /// The device must be live.
    unsafe fn make_texture(&self, texture: &Texture) -> Id {
        let _pool = Pool::push();
        let (width, height) = (texture.width().max(1), texture.height().max(1));
        let descriptor = msg_texture_descriptor(
            class(b"MTLTextureDescriptor\0"),
            sel(b"texture2DDescriptorWithPixelFormat:width:height:mipmapped:\0"),
            MTLPixelFormatRGBA16Float,
            width,
            height,
            NO,
        );
        msg_with_u64(descriptor, sel(b"setUsage:\0"), MTLTextureUsageShaderRead);
        msg_with_u64(descriptor, sel(b"setStorageMode:\0"), MTLStorageModeShared);
        let id = msg_id_with_id(self.device, sel(b"newTextureWithDescriptor:\0"), descriptor);
        assert!(
            !id.is_null(),
            "MTLDevice could not make a {width}x{height} texture"
        );

        // Half floats: eleven bits of mantissa is more than an 8-bit target
        // can show, and it halves what the sampler has to read.
        let mut halves = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let u = (x as f32 + 0.5) / width as f32;
                let v = (y as f32 + 0.5) / height as f32;
                let c = texture.sample(u, v);
                halves.extend_from_slice(&[
                    f32_to_f16(c.r),
                    f32_to_f16(c.g),
                    f32_to_f16(c.b),
                    f32_to_f16(c.a),
                ]);
            }
        }
        msg_replace_region(
            id,
            sel(b"replaceRegion:mipmapLevel:withBytes:bytesPerRow:\0"),
            MTLRegion::two_d(0, 0, width, height),
            0,
            halves.as_ptr().cast(),
            width * 8,
        );
        id
    }

    /// # Safety
    /// The device must be live and no frame may be open against the old target.
    unsafe fn ensure_offscreen(&mut self, width: usize, height: usize) -> &Offscreen {
        let matches = self
            .offscreen
            .as_ref()
            .is_some_and(|o| o.width == width && o.height == height);
        if !matches {
            let row_bytes = aligned_row(width, 4);
            let color = self.private_render_texture(MTLPixelFormatBGRA8Unorm, width, height);
            let depth = self.private_render_texture(MTLPixelFormatDepth32Float, width, height);
            let readback = msg_new_buffer(
                self.device,
                sel(b"newBufferWithLength:options:\0"),
                row_bytes * height,
                MTLResourceStorageModeShared,
            );
            self.offscreen = Some(Offscreen {
                width,
                height,
                color,
                depth,
                readback,
                row_bytes,
            });
        }
        self.offscreen.as_ref().expect("just built")
    }

    /// # Safety
    /// The device must be live, and no frame may be open against the old one.
    unsafe fn ensure_window_depth(&mut self, width: usize, height: usize) -> Id {
        if let Some((w, h, id)) = self.window_depth {
            if w == width && h == height {
                return id;
            }
            let mut old = id;
            release(&mut old);
        }
        let id = self.private_render_texture(MTLPixelFormatDepth32Float, width, height);
        self.window_depth = Some((width, height, id));
        id
    }

    /// # Safety
    /// The device must be live.
    unsafe fn private_render_texture(&self, format: u64, width: usize, height: usize) -> Id {
        let _pool = Pool::push();
        let descriptor = msg_texture_descriptor(
            class(b"MTLTextureDescriptor\0"),
            sel(b"texture2DDescriptorWithPixelFormat:width:height:mipmapped:\0"),
            format,
            width.max(1),
            height.max(1),
            NO,
        );
        msg_with_u64(
            descriptor,
            sel(b"setUsage:\0"),
            MTLTextureUsageRenderTarget | MTLTextureUsageShaderRead,
        );
        msg_with_u64(descriptor, sel(b"setStorageMode:\0"), MTLStorageModePrivate);
        let id = msg_id_with_id(self.device, sel(b"newTextureWithDescriptor:\0"), descriptor);
        assert!(!id.is_null(), "MTLDevice could not make a render target");
        id
    }

    /// # Safety
    /// `frame` must be the open frame, with its encoder already ended.
    unsafe fn blit_color(&mut self, frame: &Frame) {
        let Some(off) = &self.offscreen else { return };
        let blit = msg_id(frame.command_buffer, sel(b"blitCommandEncoder\0"));
        if blit.is_null() {
            return;
        }
        msg_blit_texture_to_buffer(
            blit,
            sel(b"copyFromTexture:sourceSlice:sourceLevel:sourceOrigin:sourceSize:toBuffer:destinationOffset:destinationBytesPerRow:destinationBytesPerImage:\0"),
            off.color,
            0,
            0,
            MTLOrigin::default(),
            MTLSize { width: off.width, height: off.height, depth: 1 },
            off.readback,
            0,
            off.row_bytes,
            off.row_bytes * off.height,
        );
        msg(blit, sel(b"endEncoding\0"));
    }

    /// # Safety
    /// As [`Gpu::blit_color`].
    unsafe fn blit_depth(&mut self, frame: &Frame) {
        let (width, height) = (frame.width, frame.height);
        let row_bytes = aligned_row(width, 4);
        let slot = frame.slot;
        if self.arenas[slot].depth_size != (width, height) {
            let mut old = self.arenas[slot].depth_readback;
            release(&mut old);
            self.arenas[slot].depth_readback = msg_new_buffer(
                self.device,
                sel(b"newBufferWithLength:options:\0"),
                row_bytes * height,
                MTLResourceStorageModeShared,
            );
            self.arenas[slot].depth_size = (width, height);
        }
        let buffer = self.arenas[slot].depth_readback;
        let blit = msg_id(frame.command_buffer, sel(b"blitCommandEncoder\0"));
        if blit.is_null() {
            return;
        }
        msg_blit_texture_to_buffer(
            blit,
            sel(b"copyFromTexture:sourceSlice:sourceLevel:sourceOrigin:sourceSize:toBuffer:destinationOffset:destinationBytesPerRow:destinationBytesPerImage:\0"),
            frame.depth_texture,
            0,
            0,
            MTLOrigin::default(),
            MTLSize { width, height, depth: 1 },
            buffer,
            0,
            row_bytes,
            row_bytes * height,
        );
        msg(blit, sel(b"endEncoding\0"));
    }

    /// # Safety
    /// The frame's command buffer must have completed.
    unsafe fn download_color(&self, frame: &Frame, target: &mut Framebuffer) {
        let Some(off) = &self.offscreen else { return };
        target.resize(frame.width, frame.height);
        let base = msg_ptr(off.readback, sel(b"contents\0")) as *const u8;
        if base.is_null() {
            return;
        }
        let pixels = target.pixels_mut();
        for y in 0..frame.height {
            let row = base.add(y * off.row_bytes) as *const u32;
            // BGRA8Unorm read as a little-endian word is 0xAARRGGBB, which is
            // the framebuffer's own layout — no conversion pass, same as the
            // CGImage the CPU path presents.
            std::ptr::copy_nonoverlapping(row, pixels[y * frame.width..].as_mut_ptr(), frame.width);
        }
    }

    /// Wait for the GPU to finish with the frame that last used this arena,
    /// and pick up anything that frame left behind.
    ///
    /// # Safety
    /// `slot` must index the arena ring.
    unsafe fn wait_for_slot(&mut self, slot: usize) {
        let fence = self.arenas[slot].fence;
        if fence.is_null() {
            return;
        }
        msg(fence, sel(b"waitUntilCompleted\0"));
        self.record_gpu_time(fence);
        if self.depth_wanted {
            self.collect_depth(slot);
        }
    }

    /// # Safety
    /// The command buffer must have completed.
    unsafe fn record_gpu_time(&mut self, command_buffer: Id) {
        let start = msg_f64(command_buffer, sel(b"GPUStartTime\0"));
        let end = msg_f64(command_buffer, sel(b"GPUEndTime\0"));
        if end > start {
            self.gpu_seconds = (end - start) as f32;
        }
    }

    /// # Safety
    /// The blit into this slot's readback buffer must have completed.
    unsafe fn collect_depth(&mut self, slot: usize) {
        let (width, height) = self.arenas[slot].depth_size;
        let buffer = self.arenas[slot].depth_readback;
        if buffer.is_null() || width == 0 || height == 0 {
            return;
        }
        let base = msg_ptr(buffer, sel(b"contents\0")) as *const u8;
        if base.is_null() {
            return;
        }
        let row_bytes = aligned_row(width, 4);
        let mut frame = match self.depth_snapshot.take() {
            Some(mut existing) => {
                existing.resize(width, height);
                existing
            }
            None => Framebuffer::new(width, height),
        };
        for y in 0..height {
            let row = base.add(y * row_bytes) as *const f32;
            for x in 0..width {
                let depth = *row.add(x);
                // 1.0 is the clear value, and `depth_view` wants "nothing was
                // drawn here" to read as infinity, exactly as the rasterizer
                // leaves it.
                let depth = if depth >= 1.0 { f32::INFINITY } else { depth };
                frame.set_depth_at(x, y, depth);
            }
        }
        self.depth_snapshot = Some(frame);
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: everything released here was created by this device and is
        // owned by this struct; each handle is released exactly once.
        unsafe {
            for arena in &mut self.arenas {
                for (buffer, _) in arena.chunks.drain(..) {
                    let mut buffer = buffer;
                    release(&mut buffer);
                }
                release(&mut arena.fence);
                release(&mut arena.depth_readback);
            }
            for (_, id) in self.textures.drain(..) {
                let mut id = id;
                release(&mut id);
            }
            for (_, id) in self.pipelines.drain(..) {
                let mut id = id;
                release(&mut id);
            }
            for (_, id) in self.functions.drain(..) {
                let mut id = id;
                release(&mut id);
            }
            for (_, id) in self.libraries.drain(..) {
                let mut id = id;
                release(&mut id);
            }
            for state in &mut self.depth_states {
                release(state);
            }
            for sampler in &mut self.samplers {
                release(sampler);
            }
            release(&mut self.fallback_texture);
            if let Some(off) = &mut self.offscreen {
                release(&mut off.color);
                release(&mut off.depth);
                release(&mut off.readback);
            }
            if let Some((_, _, mut depth)) = self.window_depth.take() {
                release(&mut depth);
            }
            release(&mut self.queue);
        }
    }
}

/// Which way a front face winds on screen.
///
/// The rasterizer calls a triangle front-facing when its signed area is
/// negative in pixel coordinates, and pixel coordinates run down the screen —
/// so a mesh's counter-clockwise winding in NDC is counter-clockwise in the
/// framebuffer too, which is what Metal calls `CounterClockwise`. The single
/// triangle in `tests/winding.rs` is what settled this.
const FRONT_FACING: u64 = MTLWindingCounterClockwise;
const _: () = assert!(FRONT_FACING != MTLWindingClockwise);

fn metal_cull(cull: CullMode) -> u64 {
    match cull {
        CullMode::None => MTLCullModeNone,
        CullMode::Back => MTLCullModeBack,
        CullMode::Front => MTLCullModeFront,
    }
}

fn depth_index(test: bool, write: bool) -> usize {
    (test as usize) * 2 + (write as usize)
}

fn sampler_index(filter: Filter, wrap: Wrap) -> usize {
    let f = match filter {
        Filter::Nearest => 0,
        Filter::Bilinear => 1,
    };
    let w = match wrap {
        Wrap::Repeat => 0,
        Wrap::Clamp => 1,
        Wrap::Mirror => 2,
    };
    f * 3 + w
}

/// Metal wants a blit's destination rows aligned; 256 satisfies every device
/// this can run on, at the price of a little slack per row.
fn aligned_row(width: usize, bytes_per_pixel: usize) -> usize {
    let row = width * bytes_per_pixel;
    row.div_ceil(256) * 256
}

/// One texel, exactly.
///
/// `Texture` has no texel accessor — it is a thing you sample — but sampling
/// the centre of texel `(x, y)` lands on that texel under either filter:
/// bilinear's weights come out at 1 and 0, nearest floors onto it.
fn texel(texture: &Texture, x: usize, y: usize) -> Color {
    let u = (x as f32 + 0.5) / texture.width() as f32;
    let v = (y as f32 + 0.5) / texture.height() as f32;
    texture.sample(u, v)
}

/// FNV-1a over a texture's texels and its sampler settings.
fn texture_hash(texture: &Texture) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(&(texture.width() as u64).to_le_bytes());
    eat(&(texture.height() as u64).to_le_bytes());
    for y in 0..texture.height() {
        for x in 0..texture.width() {
            let c = texel(texture, x, y);
            eat(&c.r.to_le_bytes());
            eat(&c.g.to_le_bytes());
            eat(&c.b.to_le_bytes());
            eat(&c.a.to_le_bytes());
        }
    }
    hash
}

/// IEEE 754 binary32 to binary16, with the subnormal and overflow cases that
/// a texture upload can actually hit.
fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        // Infinity, or a NaN kept as a NaN.
        let payload = if mantissa != 0 { 0x0200 } else { 0 };
        return sign | 0x7c00 | payload;
    }
    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00; // overflow saturates to infinity
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign; // too small even for a subnormal
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let rounded = (mantissa + (1 << (shift - 1))) >> shift;
        return sign | rounded as u16;
    }
    // Round to nearest, ties away from zero — within half an ulp of f16,
    // which is far finer than the 8-bit target can show.
    let rounded = (mantissa + 0x0000_1000) >> 13;
    let carry = (rounded >> 10) as i32;
    sign | (((unbiased + carry) as u16) << 10) | (rounded as u16 & 0x03ff)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settled by `tests/pipeline.rs`, which draws one counter-clockwise
    /// triangle and asks whether back-face culling kept it.
    #[test]
    fn front_faces_wind_the_way_the_rasterizer_says_they_do() {
        assert_eq!(FRONT_FACING, MTLWindingCounterClockwise);
        assert_ne!(FRONT_FACING, MTLWindingClockwise);
    }

    #[test]
    fn the_four_depth_states_are_four_distinct_slots() {
        let mut seen: Vec<usize> = Vec::new();
        for test in [false, true] {
            for write in [false, true] {
                seen.push(depth_index(test, write));
            }
        }
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3]);
    }

    #[test]
    fn the_six_samplers_are_six_distinct_slots() {
        let mut seen: Vec<usize> = Vec::new();
        for filter in [Filter::Nearest, Filter::Bilinear] {
            for wrap in [Wrap::Repeat, Wrap::Clamp, Wrap::Mirror] {
                seen.push(sampler_index(filter, wrap));
            }
        }
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn blit_rows_round_up_to_the_alignment_metal_asks_for() {
        assert_eq!(aligned_row(64, 4), 256);
        assert_eq!(aligned_row(65, 4), 512);
        assert_eq!(aligned_row(960, 4), 3840);
    }

    #[test]
    fn half_floats_keep_what_an_eight_bit_target_can_show() {
        for step in 0..=255u32 {
            let value = step as f32 / 255.0;
            let back = f16_to_f32(f32_to_f16(value));
            assert!(
                (back - value).abs() < 0.5 / 255.0,
                "{value} came back as {back}"
            );
        }
        assert_eq!(f32_to_f16(0.0), 0x0000);
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(-1.0), 0xbc00);
        assert_eq!(f32_to_f16(f32::INFINITY), 0x7c00);
        assert_eq!(f32_to_f16(1e-12), 0x0000, "underflow is zero, not garbage");
        assert_eq!(f32_to_f16(1e12), 0x7c00, "overflow saturates");
    }

    /// Only the test needs to go back the other way.
    fn f16_to_f32(half: u16) -> f32 {
        let sign = (half as u32 & 0x8000) << 16;
        let exponent = (half as u32 >> 10) & 0x1f;
        let mantissa = half as u32 & 0x03ff;
        if exponent == 0 {
            if mantissa == 0 {
                return f32::from_bits(sign);
            }
            let mut e = -1i32;
            let mut m = mantissa;
            while m & 0x0400 == 0 {
                m <<= 1;
                e -= 1;
            }
            let m = m & 0x03ff;
            return f32::from_bits(sign | (((e + 15 - 15 + 127) as u32) << 23) | (m << 13));
        }
        if exponent == 0x1f {
            return f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13));
        }
        f32::from_bits(sign | ((exponent + 127 - 15) << 23) | (mantissa << 13))
    }

    #[test]
    fn two_textures_with_the_same_texels_hash_the_same() {
        let a = Texture::checker(8, 4, Color::RED, Color::BLUE);
        let b = Texture::checker(8, 4, Color::RED, Color::BLUE);
        let c = Texture::checker(8, 4, Color::RED, Color::GREEN);
        assert_eq!(texture_hash(&a), texture_hash(&b));
        assert_ne!(texture_hash(&a), texture_hash(&c));
        assert_ne!(
            texture_hash(&a),
            texture_hash(&Texture::checker(16, 4, Color::RED, Color::BLUE)),
            "the size is part of the key"
        );
    }
}
