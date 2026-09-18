//! The Objective-C carpentry, again.
//!
//! This is a deliberate copy of the same ~80 lines at the top of
//! `runity-platform/src/macos.rs`: `objc_getClass`, `sel_registerName`, a
//! macro that casts `objc_msgSend` to the signature a selector actually wants,
//! and an autorelease pool. Sharing it would mean a module that both a
//! windowing crate and a rendering crate depend on, which is a worse trade
//! than eighty duplicated lines that neither crate can break for the other.
//!
//! The one thing worth reading twice is how structs are passed. On arm64 a
//! struct wider than 16 bytes travels by hidden pointer, not in registers —
//! `MTLRegion` (48 bytes), `MTLClearColor` (32) and `MTLViewport` (48) all do.
//! Declaring them `#[repr(C)]` and passing them by value is exactly right:
//! Rust's `extern "C"` implements the same rule C does, so the compiler picks
//! the indirect form on its own. `macos.rs` passes `NSRect` the same way.

#![allow(non_snake_case, non_upper_case_globals)]

use std::ffi::{c_char, c_void};

#[repr(C)]
pub struct Object {
    _private: [u8; 0],
}

pub type Id = *mut Object;
pub type Sel = *const c_void;
/// Objective-C `BOOL`, kept as `i8` for the same reason `macos.rs` does:
/// `signed char` may carry values a Rust `bool` must never hold.
pub type Bool = i8;

pub const YES: Bool = 1;
pub const NO: Bool = 0;
pub const NIL: Id = std::ptr::null_mut();

pub const NS_UTF8_STRING_ENCODING: u64 = 4;

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

/// The bare `objc_msgSend` entry point, for the modules that cast it to
/// signatures of their own.
pub fn objc_msgSend_addr() -> unsafe extern "C" fn() {
    objc_msgSend
}

/// `name` must be NUL-terminated.
pub fn sel(name: &[u8]) -> Sel {
    debug_assert_eq!(
        name.last(),
        Some(&0),
        "selector names must be NUL-terminated"
    );
    // SAFETY: the pointer is a NUL-terminated C string for the duration of the call.
    unsafe { sel_registerName(name.as_ptr().cast()) }
}

/// `name` must be NUL-terminated.
pub fn class(name: &[u8]) -> Id {
    debug_assert_eq!(name.last(), Some(&0), "class names must be NUL-terminated");
    // SAFETY: as above.
    unsafe { objc_getClass(name.as_ptr().cast()) }
}

/// An autorelease pool, pushed for the length of a frame and popped on `Drop`.
///
/// Cocoa and Metal both hand back autoreleased objects — command buffers,
/// drawables, render pass descriptors — and with no pool they accumulate for
/// the life of the process, which at 120 frames a second is quick.
pub struct Pool(*mut c_void);

impl Pool {
    pub fn push() -> Self {
        // SAFETY: push and pop are paired by this type's lifetime.
        Self(unsafe { objc_autoreleasePoolPush() })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // SAFETY: the token came from `objc_autoreleasePoolPush` and is popped once.
        unsafe { objc_autoreleasePoolPop(self.0) }
    }
}

macro_rules! msg_send_fn {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        /// # Safety
        /// `receiver` must respond to `selector` with exactly this signature.
        pub unsafe fn $name(receiver: Id, selector: Sel $(, $arg: $ty)*) -> $ret {
            let send: unsafe extern "C" fn(Id, Sel $(, $ty)*) -> $ret =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(receiver, selector $(, $arg)*)
        }
    };
}

msg_send_fn!(msg() -> ());
msg_send_fn!(msg_id() -> Id);
msg_send_fn!(msg_u64() -> u64);
msg_send_fn!(msg_f64() -> f64);
msg_send_fn!(msg_bool() -> Bool);
msg_send_fn!(msg_ptr() -> *mut c_void);
msg_send_fn!(msg_cstr() -> *const c_char);
msg_send_fn!(msg_with_id(a: Id) -> ());
msg_send_fn!(msg_id_with_id(a: Id) -> Id);
msg_send_fn!(msg_with_bool(a: Bool) -> ());
msg_send_fn!(msg_with_u64(a: u64) -> ());
msg_send_fn!(msg_with_f64(a: f64) -> ());
msg_send_fn!(msg_id_with_u64(a: u64) -> Id);
msg_send_fn!(msg_string_init(bytes: *const c_void, len: usize, encoding: u64) -> Id);

/// Read an `NSString` back as a Rust `String`, for the text in an `NSError`.
///
/// # Safety
/// `string` must be an `NSString`, or nil.
pub unsafe fn nsstring_to_string(string: Id) -> String {
    if string.is_null() {
        return String::new();
    }
    let bytes = msg_cstr(string, sel(b"UTF8String\0"));
    if bytes.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(bytes).to_string_lossy().into_owned()
}

/// Build an `NSString`. The caller owns the result and must release it.
///
/// # Safety
/// Must run on a thread with an initialized Objective-C runtime.
pub unsafe fn nsstring(text: &str) -> Id {
    let allocated = msg_id(class(b"NSString\0"), sel(b"alloc\0"));
    msg_string_init(
        allocated,
        sel(b"initWithBytes:length:encoding:\0"),
        text.as_ptr().cast(),
        text.len(),
        NS_UTF8_STRING_ENCODING,
    )
}

/// `-[NSError localizedDescription]`, which is where a Metal compiler puts the
/// line and column it did not like.
///
/// # Safety
/// `error` must be an `NSError`, or nil.
pub unsafe fn error_text(error: Id) -> String {
    if error.is_null() {
        return "no error object".to_string();
    }
    nsstring_to_string(msg_id(error, sel(b"localizedDescription\0")))
}

/// Release an object and null the handle, so a double release cannot happen.
///
/// # Safety
/// `object` must be owned by the caller (`alloc`/`new`/`copy`/`retain`).
pub unsafe fn release(object: &mut Id) {
    if !object.is_null() {
        msg(*object, sel(b"release\0"));
        *object = NIL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_and_classes_resolve() {
        assert!(!class(b"NSString\0").is_null());
        assert!(!sel(b"release\0").is_null());
    }

    #[test]
    fn a_string_round_trips_through_the_runtime() {
        // SAFETY: an NSString we allocate, read and release ourselves.
        unsafe {
            let mut s = nsstring("MTLPixelFormatBGRA8Unorm");
            assert_eq!(nsstring_to_string(s), "MTLPixelFormatBGRA8Unorm");
            release(&mut s);
            assert!(s.is_null(), "release nulls the handle it was given");
        }
    }

    #[test]
    fn a_nil_error_still_has_something_to_say() {
        // SAFETY: nil is the documented input for this branch.
        assert_eq!(unsafe { error_text(NIL) }, "no error object");
    }
}
