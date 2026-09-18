use std::io;

/// How a window should be created.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "runity".to_string(),
            width: 960,
            height: 540,
            resizable: true,
        }
    }
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            resizable: true,
        }
    }
}

/// Mouse buttons, as reported by the platform layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other(u8),
}

/// A physical key, already translated from the platform's own encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    // Letters
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    // Digits along the top row
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Escape,
    Space,
    Enter,
    Tab,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    LeftShift,
    RightShift,
    LeftControl,
    RightControl,
    LeftAlt,
    RightAlt,
    /// The Windows key, or Command on a Mac.
    LeftSuper,
    RightSuper,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    /// Anything the backend recognized but this enum does not name.
    Unknown(u32),
}

/// Something that happened to the window since the last poll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// The user asked to close the window (title bar button, Alt+F4, ...).
    CloseRequested,
    Resized {
        width: u32,
        height: u32,
    },
    KeyDown(Key),
    KeyUp(Key),
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseDown(MouseButton),
    MouseUp(MouseButton),
    /// The mouse wheel moved; positive is away from the user.
    Scroll(f32),
    FocusGained,
    FocusLost,
    /// The window contents were damaged and should be redrawn.
    Exposed,
}

/// A presentable window.
///
/// The contract is deliberately tiny: hand it a block of `0xAARRGGBB` pixels
/// and collect input. Everything a backend needs beyond that is its own
/// business.
pub trait Window {
    /// Current client-area size in pixels.
    fn size(&self) -> (u32, u32);

    /// Drain pending OS events. Never blocks.
    fn poll_events(&mut self) -> io::Result<Vec<Event>>;

    /// Copy `pixels` (row-major, top row first, `width * height` entries) to
    /// the screen.
    fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> io::Result<()>;

    /// Change the window title.
    fn set_title(&mut self, title: &str) -> io::Result<()>;

    /// Human-readable backend name, e.g. `"x11"` or `"headless"`.
    fn backend_name(&self) -> &'static str;
}
