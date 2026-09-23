//! What a player is doing, on their Discord profile — "In the garden,
//! 2 of 4" — through the Discord client running beside the game. Behind
//! the `discord` feature; without Discord running it says so and nothing
//! else happens.

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

/// A connection to the Discord client, for one application id (made on
/// Discord's developer portal).
pub struct Discord {
    client: DiscordIpcClient,
    started: i64,
}

/// What to show: two lines, how many are playing together, and the
/// picture (an asset name uploaded for the application).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Presence {
    pub details: String,
    pub state: String,
    /// (in the party, most there can be).
    pub party: Option<(u32, u32)>,
    pub image: String,
}

impl Discord {
    /// Connect to the Discord client. In words when it is not running.
    pub fn connect(application: &str) -> Result<Self, String> {
        let mut client = DiscordIpcClient::new(application).map_err(|e| e.to_string())?;
        client
            .connect()
            .map_err(|e| format!("Discord is not running, or would not talk: {e}"))?;
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        Ok(Self { client, started })
    }

    /// Show this, with the time since the game started.
    pub fn show(&mut self, presence: &Presence) -> Result<(), String> {
        let mut shown = activity::Activity::new()
            .details(&presence.details)
            .state(&presence.state)
            .timestamps(activity::Timestamps::new().start(self.started));
        if let Some((size, most)) = presence.party {
            shown = shown.party(activity::Party::new().size([size as i32, most as i32]));
        }
        if !presence.image.is_empty() {
            shown = shown.assets(activity::Assets::new().large_image(&presence.image));
        }
        self.client.set_activity(shown).map_err(|e| e.to_string())
    }

    /// Show nothing.
    pub fn clear(&mut self) -> Result<(), String> {
        self.client.clear_activity().map_err(|e| e.to_string())
    }
}

impl Drop for Discord {
    fn drop(&mut self) {
        let _ = self.client.close();
    }
}
