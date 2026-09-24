//! Playing together in the browser: the host opens a kitchen and gets a
//! room code (and a link with it); a friend opens the link — or types the
//! code under "Join a friend" — and is in. The page's script
//! (`web/runity-net.js`) finds the other browser through a public
//! signalling broker and opens a WebRTC data channel to it; the session
//! runs over that channel (`runity::net::page::Page`), browser to browser,
//! with nobody's server in the middle. What Steam's lobby was for the
//! desktop kitchen, the room code is here.
//!
//! Off the web there is no page: the kitchen is yours alone.

#[cfg(target_arch = "wasm32")]
use runity::net::PeerId;
use runity::party::Party;
use runity::Components;

#[cfg(target_arch = "wasm32")]
mod page {
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen]
    extern "C" {
        /// Open a room; its code, at once (the broker is reached after).
        #[wasm_bindgen(js_namespace = runityNet)]
        pub fn host() -> String;
        /// Ask the player for a friend's code, and join it.
        #[wasm_bindgen(js_namespace = runityNet)]
        pub fn ask();
        /// Share the room's link: the phone's share sheet, or the clipboard.
        #[wasm_bindgen(js_namespace = runityNet)]
        pub fn invite();
        /// Close every channel and the room.
        #[wasm_bindgen(js_namespace = runityNet)]
        pub fn leave();
        /// "" (nothing), "hosting", "joining", "open" (a guest's channel
        /// to the host is up), or "failed:<why>".
        #[wasm_bindgen(js_namespace = runityNet)]
        pub fn state() -> String;
        /// The name this player goes by.
        #[wasm_bindgen(js_namespace = runityNet)]
        pub fn name() -> String;
    }
}

pub struct Lobby {
    /// Why there is no way to play together, in words.
    pub why: Option<String>,
    /// This player opened the room they are in.
    hosting: bool,
    /// The host's channel came up and the session was joined over it.
    joined: bool,
    /// The room's code, to say in the lobby.
    pub room: Option<String>,
}

impl Lobby {
    /// The page's rooms, in the browser.
    pub fn start() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self {
                why: None,
                hosting: false,
                joined: false,
                room: None,
            }
        } else {
            Self::off("played together in the browser")
        }
    }

    /// No rooms: the kitchen alone.
    pub fn off(why: impl Into<String>) -> Self {
        Self {
            why: Some(why.into()),
            hosting: false,
            joined: false,
            room: None,
        }
    }

    pub fn on(&self) -> bool {
        self.why.is_none()
    }

    /// The player's name.
    pub fn name(&self) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        if self.on() {
            return Some(page::name()).filter(|n| !n.is_empty());
        }
        None
    }

    /// Open a room: the session served over the page's channels. `None`
    /// off the web.
    pub fn host(&mut self, scene: &str, name: &str, components: &Components) -> Option<Party> {
        if !self.on() {
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.room = Some(page::host());
            self.hosting = true;
            let wire: Box<dyn runity::net::wire::Transport + Send> =
                Box::new(runity::net::page::Page::new(PeerId::HOST));
            // Unthreaded: the page has one thread, and the server ticks in
            // the party's update.
            return Some(Party::host(scene, name, components, vec![wire], false));
        }
        #[allow(unreachable_code)]
        {
            let _ = (scene, name, components);
            None
        }
    }

    /// Share the room.
    pub fn invite(&self) {
        #[cfg(target_arch = "wasm32")]
        if self.on() {
            page::invite();
        }
    }

    /// A friend's room, by its code.
    pub fn friends(&self) {
        #[cfg(target_arch = "wasm32")]
        if self.on() {
            page::ask();
        }
    }

    /// Out of the room, host or guest.
    pub fn leave(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if self.on() {
            page::leave();
        }
        self.hosting = false;
        self.joined = false;
        self.room = None;
    }

    /// Why joining failed, once: said on the menu.
    pub fn problem(&self) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        if self.on() && !self.joined {
            return page::state().strip_prefix("failed:").map(str::to_string);
        }
        None
    }

    /// Whether a friend's room is being knocked on.
    pub fn knocking(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        if self.on() && !self.hosting && !self.joined {
            return page::state() == "joining";
        }
        false
    }

    /// The page's news, once a frame. When it is that the channel to a
    /// friend's room came up — a link opened, a code typed — the session to
    /// join it by: the room's owner is the server.
    pub fn poll(&mut self, scene: &str, name: &str, components: &Components) -> Option<Party> {
        if !self.on() || self.hosting || self.joined {
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        if page::state() == "open" {
            self.joined = true;
            // Anything but the server's 0, and unlikely to be another guest's.
            let me = PeerId(runity::EntityId::fresh().raw() as u32 | 1);
            return Some(Party::join(
                runity::net::page::Page::new(me),
                scene,
                name,
                components,
            ));
        }
        let _ = (scene, name, components);
        None
    }
}
