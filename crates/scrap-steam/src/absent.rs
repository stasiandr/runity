//! Steam where there is none — a phone, a browser: the same API as on the
//! desktop (`steam.rs`), so a game with the module listed builds for every
//! platform, and [`Steam::init`] says, in words, that it is not there. No
//! value of [`Steam`] or [`SteamWire`] can be made, so nothing else is
//! ever called.

use crate::net::{PeerId, Transport};

/// Steam as the wire, and its lobby: never made here.
pub struct Steam {
    never: std::convert::Infallible,
}

impl Steam {
    pub fn init(_me: PeerId) -> Result<Self, String> {
        Err("steam: not on this platform".into())
    }

    pub fn init_app(_app_id: u32, me: PeerId) -> Result<Self, String> {
        Self::init(me)
    }

    pub fn steam_id(&self) -> u64 {
        match self.never {}
    }

    pub fn connect(&mut self, _peer: PeerId, _steam_id: u64) {
        match self.never {}
    }

    pub fn wire(&self, _me: PeerId) -> SteamWire {
        match self.never {}
    }

    pub fn run_callbacks(&self) {
        match self.never {}
    }

    pub fn me(&self) -> PeerId {
        match self.never {}
    }

    pub fn leave_lobby(&self) {
        match self.never {}
    }

    pub fn open_friends(&self) {
        match self.never {}
    }

    pub fn host_lobby(&self, _max: u32, _name: &str) {
        match self.never {}
    }

    pub fn join_lobby(&self, _id: u64) {
        match self.never {}
    }

    pub fn lobby(&self) -> Option<u64> {
        match self.never {}
    }

    pub fn invite(&self) {
        match self.never {}
    }

    pub fn lobby_members(&self) -> Option<(Vec<u64>, u64)> {
        match self.never {}
    }

    pub fn unlock(&self, _achievement: &str) -> bool {
        match self.never {}
    }

    pub fn unlocked(&self, _achievement: &str) -> Option<bool> {
        match self.never {}
    }

    pub fn relock(&self, _achievement: &str) -> bool {
        match self.never {}
    }

    pub fn set_stat(&self, _name: &str, _value: i32) -> bool {
        match self.never {}
    }

    pub fn stat(&self, _name: &str) -> Option<i32> {
        match self.never {}
    }

    pub fn set_presence(&self, _key: &str, _value: Option<&str>) -> bool {
        match self.never {}
    }

    pub fn clear_presence(&self) {
        match self.never {}
    }

    pub fn avatar(&self, _steam_id: u64) -> Option<(u32, u32, Vec<u8>)> {
        match self.never {}
    }

    pub fn name_of(&self, _steam_id: u64) -> String {
        match self.never {}
    }
}

/// Steam Cloud's roots: none where there is no Steam.
pub fn auto_cloud_roots(_game: &str) -> String {
    String::new()
}

/// Steam's networking messages as a transport: never made here.
pub struct SteamWire {
    never: std::convert::Infallible,
}

impl SteamWire {
    pub fn connect(&mut self, _peer: PeerId, _steam_id: u64) {
        match self.never {}
    }
}

impl Transport for SteamWire {
    fn send(&mut self, _to: PeerId, _bytes: Vec<u8>) {
        match self.never {}
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        match self.never {}
    }
}

impl Transport for Steam {
    fn send(&mut self, _to: PeerId, _bytes: Vec<u8>) {
        match self.never {}
    }

    fn receive(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        match self.never {}
    }
}
