//! Playing together over Steam: the host makes a lobby friends can see and
//! runs the session over Steam's networking; a friend who accepts an
//! invite — or joins the host's game from the friends list — is in the
//! lobby, and joins the session with the lobby's owner as the server.
//! Valve relays the frames where two homes cannot reach each other, so
//! nobody opens a port.
//!
//! The app is Spacewar (480), which any Steam account may run while a game
//! has no app of its own; with a real app id, only [`APP_ID`] changes.
//! Without Steam running the game still plays, alone.

use scrap::net::PeerId;
use scrap::party::Party;
use scrap::steam::Steam;
use scrap::Components;

/// Valve's Spacewar: the app every Steam account may use for testing.
pub const APP_ID: u32 = 480;

/// Cooks in a kitchen, at most.
pub const MOST: u32 = 4;

pub struct Lobby {
    steam: Option<Steam>,
    /// Why there is no Steam, in words.
    pub why: Option<String>,
    /// This player made the lobby they are in.
    hosting: bool,
    /// The lobby joined as a guest, so it is joined once.
    joined: Option<u64>,
}

impl Lobby {
    /// Steam as Spacewar, or why not.
    pub fn start() -> Self {
        match Steam::init_app(APP_ID, PeerId::HOST) {
            Ok(steam) => Self {
                steam: Some(steam),
                why: None,
                hosting: false,
                joined: None,
            },
            Err(why) => Self::off(why),
        }
    }

    /// No Steam: sessions by address only.
    pub fn off(why: impl Into<String>) -> Self {
        Self {
            steam: None,
            why: Some(why.into()),
            hosting: false,
            joined: None,
        }
    }

    pub fn on(&self) -> bool {
        self.steam.is_some()
    }

    /// The player's name on Steam.
    pub fn name(&self) -> Option<String> {
        let steam = self.steam.as_ref()?;
        Some(steam.name_of(steam.steam_id()))
    }

    /// Host a kitchen: a lobby friends can see, and the session served over
    /// Steam. `None` without Steam.
    pub fn host(&mut self, scene: &str, name: &str, components: &Components) -> Option<Party> {
        let steam = self.steam.as_ref()?;
        steam.host_lobby(MOST, &format!("{name}'s kitchen"));
        self.hosting = true;
        let wire = Box::new(steam.wire(PeerId::HOST));
        Some(Party::host(scene, name, components, vec![wire], true))
    }

    /// The lobby this player is in, once Steam has made or joined it.
    #[cfg(test)]
    pub fn id(&self) -> Option<u64> {
        self.steam.as_ref()?.lobby()
    }

    /// The overlay's invite dialog, once the lobby exists.
    pub fn invite(&self) {
        if let Some(steam) = &self.steam {
            steam.invite();
        }
    }

    /// The overlay's friends list: a friend's game is joined from there.
    pub fn friends(&self) {
        if let Some(steam) = &self.steam {
            steam.open_friends();
        }
    }

    /// Out of the lobby, host or guest.
    pub fn leave(&mut self) {
        if let Some(steam) = &self.steam {
            steam.leave_lobby();
        }
        self.hosting = false;
        self.joined = None;
    }

    /// Steam's news, once a frame. When it is that this player got into a
    /// lobby somebody else owns — an invite accepted, a friend's game
    /// joined — the session to join it by: the owner is the server.
    pub fn poll(&mut self, scene: &str, name: &str, components: &Components) -> Option<Party> {
        let steam = self.steam.as_ref()?;
        steam.run_callbacks();
        let lobby = steam.lobby()?;
        if self.hosting || self.joined == Some(lobby) {
            return None;
        }
        let (_, owner) = steam.lobby_members()?;
        if owner == steam.steam_id() {
            return None;
        }
        self.joined = Some(lobby);
        // Anything but the server's 0, and unlikely to be another guest's.
        let me = PeerId(scrap::EntityId::fresh().raw() as u32 | 1);
        let mut wire = steam.wire(me);
        wire.connect(PeerId::HOST, owner);
        Some(Party::join(wire, scene, name, components))
    }
}

/// Against the Steam client running on this machine, as Spacewar: a lobby
/// is made and the session is served. `cargo test -- --ignored steam`.
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore = "needs the Steam client running"]
    fn steam_makes_a_lobby_for_a_hosted_kitchen() {
        let mut lobby = Lobby::start();
        assert!(lobby.on(), "{:?}", lobby.why);
        let name = lobby.name().unwrap();
        let components = crate::game_components();
        let party = lobby.host("main", &name, &components).expect("hosting");
        assert!(party.is_host());
        let started = std::time::Instant::now();
        while lobby.id().is_none() && started.elapsed().as_secs() < 15 {
            assert!(lobby.poll("main", &name, &components).is_none(), "our own lobby is not joined");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let id = lobby.id().expect("Steam made the lobby");
        eprintln!("lobby {id}, hosted by {name}");
        lobby.leave();
        assert_eq!(lobby.id(), None);
    }
}
