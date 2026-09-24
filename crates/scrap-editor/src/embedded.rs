//! The game Play started, drawn in the view: Unity's Game view.
//!
//! The game is its own process ([`crate::game`]); started with
//! `SCRAP_EMBED` naming the address here, it opens no window, and sends
//! its frames here instead ([`scrap::embed`]). The session shows the
//! newest in its own image while the Game view is up, tells the game how
//! big that image is, and passes on what the person does over it.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

use scrap::embed::{self, Packet, ToEditor, ToGame};
use scrap::input::InputEvent;

use crate::{EditError, EditResult, Session};

type Frame = (u32, u32, Vec<u8>);

/// The editor's end of a game drawing into its view.
pub(crate) struct Embedded {
    /// Waiting for the game to connect, which it does once it is built.
    listener: TcpListener,
    /// The game, once connected: what the editor writes to.
    stream: Option<TcpStream>,
    /// The newest frame the game sent and the view has not shown yet.
    frame: Arc<Mutex<Option<Frame>>>,
    messages: Option<Receiver<ToEditor>>,
    /// A frame of the game has been shown: the view is the game's from
    /// then on, until it stops.
    shown: bool,
    /// The view size last told.
    told: Option<(u32, u32)>,
    captured: bool,
}

impl Embedded {
    /// Listen on a free local port; the address is for the game.
    pub(crate) fn listen() -> EditResult<(Self, String)> {
        let io = |e: std::io::Error| EditError::Io(format!("the Game view could not listen: {e}"));
        let listener = TcpListener::bind("127.0.0.1:0").map_err(io)?;
        listener.set_nonblocking(true).map_err(io)?;
        let address = listener.local_addr().map_err(io)?.to_string();
        Ok((
            Self {
                listener,
                stream: None,
                frame: Arc::default(),
                messages: None,
                shown: false,
                told: None,
                captured: false,
            },
            address,
        ))
    }

    /// Take the game's connection if it came, what it said, and tell it
    /// the view's size if that changed.
    fn poll(&mut self, view: (u32, u32)) {
        if self.stream.is_none() {
            if let Ok((stream, _)) = self.listener.accept() {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_nodelay(true);
                if let Ok(reading) = stream.try_clone() {
                    self.messages = Some(read(reading, self.frame.clone()));
                    self.stream = Some(stream);
                }
            }
        }
        if let Some(messages) = &self.messages {
            while let Ok(message) = messages.try_recv() {
                match message {
                    ToEditor::Capture(on) => self.captured = on,
                }
            }
        }
        if self.stream.is_some() && self.told != Some(view) {
            self.told = Some(view);
            let _ = self.send(&ToGame::Size(view.0, view.1));
        }
    }

    /// `false` when there is no game to hear it: not connected yet, or
    /// gone.
    fn send(&mut self, message: &ToGame) -> bool {
        let Some(stream) = self.stream.as_mut() else {
            return false;
        };
        if embed::write_message(stream, message)
            .and_then(|()| stream.flush())
            .is_err()
        {
            // The game went: its exit is what the Console reports.
            self.stream = None;
            return false;
        }
        true
    }

    fn take_frame(&mut self) -> Option<Frame> {
        self.frame.lock().ok()?.take()
    }
}

/// The game's frames and messages, read on their own thread: only the
/// newest frame is kept, so a view that draws less often than the game
/// never falls behind it.
fn read(stream: TcpStream, frame: Arc<Mutex<Option<Frame>>>) -> Receiver<ToEditor> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut from = BufReader::with_capacity(1 << 20, stream);
        loop {
            match embed::read_packet::<ToEditor>(&mut from) {
                Ok(Packet::Message(m)) => {
                    if tx.send(m).is_err() {
                        break;
                    }
                }
                Ok(Packet::Frame {
                    width,
                    height,
                    pixels,
                }) => {
                    if let Ok(mut slot) = frame.lock() {
                        *slot = Some((width, height, pixels));
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

impl Session {
    fn embedded(&mut self) -> Option<&mut Embedded> {
        self.game.as_mut()?.embed.as_mut()
    }

    /// Keep the game in the view up with it: its connection, what it said,
    /// the view's size. Part of [`Session::poll_game`].
    pub(crate) fn poll_embedded(&mut self) {
        let view = self.size();
        if let Some(embedded) = self.embedded() {
            embedded.poll(view);
        }
    }

    /// Whether the game Play started draws in this view — it is running
    /// with no window of its own.
    pub fn is_game_in_view(&self) -> bool {
        self.game.as_ref().is_some_and(|g| g.embed.is_some())
    }

    /// Whether the game in the view has drawn into it yet — before, it is
    /// still building or starting, and the Game view shows the scene
    /// through its camera.
    pub fn game_draws_in_view(&self) -> bool {
        self.game
            .as_ref()
            .and_then(|g| g.embed.as_ref())
            .is_some_and(|e| e.shown)
    }

    /// Pass what the person did over the view to the game in it, positions
    /// in the view's pixels. Nothing without one.
    pub fn send_to_game(&mut self, event: InputEvent) {
        if let Some(embedded) = self.embedded() {
            let _ = embedded.send(&ToGame::Input(event));
        }
    }

    /// Type a line into the running game's own console, as if typed over
    /// the game (`scrap::console`): its cheats, `set world.gravity -3`,
    /// `help`. The game runs it and prints `> line` and its answer, which
    /// the Console shows as one entry. The game in the Game view hears it;
    /// the other players' windows do not.
    pub fn send_game_command(&mut self, line: &str) -> EditResult<()> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }
        self.poll_embedded();
        let Some(embedded) = self.embedded() else {
            return Err(EditError::Io(
                "no game is running in the Game view: start_game first".into(),
            ));
        };
        if !embedded.send(&ToGame::Command(line.to_string())) {
            return Err(EditError::Io(
                "the game is not listening yet (still building or starting); try again once it draws".into(),
            ));
        }
        Ok(())
    }

    /// [`Session::send_game_command`], then wait up to `wait` for the
    /// game's answer to come into the Console: its entry, `> line` and
    /// what it said. An agent's cheat, headless.
    pub fn game_console(&mut self, line: &str, wait: std::time::Duration) -> EditResult<String> {
        let line = line.trim();
        let asked = format!("> {line}");
        let answers = |session: &Self| -> Vec<(String, u32)> {
            session
                .console()
                .iter()
                .filter(|l| {
                    let first = l.text.lines().next().unwrap_or_default();
                    // "player 1: > …" when several play.
                    first == asked || first.ends_with(&format!(": {asked}"))
                })
                .map(|l| (l.text.clone(), l.count))
                .collect()
        };
        self.poll_game();
        let before = answers(self);
        self.send_game_command(line)?;
        let deadline = std::time::Instant::now() + wait;
        loop {
            self.poll_game();
            let now = answers(self);
            if let Some((text, _)) = now.iter().find(|a| !before.contains(a)) {
                return Ok(text.clone());
            }
            if std::time::Instant::now() >= deadline {
                return Err(EditError::Io(format!(
                    "sent `{line}`; the game has not answered in {:.1} s — see console",
                    wait.as_secs_f32()
                )));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Whether the game in the view wants the pointer captured: hidden,
    /// only its motion counting — a first-person look.
    pub fn game_captures_cursor(&self) -> bool {
        self.game
            .as_ref()
            .and_then(|g| g.embed.as_ref())
            .is_some_and(|e| e.captured)
    }

    /// While the Game view is up and the game has drawn into it: its
    /// newest frame into the session's image, instead of the scene.
    /// `true` when the image is the game's.
    pub(crate) fn show_game_frame(&mut self) -> bool {
        if !self.game_view {
            return false;
        }
        let size = self.size();
        let Some(embedded) = self.embedded() else {
            return false;
        };
        let frame = embedded.take_frame();
        let fresh = frame.filter(|(w, h, _)| (*w, *h) == size);
        if fresh.is_some() {
            embedded.shown = true;
        }
        if !embedded.shown {
            return false;
        }
        if let Some((_, _, pixels)) = fresh {
            self.target.write_rgba(&self.gpu, &pixels);
            if self.readback {
                self.pixels = pixels;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scrap::input::Key;
    use std::time::{Duration, Instant};

    /// A stand-in game at the other end: it hears the view's size, draws
    /// one red frame of it, and hears the keys.
    #[test]
    fn the_game_view_shows_what_the_game_draws_and_tells_it_the_keys() {
        let mut session = match Session::offscreen(8, 4) {
            Ok(s) => s,
            Err(e) => return eprintln!("skipping: {e}"),
        };
        let mut stand_in = std::process::Command::new("sleep");
        stand_in.arg("30");
        session.run_in_console(stand_in).unwrap();
        let (embed, address) = Embedded::listen().unwrap();
        session.game.as_mut().unwrap().embed = Some(embed);
        assert!(session.is_game_in_view());

        let game = TcpStream::connect(&address).unwrap();
        let mut from_editor = BufReader::new(game.try_clone().unwrap());
        session.poll_game();
        assert_eq!(
            embed::read_packet::<ToGame>(&mut from_editor).unwrap(),
            Packet::Message(ToGame::Size(8, 4)),
            "the game is told how big to draw"
        );
        let red: Vec<u8> = [255, 0, 0, 255].repeat(8 * 4);
        embed::write_frame(&mut &game, 8, 4, &red).unwrap();
        embed::write_message(&mut &game, &ToEditor::Capture(true)).unwrap();

        session.set_game_view(true);
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.frame_pixels() != red.as_slice() {
            assert!(Instant::now() < deadline, "the game's frame never showed");
            std::thread::sleep(Duration::from_millis(10));
            session.poll_game();
            session.render();
        }
        assert!(session.game_draws_in_view());
        assert!(session.game_captures_cursor(), "it asked for the pointer");

        session.send_to_game(InputEvent::KeyDown(Key::W));
        assert_eq!(
            embed::read_packet::<ToGame>(&mut from_editor).unwrap(),
            Packet::Message(ToGame::Input(InputEvent::KeyDown(Key::W)))
        );

        // The Scene view is the editor's again, the game still going.
        session.set_game_view(false);
        session.render();
        assert_ne!(session.frame_pixels(), red.as_slice());
        assert!(session.stop_game());
        assert!(!session.is_game_in_view());
    }

    /// A line for the game's console goes over the socket, and the answer
    /// the game prints comes back as the Console entry the call returns.
    #[test]
    fn a_console_line_reaches_the_game_and_its_answer_comes_back() {
        let mut session = match Session::offscreen(8, 4) {
            Ok(s) => s,
            Err(e) => return eprintln!("skipping: {e}"),
        };
        assert!(session.send_game_command("help").is_err(), "no game yet");
        // A stand-in that answers the way `scrap::console` prints.
        let mut stand_in = std::process::Command::new("sh");
        stand_in.args(["-c", "sleep 1; printf '> give 3\\n  3 given\\n'; sleep 30"]);
        session.run_in_console(stand_in).unwrap();
        let (embed, address) = Embedded::listen().unwrap();
        session.game.as_mut().unwrap().embed = Some(embed);
        assert!(
            session.send_game_command("help").is_err(),
            "not connected yet"
        );
        let game = TcpStream::connect(&address).unwrap();
        let mut from_editor = BufReader::new(game.try_clone().unwrap());
        session.poll_game();
        assert_eq!(
            embed::read_packet::<ToGame>(&mut from_editor).unwrap(),
            Packet::Message(ToGame::Size(8, 4))
        );

        let answer = session
            .game_console("give 3", Duration::from_secs(10))
            .unwrap();
        assert_eq!(answer, "> give 3\n  3 given");
        assert_eq!(
            embed::read_packet::<ToGame>(&mut from_editor).unwrap(),
            Packet::Message(ToGame::Command("give 3".into()))
        );
        assert!(session.stop_game());
    }
}
