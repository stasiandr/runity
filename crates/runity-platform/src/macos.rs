//! A Cocoa backend that talks to the Objective-C runtime directly.
//!
//! On macOS the window server is not reachable over a socket the way X11 is —
//! the interface is Objective-C. So "no dependencies" here means using the
//! runtime itself: `objc_getClass`, `sel_registerName` and `objc_msgSend` are
//! ordinary C functions, and every Cocoa call is one of them. That is exactly
//! what the `objc`/`cocoa` crates do; there just isn't much to it.
//!
//! The frame reaches the screen without a custom `NSView` subclass: the content
//! view is layer-backed, and each `present` hands the layer a fresh `CGImage`
//! built over our pixels. A 32-bit `kCGImageAlphaNoneSkipFirst |
//! kCGBitmapByteOrder32Little` image is byte-for-byte the `0xAARRGGBB` the
//! renderer already produces, so there is no conversion pass.
//!
//! Input avoids a delegate too: `-[NSApplication nextEventMatchingMask:...]`
//! with a past deadline is a non-blocking poll of the real event queue, and the
//! window's own state says when it was closed or lost focus.
//!
//! Most of what is polled is handed straight on to `-[NSApplication
//! sendEvent:]` — keys are the exception. Without a view subclass the content
//! view does not accept first responder, so a key reaching AppKit ends up at
//! `-[NSResponder noResponderFor:]`, which is `NSBeep()`: every keystroke in a
//! game would ring. So [`keys::route`] decides first, and keys are swallowed.
//! Cmd+key is offered to `-[NSMenu performKeyEquivalent:]` by hand, because
//! that dispatch is the menu bar's and nothing else does it for us.
//!
//! What a game gives up by swallowing keys is AppKit's own keyboard handling —
//! Tab moving focus between controls, IME composition, the system's Ctrl+F2
//! menu focus. There is no control and no text field in this engine, so there
//! is nothing for any of them to act on.
//!
//! The one piece of AppKit furniture that cannot be skipped is a main menu:
//! key equivalents are dispatched by the menu bar, so without one Cmd+M, Cmd+H
//! and Cmd+W do nothing at all.
//!
//! **Threading.** AppKit may only be used from the main thread; `open` checks
//! that and refuses otherwise.

#![allow(non_snake_case, non_upper_case_globals)]

use crate::macos_keys as keys;
use crate::window::{Event, Window, WindowConfig};
use std::ffi::{c_char, c_void};
use std::io;

#[repr(C)]
struct Object {
    _private: [u8; 0],
}

type Id = *mut Object;
type Sel = *const c_void;
/// Objective-C `BOOL`. Kept as `i8` rather than `bool`: `signed char` on
/// x86_64 may carry values other than 0 and 1, which a Rust `bool` must never.
type Bool = i8;
const YES: Bool = 1;
const NO: Bool = 0;

const NIL: Id = std::ptr::null_mut();

// NSWindowStyleMask
const STYLE_TITLED: u64 = 1 << 0;
const STYLE_CLOSABLE: u64 = 1 << 1;
const STYLE_MINIATURIZABLE: u64 = 1 << 2;
const STYLE_RESIZABLE: u64 = 1 << 3;
const NS_BACKING_STORE_BUFFERED: u64 = 2;
const NS_APPLICATION_ACTIVATION_POLICY_REGULAR: i64 = 0;
const NS_UTF8_STRING_ENCODING: u64 = 4;
const NS_EVENT_MASK_ANY: u64 = u64::MAX;

// CGImage flags: 32 bits per pixel, alpha ignored, little-endian words.
const K_CG_IMAGE_ALPHA_NONE_SKIP_FIRST: u32 = 6;
const K_CG_BITMAP_BYTE_ORDER_32_LITTLE: u32 = 2 << 12;
const BITMAP_INFO: u32 = K_CG_IMAGE_ALPHA_NONE_SKIP_FIRST | K_CG_BITMAP_BYTE_ORDER_32_LITTLE;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct NSSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

impl NSRect {
    fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: NSPoint { x, y },
            size: NSSize { width, height },
        }
    }
}

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

// Structs larger than two words are returned through a hidden pointer on
// x86_64, and that variant has its own entry point. arm64 has no such split.
#[cfg(target_arch = "x86_64")]
#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_msgSend_stret();
}

// The classes we use live in these frameworks; nothing is referenced by symbol,
// so the empty blocks exist purely to make the linker load them.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

type CGColorSpaceRef = *mut c_void;
type CGDataProviderRef = *mut c_void;
type CGImageRef = *mut c_void;
type CGDataProviderReleaseCallback = extern "C" fn(*mut c_void, *const c_void, usize);

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
    fn CGColorSpaceRelease(space: CGColorSpaceRef);
    fn CGDataProviderCreateWithData(
        info: *mut c_void,
        data: *const c_void,
        size: usize,
        release: Option<CGDataProviderReleaseCallback>,
    ) -> CGDataProviderRef;
    fn CGDataProviderRelease(provider: CGDataProviderRef);
    #[allow(clippy::too_many_arguments)]
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
        provider: CGDataProviderRef,
        decode: *const f64,
        should_interpolate: Bool,
        intent: i32,
    ) -> CGImageRef;
    fn CGImageRelease(image: CGImageRef);
}

// ---------------------------------------------------------------------------
// Objective-C message helpers
//
// `objc_msgSend` has no single signature: the caller must cast it to the exact
// one the selector expects, so the arguments land in the right registers. Each
// helper below is one such cast.
// ---------------------------------------------------------------------------

/// `name` must be NUL-terminated.
fn sel(name: &[u8]) -> Sel {
    debug_assert_eq!(
        name.last(),
        Some(&0),
        "selector names must be NUL-terminated"
    );
    // SAFETY: the pointer is a NUL-terminated C string for the duration of the call.
    unsafe { sel_registerName(name.as_ptr().cast()) }
}

/// `name` must be NUL-terminated.
fn class(name: &[u8]) -> Id {
    debug_assert_eq!(name.last(), Some(&0), "class names must be NUL-terminated");
    // SAFETY: as above.
    unsafe { objc_getClass(name.as_ptr().cast()) }
}

macro_rules! msg_send_fn {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        /// # Safety
        /// `receiver` must respond to `selector` with exactly this signature.
        unsafe fn $name(receiver: Id, selector: Sel $(, $arg: $ty)*) -> $ret {
            let send: unsafe extern "C" fn(Id, Sel $(, $ty)*) -> $ret =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(receiver, selector $(, $arg)*)
        }
    };
}

msg_send_fn!(msg() -> ());
msg_send_fn!(msg_id() -> Id);
msg_send_fn!(msg_u64() -> u64);
msg_send_fn!(msg_i64() -> i64);
msg_send_fn!(msg_u16() -> u16);
msg_send_fn!(msg_f64() -> f64);
msg_send_fn!(msg_bool() -> Bool);
msg_send_fn!(msg_point() -> NSPoint);
msg_send_fn!(msg_with_id(a: Id) -> ());
msg_send_fn!(msg_bool_with_id(a: Id) -> Bool);
msg_send_fn!(msg_with_bool(a: Bool) -> ());
msg_send_fn!(msg_with_f64(a: f64) -> ());
msg_send_fn!(msg_with_i64(a: i64) -> ());
msg_send_fn!(msg_with_ptr(a: *const c_void) -> ());
msg_send_fn!(msg_with_two_ids(a: Id, b: Id) -> ());
msg_send_fn!(msg_string_init(bytes: *const c_void, len: usize, encoding: u64) -> Id);
msg_send_fn!(msg_menu_item_init(title: Id, action: Sel, key: Id) -> Id);
msg_send_fn!(msg_window_init(rect: NSRect, style: u64, backing: u64, defer: Bool) -> Id);
msg_send_fn!(msg_next_event(mask: u64, until: Id, mode: Id, dequeue: Bool) -> Id);

/// `-[NSView bounds]` and friends return an `NSRect`, which is where the two
/// architectures' calling conventions differ.
///
/// # Safety
/// `receiver` must respond to `selector` by returning an `NSRect`.
unsafe fn msg_rect(receiver: Id, selector: Sel) -> NSRect {
    #[cfg(target_arch = "x86_64")]
    {
        let mut out = NSRect::default();
        let send: unsafe extern "C" fn(*mut NSRect, Id, Sel) =
            std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
        send(&mut out, receiver, selector);
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let send: unsafe extern "C" fn(Id, Sel) -> NSRect =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        send(receiver, selector)
    }
}

/// Build an `NSString`. The caller owns the result and must release it.
///
/// # Safety
/// Must run on a thread with an initialized Objective-C runtime.
unsafe fn nsstring(text: &str) -> Id {
    let allocated = msg_id(class(b"NSString\0"), sel(b"alloc\0"));
    msg_string_init(
        allocated,
        sel(b"initWithBytes:length:encoding:\0"),
        text.as_ptr().cast(),
        text.len(),
        NS_UTF8_STRING_ENCODING,
    )
}

/// One `NSMenuItem`. The caller owns the result and must release it; pass a
/// null `action` for an item that only carries a submenu.
///
/// # Safety
/// Must run on the main thread, with `action` a selector or null.
unsafe fn menu_item(title: &str, action: Sel, key_equivalent: &str) -> Id {
    let title = nsstring(title);
    let key = nsstring(key_equivalent);
    let item = msg_id(class(b"NSMenuItem\0"), sel(b"alloc\0"));
    let item = msg_menu_item_init(
        item,
        sel(b"initWithTitle:action:keyEquivalent:\0"),
        title,
        action,
        key,
    );
    msg(title, sel(b"release\0"));
    msg(key, sel(b"release\0"));
    item
}

/// Hang one submenu, with the items it contains, off the menu bar.
///
/// Each item is a title, the selector it sends up the responder chain, and its
/// key equivalent (the plain letter; Cmd is implied).
///
/// # Safety
/// Must run on the main thread, with every `action` a selector or null.
unsafe fn add_submenu(menu_bar: Id, title: &str, items: &[(&str, Sel, &str)]) {
    let menu = msg_id(msg_id(class(b"NSMenu\0"), sel(b"alloc\0")), sel(b"init\0"));
    for (item_title, action, key) in items {
        let item = menu_item(item_title, *action, key);
        msg_with_id(menu, sel(b"addItem:\0"), item);
        msg(item, sel(b"release\0"));
    }
    // A submenu reaches the menu bar through a carrier item of its own.
    let carrier = menu_item(title, std::ptr::null(), "");
    msg_with_id(menu_bar, sel(b"addItem:\0"), carrier);
    msg_with_two_ids(menu_bar, sel(b"setSubmenu:forItem:\0"), menu, carrier);
    msg(carrier, sel(b"release\0"));
    msg(menu, sel(b"release\0"));
}

/// Give the application the smallest main menu that still behaves like a Mac
/// application.
///
/// It is not decoration. A key equivalent is dispatched by the menu bar, so
/// with no menu at all AppKit silently drops Cmd+M, Cmd+H and Cmd+W — the
/// window cannot be sent to the Dock or hidden from the keyboard.
///
/// Quit is wired to `performClose:` rather than `terminate:` on purpose: every
/// way out of the app then follows the same path as the close button, and the
/// main loop gets to finish the frame it is on instead of the process
/// vanishing mid-render.
///
/// # Safety
/// Must run on the main thread, before `finishLaunching`.
unsafe fn install_main_menu(app: Id, app_name: &str) {
    let menu_bar = msg_id(msg_id(class(b"NSMenu\0"), sel(b"alloc\0")), sel(b"init\0"));

    // The first submenu is the application menu, whatever it is called.
    let hide = format!("Hide {app_name}");
    let quit = format!("Quit {app_name}");
    add_submenu(
        menu_bar,
        app_name,
        &[
            (hide.as_str(), sel(b"hide:\0"), "h"),
            (quit.as_str(), sel(b"performClose:\0"), "q"),
        ],
    );
    add_submenu(
        menu_bar,
        "Window",
        &[
            ("Minimize", sel(b"performMiniaturize:\0"), "m"),
            ("Close", sel(b"performClose:\0"), "w"),
        ],
    );

    msg_with_id(app, sel(b"setMainMenu:\0"), menu_bar);
    msg(menu_bar, sel(b"release\0"));
}

/// Frees the pixel buffer once Core Graphics is done with the image built over it.
extern "C" fn release_pixels(_info: *mut c_void, data: *const c_void, size: usize) {
    if data.is_null() || size == 0 {
        return;
    }
    // SAFETY: `data`/`size` are exactly what `present` handed to
    // CGDataProviderCreateWithData, and CoreGraphics calls this once.
    unsafe {
        let slice = std::slice::from_raw_parts_mut(data as *mut u8, size);
        drop(Box::from_raw(slice as *mut [u8]));
    }
}

/// Build the `CGImage` a frame is shown as.
///
/// This is the whole of the colour path: the pixels are copied out as
/// little-endian words and described as `kCGImageAlphaNoneSkipFirst |
/// kCGBitmapByteOrder32Little`, which is exactly the `0xAARRGGBB` the renderer
/// produces — no conversion pass, and nothing to get the channel order wrong.
///
/// The caller owns the result and must release it.
///
/// # Safety
/// `pixels` must hold at least `width * height` entries.
unsafe fn cg_image_from_frame(pixels: &[u32], width: u32, height: u32) -> io::Result<CGImageRef> {
    let expected = (width as usize) * (height as usize);
    if pixels.len() < expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pixel buffer is smaller than the frame it describes",
        ));
    }

    // Core Graphics keeps the pixels until the image is released, which can
    // outlive this call, so the buffer is handed over rather than borrowed.
    let bytes: Box<[u8]> = pixels[..expected]
        .iter()
        .flat_map(|p| p.to_le_bytes())
        .collect::<Vec<u8>>()
        .into();
    let size = bytes.len();
    let data = Box::into_raw(bytes) as *mut u8;

    let provider = CGDataProviderCreateWithData(
        std::ptr::null_mut(),
        data.cast(),
        size,
        Some(release_pixels),
    );
    if provider.is_null() {
        release_pixels(std::ptr::null_mut(), data.cast(), size);
        return Err(io::Error::other("CGDataProvider creation failed"));
    }
    let space = CGColorSpaceCreateDeviceRGB();
    let image = CGImageCreate(
        width as usize,
        height as usize,
        8,
        32,
        width as usize * 4,
        space,
        BITMAP_INFO,
        provider,
        std::ptr::null(),
        NO,
        0,
    );
    CGColorSpaceRelease(space);
    CGDataProviderRelease(provider);
    if image.is_null() {
        return Err(io::Error::other("CGImage creation failed"));
    }
    Ok(image)
}

/// A window on the macOS window server.
pub struct CocoaWindow {
    app: Id,
    window: Id,
    view: Id,
    layer: Id,
    run_loop_mode: Id,
    width: u32,
    height: u32,
    modifier_flags: u64,
    was_key_window: bool,
    closed: bool,
}

impl CocoaWindow {
    /// Open a window. Must be called from the main thread.
    pub fn open(config: &WindowConfig) -> io::Result<Self> {
        // SAFETY: every call below is a documented AppKit API used with its
        // declared signature, on the main thread (checked first).
        unsafe {
            if msg_bool(class(b"NSThread\0"), sel(b"isMainThread\0")) != YES {
                return Err(io::Error::other(
                    "AppKit windows must be created on the main thread",
                ));
            }

            let app = msg_id(class(b"NSApplication\0"), sel(b"sharedApplication\0"));
            if app.is_null() {
                return Err(io::Error::other(
                    "NSApplication is unavailable (is this a GUI session?)",
                ));
            }
            msg_with_i64(
                app,
                sel(b"setActivationPolicy:\0"),
                NS_APPLICATION_ACTIVATION_POLICY_REGULAR,
            );
            // Key equivalents live on the menu bar, so this has to exist before
            // launching or Cmd+M, Cmd+H and Cmd+W go nowhere.
            install_main_menu(app, &config.title);
            // Without this the app never processes events, because we drive the
            // loop ourselves instead of calling -[NSApplication run].
            msg(app, sel(b"finishLaunching\0"));
            msg_with_bool(app, sel(b"activateIgnoringOtherApps:\0"), YES);

            let (width, height) = (config.width.max(1), config.height.max(1));
            let mut style = STYLE_TITLED | STYLE_CLOSABLE | STYLE_MINIATURIZABLE;
            if config.resizable {
                style |= STYLE_RESIZABLE;
            }

            let window = msg_id(class(b"NSWindow\0"), sel(b"alloc\0"));
            let window = msg_window_init(
                window,
                sel(b"initWithContentRect:styleMask:backing:defer:\0"),
                NSRect::new(0.0, 0.0, width as f64, height as f64),
                style,
                NS_BACKING_STORE_BUFFERED,
                NO,
            );
            if window.is_null() {
                return Err(io::Error::other("NSWindow creation failed"));
            }
            // NSWindow releases itself when closed by default; take that over so
            // the handle stays valid until `Drop`.
            msg_with_bool(window, sel(b"setReleasedWhenClosed:\0"), NO);

            let title = nsstring(&config.title);
            msg_with_id(window, sel(b"setTitle:\0"), title);
            msg(title, sel(b"release\0"));

            msg_with_bool(window, sel(b"setAcceptsMouseMovedEvents:\0"), YES);
            msg(window, sel(b"center\0"));

            // Layer-backed content view: `present` only has to swap the layer's
            // contents, so there is no view subclass and no drawRect:.
            let view = msg_id(window, sel(b"contentView\0"));
            msg_with_bool(view, sel(b"setWantsLayer:\0"), YES);
            let layer = msg_id(view, sel(b"layer\0"));
            if layer.is_null() {
                return Err(io::Error::other("the content view has no layer"));
            }
            // Render at point resolution: a CPU rasterizer does not want to
            // shade 4x the fragments on a Retina display.
            msg_with_f64(layer, sel(b"setContentsScale:\0"), 1.0);

            msg_with_id(window, sel(b"makeKeyAndOrderFront:\0"), NIL);
            // Focus is reported as a change, so the starting point has to be the
            // truth: assuming "key" here costs a bogus FocusLost on the first
            // poll, because the window only becomes key once events are pumped.
            let was_key_window = msg_bool(window, sel(b"isKeyWindow\0")) != NO;

            // -[NSApplication nextEventMatchingMask:...] wants a run loop mode;
            // NSDefaultRunLoopMode is this string.
            let run_loop_mode = nsstring("kCFRunLoopDefaultMode");

            Ok(Self {
                app,
                window,
                view,
                layer,
                run_loop_mode,
                width,
                height,
                modifier_flags: 0,
                was_key_window,
                closed: false,
            })
        }
    }

    /// Turn one `NSEvent` into our own event, if it carries anything we report.
    ///
    /// # Safety
    /// `event` must be a live `NSEvent`.
    unsafe fn translate(&mut self, event: Id, out: &mut Vec<Event>) {
        let event_type = msg_u64(event, sel(b"type\0"));
        match event_type {
            keys::NS_KEY_DOWN => {
                // Auto-repeat is the OS repeating a key that is already down.
                if msg_bool(event, sel(b"isARepeat\0")) == NO {
                    let code = msg_u16(event, sel(b"keyCode\0"));
                    out.push(Event::KeyDown(keys::key_from_virtual_key(code)));
                }
            }
            keys::NS_KEY_UP => {
                let code = msg_u16(event, sel(b"keyCode\0"));
                out.push(Event::KeyUp(keys::key_from_virtual_key(code)));
            }
            keys::NS_FLAGS_CHANGED => {
                // Modifiers have no key events; Cocoa reports the whole mask.
                let flags = msg_u64(event, sel(b"modifierFlags\0"));
                for (key, pressed) in keys::modifier_changes(self.modifier_flags, flags) {
                    out.push(if pressed {
                        Event::KeyDown(key)
                    } else {
                        Event::KeyUp(key)
                    });
                }
                self.modifier_flags = flags;
            }
            keys::NS_MOUSE_MOVED
            | keys::NS_LEFT_MOUSE_DRAGGED
            | keys::NS_RIGHT_MOUSE_DRAGGED
            | keys::NS_OTHER_MOUSE_DRAGGED => {
                let point = msg_point(event, sel(b"locationInWindow\0"));
                out.push(Event::MouseMove {
                    x: point.x as i32,
                    y: keys::flip_y(point.y, self.height as f64) as i32,
                });
            }
            keys::NS_SCROLL_WHEEL => {
                let delta = msg_f64(event, sel(b"scrollingDeltaY\0"));
                if delta != 0.0 {
                    out.push(Event::Scroll(delta as f32));
                }
            }
            keys::NS_LEFT_MOUSE_DOWN | keys::NS_RIGHT_MOUSE_DOWN | keys::NS_OTHER_MOUSE_DOWN => {
                let number = msg_i64(event, sel(b"buttonNumber\0"));
                if let Some(button) = keys::mouse_button(event_type, number) {
                    out.push(Event::MouseDown(button));
                }
            }
            keys::NS_LEFT_MOUSE_UP | keys::NS_RIGHT_MOUSE_UP | keys::NS_OTHER_MOUSE_UP => {
                let number = msg_i64(event, sel(b"buttonNumber\0"));
                if let Some(button) = keys::mouse_button(event_type, number) {
                    out.push(Event::MouseUp(button));
                }
            }
            _ => {}
        }
    }
}

impl Window for CocoaWindow {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn poll_events(&mut self) -> io::Result<Vec<Event>> {
        let mut events = Vec::new();
        if self.closed {
            return Ok(events);
        }

        // SAFETY: AppKit calls on the main thread, each with its real signature.
        unsafe {
            // Cocoa hands back autoreleased objects; without a pool per frame
            // they would pile up for the lifetime of the process.
            let pool = objc_autoreleasePoolPush();
            let distant_past = msg_id(class(b"NSDate\0"), sel(b"distantPast\0"));

            loop {
                let event = msg_next_event(
                    self.app,
                    sel(b"nextEventMatchingMask:untilDate:inMode:dequeue:\0"),
                    NS_EVENT_MASK_ANY,
                    distant_past, // a deadline in the past makes this a poll
                    self.run_loop_mode,
                    YES,
                );
                if event.is_null() {
                    break;
                }

                // Route first, translate second: whether the game hears a key
                // at all depends on what the menu does with it.
                let event_type = msg_u64(event, sel(b"type\0"));
                // `modifierFlags` is only meaningful on key and mouse events,
                // and `route` only reads it for a key down.
                let modifier_flags = match event_type {
                    keys::NS_KEY_DOWN | keys::NS_KEY_UP => msg_u64(event, sel(b"modifierFlags\0")),
                    _ => 0,
                };
                match keys::route(event_type, modifier_flags) {
                    // Keys never reach AppKit: with no responder for them,
                    // `-[NSResponder noResponderFor:]` answers with NSBeep().
                    keys::Route::Swallow => self.translate(event, &mut events),
                    keys::Route::OfferToMenu => {
                        // Key equivalents are the menu bar's to dispatch, so
                        // Cmd+W and friends are handed to it by name instead.
                        let menu = msg_id(self.app, sel(b"mainMenu\0"));
                        let claimed = !menu.is_null()
                            && msg_bool_with_id(menu, sel(b"performKeyEquivalent:\0"), event) != NO;
                        // Cmd+K means nothing to the menu, so it is the game's.
                        if !claimed {
                            self.translate(event, &mut events);
                        }
                    }
                    keys::Route::AppKit => {
                        self.translate(event, &mut events);
                        // AppKit still needs everything else: window dragging,
                        // the close button and the menu bar all live here.
                        msg_with_id(self.app, sel(b"sendEvent:\0"), event);
                    }
                }
            }

            objc_autoreleasePoolPop(pool);

            // There is no delegate, so the window's own state is the source of
            // truth for closing, resizing and focus. Invisible is not the same
            // as closed: Cmd+M and Cmd+H make a window invisible too.
            let visible = msg_bool(self.window, sel(b"isVisible\0")) != NO;
            let miniaturized = msg_bool(self.window, sel(b"isMiniaturized\0")) != NO;
            let app_hidden = msg_bool(self.app, sel(b"isHidden\0")) != NO;
            if keys::window_was_closed(visible, miniaturized, app_hidden) {
                self.closed = true;
                events.push(Event::CloseRequested);
                return Ok(events);
            }

            let bounds = msg_rect(self.view, sel(b"bounds\0"));
            let width = bounds.size.width.max(1.0) as u32;
            let height = bounds.size.height.max(1.0) as u32;
            if width != self.width || height != self.height {
                self.width = width;
                self.height = height;
                events.push(Event::Resized { width, height });
            }

            let is_key = msg_bool(self.window, sel(b"isKeyWindow\0")) == YES;
            if is_key != self.was_key_window {
                self.was_key_window = is_key;
                events.push(if is_key {
                    Event::FocusGained
                } else {
                    Event::FocusLost
                });
            }
        }
        Ok(events)
    }

    fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> io::Result<()> {
        if self.closed || width == 0 || height == 0 {
            return Ok(());
        }

        // SAFETY: the image is ours until `CGImageRelease` below, and the layer
        // retains it for as long as it displays it.
        unsafe {
            let image = cg_image_from_frame(pixels, width, height)?;

            // We never run the main run loop, so the implicit transaction may
            // not commit on its own — make it explicit.
            let transaction = class(b"CATransaction\0");
            msg(transaction, sel(b"begin\0"));
            msg_with_bool(transaction, sel(b"setDisableActions:\0"), YES);
            msg_with_ptr(self.layer, sel(b"setContents:\0"), image);
            msg(transaction, sel(b"commit\0"));

            CGImageRelease(image);
        }
        Ok(())
    }

    fn set_title(&mut self, title: &str) -> io::Result<()> {
        // SAFETY: `setTitle:` takes an NSString, which is what we build here.
        unsafe {
            let string = nsstring(title);
            msg_with_id(self.window, sel(b"setTitle:\0"), string);
            msg(string, sel(b"release\0"));
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "macos"
    }
}

impl Drop for CocoaWindow {
    fn drop(&mut self) {
        // SAFETY: both objects are ours, released once, and `setReleasedWhenClosed:NO`
        // means closing the window did not already free it.
        unsafe {
            if !self.window.is_null() {
                msg(self.window, sel(b"close\0"));
                msg(self.window, sel(b"release\0"));
                self.window = NIL;
            }
            if !self.run_loop_mode.is_null() {
                msg(self.run_loop_mode, sel(b"release\0"));
                self.run_loop_mode = NIL;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These layouts are part of the ABI contract with AppKit: `NSRect` is four
    /// `CGFloat`s, and on 64-bit macOS `CGFloat` is `f64`. If this ever changed,
    /// every message send taking a rect would silently corrupt its arguments.
    #[test]
    fn cocoa_struct_layouts_match_the_platform_abi() {
        assert_eq!(std::mem::size_of::<NSPoint>(), 16);
        assert_eq!(std::mem::size_of::<NSSize>(), 16);
        assert_eq!(std::mem::size_of::<NSRect>(), 32);
        assert_eq!(std::mem::align_of::<NSRect>(), 8);
    }

    #[test]
    fn the_bitmap_format_describes_our_pixels() {
        // 0xAARRGGBB read as a little-endian 32-bit word, alpha ignored.
        assert_eq!(BITMAP_INFO, 6 | (2 << 12));
        let pixel: u32 = 0x00_44_88_cc;
        assert_eq!(
            pixel.to_le_bytes(),
            [0xcc, 0x88, 0x44, 0x00],
            "B, G, R, then the skipped byte"
        );
    }

    // Reading the image back needs a bitmap context to draw it into; nothing
    // outside the test wants these, so they are declared here.
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: CGColorSpaceRef,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextDrawImage(context: *mut c_void, rect: NSRect, image: CGImageRef);
        fn CGContextRelease(context: *mut c_void);
    }

    /// The image `present` hands the layer, read back through Core Graphics as
    /// plain R, G, B bytes. Colours on screen are exactly this, and this is as
    /// close as a test can get to looking at the window.
    #[test]
    fn a_presented_frame_keeps_its_colours_and_its_orientation() {
        // Top row, then bottom row: red, green, blue, and one mixed colour
        // whose channels are all different so a swap cannot hide.
        let frame: [u32; 8] = [
            0x00_ff_00_00,
            0x00_00_ff_00,
            0x00_00_00_ff,
            0x00_11_22_33,
            0x00_ff_ff_ff,
            0x00_00_00_00,
            0x00_44_88_cc,
            0x00_cc_88_44,
        ];
        const K_CG_IMAGE_ALPHA_NONE_SKIP_LAST: u32 = 5;

        // SAFETY: a 4x2 bitmap context matching the image we draw into it.
        let readback = unsafe {
            let image = cg_image_from_frame(&frame, 4, 2).expect("the image should build");
            let mut buffer = vec![0u8; 4 * 2 * 4];
            let space = CGColorSpaceCreateDeviceRGB();
            // Default (big-endian) byte order, alpha last: bytes are R, G, B, X.
            let context = CGBitmapContextCreate(
                buffer.as_mut_ptr().cast(),
                4,
                2,
                8,
                4 * 4,
                space,
                K_CG_IMAGE_ALPHA_NONE_SKIP_LAST,
            );
            assert!(!context.is_null(), "bitmap context creation failed");
            CGContextDrawImage(context, NSRect::new(0.0, 0.0, 4.0, 2.0), image);
            CGContextRelease(context);
            CGColorSpaceRelease(space);
            CGImageRelease(image);
            buffer
        };

        let pixel = |i: usize| (readback[i * 4], readback[i * 4 + 1], readback[i * 4 + 2]);
        assert_eq!(pixel(0), (0xff, 0x00, 0x00), "0x00ff0000 must read as red");
        assert_eq!(
            pixel(1),
            (0x00, 0xff, 0x00),
            "0x0000ff00 must read as green"
        );
        assert_eq!(pixel(2), (0x00, 0x00, 0xff), "0x000000ff must read as blue");
        assert_eq!(pixel(3), (0x11, 0x22, 0x33), "channels keep their order");
        assert_eq!(
            pixel(4),
            (0xff, 0xff, 0xff),
            "the second row is the second row: the frame is not flipped"
        );
        assert_eq!(pixel(5), (0x00, 0x00, 0x00));
        assert_eq!(pixel(6), (0x44, 0x88, 0xcc));
        assert_eq!(pixel(7), (0xcc, 0x88, 0x44));
    }

    #[test]
    fn a_frame_shorter_than_its_own_size_is_refused() {
        // SAFETY: the short slice is exactly what this call has to reject.
        let too_short = unsafe { cg_image_from_frame(&[0u32; 3], 4, 2) };
        assert!(too_short.is_err(), "a truncated frame must not be drawn");
    }

    #[test]
    fn selectors_and_classes_resolve() {
        // Cheap smoke test that the runtime is actually linked in.
        assert!(!class(b"NSWindow\0").is_null());
        assert!(!sel(b"setTitle:\0").is_null());
    }
}
