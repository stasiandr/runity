//! The wire in the browser: the page's data channels (WebRTC, through the
//! page's script — `web/runity-net.js` in a game's web build). A page has
//! no sockets, so the bytes go out through `runityNet.send` and come back
//! through [`runity_net_receive`]; how the channels are found and opened
//! (a room code, a signalling broker) is the page's business, as a Steam
//! lobby is Steam's.
//!
//! Like [`super::wire::Udp`], each datagram starts with the sender's
//! [`PeerId`], so the host learns which channel is whose from what comes
//! in, and the script need not know the game's ids at all.

use std::collections::VecDeque;
use std::sync::Mutex;

use wasm_bindgen::prelude::wasm_bindgen;

use super::wire::Transport;
use super::PeerId;

/// What came in from the page, not yet taken by the transport.
static INBOX: Mutex<VecDeque<(PeerId, Vec<u8>)>> = Mutex::new(VecDeque::new());

#[wasm_bindgen]
extern "C" {
    /// The page's `runityNet.send(to, bytes)`: to endpoint `to` (a guest
    /// sends everything to the host, whatever `to` says).
    #[wasm_bindgen(js_namespace = runityNet, js_name = send)]
    fn page_send(to: u32, bytes: &[u8]);
}

/// A datagram came in over one of the page's channels, sender first.
#[wasm_bindgen]
pub fn runity_net_receive(bytes: &[u8]) {
    if bytes.len() < 4 {
        return;
    }
    let from = PeerId(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
    INBOX.lock().unwrap().push_back((from, bytes[4..].to_vec()));
}

/// This end of the page's channels, as `me`. There is one page, so there
/// is one of these at a time; a new one starts with nothing waiting.
pub struct Page {
    me: PeerId,
}

impl Page {
    pub fn new(me: PeerId) -> Self {
        INBOX.lock().unwrap().clear();
        Self { me }
    }
}

impl Transport for Page {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        let mut datagram = self.me.0.to_le_bytes().to_vec();
        datagram.extend_from_slice(&bytes);
        page_send(to.0, &datagram);
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        INBOX.lock().unwrap().drain(..).collect()
    }
}
