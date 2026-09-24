//! A game drawn in the editor's view instead of a window of its own.
//!
//! The game stays its own process — the editor does not link its code,
//! hot patches still land in it, and its panic does not take the editor
//! down — but when [`EMBED_VAR`] names an address the shell opens no
//! window: it connects there, draws into a texture and sends each frame
//! back as pixels, and plays on what the editor sends it — the view's size
//! and the input that falls on the view. Unity's Game view, across a
//! process boundary.
//!
//! On the wire, a packet is a kind byte, a little-endian `u32` length and
//! that many bytes: a [`ToGame`] or [`ToEditor`] message as RON, or a frame
//! — width, height, then RGBA8 rows (sRGB bytes, as a PNG has them).

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::input::InputEvent;

/// The variable naming the address the editor listens on for the game.
pub const EMBED_VAR: &str = "SCRAP_EMBED";

/// What the editor tells the game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToGame {
    /// How big the view is, in pixels: the game draws at this size.
    Size(u32, u32),
    /// What happened over the view, positions in the view's pixels.
    Input(InputEvent),
}

/// What the game tells the editor, beside its frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToEditor {
    /// The game wants the pointer captured (hidden, reporting motion) or
    /// let go — `Context::capture_cursor`.
    Capture(bool),
}

/// One thing read off the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Packet<T> {
    Message(T),
    Frame {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
}

const MESSAGE: u8 = 0;
const FRAME: u8 = 1;

/// Frames larger than this are refused as garbage: 8K is 132 MB.
const MOST: u32 = 8192 * 8192 * 4 + 8;

/// Send a message.
pub fn write_message<T: Serialize>(to: &mut impl Write, message: &T) -> io::Result<()> {
    let text = ron::to_string(message).map_err(|e| io::Error::other(e.to_string()))?;
    to.write_all(&[MESSAGE])?;
    to.write_all(&(text.len() as u32).to_le_bytes())?;
    to.write_all(text.as_bytes())?;
    to.flush()
}

/// Send a frame: `pixels` is `width × height` RGBA8.
pub fn write_frame(to: &mut impl Write, width: u32, height: u32, pixels: &[u8]) -> io::Result<()> {
    debug_assert_eq!(pixels.len(), (width * height * 4) as usize);
    to.write_all(&[FRAME])?;
    to.write_all(&(pixels.len() as u32 + 8).to_le_bytes())?;
    to.write_all(&width.to_le_bytes())?;
    to.write_all(&height.to_le_bytes())?;
    to.write_all(pixels)?;
    to.flush()
}

/// Read the next packet; blocks until one is whole.
pub fn read_packet<T: DeserializeOwned>(from: &mut impl Read) -> io::Result<Packet<T>> {
    let mut head = [0u8; 5];
    from.read_exact(&mut head)?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]);
    if len > MOST {
        return Err(io::Error::other(format!("a packet of {len} bytes")));
    }
    let mut body = vec![0u8; len as usize];
    from.read_exact(&mut body)?;
    match head[0] {
        MESSAGE => {
            let text = std::str::from_utf8(&body).map_err(io::Error::other)?;
            ron::from_str(text)
                .map(Packet::Message)
                .map_err(|e| io::Error::other(e.to_string()))
        }
        FRAME if body.len() >= 8 => {
            let width = u32::from_le_bytes(body[0..4].try_into().unwrap());
            let height = u32::from_le_bytes(body[4..8].try_into().unwrap());
            if (width as u64 * height as u64 * 4) != (body.len() - 8) as u64 {
                return Err(io::Error::other("a frame whose size is not its pixels"));
            }
            body.drain(..8);
            Ok(Packet::Frame {
                width,
                height,
                pixels: body,
            })
        }
        kind => Err(io::Error::other(format!("a packet of kind {kind}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Key, MouseButton};

    #[test]
    fn messages_and_frames_come_off_the_wire_as_they_went_on() {
        let mut wire = Vec::new();
        write_message(&mut wire, &ToGame::Size(320, 200)).unwrap();
        write_message(&mut wire, &ToGame::Input(InputEvent::KeyDown(Key::W))).unwrap();
        write_message(
            &mut wire,
            &ToGame::Input(InputEvent::MouseMoved { x: 1.5, y: 2.0 }),
        )
        .unwrap();
        write_frame(&mut wire, 2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        write_message(&mut wire, &ToGame::Input(InputEvent::MouseDown(MouseButton::Left))).unwrap();

        let mut from = wire.as_slice();
        let mut next = || read_packet::<ToGame>(&mut from).unwrap();
        assert_eq!(next(), Packet::Message(ToGame::Size(320, 200)));
        assert_eq!(next(), Packet::Message(ToGame::Input(InputEvent::KeyDown(Key::W))));
        assert_eq!(
            next(),
            Packet::Message(ToGame::Input(InputEvent::MouseMoved { x: 1.5, y: 2.0 }))
        );
        assert_eq!(
            next(),
            Packet::Frame {
                width: 2,
                height: 1,
                pixels: vec![1, 2, 3, 4, 5, 6, 7, 8]
            }
        );
        assert_eq!(
            next(),
            Packet::Message(ToGame::Input(InputEvent::MouseDown(MouseButton::Left)))
        );
        assert!(read_packet::<ToGame>(&mut from).is_err(), "and then the end");
    }

    #[test]
    fn a_frame_that_lies_about_its_size_is_refused() {
        let mut wire = Vec::new();
        wire.push(FRAME);
        wire.extend_from_slice(&12u32.to_le_bytes());
        wire.extend_from_slice(&100u32.to_le_bytes());
        wire.extend_from_slice(&100u32.to_le_bytes());
        wire.extend_from_slice(&[0; 4]);
        assert!(read_packet::<ToEditor>(&mut wire.as_slice()).is_err());
    }
}
