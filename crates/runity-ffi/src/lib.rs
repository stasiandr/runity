//! The editor's side of the engine, as a C ABI.
//!
//! A native editor — Swift on macOS today — owns its window and its event
//! loop, and reaches the engine through these calls. The boundary is
//! deliberately narrow and deliberately dull: ids, floats and paths. Nothing
//! here mirrors an engine type, because the moment the editor starts
//! declaring its own copy of a component, every new component becomes work
//! in two languages.
//!
//! Two planes, as `docs/stack.md` describes. The control plane is scene and
//! selection: opening, listing, moving, saving — called when a person does
//! something, so it can afford to copy a string. The data plane is the
//! frame: render, resize, input — called sixty times a second, so it
//! allocates nothing.
//!
//! Every call is safe to make on a null handle. An editor's UI outlives its
//! document, and a boundary that segfaults when a panel repaints after a
//! close is a boundary that gets wrapped in defensive code on the other
//! side.

use std::ffi::{c_char, c_float, c_int, c_uint, CStr, CString};
use std::path::PathBuf;
use std::ptr;

use runity::gizmo::{self, Drag, GizmoStyle, Handle};
use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, FogSettings, Frame, Lighting, MeshHandle};
use runity::{builtin, Gpu, Library, OffscreenTarget, Renderer, Scene};

/// Everything one open document needs.
pub struct Editor {
    gpu: Gpu,
    renderer: Renderer,
    target: OffscreenTarget,
    /// Set when a host gave us its layer; absent when rendering offscreen.
    surface: Option<runity::surface::Surface>,
    world: hecs::World,
    history: runity::edit::History,
    scene_path: Option<PathBuf>,
    library: Option<Library>,
    uploaded: Vec<(String, MeshHandle)>,
    camera: Camera,
    /// Entity handles in the order the editor lists them, so an index from
    /// the UI means the same thing on both sides for as long as the document
    /// is open.
    order: Vec<hecs::Entity>,
    pixels: Vec<u8>,
    /// Which entity the gizmo is on, by flattened index.
    selected: Option<usize>,
    gizmo_style: GizmoStyle,
    drag: Option<Drag>,
    /// A unit cube, uploaded once, that the gizmo's three arms are made of.
    gizmo_arm: Option<MeshHandle>,
}

thread_local! {
    /// The last failure, for [`runity_last_error`]. Thread-local because an
    /// editor may drive several documents from several threads, and one
    /// global would let one document's error surface under another's.
    static LAST_ERROR: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn fail(message: impl Into<String>) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = message.into());
}

/// Copy a string into a caller's buffer, NUL-terminated and truncated to fit.
///
/// Returns the length it wanted, so a caller can size a buffer and ask
/// again — the usual C shape, and the one that does not require the caller
/// to trust a length it was not told.
fn write_string(value: &str, buffer: *mut c_char, capacity: c_uint) -> c_uint {
    let needed = value.len() as c_uint;
    if buffer.is_null() || capacity == 0 {
        return needed;
    }
    let Ok(text) = CString::new(value) else {
        // A NUL inside the string: report nothing rather than a truncation
        // the caller would read as the whole value.
        unsafe { *buffer = 0 };
        return 0;
    };
    let bytes = text.as_bytes_with_nul();
    let room = (capacity as usize).min(bytes.len());
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buffer, room);
        *buffer.add(room - 1) = 0;
    }
    needed
}

unsafe fn borrow<'a>(editor: *mut Editor) -> Option<&'a mut Editor> {
    unsafe { editor.as_mut() }
}

unsafe fn path_from(raw: *const c_char) -> Option<PathBuf> {
    if raw.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(raw) }
        .to_str()
        .ok()
        .map(PathBuf::from)
}

/// Open an editor that renders into its own image rather than a window.
///
/// Marked unsafe for symmetry with the rest: every call in this module takes
/// a handle it dereferences, and a module where some are unsafe and some are
/// not invites a reader to think the difference is meaningful.
///
/// This is the whole API minus the surface, which means the editor's logic
/// can be exercised — by tests here, and by a host before it has a view to
/// give. Returns null on failure; [`runity_last_error`] says why.
///
/// # Safety
/// The returned pointer is owned by the caller and must be released with
/// [`runity_editor_free`].
/// # Safety
/// The returned pointer is owned by the caller and must be released with
/// [`runity_editor_free`].
#[no_mangle]
pub unsafe extern "C" fn runity_editor_create_offscreen(
    width: c_uint,
    height: c_uint,
) -> *mut Editor {
    let gpu = match Gpu::headless_blocking(false) {
        Ok(gpu) => gpu,
        Err(e) => {
            fail(e.to_string());
            return ptr::null_mut();
        }
    };
    let target = OffscreenTarget::new(&gpu, width.max(1), height.max(1));
    let renderer = Renderer::new(&gpu, &target);
    Box::into_raw(Box::new(Editor {
        gpu,
        renderer,
        target,
        surface: None,
        world: hecs::World::new(),
        history: runity::edit::History::new(Scene::default(), 64),
        scene_path: None,
        library: None,
        uploaded: Vec::new(),
        camera: Camera::default(),
        order: Vec::new(),
        pixels: Vec::new(),
        selected: None,
        gizmo_style: GizmoStyle::default(),
        drag: None,
        gizmo_arm: None,
    }))
}

/// Open an editor that draws into a layer the host already owns.
///
/// On macOS `layer` is a `CAMetalLayer*` — the layer of the view the editor
/// put on screen. The engine never makes a window; the host does, and hands
/// its surface over. Returns null on failure, and on a platform without this
/// path.
///
/// # Safety
/// `layer` must be a live `CAMetalLayer` that outlives the editor. Releasing
/// the view while the editor still holds a swapchain is a use after free
/// that nothing here can detect.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_create_for_layer(
    layer: *mut core::ffi::c_void,
    width: c_uint,
    height: c_uint,
) -> *mut Editor {
    #[cfg(target_os = "macos")]
    {
        let gpu = match Gpu::headless_blocking(false) {
            Ok(gpu) => gpu,
            Err(e) => {
                fail(e.to_string());
                return ptr::null_mut();
            }
        };
        let surface = match unsafe {
            runity::surface::Surface::from_metal_layer(&gpu, layer, width.max(1), height.max(1))
        } {
            Ok(surface) => surface,
            Err(e) => {
                fail(e.to_string());
                return ptr::null_mut();
            }
        };
        let renderer = Renderer::for_surface(&gpu, &surface);
        // The offscreen target stays, small, for `runity_editor_frame_pixels`
        // — a thumbnail, a test, or a host that wants the image before it has
        // a view.
        let target = OffscreenTarget::new(&gpu, 1, 1);
        return Box::into_raw(Box::new(Editor {
            gpu,
            renderer,
            target,
            surface: Some(surface),
            world: hecs::World::new(),
            history: runity::edit::History::new(Scene::default(), 64),
            scene_path: None,
            library: None,
            uploaded: Vec::new(),
            camera: Camera::default(),
            order: Vec::new(),
            pixels: Vec::new(),
            selected: None,
            gizmo_style: GizmoStyle::default(),
            drag: None,
            gizmo_arm: None,
        }));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (layer, width, height);
        fail("no native layer surface on this platform");
        ptr::null_mut()
    }
}

/// Tell the editor its view changed size.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_resize(
    editor: *mut Editor,
    width: c_uint,
    height: c_uint,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    match editor.surface.as_mut() {
        Some(surface) => {
            surface.resize(&editor.gpu, width, height);
            true
        }
        None => {
            // Offscreen: a new image, because an offscreen target cannot be
            // resized in place and pretending otherwise would silently keep
            // rendering at the old size.
            editor.target = OffscreenTarget::new(&editor.gpu, width.max(1), height.max(1));
            true
        }
    }
}

/// Release an editor. Null is a no-op.
///
/// # Safety
/// `editor` must have come from a create call and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_free(editor: *mut Editor) {
    if !editor.is_null() {
        drop(unsafe { Box::from_raw(editor) });
    }
}

/// The last failure on this thread, into a caller's buffer. Returns the
/// length the message wanted.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_last_error(buffer: *mut c_char, capacity: c_uint) -> c_uint {
    LAST_ERROR.with(|slot| write_string(&slot.borrow(), buffer, capacity))
}

/// Point the editor at a library of imported assets.
///
/// # Safety
/// `path` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_library(
    editor: *mut Editor,
    path: *const c_char,
) -> bool {
    let (Some(editor), Some(path)) = (unsafe { borrow(editor) }, unsafe { path_from(path) }) else {
        fail("no editor or no path");
        return false;
    };
    match Library::open(&path) {
        Ok((library, problems)) => {
            if !problems.is_empty() {
                fail(format!("{} asset(s) skipped", problems.len()));
            }
            editor.library = Some(library);
            // Handles from the old library refer to meshes uploaded for it.
            editor.uploaded.clear();
            true
        }
        Err(e) => {
            fail(e.to_string());
            false
        }
    }
}

/// Open a scene file.
///
/// # Safety
/// `path` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_open_scene(
    editor: *mut Editor,
    path: *const c_char,
) -> bool {
    let (Some(editor), Some(path)) = (unsafe { borrow(editor) }, unsafe { path_from(path) }) else {
        fail("no editor or no path");
        return false;
    };
    match Scene::load(&path) {
        Ok(scene) => {
            editor.history.replace(scene);
            editor.scene_path = Some(path);
            editor.respawn();
            true
        }
        Err(e) => {
            fail(e.to_string());
            false
        }
    }
}

/// Write the scene back. With a null path, writes where it was opened from.
///
/// # Safety
/// `path` must be a NUL-terminated string, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_save_scene(
    editor: *mut Editor,
    path: *const c_char,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        fail("no editor");
        return false;
    };
    let target = unsafe { path_from(path) }.or_else(|| editor.scene_path.clone());
    let Some(target) = target else {
        fail("no path to save to");
        return false;
    };
    match editor.history.scene().save(&target) {
        Ok(()) => true,
        Err(e) => {
            fail(e.to_string());
            false
        }
    }
}

/// How many entities the open scene has, counting children.
/// # Safety
/// `editor` must be null or a handle from a create call that has not been
/// freed.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_entity_count(editor: *mut Editor) -> c_uint {
    unsafe { borrow(editor) }.map_or(0, |e| e.flat_count() as c_uint)
}

/// One entity's name.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_entity_name(
    editor: *mut Editor,
    index: c_uint,
    buffer: *mut c_char,
    capacity: c_uint,
) -> c_uint {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return 0;
    };
    let flat = editor.history.scene().flatten();
    let Some((desc, _)) = flat.get(index as usize) else {
        return 0;
    };
    let name = desc.name.clone();
    write_string(&name, buffer, capacity)
}

/// An entity's local transform, as nine floats: position, Euler degrees,
/// scale.
///
/// # Safety
/// `out` must be writable for nine floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_get_transform(
    editor: *mut Editor,
    index: c_uint,
    out: *mut c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if out.is_null() {
        return false;
    }
    let flat = editor.history.scene().flatten();
    let Some((desc, _)) = flat.get(index as usize) else {
        return false;
    };
    let t = desc.transform;
    let values = [
        t.position.x,
        t.position.y,
        t.position.z,
        t.rotation_deg.x,
        t.rotation_deg.y,
        t.rotation_deg.z,
        t.scale.x,
        t.scale.y,
        t.scale.z,
    ];
    unsafe { ptr::copy_nonoverlapping(values.as_ptr(), out, values.len()) };
    true
}

/// Set an entity's local transform from nine floats.
///
/// # Safety
/// `values` must be readable for nine floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_transform(
    editor: *mut Editor,
    index: c_uint,
    values: *const c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if values.is_null() {
        return false;
    }
    let v = unsafe { std::slice::from_raw_parts(values, 9) };
    // Recorded: a value typed into an inspector is one undoable edit. A
    // drag is not, because `gizmo_begin` already took the snapshot that
    // covers the whole gesture.
    let scene = editor.history.edit();
    let Some(desc) = runity::edit::nth_mut(scene, index as usize) else {
        return false;
    };
    desc.transform.position = Vec3::new(v[0], v[1], v[2]);
    desc.transform.rotation_deg = Vec3::new(v[3], v[4], v[5]);
    desc.transform.scale = Vec3::new(v[6], v[7], v[8]);
    // Respawned rather than patched in place: a moved parent moves its
    // children, and keeping two ways to apply that is how they drift.
    editor.respawn();
    true
}

/// Move the camera: eye and target, three floats each.
///
/// # Safety
/// `eye` and `target` must each be readable for three floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_camera(
    editor: *mut Editor,
    eye: *const c_float,
    target: *const c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if eye.is_null() || target.is_null() {
        return false;
    }
    let e = unsafe { std::slice::from_raw_parts(eye, 3) };
    let t = unsafe { std::slice::from_raw_parts(target, 3) };
    editor.camera.position = Vec3::new(e[0], e[1], e[2]);
    editor.camera.target = Vec3::new(t[0], t[1], t[2]);
    true
}

/// Draw one frame into the editor's image.
/// # Safety
/// `editor` must be null or a handle from a create call that has not been
/// freed.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_render(editor: *mut Editor) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let frame = Frame {
        camera: editor.camera,
        lighting: Lighting::default(),
        fog: FogSettings {
            color: Vec3::from_array(editor.history.scene().fog.color),
            start: editor.history.scene().fog.start,
            end: editor.history.scene().fog.end,
        },
        clear_color: Vec3::from_array(editor.history.scene().fog.color),
        ..runity::build_frame(
            &editor.world,
            editor.camera,
            Lighting::default(),
            FogSettings::default(),
        )
    };
    let mut frame = frame;
    // The gizmo goes in after the scene's own draws and before the frame is
    // submitted, so it is part of the same pass and does not need a second
    // one. It is unlit and drawn last, which is what keeps a handle visible
    // against anything.
    if let (Some(origin), true) = (editor.selected_origin(), editor.selected.is_some()) {
        let arm = match editor.gizmo_arm {
            Some(arm) => arm,
            None => {
                let cube = builtin::cube(1.0);
                let arm = editor.renderer.upload_mesh_owned(&editor.gpu, &cube);
                editor.gizmo_arm = Some(arm);
                arm
            }
        };
        frame.overlay_draws.extend(runity::gizmo::draws(
            arm,
            &editor.camera,
            &editor.gizmo_style,
            origin,
            editor.drag.map(|d| d.handle),
        ));
    }
    match editor.surface.as_ref() {
        Some(surface) => match surface.begin_frame() {
            Ok(acquired) => {
                editor
                    .renderer
                    .render_to_frame(&editor.gpu, &acquired, &frame);
                acquired.present(&editor.gpu);
            }
            Err(runity::SurfaceError::Outdated) => {
                // The view is mid-resize. Reconfiguring and skipping this
                // frame is what every host does, and reporting it as a
                // failure would make an editor log a line per drag.
                surface.reconfigure(&editor.gpu);
            }
            Err(e) => {
                fail(e.to_string());
                return false;
            }
        },
        None => {
            editor.renderer.render(&editor.gpu, &editor.target, &frame);
            editor.pixels = editor.target.read_rgba(&editor.gpu);
        }
    }
    true
}

/// The last rendered frame as RGBA8, copied into a caller's buffer.
///
/// Returns the number of bytes the frame needs. A host with a real surface
/// never calls this; it exists so an editor can show something before it has
/// a view, and so tests can look at what the engine drew.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_frame_pixels(
    editor: *mut Editor,
    buffer: *mut u8,
    capacity: c_uint,
) -> c_uint {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return 0;
    };
    let needed = editor.pixels.len() as c_uint;
    if buffer.is_null() || capacity == 0 {
        return needed;
    }
    let room = (capacity as usize).min(editor.pixels.len());
    unsafe { ptr::copy_nonoverlapping(editor.pixels.as_ptr(), buffer, room) };
    needed
}

/// Which entity is under a point in the image, or -1 for none.
///
/// Uses the entities' bounding boxes against a ray through the pixel. A
/// triangle-exact pick is better and much slower, and for a box the
/// difference only shows on thin diagonal geometry.
/// # Safety
/// `editor` must be null or a handle from a create call that has not been
/// freed.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_pick(editor: *mut Editor, x: c_uint, y: c_uint) -> c_int {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return -1;
    };
    editor.pick(x, y).map_or(-1, |i| i as c_int)
}

/// The width and height of the editor's image.
/// # Safety
/// `editor` must be null or a handle from a create call that has not been
/// freed.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_width(editor: *mut Editor) -> c_uint {
    unsafe { borrow(editor) }.map_or(0, |e| e.view_size().0)
}

/// See [`runity_editor_width`].
/// # Safety
/// `editor` must be null or a handle from a create call that has not been
/// freed.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_height(editor: *mut Editor) -> c_uint {
    unsafe { borrow(editor) }.map_or(0, |e| e.view_size().1)
}

/// Put the gizmo on an entity, or pass -1 to clear the selection.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_select(editor: *mut Editor, index: c_int) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if index < 0 {
        editor.selected = None;
        editor.drag = None;
        return true;
    }
    let index = index as usize;
    if index >= editor.flat_count() {
        return false;
    }
    editor.selected = Some(index);
    true
}

/// Which entity the gizmo is on, or -1.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_selected(editor: *mut Editor) -> c_int {
    unsafe { borrow(editor) }.map_or(-1, |e| e.selected.map_or(-1, |i| i as c_int))
}

/// Which gizmo arm is under a point: 0 for X, 1 for Y, 2 for Z, -1 for none.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_gizmo_hover(
    editor: *mut Editor,
    x: c_uint,
    y: c_uint,
) -> c_int {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return -1;
    };
    let Some(origin) = editor.selected_origin() else {
        return -1;
    };
    let (from, direction) = editor.ray(x, y);
    gizmo::hit(&editor.camera, &editor.gizmo_style, origin, from, direction)
        .map_or(-1, handle_index)
}

/// Grab whatever arm is under a point. Returns the arm, or -1 if none is.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_gizmo_begin(
    editor: *mut Editor,
    x: c_uint,
    y: c_uint,
) -> c_int {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return -1;
    };
    let Some(origin) = editor.selected_origin() else {
        return -1;
    };
    let (from, direction) = editor.ray(x, y);
    let Some(handle) = gizmo::hit(&editor.camera, &editor.gizmo_style, origin, from, direction)
    else {
        return -1;
    };
    // One snapshot for the whole gesture: everything until the next one
    // undoes as a single step, however many frames the drag lasts.
    editor.history.snapshot();
    editor.drag = Some(gizmo::begin(origin, handle, from, direction));
    handle_index(handle)
}

/// Move the held arm to follow a point. Does nothing without a grab.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_gizmo_drag(
    editor: *mut Editor,
    x: c_uint,
    y: c_uint,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let (Some(drag), Some(index)) = (editor.drag, editor.selected) else {
        return false;
    };
    let (from, direction) = editor.ray(x, y);
    let moved = gizmo::update(&drag, from, direction);

    // The gizmo sits at the entity's world position, but what is edited is
    // its local one. The difference is the parent's transform, and applying
    // the move in world space without undoing it drags a child out of its
    // parent by however much the parent is offset.
    let parent = editor.parent_matrix(index);
    // Untracked: the snapshot for this gesture was taken at `gizmo_begin`.
    let Some(desc) = runity::edit::nth_mut(editor.history.scene_mut_untracked(), index) else {
        return false;
    };
    let local = parent.inverse().transform_point3(moved);
    desc.transform.position = local;
    editor.respawn();
    true
}

/// Let go. Safe to call without a grab.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_gizmo_end(editor: *mut Editor) {
    if let Some(editor) = unsafe { borrow(editor) } {
        editor.drag = None;
    }
}

/// Add an entity with a model, under `parent` or at the top with -1.
/// Returns its index, or -1.
///
/// # Safety
/// `editor` must be null or a live handle; `model` a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_add(
    editor: *mut Editor,
    parent: c_int,
    model: *const c_char,
) -> c_int {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return -1;
    };
    let Some(model) = (unsafe { path_from(model) }) else {
        return -1;
    };
    let desc = runity::EntityDesc {
        name: "entity".into(),
        model: model.to_string_lossy().into_owned(),
        transform: Default::default(),
        material: Default::default(),
        body: Default::default(),
        collider: Default::default(),
        children: Vec::new(),
    };
    let parent = (parent >= 0).then_some(parent as usize);
    let scene = editor.history.edit();
    let Some(index) = runity::edit::add(scene, parent, desc) else {
        return -1;
    };
    editor.respawn();
    index as c_int
}

/// Read the material an entity is drawn with: r, g, b in **linear** space,
/// then 1 or 0 for unlit.
///
/// Resolved rather than raw: an entity naming `stone` reports the colour
/// `stone` actually is, so an inspector's swatch shows what is on screen
/// rather than the word. [`runity_editor_material_name`] is how the host
/// tells the two apart.
///
/// Linear because that is what the engine holds and what a lossless round
/// trip needs. A picker working in sRGB converts with
/// [`runity_linear_to_srgb`] rather than carrying its own formula.
///
/// # Safety
/// `out_four` must be writable for four floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_get_material(
    editor: *mut Editor,
    index: c_uint,
    out_four: *mut c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if out_four.is_null() {
        return false;
    }
    let flat = editor.history.scene().flatten();
    let Some((desc, _)) = flat.get(index as usize) else {
        return false;
    };
    let material = editor.resolve_material(desc);
    let values = [
        material.base_color[0],
        material.base_color[1],
        material.base_color[2],
        (material.shading == runity::Shading::Unlit) as u8 as f32,
    ];
    unsafe { ptr::copy_nonoverlapping(values.as_ptr(), out_four, values.len()) };
    true
}

/// Give an entity a colour of its own: r, g, b linear, then unlit as 1 or 0.
///
/// This breaks any link to a named material, which is what dragging a slider
/// on one object means. Pointing it back at the palette is
/// [`runity_editor_set_material_name`] — a separate call, because the
/// difference between "this rock is a bit greener" and "this rock is moss"
/// is a difference the scene file has to keep.
///
/// # Safety
/// `four` must be readable for four floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_material(
    editor: *mut Editor,
    index: c_uint,
    four: *const c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if four.is_null() {
        return false;
    }
    let v = unsafe { std::slice::from_raw_parts(four, 4) };
    let material = runity::Material {
        base_color: [v[0], v[1], v[2]],
        shading: if v[3] != 0.0 {
            runity::Shading::Unlit
        } else {
            runity::Shading::Lit
        },
    };
    // One undoable step, like a value typed into an inspector. A drag along
    // a colour slider that wants to be one step takes its own snapshot the
    // way a gizmo drag does.
    let scene = editor.history.edit();
    let Some(desc) = runity::edit::nth_mut(scene, index as usize) else {
        return false;
    };
    desc.material = runity::scene::MaterialRef::Inline(material);
    editor.respawn();
    true
}

/// The name of the material an entity points at, or nothing when it carries
/// its own colour.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_material_name(
    editor: *mut Editor,
    index: c_uint,
    buffer: *mut c_char,
    capacity: c_uint,
) -> c_uint {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return 0;
    };
    let flat = editor.history.scene().flatten();
    let name = match flat.get(index as usize).map(|(desc, _)| &desc.material) {
        Some(runity::scene::MaterialRef::Named(name)) => name.clone(),
        _ => String::new(),
    };
    write_string(&name, buffer, capacity)
}

/// Point an entity at a material by name — a `.rmat` in the library, or a
/// builtin. An empty name is refused: clearing the link means giving the
/// entity a colour, which is [`runity_editor_set_material`].
///
/// An unknown name is accepted, and draws grey. Refusing it would mean an
/// editor could not name a material before importing it, and the scene
/// format already treats a name nothing answers to as a visible mistake
/// rather than a failure.
///
/// # Safety
/// `name` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_material_name(
    editor: *mut Editor,
    index: c_uint,
    name: *const c_char,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if name.is_null() {
        return false;
    }
    let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else {
        fail("material name is not utf-8");
        return false;
    };
    if name.is_empty() {
        fail("a material name cannot be empty");
        return false;
    }
    let name = name.to_string();
    let scene = editor.history.edit();
    let Some(desc) = runity::edit::nth_mut(scene, index as usize) else {
        return false;
    };
    desc.material = runity::scene::MaterialRef::Named(name);
    editor.respawn();
    true
}

/// How many materials the editor can offer: the library's, then the
/// builtins the library does not shadow.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_palette_count(editor: *mut Editor) -> c_uint {
    unsafe { borrow(editor) }.map_or(0, |e| e.palette().len() as c_uint)
}

/// The name of one entry in the palette.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_palette_name(
    editor: *mut Editor,
    index: c_uint,
    buffer: *mut c_char,
    capacity: c_uint,
) -> c_uint {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return 0;
    };
    let palette = editor.palette();
    let Some((name, _)) = palette.get(index as usize) else {
        return 0;
    };
    write_string(name, buffer, capacity)
}

/// One palette entry's colour, in the same four floats as
/// [`runity_editor_get_material`] — for drawing the swatch beside the name.
///
/// # Safety
/// `out_four` must be writable for four floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_palette_color(
    editor: *mut Editor,
    index: c_uint,
    out_four: *mut c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if out_four.is_null() {
        return false;
    }
    let palette = editor.palette();
    let Some((_, material)) = palette.get(index as usize) else {
        return false;
    };
    let values = [
        material.base_color[0],
        material.base_color[1],
        material.base_color[2],
        (material.shading == runity::Shading::Unlit) as u8 as f32,
    ];
    unsafe { ptr::copy_nonoverlapping(values.as_ptr(), out_four, values.len()) };
    true
}

/// One channel from sRGB to linear.
///
/// Here so that a host with an sRGB colour picker does not carry its own
/// copy of the curve. The usual home-made version is `powf(2.2)`, which is
/// close enough to look right and wrong enough that a colour picked in the
/// editor is not the colour the engine draws.
#[no_mangle]
pub extern "C" fn runity_srgb_to_linear(channel: c_float) -> c_float {
    runity::material::srgb_to_linear(channel)
}

/// One channel from linear back to sRGB, for showing a colour in a picker.
#[no_mangle]
pub extern "C" fn runity_linear_to_srgb(channel: c_float) -> c_float {
    runity::material::linear_to_srgb(channel)
}

/// Delete an entity and everything under it.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_delete(editor: *mut Editor, index: c_uint) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let removed = runity::edit::remove(editor.history.edit(), index as usize).is_some();
    if removed {
        // The selection is an index into a list that just changed shape.
        // Keeping it would point the gizmo at whatever slid into the gap.
        editor.selected = None;
        editor.drag = None;
        editor.respawn();
    }
    removed
}

/// Copy an entity beside itself. Returns the copy's index, or -1.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_duplicate(editor: *mut Editor, index: c_uint) -> c_int {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return -1;
    };
    let Some(copy) = runity::edit::duplicate(editor.history.edit(), index as usize) else {
        return -1;
    };
    editor.respawn();
    copy as c_int
}

/// Move an entity under another, or to the top with -1.
///
/// Refuses to make something its own ancestor.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_reparent(
    editor: *mut Editor,
    index: c_uint,
    new_parent: c_int,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let parent = (new_parent >= 0).then_some(new_parent as usize);
    let moved = runity::edit::reparent(editor.history.edit(), index as usize, parent);
    if moved {
        editor.selected = None;
        editor.respawn();
    }
    moved
}

/// Step back. Returns false when there is nothing to undo.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_undo(editor: *mut Editor) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let stepped = editor.history.undo();
    if stepped {
        editor.selected = None;
        editor.drag = None;
        editor.respawn();
    }
    stepped
}

/// Step forward again.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_redo(editor: *mut Editor) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let stepped = editor.history.redo();
    if stepped {
        editor.selected = None;
        editor.drag = None;
        editor.respawn();
    }
    stepped
}

/// Whether there is anything to undo, for greying out a menu item.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_can_undo(editor: *mut Editor) -> bool {
    unsafe { borrow(editor) }.is_some_and(|e| e.history.can_undo())
}

/// See [`runity_editor_can_undo`].
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_can_redo(editor: *mut Editor) -> bool {
    unsafe { borrow(editor) }.is_some_and(|e| e.history.can_redo())
}

fn handle_index(handle: Handle) -> c_int {
    match handle {
        Handle::X => 0,
        Handle::Y => 1,
        Handle::Z => 2,
    }
}

impl Editor {
    fn flat_count(&self) -> usize {
        self.history.scene().flatten().len()
    }

    /// Rebuild the world from the scene.
    fn respawn(&mut self) {
        self.world.clear();
        self.order.clear();
        let renderer = &mut self.renderer;
        let gpu = &self.gpu;
        let library = self.library.as_ref();
        let uploaded = &mut self.uploaded;
        runity::spawn_scene_with(
            self.history.scene(),
            &mut self.world,
            |name| {
                if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
                    return Some(found.1);
                }
                let handle = if let Some(mesh) = builtin::by_name(name) {
                    renderer.upload_mesh_owned(gpu, &mesh)
                } else {
                    renderer.upload_mesh(gpu, library?.mesh_by_name(name)?)
                };
                uploaded.push((name.to_string(), handle));
                Some(handle)
            },
            |name| library?.material_by_name(name),
        );
    }

    /// The material an entity is actually drawn with.
    ///
    /// The one place that knows the resolution order, so the inspector and
    /// the frame cannot disagree about what colour something is.
    fn resolve_material(&self, desc: &runity::EntityDesc) -> runity::Material {
        desc.material_from(|name| self.library.as_ref()?.material_by_name(name))
    }

    /// Every material the editor can offer, in the order to show them:
    /// the library's palette first, then the builtins it does not shadow.
    fn palette(&self) -> Vec<(String, runity::Material)> {
        let mut out: Vec<(String, runity::Material)> = Vec::new();
        if let Some(library) = self.library.as_ref() {
            let names: Vec<String> = library
                .names_of(runity::asset::AssetKind::Material)
                .map(|n| n.to_string())
                .collect();
            for name in names {
                if let Some(material) = library.material_by_name(&name) {
                    out.push((name, material));
                }
            }
        }
        for name in runity::material::builtin::NAMES {
            if out.iter().any(|(existing, _)| existing == name) {
                // Shadowed by the project's own. Listing both would offer a
                // name that means one thing in the list and another in the
                // scene.
                continue;
            }
            if let Some(material) = runity::material::builtin::by_name(name) {
                out.push((name.to_string(), material));
            }
        }
        out
    }

    /// The size of whatever is being drawn into.
    ///
    /// One accessor rather than two call sites, because picking a ray
    /// against the offscreen image while drawing into a window is a mistake
    /// that puts the cursor somewhere else entirely.
    fn view_size(&self) -> (u32, u32) {
        match self.surface.as_ref() {
            Some(surface) => (surface.width(), surface.height()),
            None => (self.target.width, self.target.height),
        }
    }

    /// The world ray through a pixel.
    fn ray(&self, x: u32, y: u32) -> (Vec3, Vec3) {
        let (width, height) = {
            let (w, h) = self.view_size();
            (w as f32, h as f32)
        };
        let ndc_x = (x as f32 + 0.5) / width * 2.0 - 1.0;
        let ndc_y = 1.0 - (y as f32 + 0.5) / height * 2.0;
        let inverse = self.camera.view_projection(width / height).inverse();
        let near = inverse.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far = inverse.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (near, (far - near).normalize_or_zero())
    }

    /// Where the gizmo sits: the selected entity's world position.
    fn selected_origin(&self) -> Option<Vec3> {
        let index = self.selected?;
        let flat = self.history.scene().flatten();
        let (_, world) = flat.get(index)?;
        Some(world.w_axis.truncate())
    }

    /// The transform an entity's parents impose on it.
    fn parent_matrix(&self, index: usize) -> Mat4 {
        let flat = self.history.scene().flatten();
        let Some((desc, world)) = flat.get(index) else {
            return Mat4::IDENTITY;
        };
        *world * desc.transform.matrix().inverse()
    }

    fn pick(&self, x: u32, y: u32) -> Option<usize> {
        let (near, direction) = self.ray(x, y);

        let mut best: Option<(f32, usize)> = None;
        for (index, (desc, world)) in self.history.scene().flatten().iter().enumerate() {
            let Some(bounds) = self.bounds_of(&desc.model) else {
                continue;
            };
            if let Some(distance) = ray_box(near, direction, bounds, *world) {
                if best.is_none_or(|(closest, _)| distance < closest) {
                    best = Some((distance, index));
                }
            }
        }
        best.map(|(_, index)| index)
    }

    fn bounds_of(&self, model: &str) -> Option<(Vec3, Vec3)> {
        if let Some(mesh) = builtin::by_name(model) {
            return Some((
                Vec3::from_array(mesh.bounds.min),
                Vec3::from_array(mesh.bounds.max),
            ));
        }
        let mesh = self.library.as_ref()?.mesh_by_name(model)?;
        Some((
            Vec3::new(
                mesh.bounds.min[0].to_native(),
                mesh.bounds.min[1].to_native(),
                mesh.bounds.min[2].to_native(),
            ),
            Vec3::new(
                mesh.bounds.max[0].to_native(),
                mesh.bounds.max[1].to_native(),
                mesh.bounds.max[2].to_native(),
            ),
        ))
    }
}

/// Distance along a ray to a transformed box, if it hits.
///
/// The ray is moved into the box's space rather than the box into the
/// world's: a rotated box is not a box any more, and testing its world-space
/// bounds would pick things the cursor is nowhere near.
fn ray_box(origin: Vec3, direction: Vec3, bounds: (Vec3, Vec3), transform: Mat4) -> Option<f32> {
    let inverse = transform.inverse();
    let local_origin = inverse.transform_point3(origin);
    let local_direction = inverse.transform_vector3(direction);

    let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
    for axis in 0..3 {
        let d = local_direction[axis];
        let (lo, hi) = (bounds.0[axis], bounds.1[axis]);
        if d.abs() < 1e-6 {
            // Parallel to this slab: a miss unless it starts inside it.
            if local_origin[axis] < lo || local_origin[axis] > hi {
                return None;
            }
            continue;
        }
        let t0 = (lo - local_origin[axis]) / d;
        let t1 = (hi - local_origin[axis]) / d;
        let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        near = near.max(t0);
        far = far.min(t1);
        if near > far {
            return None;
        }
    }
    (far >= 0.0).then(|| near.max(0.0))
}
