//! A Win32 backend that declares the OS entry points it needs itself.
//!
//! On Windows there is no socket protocol to speak — the window manager lives
//! behind `user32.dll` — so "no dependencies" here means declaring the handful
//! of functions we use with `extern "system"` and linking the system import
//! libraries, instead of pulling in the `windows`/`winapi` crates.
//!
//! Frames reach the screen through `StretchDIBits` with a top-down 32-bit
//! `BI_RGB` DIB, whose pixel layout is exactly the `0xAARRGGBB` the renderer
//! already produces.

// Types and fields keep their Windows SDK spelling: a reader comparing this
// against MSDN should see the same names.
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use crate::window::{Event, Key, MouseButton, Window, WindowConfig};
use std::cell::RefCell;
use std::ffi::c_void;
use std::io;

type HANDLE = *mut c_void;
type HWND = HANDLE;
type HDC = HANDLE;
type HINSTANCE = HANDLE;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const SW_SHOW: i32 = 5;
const PM_REMOVE: u32 = 0x0001;
const CS_OWNDC: u32 = 0x0020;
const CW_USEDEFAULT: i32 = i32::MIN; // 0x8000_0000 as i32
const IDC_ARROW: u32 = 32512;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const SRCCOPY: u32 = 0x00CC_0020;

const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_SETFOCUS: u32 = 0x0007;
const WM_KILLFOCUS: u32 = 0x0008;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020A;

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: WPARAM,
    lParam: LPARAM,
    time: u32,
    pt: POINT,
}

#[repr(C)]
struct WNDCLASSW {
    style: u32,
    lpfnWndProc: Option<WndProc>,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HINSTANCE,
    hIcon: HANDLE,
    hCursor: HANDLE,
    hbrBackground: HANDLE,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(class: *const WNDCLASSW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: HWND,
        menu: HANDLE,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn ShowWindow(hwnd: HWND, cmd: i32) -> i32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, dc: HDC) -> i32;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> i32;
    fn AdjustWindowRect(rect: *mut RECT, style: u32, menu: i32) -> i32;
    fn SetWindowTextW(hwnd: HWND, text: *const u16) -> i32;
    fn DestroyWindow(hwnd: HWND) -> i32;
    fn LoadCursorW(instance: HINSTANCE, name: usize) -> HANDLE;
}

#[link(name = "gdi32")]
extern "system" {
    fn StretchDIBits(
        dc: HDC,
        x_dst: i32,
        y_dst: i32,
        w_dst: i32,
        h_dst: i32,
        x_src: i32,
        y_src: i32,
        w_src: i32,
        h_src: i32,
        bits: *const c_void,
        info: *const BITMAPINFOHEADER,
        usage: u32,
        rop: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
}

thread_local! {
    /// Filled by the window procedure, drained by `poll_events`.
    ///
    /// Both run on the thread that owns the window — Windows dispatches
    /// messages on the thread that pumps them — so a thread-local queue needs
    /// no locking and no raw pointer stashed in the window's user data.
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
}

fn push_event(event: Event) {
    EVENTS.with(|q| q.borrow_mut().push(event));
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn key_from_virtual_key(vk: u32) -> Key {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Num0,
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    const FUNCTION: [Key; 12] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    match vk {
        0x41..=0x5A => LETTERS[(vk - 0x41) as usize],
        0x30..=0x39 => DIGITS[(vk - 0x30) as usize],
        0x70..=0x7B => FUNCTION[(vk - 0x70) as usize],
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x0D => Key::Enter,
        0x10 => Key::LeftShift,
        0x11 => Key::LeftControl,
        0x12 => Key::LeftAlt,
        0x1B => Key::Escape,
        0x5B => Key::LeftSuper,
        0x5C => Key::RightSuper,
        0x20 => Key::Space,
        0x25 => Key::Left,
        0x26 => Key::Up,
        0x27 => Key::Right,
        0x28 => Key::Down,
        other => Key::Unknown(other),
    }
}

/// # Safety
/// Called by Windows with a valid window handle and message parameters.
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            // Do not destroy the window here: let the application decide.
            push_event(Event::CloseRequested);
            0
        }
        WM_DESTROY => {
            push_event(Event::CloseRequested);
            0
        }
        WM_SIZE => {
            let width = (l & 0xffff) as u32;
            let height = ((l >> 16) & 0xffff) as u32;
            if width > 0 && height > 0 {
                push_event(Event::Resized { width, height });
            }
            0
        }
        WM_PAINT => {
            push_event(Event::Exposed);
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_SETFOCUS => {
            push_event(Event::FocusGained);
            0
        }
        WM_KILLFOCUS => {
            push_event(Event::FocusLost);
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            // Bit 30 of lParam is set when this is a key-repeat.
            if l & (1 << 30) == 0 {
                push_event(Event::KeyDown(key_from_virtual_key(w as u32)));
            }
            0
        }
        WM_KEYUP | WM_SYSKEYUP => {
            push_event(Event::KeyUp(key_from_virtual_key(w as u32)));
            0
        }
        WM_MOUSEMOVE => {
            push_event(Event::MouseMove {
                x: (l & 0xffff) as i16 as i32,
                y: ((l >> 16) & 0xffff) as i16 as i32,
            });
            0
        }
        WM_LBUTTONDOWN => {
            push_event(Event::MouseDown(MouseButton::Left));
            0
        }
        WM_LBUTTONUP => {
            push_event(Event::MouseUp(MouseButton::Left));
            0
        }
        WM_RBUTTONDOWN => {
            push_event(Event::MouseDown(MouseButton::Right));
            0
        }
        WM_RBUTTONUP => {
            push_event(Event::MouseUp(MouseButton::Right));
            0
        }
        WM_MBUTTONDOWN => {
            push_event(Event::MouseDown(MouseButton::Middle));
            0
        }
        WM_MBUTTONUP => {
            push_event(Event::MouseUp(MouseButton::Middle));
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((w >> 16) & 0xffff) as i16 as f32 / 120.0;
            push_event(Event::Scroll(delta));
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

/// A window backed by `user32`.
pub struct Win32Window {
    hwnd: HWND,
    width: u32,
    height: u32,
}

impl Win32Window {
    pub fn open(config: &WindowConfig) -> io::Result<Self> {
        let class_name = wide("runity_window");
        let title = wide(&config.title);
        let (width, height) = (config.width.max(1), config.height.max(1));

        let style = if config.resizable {
            WS_OVERLAPPEDWINDOW
        } else {
            WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
        };

        // SAFETY: every pointer below points at a live local, and the strings
        // are NUL-terminated by `wide`.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                style: CS_OWNDC,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: std::ptr::null_mut(),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW as usize),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };
            // A duplicate registration is fine: a second window reuses the class.
            RegisterClassW(&class);

            // Size the frame so the *client area* matches the requested size.
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            AdjustWindowRect(&mut rect, style, 0);

            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                rect.right - rect.left,
                rect.bottom - rect.top,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            )
        };

        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `hwnd` is a window we just created.
        unsafe { ShowWindow(hwnd, SW_SHOW) };

        Ok(Self {
            hwnd,
            width,
            height,
        })
    }
}

impl Window for Win32Window {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn poll_events(&mut self) -> io::Result<Vec<Event>> {
        // SAFETY: `MSG` is only read after PeekMessageW reports a message.
        unsafe {
            let mut msg = std::mem::zeroed::<MSG>();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let mut events = EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()));

        // Keep the cached size in step with what the OS reports.
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: `self.hwnd` is valid for the lifetime of this struct.
        if unsafe { GetClientRect(self.hwnd, &mut rect) } != 0 {
            let width = (rect.right - rect.left).max(0) as u32;
            let height = (rect.bottom - rect.top).max(0) as u32;
            if width > 0 && height > 0 && (width != self.width || height != self.height) {
                self.width = width;
                self.height = height;
                if !events.iter().any(|e| matches!(e, Event::Resized { .. })) {
                    events.push(Event::Resized { width, height });
                }
            }
        }
        Ok(events)
    }

    fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> io::Result<()> {
        if width == 0 || height == 0 || pixels.len() < (width * height) as usize {
            return Ok(());
        }
        let info = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative height means top-down rows, matching our framebuffer.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };

        // SAFETY: the bitmap header describes exactly `pixels`, and the DC is
        // released on every path out.
        unsafe {
            let dc = GetDC(self.hwnd);
            if dc.is_null() {
                return Err(io::Error::last_os_error());
            }
            let result = StretchDIBits(
                dc,
                0,
                0,
                self.width as i32,
                self.height as i32,
                0,
                0,
                width as i32,
                height as i32,
                pixels.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            ReleaseDC(self.hwnd, dc);
            if result == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn set_title(&mut self, title: &str) -> io::Result<()> {
        let text = wide(title);
        // SAFETY: `text` is NUL-terminated and outlives the call.
        if unsafe { SetWindowTextW(self.hwnd, text.as_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "win32"
    }
}

impl Drop for Win32Window {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            // SAFETY: the handle is ours and is dropped exactly once.
            unsafe { DestroyWindow(self.hwnd) };
            self.hwnd = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_keys_map_to_the_enum() {
        assert_eq!(key_from_virtual_key(0x57), Key::W);
        assert_eq!(key_from_virtual_key(0x1B), Key::Escape);
        assert_eq!(key_from_virtual_key(0x7B), Key::F12);
        assert_eq!(key_from_virtual_key(0x0100), Key::Unknown(0x0100));
    }

    #[test]
    fn wide_strings_are_nul_terminated() {
        let w = wide("hi");
        assert_eq!(w, vec![b'h' as u16, b'i' as u16, 0]);
    }
}
