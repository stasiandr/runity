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

use runity::gizmo::{self, Drag, GizmoStyle, Handle, Motion, Tool};
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
    /// Prefabs a scene's instances name. Loaded from `prefabs/` beside the
    /// scene when one is opened, so the editor finds the same ones the
    /// headless render does.
    prefabs: runity::Prefabs,
    /// Where those came from, so the editor can write a new one back.
    prefab_dir: Option<PathBuf>,
    /// The document with its instances expanded: what is drawn, and what a
    /// click is tested against. Rebuilt with the world, so the two cannot
    /// disagree about what is in the scene.
    instanced: runity::Instanced,
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
    /// Which handles are shown and what a drag does with them.
    tool: Tool,
    /// Grid for a drag: metres, degrees, and scale steps. Zero means off.
    snap: (f32, f32, f32),
    drag: Option<Drag>,
    /// The selected entity's transform when the drag began.
    ///
    /// Rotation and scale are asked for relative to where the gesture
    /// started, never to the last frame: a caller that multiplied a delta in
    /// every frame would accumulate its rounding, and a long drag would
    /// drift away from what the cursor says.
    drag_from: Option<runity::Transform>,
    /// A unit cube, uploaded once, that the gizmo's three arms are made of.
    gizmo_arm: Option<MeshHandle>,
    /// Set while the scene is being simulated rather than edited.
    play: Option<Play>,
}

/// What play mode holds while it runs.
///
/// The engine is a guest, so this does not own a loop or a thread: the host
/// calls [`runity_editor_step`] with however long its frame took, and the
/// clock inside decides how many fixed steps that is worth. An editor that
/// started a thread here would be an editor whose simulation kept running
/// while a modal dialog was open.
struct Play {
    physics: runity::PhysicsWorld,
    clock: runity::Time,
    /// The document as it was when play began, restored when it stops.
    ///
    /// Play mode is a preview. Unity lets you edit while it runs and throws
    /// the changes away when you stop, which is famous for losing an hour's
    /// work; here an edit is refused with a sentence instead.
    before: Scene,
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
        prefabs: runity::Prefabs::new(),
        prefab_dir: None,
        instanced: runity::Instanced::default(),
        uploaded: Vec::new(),
        camera: Camera::default(),
        order: Vec::new(),
        pixels: Vec::new(),
        selected: None,
        gizmo_style: GizmoStyle::default(),
        tool: Tool::default(),
        snap: (0.0, 0.0, 0.0),
        drag: None,
        drag_from: None,
        gizmo_arm: None,
        play: None,
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
            prefabs: runity::Prefabs::new(),
            prefab_dir: None,
            instanced: runity::Instanced::default(),
            uploaded: Vec::new(),
            camera: Camera::default(),
            order: Vec::new(),
            pixels: Vec::new(),
            selected: None,
            gizmo_style: GizmoStyle::default(),
            tool: Tool::default(),
            snap: (0.0, 0.0, 0.0),
            drag: None,
            drag_from: None,
            gizmo_arm: None,
            play: None,
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
            // The scene says where it is looked at from, and opening it puts
            // the view there: a file that renders one way headlessly and
            // opens pointing somewhere else in the editor is a file whose
            // picture nobody can predict.
            editor.camera = runity::scene_camera(&scene.view);
            // Prefabs come from `prefabs/` beside the scene, by the same
            // convention the headless render uses. An editor that had to be
            // told where they are would be an editor that shows a different
            // scene than the one CI renders.
            let (prefabs, problems) = runity::Prefabs::beside(&path);
            if !problems.is_empty() {
                fail(format!("{} prefab(s) skipped", problems.len()));
            }
            editor.prefab_dir = path.parent().map(|d| d.join("prefabs"));
            editor.prefabs = prefabs;
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
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

/// Read the camera: eye xyz, then target xyz.
///
/// # Safety
/// `out_eye` and `out_target` must each be writable for three floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_get_camera(
    editor: *mut Editor,
    out_eye: *mut c_float,
    out_target: *mut c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if out_eye.is_null() || out_target.is_null() {
        return false;
    }
    let eye = editor.camera.position.to_array();
    let target = editor.camera.target.to_array();
    unsafe {
        ptr::copy_nonoverlapping(eye.as_ptr(), out_eye, 3);
        ptr::copy_nonoverlapping(target.as_ptr(), out_target, 3);
    }
    true
}

/// Write where the editor is looking into the scene, as one undoable step.
///
/// Separate from [`runity_editor_set_camera`] on purpose: flying around is
/// not an edit, and a scene that changed every time someone looked at it
/// from a different angle would produce a diff on every open. Saving the
/// viewpoint is a decision, so it is a call.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_capture_camera(editor: *mut Editor) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let view = runity::captured_view(&editor.camera);
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
    editor.history.edit().view = view;
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
        frame.overlay_draws.extend(runity::gizmo::draws_for(
            editor.tool,
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
    gizmo::hit_for(
        editor.tool,
        &editor.camera,
        &editor.gizmo_style,
        origin,
        from,
        direction,
    )
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
    let Some(handle) = gizmo::hit_for(
        editor.tool,
        &editor.camera,
        &editor.gizmo_style,
        origin,
        from,
        direction,
    ) else {
        return -1;
    };
    // One snapshot for the whole gesture: everything until the next one
    // undoes as a single step, however many frames the drag lasts.
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return -1;
    }
    editor.history.snapshot();
    editor.drag_from = editor.selected.and_then(|index| {
        editor
            .history
            .scene()
            .flatten()
            .get(index)
            .map(|(d, _)| d.transform)
    });
    editor.drag = Some(gizmo::begin_for(
        editor.tool,
        origin,
        handle,
        from,
        direction,
    ));
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
    let motion = gizmo::update_for(&drag, from, direction);

    // The gizmo sits at the entity's world position, but what is edited is
    // its local one. The difference is the parent's transform, and applying
    // the move in world space without undoing it drags a child out of its
    // parent by however much the parent is offset.
    let parent = editor.parent_matrix(index);
    let started = editor.drag_from;
    let snap = editor.snap;
    // Untracked: the snapshot for this gesture was taken at `gizmo_begin`.
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
    let Some(desc) = runity::edit::nth_mut(editor.history.scene_mut_untracked(), index) else {
        return false;
    };
    match motion {
        Motion::Position(moved) => {
            // Snapped in local space, which is the space the file holds and
            // the space a person means: a child snapped in world space lands
            // on a grid its parent is not on.
            let local = parent.inverse().transform_point3(moved);
            desc.transform.position = gizmo::snap_all(local, snap.0);
        }
        Motion::Rotation(delta) => {
            let Some(started) = started else {
                return false;
            };
            // The turn is in world axes and the file holds a local
            // rotation, so the parent's own turn has to come out first —
            // the same correction the move path makes for position.
            let (_, parent_rotation, _) = parent.to_scale_rotation_translation();
            let local = parent_rotation.inverse() * delta * parent_rotation;
            desc.transform.set_rotation(local * started.rotation());
            // Snapped as degrees, which is what the file holds and what an
            // inspector shows; snapping a quaternion is not a thing.
            desc.transform.rotation_deg = gizmo::snap_all(desc.transform.rotation_deg, snap.1);
        }
        Motion::Scale(factor) => {
            let Some(started) = started else {
                return false;
            };
            desc.transform.scale = gizmo::snap_all(started.scale * factor, snap.2);
        }
    }
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
        editor.drag_from = None;
    }
}

/// Choose what the gizmo does: 0 move, 1 rotate, 2 scale.
///
/// Anything else is refused rather than silently treated as move — a host
/// passing a number it made up should find out, not discover later that its
/// rotate button moves things.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_tool(editor: *mut Editor, tool: c_int) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    editor.tool = match tool {
        0 => Tool::Move,
        1 => Tool::Rotate,
        2 => Tool::Scale,
        other => {
            fail(format!("{other} is not a tool"));
            return false;
        }
    };
    // A held handle belongs to the tool that was showing when it was
    // grabbed; keeping it across a change would drag a ring that is no
    // longer drawn.
    editor.drag = None;
    editor.drag_from = None;
    true
}

/// Which tool is showing: 0 move, 1 rotate, 2 scale.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_tool(editor: *mut Editor) -> c_int {
    unsafe { borrow(editor) }.map_or(0, |e| match e.tool {
        Tool::Move => 0,
        Tool::Rotate => 1,
        Tool::Scale => 2,
    })
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
        prefab: String::new(),
        transform: Default::default(),
        material: Default::default(),
        body: Default::default(),
        collider: Default::default(),
        children: Vec::new(),
    };
    let parent = (parent >= 0).then_some(parent as usize);
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return -1;
    }
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
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

/// Point the editor at a directory of prefabs.
///
/// Opening a scene already loads the `prefabs/` beside it, which is the
/// convention every tool follows. This is for a host that keeps them
/// somewhere else.
///
/// # Safety
/// `path` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_prefabs(
    editor: *mut Editor,
    path: *const c_char,
) -> bool {
    let (Some(editor), Some(path)) = (unsafe { borrow(editor) }, unsafe { path_from(path) }) else {
        fail("no editor or no path");
        return false;
    };
    match runity::Prefabs::open(&path) {
        Ok((prefabs, problems)) => {
            if !problems.is_empty() {
                fail(format!("{} prefab(s) skipped", problems.len()));
            }
            editor.prefabs = prefabs;
            editor.prefab_dir = Some(path);
            editor.respawn();
            true
        }
        Err(e) => {
            fail(e.to_string());
            false
        }
    }
}

/// How many prefabs there are to place.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_prefab_count(editor: *mut Editor) -> c_uint {
    unsafe { borrow(editor) }.map_or(0, |e| e.prefabs.len() as c_uint)
}

/// One prefab's name. Sorted, so a list does not reshuffle between openings.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_prefab_name(
    editor: *mut Editor,
    index: c_uint,
    buffer: *mut c_char,
    capacity: c_uint,
) -> c_uint {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return 0;
    };
    let names = editor.prefabs.names();
    let Some(name) = names.get(index as usize) else {
        return 0;
    };
    write_string(name, buffer, capacity)
}

/// What prefab an entity is an instance of, or "" when it is a plain entity.
///
/// The editor's tree needs this to say so: an instance is one row whose
/// insides belong to a file, and a row that looks like every other row hides
/// the difference until someone tries to move a stone and moves twelve.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes, or null.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_entity_prefab(
    editor: *mut Editor,
    index: c_uint,
    buffer: *mut c_char,
    capacity: c_uint,
) -> c_uint {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return 0;
    };
    let flat = editor.history.scene().flatten();
    let name = flat
        .get(index as usize)
        .map(|(desc, _)| desc.prefab.clone())
        .unwrap_or_default();
    write_string(&name, buffer, capacity)
}

/// Place an instance of a prefab, under `parent` or at the top with -1.
/// Returns its index, or -1.
///
/// # Safety
/// `prefab` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_add_instance(
    editor: *mut Editor,
    parent: c_int,
    prefab: *const c_char,
) -> c_int {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return -1;
    };
    let Some(name) = (unsafe { path_from(prefab) }) else {
        return -1;
    };
    let name = name.to_string_lossy().into_owned();
    if editor.prefabs.get(&name).is_none() {
        // Refused, unlike an unknown material name. A colour that does not
        // resolve shows grey and can be fixed by typing; an instance of
        // nothing is an entity with no model and no way to tell why.
        fail(format!("no prefab named {name}"));
        return -1;
    }
    let desc = runity::EntityDesc {
        name: name.clone(),
        model: String::new(),
        prefab: name,
        ..Default::default()
    };
    let parent = (parent >= 0).then_some(parent as usize);
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return -1;
    }
    let Some(index) = runity::edit::add(editor.history.edit(), parent, desc) else {
        return -1;
    };
    editor.respawn();
    index as c_int
}

/// Save an entity's subtree as a prefab and make it an instance of it.
///
/// The move that turns a thing arranged once into a thing placed many times,
/// and the reason it is one call rather than "save it, then retype it as an
/// instance": doing it by hand leaves the scene holding a copy that drifts
/// from the file the moment either changes.
///
/// # Safety
/// `name` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_make_prefab(
    editor: *mut Editor,
    index: c_uint,
    name: *const c_char,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
    let Some(name) = (unsafe { path_from(name) }) else {
        fail("no name");
        return false;
    };
    let name = name.to_string_lossy().into_owned();
    if name.is_empty() {
        fail("a prefab needs a name");
        return false;
    }
    let Some(directory) = editor.prefab_dir.clone() else {
        fail("no prefab directory — open a scene, or set one");
        return false;
    };

    // Taken from the expanded document, so making a prefab out of something
    // that already contains an instance writes what it stands for rather
    // than a reference the new file's neighbours may not have.
    let Some(desc) = editor
        .instanced
        .scene
        .flatten()
        .iter()
        .zip(&editor.instanced.source)
        .find(|(_, source)| **source == index as usize)
        .map(|((desc, _), _)| (*desc).clone())
    else {
        fail("no entity at that index");
        return false;
    };

    if let Err(e) = std::fs::create_dir_all(&directory) {
        fail(e.to_string());
        return false;
    }
    let path = directory.join(format!("{name}.{}", runity::prefab::EXTENSION));
    if let Err(e) = runity::Prefabs::save(&desc, &path) {
        fail(e);
        return false;
    }
    editor.prefabs.insert(name.clone(), desc);

    // The entity becomes an instance: its children now live in the file, and
    // leaving a copy of them in the scene is how the two start to drift.
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
    let scene = editor.history.edit();
    let Some(entity) = runity::edit::nth_mut(scene, index as usize) else {
        return false;
    };
    entity.prefab = name;
    entity.model = String::new();
    entity.children.clear();
    editor.respawn();
    true
}

/// Set the grid a drag lands on: metres, degrees, and scale steps.
///
/// Zero on any of them turns that one off. Applied to the result rather than
/// to the movement, so a drag lands on the grid instead of on wherever it
/// started plus a whole number of steps.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_set_snap(
    editor: *mut Editor,
    meters: c_float,
    degrees: c_float,
    scale_step: c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let sane = |v: c_float| if v.is_finite() && v > 0.0 { v } else { 0.0 };
    editor.snap = (sane(meters), sane(degrees), sane(scale_step));
    true
}

/// Read the grid back: metres, degrees, scale steps.
///
/// # Safety
/// `out_three` must be writable for three floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_snap(editor: *mut Editor, out_three: *mut c_float) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if out_three.is_null() {
        return false;
    }
    let values = [editor.snap.0, editor.snap.1, editor.snap.2];
    unsafe { ptr::copy_nonoverlapping(values.as_ptr(), out_three, 3) };
    true
}

/// Point the camera at the selected entity, close enough to fill the view.
///
/// The one editor command nobody notices until it is missing: without it,
/// selecting something in a list means hunting for it by flying around, and
/// anything small enough to be hard to see is also too small to fly to.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_focus_selected(editor: *mut Editor) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let Some(index) = editor.selected else {
        return false;
    };
    let Some(target) = editor.world_position(index) else {
        return false;
    };
    // How big the thing is, so a boulder and a pebble both end up filling
    // the frame rather than one of them being a dot.
    let radius = editor.selected_radius(index).max(0.05);
    let half_fov = (editor.camera.fov_y_degrees * 0.5).to_radians().max(1e-3);
    // A little further than the geometry needs, so the thing is framed
    // rather than touching the edges.
    let distance = radius / half_fov.sin() * 1.3;
    let back = (editor.camera.position - editor.camera.target).normalize_or_zero();
    let back = if back.length_squared() < 1e-6 {
        Vec3::new(0.0, 0.4, 1.0).normalize()
    } else {
        back
    };
    editor.camera.target = target;
    editor.camera.position = target + back * distance;
    true
}

/// Where an entity actually is: three floats, in world space.
///
/// Not the same as its transform. The transform is local and belongs to the
/// file; this is where the thing ends up once its parents — and, while play
/// is running, the simulation — have had their say. An inspector that showed
/// only the local one would say a falling crate is still four metres up.
///
/// # Safety
/// `out_three` must be writable for three floats.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_world_position(
    editor: *mut Editor,
    index: c_uint,
    out_three: *mut c_float,
) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if out_three.is_null() {
        return false;
    }
    let Some(position) = editor.world_position(index as usize) else {
        return false;
    };
    let values = position.to_array();
    unsafe { ptr::copy_nonoverlapping(values.as_ptr(), out_three, 3) };
    true
}

/// Start simulating the scene.
///
/// The document is kept aside and restored when play stops, so a thing that
/// fell over stays fallen only as long as you are watching it.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_play(editor: *mut Editor) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    if editor.play.is_some() {
        return true;
    }
    let fixed = runity::TimeSettings::default().fixed_delta;
    let mut physics = runity::PhysicsWorld::new(fixed);
    // Built from the world rather than the scene, so what is simulated is
    // exactly what is drawn — prefabs already expanded, hierarchy already
    // applied.
    physics.sync_from_world(&mut editor.world);
    editor.play = Some(Play {
        physics,
        clock: runity::Time::new(runity::TimeSettings::default()),
        before: editor.history.scene().clone(),
    });
    editor.drag = None;
    editor.drag_from = None;
    true
}

/// Advance the simulation by however long the host's frame took, in seconds.
///
/// Returns how many fixed steps were taken, which can be zero on a fast
/// frame and several on a slow one. Nothing happens unless play has started.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_step(editor: *mut Editor, seconds: c_float) -> c_uint {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return 0;
    };
    let Some(play) = editor.play.as_mut() else {
        return 0;
    };
    play.clock.advance(seconds.max(0.0));
    let mut steps = 0;
    while play.clock.next_step().is_some() {
        play.physics.step();
        steps += 1;
    }
    if steps > 0 {
        play.physics.sync_to_world(&mut editor.world);
    }
    // Animation runs on the frame rather than the step: a pose interpolates
    // and does not need to be deterministic the way a solver does.
    runity::advance_animations(&mut editor.world, seconds.max(0.0));
    steps
}

/// Stop simulating and put the scene back as it was.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_stop(editor: *mut Editor) -> bool {
    let Some(editor) = (unsafe { borrow(editor) }) else {
        return false;
    };
    let Some(play) = editor.play.take() else {
        return false;
    };
    // Untracked: starting and stopping a preview is not something to undo,
    // and putting it on the stack would mean pressing play cost a step of
    // real editing history.
    *editor.history.scene_mut_untracked() = play.before;
    editor.respawn();
    true
}

/// Whether the scene is being simulated.
///
/// # Safety
/// `editor` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn runity_editor_is_playing(editor: *mut Editor) -> bool {
    unsafe { borrow(editor) }.is_some_and(|e| e.play.is_some())
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return -1;
    }
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
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
    if editor.playing() {
        fail("stop playing first — an edit made in play mode is an edit you lose");
        return false;
    }
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
        // Instances expanded first, so everything past this point — the
        // world, the frame, a click — sees a plain tree and knows nothing
        // about prefabs.
        self.instanced = runity::instantiate(self.history.scene(), &self.prefabs);
        let renderer = &mut self.renderer;
        let gpu = &self.gpu;
        let library = self.library.as_ref();
        let uploaded = &mut self.uploaded;
        let scene = &self.instanced.scene;
        runity::spawn_scene_with(
            scene,
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

    fn playing(&self) -> bool {
        self.play.is_some()
    }

    /// How big the thing at a document index is, as a radius around it.
    ///
    /// Measured over the expanded subtree, so focusing on a campfire frames
    /// the ring of stones rather than the patch of earth under it.
    fn selected_radius(&self, index: usize) -> f32 {
        let Some(centre) = self.world_position(index) else {
            return 0.0;
        };
        let mut radius: f32 = 0.0;
        for (i, (desc, world)) in self.instanced.scene.flatten().iter().enumerate() {
            if self.instanced.source.get(i).copied().unwrap_or(i) != index {
                continue;
            }
            let Some(bounds) = self.bounds_of(&desc.model) else {
                continue;
            };
            let (low, high) = bounds;
            for corner in 0..8u32 {
                // Every corner of the box, transformed: the far corner of a
                // rotated box is further away than the box's own extents
                // suggest, and framing by extents alone clips it.
                let pick = |axis: usize| {
                    if corner & (1 << axis) == 0 {
                        low[axis]
                    } else {
                        high[axis]
                    }
                };
                let local = Vec3::new(pick(0), pick(1), pick(2));
                radius = radius.max((world.transform_point3(local) - centre).length());
            }
        }
        radius
    }

    /// Where the entity at a document index has ended up.
    ///
    /// Read out of the world rather than off the scene, because the world is
    /// what the simulation moves. The first spawned entity belonging to that
    /// row is the answer: for an instance that is the prefab's root, which
    /// is the thing the row stands for.
    fn world_position(&self, index: usize) -> Option<Vec3> {
        let mut best: Option<(usize, Vec3)> = None;
        for (scene_index, placed) in self
            .world
            .query::<(&runity::SceneIndex, &runity::world::WorldTransform)>()
            .iter()
        {
            let row = self
                .instanced
                .source
                .get(scene_index.0)
                .copied()
                .unwrap_or(scene_index.0);
            if row != index {
                continue;
            }
            if best.is_none_or(|(first, _)| scene_index.0 < first) {
                best = Some((scene_index.0, placed.0.w_axis.truncate()));
            }
        }
        best.map(|(_, position)| position)
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

        // Tested against the expanded scene and answered with a document
        // index: clicking a stone that came out of a prefab selects the fire
        // that brought it, because the fire is the thing the document can
        // move. Testing the document instead would make everything a prefab
        // brought unclickable.
        let mut best: Option<(f32, usize)> = None;
        for (index, (desc, world)) in self.instanced.scene.flatten().iter().enumerate() {
            let Some(bounds) = self.bounds_of(&desc.model) else {
                continue;
            };
            if let Some(distance) = ray_box(near, direction, bounds, *world) {
                let owner = self.instanced.source.get(index).copied().unwrap_or(index);
                if best.is_none_or(|(closest, _)| distance < closest) {
                    best = Some((distance, owner));
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
