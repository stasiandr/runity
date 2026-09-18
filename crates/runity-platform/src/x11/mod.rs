//! An X11 backend written directly against the wire protocol.
//!
//! There is no `libX11`, no `xcb`, no C at all: the X protocol is a stream of
//! little-endian structs over a socket, and `std` can open sockets. That is the
//! whole trick behind "a renderer with no dependencies" on Linux — we open
//! `/tmp/.X11-unix/X0`, do the handshake, ask for a window, and then push
//! frames with `PutImage`.
//!
//! Reference: the X Window System Protocol, version 11 (X.Org's `x11protocol`).

mod auth;
mod keys;

use crate::window::{Event, Key, MouseButton, Window, WindowConfig};
use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;

// Request opcodes.
const OP_CREATE_WINDOW: u8 = 1;
const OP_MAP_WINDOW: u8 = 8;
const OP_INTERN_ATOM: u8 = 16;
const OP_CHANGE_PROPERTY: u8 = 18;
const OP_CREATE_GC: u8 = 55;
const OP_PUT_IMAGE: u8 = 72;
const OP_GET_KEYBOARD_MAPPING: u8 = 101;

// Event codes.
const EV_ERROR: u8 = 0;
const EV_REPLY: u8 = 1;
const EV_KEY_PRESS: u8 = 2;
const EV_KEY_RELEASE: u8 = 3;
const EV_BUTTON_PRESS: u8 = 4;
const EV_BUTTON_RELEASE: u8 = 5;
const EV_MOTION_NOTIFY: u8 = 6;
const EV_FOCUS_IN: u8 = 9;
const EV_FOCUS_OUT: u8 = 10;
const EV_EXPOSE: u8 = 12;
const EV_CONFIGURE_NOTIFY: u8 = 22;
const EV_CLIENT_MESSAGE: u8 = 33;

// Predefined atoms (X11 assigns these fixed values).
const ATOM_ATOM: u32 = 4;
const ATOM_STRING: u32 = 31;
const ATOM_WM_NAME: u32 = 39;
const ATOM_WM_NORMAL_HINTS: u32 = 40;
const ATOM_WM_SIZE_HINTS: u32 = 41;

const CLASS_INPUT_OUTPUT: u16 = 1;
const CW_BACK_PIXEL: u32 = 0x0000_0002;
const CW_EVENT_MASK: u32 = 0x0000_0800;
const ZPIXMAP: u8 = 2;

const EVENT_MASK: u32 = 0x0000_0001 // KeyPress
    | 0x0000_0002 // KeyRelease
    | 0x0000_0004 // ButtonPress
    | 0x0000_0008 // ButtonRelease
    | 0x0000_0040 // PointerMotion
    | 0x0000_8000 // Exposure
    | 0x0002_0000 // StructureNotify
    | 0x0020_0000; // FocusChange

/// An error reported by the X server. Kept for diagnostics; a failed request
/// does not tear down the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X11Error {
    pub code: u8,
    pub major_opcode: u8,
    pub minor_opcode: u16,
    pub bad_value: u32,
}

/// Where the X server lives, parsed out of `$DISPLAY`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplaySpec {
    host: Option<String>,
    display: u32,
}

fn parse_display(value: &str) -> Option<DisplaySpec> {
    // Accepted forms: ":0", ":0.1", "unix/:0", "host:1", "host:1.0".
    let value = value.strip_prefix("unix/").unwrap_or(value);
    let value = value.strip_prefix("tcp/").unwrap_or(value);
    let (host, tail) = value.rsplit_once(':')?;
    let display_part = tail.split('.').next()?;
    let display: u32 = display_part.parse().ok()?;
    let host = match host {
        "" | "unix" | "localhost" => None,
        h => Some(h.to_string()),
    };
    Some(DisplaySpec { host, display })
}

enum Transport {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Transport {
    fn set_nonblocking(&self, on: bool) -> io::Result<()> {
        match self {
            Transport::Unix(s) => s.set_nonblocking(on),
            Transport::Tcp(s) => s.set_nonblocking(on),
        }
    }
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Transport::Unix(s) => s.read(buf),
            Transport::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Transport::Unix(s) => s.write(buf),
            Transport::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Transport::Unix(s) => s.flush(),
            Transport::Tcp(s) => s.flush(),
        }
    }
}

/// Little-endian cursor over a server reply.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn u8(&mut self) -> io::Result<u8> {
        let b = *self.bytes.get(self.at).ok_or_else(truncated)?;
        self.at += 1;
        Ok(b)
    }
    fn u16(&mut self) -> io::Result<u16> {
        let b = self.bytes.get(self.at..self.at + 2).ok_or_else(truncated)?;
        self.at += 2;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> io::Result<u32> {
        let b = self.bytes.get(self.at..self.at + 4).ok_or_else(truncated)?;
        self.at += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn skip(&mut self, n: usize) -> io::Result<()> {
        if self.at + n > self.bytes.len() {
            return Err(truncated());
        }
        self.at += n;
        Ok(())
    }
}

fn truncated() -> io::Error {
    io::Error::new(ErrorKind::UnexpectedEof, "truncated X11 reply")
}

fn protocol_error(msg: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, msg.into())
}

/// The parts of the server's setup reply we actually use.
#[derive(Debug, Clone)]
struct Setup {
    resource_id_base: u32,
    resource_id_mask: u32,
    root: u32,
    root_visual: u32,
    root_depth: u8,
    image_byte_order_msb: bool,
    min_keycode: u8,
    max_keycode: u8,
    max_request_length: u16,
    bits_per_pixel: u8,
}

fn parse_setup(body: &[u8]) -> io::Result<Setup> {
    let mut c = Cursor::new(body);
    let _release = c.u32()?;
    let resource_id_base = c.u32()?;
    let resource_id_mask = c.u32()?;
    let _motion_buffer = c.u32()?;
    let vendor_len = c.u16()? as usize;
    let max_request_length = c.u16()?;
    let screen_count = c.u8()?;
    let format_count = c.u8()? as usize;
    let image_byte_order_msb = c.u8()? == 1;
    let _bitmap_bit_order = c.u8()?;
    let _scanline_unit = c.u8()?;
    let _scanline_pad = c.u8()?;
    let min_keycode = c.u8()?;
    let max_keycode = c.u8()?;
    c.skip(4)?; // pad
    c.skip((vendor_len + 3) & !3)?;

    // Pixmap formats: 8 bytes each (depth, bits-per-pixel, scanline-pad, 5 pad).
    let mut formats = Vec::with_capacity(format_count);
    for _ in 0..format_count {
        let depth = c.u8()?;
        let bits_per_pixel = c.u8()?;
        c.skip(6)?;
        formats.push((depth, bits_per_pixel));
    }

    if screen_count == 0 {
        return Err(protocol_error("X server reported no screens"));
    }
    let root = c.u32()?;
    let _default_colormap = c.u32()?;
    let _white = c.u32()?;
    let _black = c.u32()?;
    let _input_masks = c.u32()?;
    let _width_px = c.u16()?;
    let _height_px = c.u16()?;
    let _width_mm = c.u16()?;
    let _height_mm = c.u16()?;
    let _min_maps = c.u16()?;
    let _max_maps = c.u16()?;
    let root_visual = c.u32()?;
    let _backing_stores = c.u8()?;
    let _save_unders = c.u8()?;
    let root_depth = c.u8()?;

    let bits_per_pixel = formats
        .iter()
        .find(|(d, _)| *d == root_depth)
        .map(|(_, bpp)| *bpp)
        .ok_or_else(|| protocol_error("no pixmap format for the root depth"))?;

    Ok(Setup {
        resource_id_base,
        resource_id_mask,
        root,
        root_visual,
        root_depth,
        image_byte_order_msb,
        min_keycode,
        max_keycode,
        max_request_length,
        bits_per_pixel,
    })
}

/// A connection to the X server, plus the plumbing for requests and replies.
struct Conn {
    transport: Transport,
    /// Events that arrived while we were waiting for a reply.
    stashed: Vec<[u8; 32]>,
    incoming: Vec<u8>,
    next_id: u32,
    setup: Setup,
}

impl Conn {
    fn open(spec: &DisplaySpec) -> io::Result<Self> {
        let mut transport = match &spec.host {
            None => {
                let path = format!("/tmp/.X11-unix/X{}", spec.display);
                Transport::Unix(UnixStream::connect(path)?)
            }
            Some(host) => {
                let stream = TcpStream::connect((host.as_str(), 6000 + spec.display as u16))?;
                stream.set_nodelay(true).ok();
                Transport::Tcp(stream)
            }
        };

        let cookie = auth::cookie_for_display(spec.display).unwrap_or_default();
        let request = setup_request(&cookie.name, &cookie.data);
        transport.write_all(&request)?;
        transport.flush()?;

        let mut head = [0u8; 8];
        read_exact(&mut transport, &mut head)?;
        let extra = u16::from_le_bytes([head[6], head[7]]) as usize * 4;
        let mut body = vec![0u8; extra];
        read_exact(&mut transport, &mut body)?;

        match head[0] {
            1 => {}
            0 => {
                let reason_len = head[1] as usize;
                let reason = String::from_utf8_lossy(&body[..reason_len.min(body.len())]);
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    format!("X server refused the connection: {reason}"),
                ));
            }
            2 => {
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "X server requires further authentication",
                ))
            }
            other => return Err(protocol_error(format!("unexpected setup status {other}"))),
        }

        let setup = parse_setup(&body)?;
        if setup.bits_per_pixel != 32 {
            return Err(protocol_error(format!(
                "unsupported server pixel format: {} bits per pixel at depth {}",
                setup.bits_per_pixel, setup.root_depth
            )));
        }

        Ok(Self {
            transport,
            stashed: Vec::new(),
            incoming: Vec::new(),
            next_id: 0,
            setup,
        })
    }

    /// Allocate a resource ID out of the range the server handed us.
    fn new_id(&mut self) -> io::Result<u32> {
        self.next_id += 1;
        let mask = self.setup.resource_id_mask;
        if self.next_id & mask != self.next_id {
            return Err(protocol_error("ran out of X11 resource ids"));
        }
        Ok(self.setup.resource_id_base | self.next_id)
    }

    fn send(&mut self, request: &[u8]) -> io::Result<()> {
        debug_assert_eq!(request.len() % 4, 0, "X11 requests are padded to 4 bytes");
        self.transport.write_all(request)?;
        self.transport.flush()
    }

    /// Send a request and block until its reply arrives, stashing any events
    /// that overtake it.
    fn request_reply(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
        self.send(request)?;
        loop {
            let mut packet = [0u8; 32];
            read_exact(&mut self.transport, &mut packet)?;
            match packet[0] {
                EV_REPLY => {
                    let extra = u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]])
                        as usize
                        * 4;
                    let mut reply = packet.to_vec();
                    if extra > 0 {
                        let mut rest = vec![0u8; extra];
                        read_exact(&mut self.transport, &mut rest)?;
                        reply.extend_from_slice(&rest);
                    }
                    return Ok(reply);
                }
                EV_ERROR => {
                    return Err(protocol_error(format!(
                        "X11 error {} for request opcode {}",
                        packet[1], packet[10]
                    )))
                }
                _ => self.stashed.push(packet),
            }
        }
    }
}

fn read_exact(transport: &mut Transport, buf: &mut [u8]) -> io::Result<()> {
    transport.read_exact(buf)
}

fn pad4(len: usize) -> usize {
    (4 - (len % 4)) % 4
}

fn setup_request(auth_name: &[u8], auth_data: &[u8]) -> Vec<u8> {
    let mut r = Vec::with_capacity(12 + auth_name.len() + auth_data.len() + 6);
    r.push(b'l'); // little-endian client
    r.push(0);
    r.extend_from_slice(&11u16.to_le_bytes()); // protocol major
    r.extend_from_slice(&0u16.to_le_bytes()); // protocol minor
    r.extend_from_slice(&(auth_name.len() as u16).to_le_bytes());
    r.extend_from_slice(&(auth_data.len() as u16).to_le_bytes());
    r.extend_from_slice(&0u16.to_le_bytes()); // pad
    r.extend_from_slice(auth_name);
    r.resize(r.len() + pad4(auth_name.len()), 0);
    r.extend_from_slice(auth_data);
    r.resize(r.len() + pad4(auth_data.len()), 0);
    r
}

/// A window on an X server.
pub struct X11Window {
    conn: Conn,
    window: u32,
    gc: u32,
    width: u32,
    height: u32,
    wm_delete_window: u32,
    /// Keysyms indexed by `(keycode - min_keycode) * keysyms_per_keycode`.
    keymap: Vec<u32>,
    keysyms_per_keycode: usize,
    scratch: Vec<u8>,
    last_error: Option<X11Error>,
}

impl X11Window {
    /// Open a window on the server named by `$DISPLAY`.
    pub fn open(config: &WindowConfig) -> io::Result<Self> {
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
        let spec = parse_display(&display)
            .ok_or_else(|| protocol_error(format!("malformed DISPLAY value {display:?}")))?;
        let mut conn = Conn::open(&spec)?;

        let window = conn.new_id()?;
        let gc = conn.new_id()?;
        let (width, height) = (config.width.max(1), config.height.max(1));

        conn.send(&create_window_request(
            &conn.setup,
            window,
            width as u16,
            height as u16,
        ))?;

        // Title: both the legacy property and the EWMH one modern WMs read.
        let utf8_string = intern_atom(&mut conn, b"UTF8_STRING")?;
        let net_wm_name = intern_atom(&mut conn, b"_NET_WM_NAME")?;
        conn.send(&change_property_request(
            window,
            ATOM_WM_NAME,
            ATOM_STRING,
            8,
            config.title.as_bytes(),
        ))?;
        conn.send(&change_property_request(
            window,
            net_wm_name,
            utf8_string,
            8,
            config.title.as_bytes(),
        ))?;

        // Ask the window manager to tell us about close requests instead of
        // killing the connection.
        let wm_protocols = intern_atom(&mut conn, b"WM_PROTOCOLS")?;
        let wm_delete_window = intern_atom(&mut conn, b"WM_DELETE_WINDOW")?;
        conn.send(&change_property_request(
            window,
            wm_protocols,
            ATOM_ATOM,
            32,
            &wm_delete_window.to_le_bytes(),
        ))?;

        if !config.resizable {
            conn.send(&size_hints_request(window, width, height))?;
        }

        conn.send(&create_gc_request(gc, window))?;
        conn.send(&map_window_request(window))?;

        let (keymap, keysyms_per_keycode) = load_keymap(&mut conn)?;

        Ok(Self {
            conn,
            window,
            gc,
            width,
            height,
            wm_delete_window,
            keymap,
            keysyms_per_keycode,
            scratch: Vec::new(),
            last_error: None,
        })
    }

    /// The most recent protocol error, if the server complained about anything.
    pub fn last_error(&self) -> Option<X11Error> {
        self.last_error
    }

    fn keysym(&self, keycode: u8) -> u32 {
        let min = self.conn.setup.min_keycode;
        if keycode < min || self.keysyms_per_keycode == 0 {
            return 0;
        }
        let index = (keycode - min) as usize * self.keysyms_per_keycode;
        self.keymap.get(index).copied().unwrap_or(0)
    }

    fn translate(&mut self, packet: &[u8; 32]) -> Option<Event> {
        // Bit 7 of the event type marks events sent by another client with
        // SendEvent; the type itself is in the low bits.
        match packet[0] & 0x7f {
            EV_ERROR => {
                self.last_error = Some(X11Error {
                    code: packet[1],
                    bad_value: u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]),
                    minor_opcode: u16::from_le_bytes([packet[8], packet[9]]),
                    major_opcode: packet[10],
                });
                None
            }
            EV_KEY_PRESS => Some(Event::KeyDown(self.key(packet[1]))),
            EV_KEY_RELEASE => Some(Event::KeyUp(self.key(packet[1]))),
            EV_BUTTON_PRESS => match packet[1] {
                4 => Some(Event::Scroll(1.0)),
                5 => Some(Event::Scroll(-1.0)),
                b => Some(Event::MouseDown(mouse_button(b))),
            },
            EV_BUTTON_RELEASE => match packet[1] {
                4 | 5 => None, // the wheel "release" carries no information
                b => Some(Event::MouseUp(mouse_button(b))),
            },
            EV_MOTION_NOTIFY => Some(Event::MouseMove {
                x: i16::from_le_bytes([packet[24], packet[25]]) as i32,
                y: i16::from_le_bytes([packet[26], packet[27]]) as i32,
            }),
            EV_FOCUS_IN => Some(Event::FocusGained),
            EV_FOCUS_OUT => Some(Event::FocusLost),
            EV_EXPOSE => Some(Event::Exposed),
            EV_CONFIGURE_NOTIFY => {
                let width = u16::from_le_bytes([packet[20], packet[21]]) as u32;
                let height = u16::from_le_bytes([packet[22], packet[23]]) as u32;
                if width == 0 || height == 0 || (width == self.width && height == self.height) {
                    return None;
                }
                self.width = width;
                self.height = height;
                Some(Event::Resized { width, height })
            }
            EV_CLIENT_MESSAGE => {
                let atom = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
                (atom == self.wm_delete_window).then_some(Event::CloseRequested)
            }
            _ => None,
        }
    }

    fn key(&self, keycode: u8) -> Key {
        keys::key_from_keysym(self.keysym(keycode))
    }
}

fn mouse_button(detail: u8) -> MouseButton {
    match detail {
        1 => MouseButton::Left,
        2 => MouseButton::Middle,
        3 => MouseButton::Right,
        other => MouseButton::Other(other),
    }
}

impl Window for X11Window {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn poll_events(&mut self) -> io::Result<Vec<Event>> {
        // Reads are non-blocking, writes are not: a blocking write cannot come
        // back as a partial `WouldBlock` mid-frame.
        self.conn.transport.set_nonblocking(true)?;
        let mut chunk = [0u8; 4096];
        let result = loop {
            match self.conn.transport.read(&mut chunk) {
                Ok(0) => {
                    break Err(io::Error::new(
                        ErrorKind::ConnectionAborted,
                        "X server closed the connection",
                    ))
                }
                Ok(n) => self.conn.incoming.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break Ok(()),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => break Err(e),
            }
        };
        self.conn.transport.set_nonblocking(false)?;
        result?;

        let mut packets: Vec<[u8; 32]> = std::mem::take(&mut self.conn.stashed);
        let whole = self.conn.incoming.len() / 32 * 32;
        for packet in self
            .conn
            .incoming
            .drain(..whole)
            .collect::<Vec<u8>>()
            .chunks_exact(32)
        {
            let mut fixed = [0u8; 32];
            fixed.copy_from_slice(packet);
            packets.push(fixed);
        }

        let mut events = Vec::with_capacity(packets.len());
        for packet in &packets {
            if let Some(event) = self.translate(packet) {
                events.push(event);
            }
        }
        Ok(events)
    }

    fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> io::Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        let copy_w = width.min(self.width) as usize;
        let copy_h = height.min(self.height) as usize;
        let bytes_per_row = copy_w * 4;

        // Requests are length-limited (in 4-byte units), so a frame goes up in
        // horizontal strips.
        let max_request_bytes = self.conn.setup.max_request_length as usize * 4;
        let rows_per_chunk = ((max_request_bytes.saturating_sub(24)) / bytes_per_row).max(1);
        let msb = self.conn.setup.image_byte_order_msb;
        let depth = self.conn.setup.root_depth;

        let mut y = 0usize;
        while y < copy_h {
            let rows = rows_per_chunk.min(copy_h - y);
            let payload = bytes_per_row * rows;
            self.scratch.clear();
            self.scratch.reserve(24 + payload);
            self.scratch.push(OP_PUT_IMAGE);
            self.scratch.push(ZPIXMAP);
            self.scratch
                .extend_from_slice(&(((24 + payload) / 4) as u16).to_le_bytes());
            self.scratch.extend_from_slice(&self.window.to_le_bytes());
            self.scratch.extend_from_slice(&self.gc.to_le_bytes());
            self.scratch
                .extend_from_slice(&(copy_w as u16).to_le_bytes());
            self.scratch.extend_from_slice(&(rows as u16).to_le_bytes());
            self.scratch.extend_from_slice(&0i16.to_le_bytes()); // dst-x
            self.scratch.extend_from_slice(&(y as i16).to_le_bytes()); // dst-y
            self.scratch.push(0); // left-pad
            self.scratch.push(depth);
            self.scratch.extend_from_slice(&0u16.to_le_bytes()); // pad

            for row in 0..rows {
                let start = (y + row) * width as usize;
                let line = &pixels[start..start + copy_w];
                for p in line {
                    // 0xAARRGGBB in memory order the server expects.
                    if msb {
                        self.scratch.extend_from_slice(&p.to_be_bytes());
                    } else {
                        self.scratch.extend_from_slice(&p.to_le_bytes());
                    }
                }
            }

            self.conn.transport.write_all(&self.scratch)?;
            y += rows;
        }
        self.conn.transport.flush()
    }

    fn set_title(&mut self, title: &str) -> io::Result<()> {
        let request =
            change_property_request(self.window, ATOM_WM_NAME, ATOM_STRING, 8, title.as_bytes());
        self.conn.send(&request)
    }

    fn backend_name(&self) -> &'static str {
        "x11"
    }
}

// ---------------------------------------------------------------------------
// Request encoding
// ---------------------------------------------------------------------------

fn create_window_request(setup: &Setup, window: u32, width: u16, height: u16) -> Vec<u8> {
    let values: [u32; 2] = [0x0010_1418 /* background */, EVENT_MASK];
    let mut r = Vec::with_capacity(32 + values.len() * 4);
    r.push(OP_CREATE_WINDOW);
    r.push(setup.root_depth);
    r.extend_from_slice(&((8 + values.len()) as u16).to_le_bytes());
    r.extend_from_slice(&window.to_le_bytes());
    r.extend_from_slice(&setup.root.to_le_bytes());
    r.extend_from_slice(&0i16.to_le_bytes()); // x
    r.extend_from_slice(&0i16.to_le_bytes()); // y
    r.extend_from_slice(&width.to_le_bytes());
    r.extend_from_slice(&height.to_le_bytes());
    r.extend_from_slice(&0u16.to_le_bytes()); // border width
    r.extend_from_slice(&CLASS_INPUT_OUTPUT.to_le_bytes());
    r.extend_from_slice(&setup.root_visual.to_le_bytes());
    r.extend_from_slice(&(CW_BACK_PIXEL | CW_EVENT_MASK).to_le_bytes());
    for v in values {
        r.extend_from_slice(&v.to_le_bytes());
    }
    r
}

fn map_window_request(window: u32) -> Vec<u8> {
    let mut r = Vec::with_capacity(8);
    r.push(OP_MAP_WINDOW);
    r.push(0);
    r.extend_from_slice(&2u16.to_le_bytes());
    r.extend_from_slice(&window.to_le_bytes());
    r
}

fn create_gc_request(gc: u32, drawable: u32) -> Vec<u8> {
    let mut r = Vec::with_capacity(16);
    r.push(OP_CREATE_GC);
    r.push(0);
    r.extend_from_slice(&4u16.to_le_bytes());
    r.extend_from_slice(&gc.to_le_bytes());
    r.extend_from_slice(&drawable.to_le_bytes());
    r.extend_from_slice(&0u32.to_le_bytes()); // empty value mask
    r
}

fn change_property_request(
    window: u32,
    property: u32,
    kind: u32,
    format: u8,
    data: &[u8],
) -> Vec<u8> {
    let units = match format {
        8 => data.len(),
        16 => data.len() / 2,
        _ => data.len() / 4,
    };
    let padded = data.len() + pad4(data.len());
    let mut r = Vec::with_capacity(24 + padded);
    r.push(OP_CHANGE_PROPERTY);
    r.push(0); // mode: Replace
    r.extend_from_slice(&(((24 + padded) / 4) as u16).to_le_bytes());
    r.extend_from_slice(&window.to_le_bytes());
    r.extend_from_slice(&property.to_le_bytes());
    r.extend_from_slice(&kind.to_le_bytes());
    r.push(format);
    r.extend_from_slice(&[0, 0, 0]);
    r.extend_from_slice(&(units as u32).to_le_bytes());
    r.extend_from_slice(data);
    r.resize(r.len() + pad4(data.len()), 0);
    r
}

/// `WM_NORMAL_HINTS` with min == max, which is how you ask for a fixed size.
fn size_hints_request(window: u32, width: u32, height: u32) -> Vec<u8> {
    const P_MIN_SIZE: u32 = 1 << 4;
    const P_MAX_SIZE: u32 = 1 << 5;
    let mut hints = [0u32; 18];
    hints[0] = P_MIN_SIZE | P_MAX_SIZE;
    hints[5] = width; // min width
    hints[6] = height; // min height
    hints[7] = width; // max width
    hints[8] = height; // max height
    let bytes: Vec<u8> = hints.iter().flat_map(|v| v.to_le_bytes()).collect();
    change_property_request(window, ATOM_WM_NORMAL_HINTS, ATOM_WM_SIZE_HINTS, 32, &bytes)
}

fn intern_atom(conn: &mut Conn, name: &[u8]) -> io::Result<u32> {
    let padded = name.len() + pad4(name.len());
    let mut r = Vec::with_capacity(8 + padded);
    r.push(OP_INTERN_ATOM);
    r.push(0); // only-if-exists = false
    r.extend_from_slice(&(((8 + padded) / 4) as u16).to_le_bytes());
    r.extend_from_slice(&(name.len() as u16).to_le_bytes());
    r.extend_from_slice(&0u16.to_le_bytes());
    r.extend_from_slice(name);
    r.resize(r.len() + pad4(name.len()), 0);

    let reply = conn.request_reply(&r)?;
    Ok(u32::from_le_bytes([
        reply[8], reply[9], reply[10], reply[11],
    ]))
}

fn load_keymap(conn: &mut Conn) -> io::Result<(Vec<u32>, usize)> {
    let first = conn.setup.min_keycode;
    let count = conn
        .setup
        .max_keycode
        .saturating_sub(first)
        .saturating_add(1);
    let mut r = Vec::with_capacity(8);
    r.push(OP_GET_KEYBOARD_MAPPING);
    r.push(0);
    r.extend_from_slice(&2u16.to_le_bytes());
    r.push(first);
    r.push(count);
    r.extend_from_slice(&0u16.to_le_bytes());

    let reply = conn.request_reply(&r)?;
    let per_keycode = reply[1] as usize;
    let words = u32::from_le_bytes([reply[4], reply[5], reply[6], reply[7]]) as usize;
    let mut keysyms = Vec::with_capacity(words);
    for i in 0..words {
        let at = 32 + i * 4;
        let Some(b) = reply.get(at..at + 4) else {
            break;
        };
        keysyms.push(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    }
    Ok((keysyms, per_keycode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_strings_are_parsed() {
        assert_eq!(
            parse_display(":0"),
            Some(DisplaySpec {
                host: None,
                display: 0
            })
        );
        assert_eq!(
            parse_display(":12.1"),
            Some(DisplaySpec {
                host: None,
                display: 12
            })
        );
        assert_eq!(
            parse_display("unix/:3"),
            Some(DisplaySpec {
                host: None,
                display: 3
            })
        );
        assert_eq!(
            parse_display("box.local:2"),
            Some(DisplaySpec {
                host: Some("box.local".into()),
                display: 2
            })
        );
        assert_eq!(
            parse_display("localhost:0"),
            Some(DisplaySpec {
                host: None,
                display: 0
            })
        );
        assert_eq!(parse_display("nonsense"), None);
        assert_eq!(parse_display(":abc"), None);
        assert_eq!(parse_display(""), None);
    }

    #[test]
    fn requests_are_padded_to_four_bytes_and_carry_the_right_length() {
        // `WM_PROTOCOLS` is 12 bytes: 8 header + 12 name = 20, no padding.
        let title = change_property_request(1, ATOM_WM_NAME, ATOM_STRING, 8, b"hi");
        assert_eq!(title.len() % 4, 0);
        assert_eq!(
            u16::from_le_bytes([title[2], title[3]]) as usize * 4,
            title.len()
        );
        // Length is counted in format units, not bytes.
        assert_eq!(
            u32::from_le_bytes([title[20], title[21], title[22], title[23]]),
            2
        );

        let map = map_window_request(7);
        assert_eq!(map.len(), 8);
        assert_eq!(u16::from_le_bytes([map[2], map[3]]), 2);

        let gc = create_gc_request(1, 2);
        assert_eq!(gc.len(), 16);
        assert_eq!(u16::from_le_bytes([gc[2], gc[3]]), 4);
    }

    #[test]
    fn create_window_encodes_both_values_it_sets() {
        let setup = Setup {
            resource_id_base: 0x0040_0000,
            resource_id_mask: 0x001f_ffff,
            root: 0x123,
            root_visual: 0x21,
            root_depth: 24,
            image_byte_order_msb: false,
            min_keycode: 8,
            max_keycode: 255,
            max_request_length: 65535,
            bits_per_pixel: 32,
        };
        let r = create_window_request(&setup, 0x40_0001, 640, 480);
        assert_eq!(r.len(), 40);
        assert_eq!(u16::from_le_bytes([r[2], r[3]]) as usize * 4, r.len());
        assert_eq!(r[1], 24, "window depth comes from the root");
        let mask = u32::from_le_bytes([r[28], r[29], r[30], r[31]]);
        assert_eq!(mask, CW_BACK_PIXEL | CW_EVENT_MASK);
        assert_eq!(u32::from_le_bytes([r[36], r[37], r[38], r[39]]), EVENT_MASK);
    }

    #[test]
    fn setup_request_pads_variable_length_auth_fields() {
        let r = setup_request(b"MIT-MAGIC-COOKIE-1", &[0xab; 16]);
        assert_eq!(r[0], b'l');
        assert_eq!(u16::from_le_bytes([r[2], r[3]]), 11);
        assert_eq!(u16::from_le_bytes([r[6], r[7]]), 18, "name length");
        assert_eq!(u16::from_le_bytes([r[8], r[9]]), 16, "data length");
        // 12 header + 18 name + 2 pad + 16 data
        assert_eq!(r.len(), 48);
    }

    #[test]
    fn setup_reply_is_parsed_down_to_the_first_screen() {
        // A hand-built setup body with one pixmap format and one screen.
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes()); // release
        body.extend_from_slice(&0x0040_0000u32.to_le_bytes()); // id base
        body.extend_from_slice(&0x001f_ffffu32.to_le_bytes()); // id mask
        body.extend_from_slice(&256u32.to_le_bytes()); // motion buffer
        body.extend_from_slice(&5u16.to_le_bytes()); // vendor length
        body.extend_from_slice(&65535u16.to_le_bytes()); // max request length
        body.push(1); // screens
        body.push(1); // formats
        body.push(0); // image byte order: LSB first
        body.push(0); // bitmap bit order
        body.push(32); // scanline unit
        body.push(32); // scanline pad
        body.push(8); // min keycode
        body.push(255); // max keycode
        body.extend_from_slice(&0u32.to_le_bytes()); // pad
        body.extend_from_slice(b"ACME\0"); // vendor, padded to 8 below
        body.extend_from_slice(&[0, 0, 0]);
        body.extend_from_slice(&[24, 32, 32, 0, 0, 0, 0, 0]); // format: depth 24, 32bpp
        body.extend_from_slice(&0x0000_02a1u32.to_le_bytes()); // root window
        body.extend_from_slice(&[0u8; 16]); // colormap, white, black, input masks
        body.extend_from_slice(&1920u16.to_le_bytes());
        body.extend_from_slice(&1080u16.to_le_bytes());
        body.extend_from_slice(&[0u8; 4]); // mm sizes
        body.extend_from_slice(&[0u8; 4]); // installed maps
        body.extend_from_slice(&0x21u32.to_le_bytes()); // root visual
        body.push(0); // backing stores
        body.push(0); // save unders
        body.push(24); // root depth
        body.push(0); // number of depths

        let setup = parse_setup(&body).expect("parses");
        assert_eq!(setup.root, 0x2a1);
        assert_eq!(setup.root_visual, 0x21);
        assert_eq!(setup.root_depth, 24);
        assert_eq!(setup.bits_per_pixel, 32);
        assert_eq!(setup.resource_id_base, 0x0040_0000);
        assert_eq!(setup.min_keycode, 8);
        assert!(!setup.image_byte_order_msb);
    }

    #[test]
    fn a_truncated_setup_reply_is_an_error_not_a_panic() {
        assert!(parse_setup(&[0u8; 12]).is_err());
    }

    #[test]
    fn size_hints_ask_for_a_fixed_size() {
        let r = size_hints_request(1, 800, 600);
        // 24-byte header, then 18 words of WM_SIZE_HINTS.
        assert_eq!(r.len(), 24 + 18 * 4);
        assert_eq!(
            u32::from_le_bytes([r[24], r[25], r[26], r[27]]),
            (1 << 4) | (1 << 5)
        );
        let word = |i: usize| {
            let at = 24 + i * 4;
            u32::from_le_bytes([r[at], r[at + 1], r[at + 2], r[at + 3]])
        };
        assert_eq!((word(5), word(6), word(7), word(8)), (800, 600, 800, 600));
    }
}
