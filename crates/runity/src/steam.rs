//! Steam: lobbies, invites through friends, and peer-to-peer through NAT —
//! the platform half of DNA postulate 4's infrastructure, behind the
//! `steam` feature.
//!
//! [`Steam`] is a [`Transport`] like [`crate::net::Udp`] and
//! [`crate::relay::Relayed`]: Steam's networking messages carry the same
//! frames, relayed by Valve where two homes cannot reach each other. The
//! lobby is Steam's: [`Steam::host_lobby`] makes one friends can see,
//! [`Steam::invite`] opens the overlay's invite dialog, and a friend who
//! accepts — or [`Steam::join_lobby`] with its id — is joined, the lobby's
//! owner being endpoint [`PeerId::HOST`] — the server — to them. On a
//! transport a [`PeerId`] names an endpoint; which client that is in the
//! game is the server's to say ([`crate::net::server`]).
//!
//! Built and type-checked with the SDK; it needs the Steam client running
//! and an app id (`steam_appid.txt`, 480 for testing) to do anything, so
//! no automated test exercises it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use steamworks::networking_types::{NetworkingIdentity, SendFlags};
use steamworks::{Client, LobbyId, LobbyType, SteamId};

use crate::net::{PeerId, Transport};

/// Steam as the wire, and its lobby.
pub struct Steam {
    client: Client,
    me: PeerId,
    peers: HashMap<PeerId, SteamId>,
    /// The lobby this peer is in, once Steam says so.
    lobby: Arc<Mutex<Option<LobbyId>>>,
    /// Kept alive: an invite accepted through the overlay joins.
    _join_requests: steamworks::CallbackHandle,
}

impl Steam {
    /// Start Steam for this game as peer `me`. Fails, in words, when the
    /// Steam client is not running or the app id is unknown.
    pub fn init(me: PeerId) -> Result<Self, String> {
        let client = Client::init().map_err(|e| format!("steam: {e}"))?;
        // Messages from whoever opens a session are accepted; what they may
        // change is still decided by the server, which drops anything from
        // a peer about what it does not own.
        client
            .networking_messages()
            .session_request_callback(|request| {
                request.accept();
            });
        let lobby: Arc<Mutex<Option<LobbyId>>> = Arc::new(Mutex::new(None));
        let joined = lobby.clone();
        let inside = client.clone();
        let join_requests =
            client.register_callback(move |r: steamworks::GameLobbyJoinRequested| {
                let joined = joined.clone();
                inside
                    .matchmaking()
                    .join_lobby(r.lobby_steam_id, move |result| {
                        if let Ok(id) = result {
                            *joined.lock().unwrap() = Some(id);
                        }
                    });
            });
        Ok(Self {
            client,
            me,
            peers: HashMap::new(),
            lobby,
            _join_requests: join_requests,
        })
    }

    /// This player on Steam.
    pub fn steam_id(&self) -> u64 {
        self.client.user().steam_id().raw()
    }

    /// Who a peer is on Steam.
    pub fn connect(&mut self, peer: PeerId, steam_id: u64) {
        self.peers.insert(peer, SteamId::from_raw(steam_id));
    }

    /// Make a lobby of at most `max` that friends can see and join, named
    /// for the list. [`Steam::lobby`] says when it exists.
    pub fn host_lobby(&self, max: u32, name: &str) {
        let slot = self.lobby.clone();
        let inside = self.client.clone();
        let name = name.to_string();
        self.client.matchmaking().create_lobby(
            LobbyType::FriendsOnly,
            max.min(250),
            move |result| {
                if let Ok(id) = result {
                    inside.matchmaking().set_lobby_data(id, "name", &name);
                    *slot.lock().unwrap() = Some(id);
                }
            },
        );
    }

    /// Join a lobby by its id — from a list, or a friend's invite.
    pub fn join_lobby(&self, id: u64) {
        let slot = self.lobby.clone();
        self.client
            .matchmaking()
            .join_lobby(LobbyId::from_raw(id), move |result| {
                if let Ok(id) = result {
                    *slot.lock().unwrap() = Some(id);
                }
            });
    }

    /// The lobby this peer is in, by id.
    pub fn lobby(&self) -> Option<u64> {
        self.lobby.lock().unwrap().map(|l| l.raw())
    }

    /// Open the overlay's invite dialog for the lobby: the friends list.
    pub fn invite(&self) {
        if let Some(lobby) = *self.lobby.lock().unwrap() {
            self.client.friends().activate_invite_dialog(lobby);
        }
    }

    /// Everyone in the lobby, on Steam, and which is its owner — the host.
    /// A guest calls this once in, and connects the owner as the host.
    pub fn lobby_members(&self) -> Option<(Vec<u64>, u64)> {
        let lobby = (*self.lobby.lock().unwrap())?;
        let matchmaking = self.client.matchmaking();
        let members = matchmaking
            .lobby_members(lobby)
            .iter()
            .map(|m| m.raw())
            .collect();
        Some((members, matchmaking.lobby_owner(lobby).raw()))
    }

    /// Unlock an achievement by its API name, as set on Steamworks, and
    /// send it: the pop-up shows at once. `false` when Steam refused (an
    /// unknown name, stats not yet received).
    pub fn unlock(&self, achievement: &str) -> bool {
        let stats = self.client.user_stats();
        stats.achievement(achievement).set().is_ok() && stats.store_stats().is_ok()
    }

    /// Whether an achievement is unlocked; `None` when Steam does not know
    /// it.
    pub fn unlocked(&self, achievement: &str) -> Option<bool> {
        self.client.user_stats().achievement(achievement).get().ok()
    }

    /// Lock an achievement again — for testing a game's unlocks.
    pub fn relock(&self, achievement: &str) -> bool {
        let stats = self.client.user_stats();
        stats.achievement(achievement).clear().is_ok() && stats.store_stats().is_ok()
    }

    /// A whole-number stat by its API name — crates carried, days survived
    /// — set and sent. Steam unlocks the achievements that hang on it.
    pub fn set_stat(&self, name: &str, value: i32) -> bool {
        let stats = self.client.user_stats();
        stats.set_stat_i32(name, value).is_ok() && stats.store_stats().is_ok()
    }

    pub fn stat(&self, name: &str) -> Option<i32> {
        self.client.user_stats().get_stat_i32(name).ok()
    }

    /// What friends see this player doing, by key: `steam_display` with a
    /// localisation token, or any key of the game's own; `None` clears it.
    /// Steam's Rich Presence.
    pub fn set_presence(&self, key: &str, value: Option<&str>) -> bool {
        self.client.friends().set_rich_presence(key, value)
    }

    /// Clear everything this player shows friends.
    pub fn clear_presence(&self) {
        self.client.friends().clear_rich_presence();
    }

    /// A player's picture, 64 pixels square, RGBA: for a name tag over a
    /// friend's head or the lobby's list. `None` until Steam has it.
    pub fn avatar(&self, steam_id: u64) -> Option<(u32, u32, Vec<u8>)> {
        self.client
            .friends()
            .get_friend(SteamId::from_raw(steam_id))
            .medium_avatar()
            .map(|pixels| (64, 64, pixels))
    }

    /// A player's name on Steam.
    pub fn name_of(&self, steam_id: u64) -> String {
        self.client
            .friends()
            .get_friend(SteamId::from_raw(steam_id))
            .name()
    }
}

/// Where Steam Auto-Cloud should look for a game's saves, as the
/// Steamworks page asks for them (Root Overrides): the same folders
/// [`crate::player_prefs::user_dir`] keeps the player's files in, `saves/`
/// under the game's name. One line an OS, `root / subdirectory`.
pub fn auto_cloud_roots(game: &str) -> String {
    format!(
        "Windows: WinAppDataRoaming / {game}/saves\n\
         macOS:   MacAppSupport / {game}/saves\n\
         Linux:   LinuxXdgDataHome / {game}/saves\n\
         Pattern: *.ron"
    )
}

impl Transport for Steam {
    fn send(&mut self, to: PeerId, bytes: Vec<u8>) {
        let Some(steam_id) = self.peers.get(&to) else {
            return;
        };
        let mut frame = self.me.0.to_le_bytes().to_vec();
        frame.extend_from_slice(&bytes);
        let _ = self.client.networking_messages().send_message_to_user(
            NetworkingIdentity::new_steam_id(*steam_id),
            SendFlags::UNRELIABLE_NO_DELAY | SendFlags::AUTO_RESTART_BROKEN_SESSION,
            &frame,
            0,
        );
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        self.client.run_callbacks();
        let mut out = Vec::new();
        for message in self
            .client
            .networking_messages()
            .receive_messages_on_channel(0, 256)
        {
            let data = message.data();
            if data.len() < 4 {
                continue;
            }
            let from = PeerId(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
            // Learn who a peer is from what it sends, as Udp learns where.
            if let Some(steam_id) = message.identity_peer().steam_id() {
                self.peers.entry(from).or_insert(steam_id);
            }
            out.push((from, data[4..].to_vec()));
        }
        out
    }
}
