//! The game in the editor's view: no window, the same loop.
//!
//! With `SCRAP_EMBED` set ([`crate::embed`]) the shell connects to the
//! editor instead of opening a window, draws each frame into a texture of
//! the view's size, reads it back and sends it; what the person does over
//! the view comes in as input. Everything else — the fixed step, the
//! frame, hot patches, gamepads — is the windowed loop's. Its turns run
//! one after the other: the frame is read back here anyway, so a render
//! thread would only wait beside it.

use std::io::{BufReader, BufWriter};
use std::net::TcpStream;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use scrap_core::embed::{self, Packet, ToEditor, ToGame};

use super::{hot, run_steps, translate_pad, Context, Game, WindowConfig, PATCHED};
use crate::gpu::{Gpu, OffscreenTarget};
use crate::input::Input;
use crate::render::Renderer;
use crate::time::Time;

/// What waits to go to the editor: the newest frame only — one the editor
/// has not taken by the time the next is drawn is not worth sending — and
/// every message.
#[derive(Default)]
struct Outbox {
    frame: Option<(u32, u32, Vec<u8>)>,
    messages: Vec<ToEditor>,
    closed: bool,
}

type Shared = Arc<(Mutex<Outbox>, Condvar)>;

fn post(outbox: &Shared, f: impl FnOnce(&mut Outbox)) {
    let (lock, ready) = &**outbox;
    if let Ok(mut out) = lock.lock() {
        f(&mut out);
        ready.notify_one();
    }
}

/// Writes what is posted, on its own thread, so a slow editor slows only
/// how many frames it sees, never the game.
fn sender(stream: TcpStream, outbox: Shared) {
    std::thread::spawn(move || {
        let mut to = BufWriter::with_capacity(1 << 20, stream);
        loop {
            let (frame, messages) = {
                let (lock, ready) = &*outbox;
                let Ok(mut out) = lock.lock() else { return };
                while out.frame.is_none() && out.messages.is_empty() && !out.closed {
                    out = match ready.wait(out) {
                        Ok(out) => out,
                        Err(_) => return,
                    };
                }
                if out.closed {
                    return;
                }
                (out.frame.take(), std::mem::take(&mut out.messages))
            };
            for m in &messages {
                if embed::write_message(&mut to, m).is_err() {
                    return;
                }
            }
            if let Some((w, h, pixels)) = frame {
                if embed::write_frame(&mut to, w, h, &pixels).is_err() {
                    return;
                }
            }
        }
    });
}

/// Reads what the editor says, on its own thread. The channel closes when
/// the editor goes.
fn receiver(stream: TcpStream) -> Receiver<ToGame> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut from = BufReader::new(stream);
        while let Ok(packet) = embed::read_packet::<ToGame>(&mut from) {
            if let Packet::Message(m) = packet {
                if tx.send(m).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

/// Run `game` drawing into the editor at `address` until the game quits
/// or the editor lets go.
pub(super) fn run<G: Game>(address: &str, config: WindowConfig, mut game: G) -> anyhow::Result<()> {
    subsecond::register_handler(Arc::new(|| {
        PATCHED.store(true, std::sync::atomic::Ordering::Release)
    }));
    let stream = TcpStream::connect(address)
        .map_err(|e| anyhow::anyhow!("the editor at {address}: {e}"))?;
    let _ = stream.set_nodelay(true);
    let from_editor = receiver(stream.try_clone()?);
    let outbox: Shared = Arc::default();
    sender(stream, outbox.clone());

    let gpu = Gpu::headless_blocking(false).map_err(anyhow::Error::msg)?;
    // The configured size until the editor says how big its view is —
    // which it does first thing.
    let mut target = OffscreenTarget::new(&gpu, config.width.max(1), config.height.max(1));
    let mut renderer = Renderer::new(&gpu, &target);
    let mut overlay = crate::ui_render::UiRenderer::new(&gpu, &target);
    let mut time = Time::new(config.time);
    let mut input = Input::new();
    let mut pads = gilrs::Gilrs::new().ok();
    let mut captured = false;
    let mut loop_times = scrap_core::perf::Profiler::new(600);

    macro_rules! ctx {
        () => {
            Context {
                time: &time,
                input: &input,
                gpu: &gpu,
                size: (target.width, target.height),
                renderer: &mut renderer,
                overlay: &mut overlay,
                cursor_captured: captured,
                loop_times: &loop_times,
                quit: false,
                capture: None,
            }
        };
    }

    // The frames are paced here: nothing waits for a display.
    let pace = Duration::from_secs_f32(1.0 / 60.0);
    let mut started = false;
    loop {
        let began = Instant::now();
        loop {
            match from_editor.try_recv() {
                Ok(ToGame::Size(w, h)) => {
                    let (w, h) = (w.max(1), h.max(1));
                    if (w, h) != (target.width, target.height) {
                        target = OffscreenTarget::new(&gpu, w, h);
                    }
                }
                Ok(ToGame::Input(event)) => input.handle(&event),
                Err(TryRecvError::Empty) => break,
                // The editor stopped playing, or went.
                Err(TryRecvError::Disconnected) => {
                    post(&outbox, |o| o.closed = true);
                    return Ok(());
                }
            }
        }
        if let Some(pads) = pads.as_mut() {
            while let Some(event) = pads.next_event() {
                if let Some(event) = translate_pad(event.event) {
                    input.handle(&event);
                }
            }
        }

        let mut quit = false;
        let mut wanted: Option<bool> = None;
        if !started {
            let mut ctx = ctx!();
            game.start(&mut ctx);
            wanted = ctx.capture;
            started = true;
        }
        if PATCHED.swap(false, std::sync::atomic::Ordering::AcqRel) {
            let mut ctx = ctx!();
            hot(|| game.patched(&mut ctx));
            quit |= ctx.quit;
            wanted = ctx.capture.or(wanted);
        }
        let steps = Instant::now();
        quit |= run_steps(&mut game, &mut time, &input, (target.width, target.height));
        loop_times.record("steps", steps.elapsed());
        let framed = Instant::now();
        let mut ctx = ctx!();
        let frame = hot(|| game.frame(&mut ctx));
        quit |= ctx.quit;
        wanted = ctx.capture.or(wanted);
        loop_times.record("frame", framed.elapsed());
        if let Some(on) = wanted.filter(|on| *on != captured) {
            captured = on;
            post(&outbox, |o| o.messages.push(ToEditor::Capture(on)));
        }

        let drawn = Instant::now();
        renderer.draw_ui_pictures(&gpu, &mut overlay, &frame);
        renderer.render(&gpu, &target, &frame);
        overlay.render(&gpu, &target, game.overlay());
        let pixels = target.read_rgba(&gpu);
        loop_times.record("render", drawn.elapsed());
        let size = (target.width, target.height);
        post(&outbox, |o| o.frame = Some((size.0, size.1, pixels)));
        input.begin_frame();

        if quit {
            post(&outbox, |o| o.closed = true);
            return Ok(());
        }
        if let Some(rest) = pace.checked_sub(began.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}
