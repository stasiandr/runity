//! Drives the X11 backend against a fake X server.
//!
//! The backend speaks the wire protocol itself, so it can be tested the same
//! way: stand up a Unix socket that answers like an X server, point `$DISPLAY`
//! at it, and check both directions of the conversation. No display needed.

#![cfg(all(unix, not(target_os = "macos")))]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use runity_platform::window::{Event, Key, Window, WindowConfig};
use runity_platform::x11::X11Window;

const DISPLAY_NUMBER: u32 = 97;

/// A minimal, hand-built setup reply body: one 32bpp format, one screen.
fn setup_body() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&1u32.to_le_bytes()); // release number
    body.extend_from_slice(&0x0040_0000u32.to_le_bytes()); // resource id base
    body.extend_from_slice(&0x001f_ffffu32.to_le_bytes()); // resource id mask
    body.extend_from_slice(&256u32.to_le_bytes()); // motion buffer size
    body.extend_from_slice(&4u16.to_le_bytes()); // vendor length
    body.extend_from_slice(&65535u16.to_le_bytes()); // max request length
    body.push(1); // screens
    body.push(1); // pixmap formats
    body.push(0); // image byte order: LSB first
    body.push(0); // bitmap bit order
    body.push(32); // bitmap scanline unit
    body.push(32); // bitmap scanline pad
    body.push(8); // min keycode
    body.push(255); // max keycode
    body.extend_from_slice(&0u32.to_le_bytes()); // pad
    body.extend_from_slice(b"fake"); // vendor, already 4-byte aligned
    body.extend_from_slice(&[24, 32, 32, 0, 0, 0, 0, 0]); // format: depth 24, 32 bpp
    body.extend_from_slice(&0x0000_02a1u32.to_le_bytes()); // root window
    body.extend_from_slice(&[0u8; 16]); // colormap / white / black / input masks
    body.extend_from_slice(&1920u16.to_le_bytes());
    body.extend_from_slice(&1080u16.to_le_bytes());
    body.extend_from_slice(&[0u8; 4]); // physical size
    body.extend_from_slice(&[0u8; 4]); // installed colormaps
    body.extend_from_slice(&0x21u32.to_le_bytes()); // root visual
    body.push(0); // backing stores
    body.push(0); // save unders
    body.push(24); // root depth
    body.push(0); // allowed depths
    debug_assert_eq!(
        body.len() % 4,
        0,
        "the setup body is measured in 4-byte units"
    );
    body
}

#[derive(Debug, Default)]
struct PutImage {
    width: u16,
    height: u16,
    dst_y: i16,
    depth: u8,
    first_pixel: [u8; 4],
}

#[derive(Debug, Default)]
struct Report {
    window_created: bool,
    window_mapped: bool,
    title: Option<String>,
    put_images: Vec<PutImage>,
}

fn read_exact(stream: &mut UnixStream, n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Answer one client the way an X server would, then report what it asked for.
fn serve(mut stream: UnixStream, report_tx: mpsc::Sender<Report>) {
    // --- Connection setup -------------------------------------------------
    let head = read_exact(&mut stream, 12).expect("setup request header");
    assert_eq!(
        head[0], b'l',
        "client must announce little-endian byte order"
    );
    assert_eq!(
        u16::from_le_bytes([head[2], head[3]]),
        11,
        "protocol major version"
    );
    let name_len = u16::from_le_bytes([head[6], head[7]]) as usize;
    let data_len = u16::from_le_bytes([head[8], head[9]]) as usize;
    let pad = |n: usize| (4 - n % 4) % 4;
    read_exact(&mut stream, name_len + pad(name_len)).expect("auth name");
    read_exact(&mut stream, data_len + pad(data_len)).expect("auth data");

    let body = setup_body();
    let mut reply = vec![1u8, 0];
    reply.extend_from_slice(&11u16.to_le_bytes());
    reply.extend_from_slice(&0u16.to_le_bytes());
    reply.extend_from_slice(&((body.len() / 4) as u16).to_le_bytes());
    reply.extend_from_slice(&body);
    stream.write_all(&reply).unwrap();

    // --- Requests ---------------------------------------------------------
    let mut report = Report::default();
    let mut atoms: Vec<(String, u32)> = Vec::new();
    let mut injected = false;

    loop {
        let Some(header) = read_exact(&mut stream, 4) else {
            break;
        };
        let opcode = header[0];
        let length = u16::from_le_bytes([header[2], header[3]]) as usize;
        assert!(length >= 1, "request length is counted in 4-byte units");
        let body = read_exact(&mut stream, (length - 1) * 4).unwrap_or_default();

        match opcode {
            1 => report.window_created = true,
            8 => report.window_mapped = true,
            16 => {
                // InternAtom: reply with a fresh id and remember the name.
                let n = u16::from_le_bytes([body[0], body[1]]) as usize;
                let name = String::from_utf8_lossy(&body[4..4 + n]).to_string();
                let atom = 1000 + atoms.len() as u32;
                atoms.push((name, atom));
                let mut r = vec![1u8, 0];
                r.extend_from_slice(&0u16.to_le_bytes()); // sequence
                r.extend_from_slice(&0u32.to_le_bytes()); // no extra data
                r.extend_from_slice(&atom.to_le_bytes());
                r.extend_from_slice(&[0u8; 20]);
                stream.write_all(&r).unwrap();
            }
            18 => {
                // ChangeProperty; capture WM_NAME so we can check the title.
                let property = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                let len = u32::from_le_bytes([body[16], body[17], body[18], body[19]]) as usize;
                if property == 39 {
                    report.title = Some(String::from_utf8_lossy(&body[20..20 + len]).to_string());
                }
            }
            101 => {
                // GetKeyboardMapping: one keysym per keycode, 'w' on keycode 25.
                let first = body[0];
                let count = body[1] as usize;
                let mut keysyms = vec![0u32; count];
                keysyms[(25 - first) as usize] = 0x77; // XK_w
                let mut r = vec![1u8, 1];
                r.extend_from_slice(&0u16.to_le_bytes());
                r.extend_from_slice(&(keysyms.len() as u32).to_le_bytes());
                r.extend_from_slice(&[0u8; 24]);
                for k in &keysyms {
                    r.extend_from_slice(&k.to_le_bytes());
                }
                stream.write_all(&r).unwrap();

                // The client is now up; push the events the test expects.
                if !injected {
                    injected = true;
                    let delete_atom = atoms
                        .iter()
                        .find(|(name, _)| name == "WM_DELETE_WINDOW")
                        .map(|(_, a)| *a)
                        .expect("client interned WM_DELETE_WINDOW");

                    let mut key_press = [0u8; 32];
                    key_press[0] = 2; // KeyPress
                    key_press[1] = 25; // keycode
                    stream.write_all(&key_press).unwrap();

                    let mut configure = [0u8; 32];
                    configure[0] = 22; // ConfigureNotify
                    configure[20..22].copy_from_slice(&320u16.to_le_bytes());
                    configure[22..24].copy_from_slice(&240u16.to_le_bytes());
                    stream.write_all(&configure).unwrap();

                    let mut client_message = [0u8; 32];
                    client_message[0] = 33; // ClientMessage
                    client_message[1] = 32; // format
                    client_message[12..16].copy_from_slice(&delete_atom.to_le_bytes());
                    stream.write_all(&client_message).unwrap();
                }
            }
            72 => {
                report.put_images.push(PutImage {
                    width: u16::from_le_bytes([body[8], body[9]]),
                    height: u16::from_le_bytes([body[10], body[11]]),
                    dst_y: i16::from_le_bytes([body[14], body[15]]),
                    depth: body[17],
                    first_pixel: [body[20], body[21], body[22], body[23]],
                });
            }
            _ => {}
        }
    }

    let _ = report_tx.send(report);
}

#[test]
fn x11_backend_talks_to_a_server_and_reports_events() {
    let socket_dir = std::path::Path::new("/tmp/.X11-unix");
    std::fs::create_dir_all(socket_dir).expect("create the X11 socket directory");
    let socket_path = socket_dir.join(format!("X{DISPLAY_NUMBER}"));
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path).expect("bind the fake X server socket");

    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("client connects");
        serve(stream, tx);
    });

    std::env::set_var("DISPLAY", format!(":{DISPLAY_NUMBER}"));
    std::env::set_var("XAUTHORITY", "/nonexistent/runity/.Xauthority");
    let config = WindowConfig::new("runity test", 640, 480);
    let mut window = X11Window::open(&config).expect("open a window on the fake server");
    assert_eq!(window.backend_name(), "x11");
    assert_eq!(window.size(), (640, 480));

    // Collect the injected events (they may arrive across several polls).
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while events.len() < 3 && Instant::now() < deadline {
        events.extend(window.poll_events().expect("poll"));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        events,
        vec![
            Event::KeyDown(Key::W),
            Event::Resized {
                width: 320,
                height: 240
            },
            Event::CloseRequested,
        ],
        "keycode 25 must be translated through the server's keymap"
    );
    assert_eq!(
        window.size(),
        (320, 240),
        "ConfigureNotify updates the cached size"
    );
    assert_eq!(
        window.last_error(),
        None,
        "the server reported no protocol errors"
    );

    // Present a frame that is larger than the (resized) window.
    let pixels = vec![0x00_44_88_ccu32; 640 * 480];
    window.present(&pixels, 640, 480).expect("present");
    drop(window);

    let report = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("server report");
    server.join().unwrap();
    let _ = std::fs::remove_file(&socket_path);

    assert!(report.window_created, "CreateWindow was sent");
    assert!(report.window_mapped, "MapWindow was sent");
    assert_eq!(report.title.as_deref(), Some("runity test"));

    assert!(
        !report.put_images.is_empty(),
        "the frame was pushed with PutImage"
    );
    let total_rows: u32 = report.put_images.iter().map(|p| p.height as u32).sum();
    assert_eq!(total_rows, 240, "the frame is clipped to the window height");
    assert!(
        report.put_images.len() > 1,
        "a 320x240 frame exceeds one request, so it must be split into strips"
    );
    assert_eq!(report.put_images[0].dst_y, 0);
    assert_eq!(
        report.put_images[1].dst_y,
        report.put_images[0].height as i16
    );
    for image in &report.put_images {
        assert_eq!(image.width, 320, "clipped to the window width");
        assert_eq!(image.depth, 24, "PutImage depth must match the drawable");
        assert_eq!(
            image.first_pixel,
            0x00_44_88_ccu32.to_le_bytes(),
            "pixels go out as BGRA"
        );
    }
}
