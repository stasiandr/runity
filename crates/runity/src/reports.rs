//! Sending what a game learns about itself: crash reports to Sentry and
//! play to GameAnalytics. Behind the `reports` feature — the network, TLS
//! and the player's consent, which a game asks for and this does not.
//!
//! What goes over the wire is built by plain functions ([`sentry_envelope`],
//! [`GameAnalytics::body`], [`game_analytics_auth`]) the tests read; the
//! sending is one call each, and never happens on its own.

use std::time::Duration;

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::crash::Report;

/// Where a Sentry project takes events, from its DSN
/// (`https://KEY@HOST/PROJECT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dsn {
    pub key: String,
    pub scheme: String,
    pub host: String,
    pub project: String,
    text: String,
}

impl Dsn {
    pub fn parse(dsn: &str) -> Result<Self, String> {
        let bad = || format!("`{dsn}` is not a Sentry DSN (https://KEY@HOST/PROJECT)");
        let (scheme, rest) = dsn.split_once("://").ok_or_else(bad)?;
        let (key, rest) = rest.split_once('@').ok_or_else(bad)?;
        let (host, project) = rest.rsplit_once('/').ok_or_else(bad)?;
        if key.is_empty() || host.is_empty() || project.is_empty() {
            return Err(bad());
        }
        Ok(Self {
            key: key.split(':').next().unwrap_or(key).to_string(),
            scheme: scheme.to_string(),
            host: host.to_string(),
            project: project.to_string(),
            text: dsn.to_string(),
        })
    }

    fn url(&self) -> String {
        format!(
            "{}://{}/api/{}/envelope/",
            self.scheme, self.host, self.project
        )
    }
}

/// A crash report as a Sentry envelope: a header line, an item header
/// line, the event.
pub fn sentry_envelope(dsn: &Dsn, report: &Report, event_id: &str) -> String {
    let event = json!({
        "event_id": event_id,
        "timestamp": report.at,
        "platform": "native",
        "level": "fatal",
        "release": format!("{}@{}", report.game, report.version),
        "tags": { "os": report.os },
        "exception": { "values": [{
            "type": "panic",
            "value": report.message,
            "mechanism": { "type": "panic", "handled": false },
        }]},
        "extra": { "location": report.location, "backtrace": report.backtrace },
    });
    format!(
        "{}\n{}\n{}\n",
        json!({ "event_id": event_id, "dsn": dsn.text }),
        json!({ "type": "event", "content_type": "application/json" }),
        event
    )
}

/// Send a crash report to Sentry. The player said yes first.
pub fn send_to_sentry(dsn: &Dsn, report: &Report) -> Result<(), String> {
    let event_id = format!(
        "{:032x}",
        crate::id::EntityId::fresh().raw() as u128 * 0x9e37_79b9
    );
    let body = sentry_envelope(dsn, report, &event_id);
    ureq::post(&dsn.url())
        .timeout(Duration::from_secs(10))
        .set("Content-Type", "application/x-sentry-envelope")
        .set(
            "X-Sentry-Auth",
            &format!(
                "Sentry sentry_version=7, sentry_key={}, sentry_client=runity/{}",
                dsn.key,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .send_string(&body)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// GameAnalytics signs a request with its secret: the body's HMAC-SHA256,
/// in base64.
pub fn game_analytics_auth(secret: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(body.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// Play reported to GameAnalytics: design events — `Harvest:Carrot` with a
/// value — queued, and sent together on [`GameAnalytics::flush`].
#[derive(Debug, Clone)]
pub struct GameAnalytics {
    pub game_key: String,
    secret: String,
    /// The player, anonymously: stable across sessions, meaning nothing.
    pub user: String,
    pub session: String,
    pub session_number: u32,
    pub build: String,
    queued: Vec<Value>,
}

impl GameAnalytics {
    pub fn new(game_key: &str, secret: &str, user: &str, build: &str, session_number: u32) -> Self {
        Self {
            game_key: game_key.into(),
            secret: secret.into(),
            user: user.into(),
            session: format!("{:016x}", crate::id::EntityId::fresh().raw()),
            session_number,
            build: build.into(),
            queued: Vec::new(),
        }
    }

    fn common(&self, category: &str) -> Value {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        json!({
            "category": category,
            "v": 2,
            "user_id": self.user,
            "client_ts": now,
            "sdk_version": format!("rest api v2 runity {}", env!("CARGO_PKG_VERSION")),
            "os_version": std::env::consts::OS,
            "manufacturer": "unknown",
            "device": "desktop",
            "platform": std::env::consts::OS,
            "session_id": self.session,
            "session_num": self.session_number,
            "build": self.build,
        })
    }

    /// The session began: the first event of each.
    pub fn start(&mut self) {
        let event = self.common("user");
        self.queued.push(event);
    }

    /// Something that happened in the game, `Part:Part:Part`, with a
    /// number if it has one: `design("Harvest:Carrot", Some(3.0))`.
    pub fn design(&mut self, event_id: &str, value: Option<f64>) {
        let mut event = self.common("design");
        event["event_id"] = json!(event_id);
        if let Some(value) = value {
            event["value"] = json!(value);
        }
        self.queued.push(event);
    }

    /// The session is over, after this long.
    pub fn end(&mut self, seconds: u64) {
        let mut event = self.common("session_end");
        event["length"] = json!(seconds);
        self.queued.push(event);
    }

    /// What would be sent now.
    pub fn body(&self) -> String {
        Value::Array(self.queued.clone()).to_string()
    }

    pub fn queued(&self) -> usize {
        self.queued.len()
    }

    /// Send what is queued; kept for next time if it did not go.
    pub fn flush(&mut self) -> Result<(), String> {
        if self.queued.is_empty() {
            return Ok(());
        }
        let body = self.body();
        ureq::post(&format!(
            "https://api.gameanalytics.com/v2/{}/events",
            self.game_key
        ))
        .timeout(Duration::from_secs(10))
        .set("Content-Type", "application/json")
        .set("Authorization", &game_analytics_auth(&self.secret, &body))
        .send_string(&body)
        .map_err(|e| e.to_string())?;
        self.queued.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            at: 1_790_000_000,
            message: "the well ran dry".into(),
            location: "src/well.rs:12:5".into(),
            backtrace: "0: well::draw".into(),
            game: "dacha".into(),
            version: "0.9.1".into(),
            os: "macos aarch64".into(),
        }
    }

    #[test]
    fn a_dsn_names_its_key_host_and_project() {
        let dsn = Dsn::parse("https://abc123@o42.ingest.sentry.io/7").unwrap();
        assert_eq!(
            (dsn.key.as_str(), dsn.host.as_str(), dsn.project.as_str()),
            ("abc123", "o42.ingest.sentry.io", "7")
        );
        assert_eq!(dsn.url(), "https://o42.ingest.sentry.io/api/7/envelope/");
        assert!(Dsn::parse("not a dsn").is_err());
    }

    #[test]
    fn a_crash_becomes_an_envelope_of_three_lines() {
        let dsn = Dsn::parse("https://abc123@o42.ingest.sentry.io/7").unwrap();
        let text = sentry_envelope(&dsn, &report(), "0123456789abcdef0123456789abcdef");
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1]["type"], "event");
        assert_eq!(
            lines[2]["exception"]["values"][0]["value"],
            "the well ran dry"
        );
        assert_eq!(lines[2]["release"], "dacha@0.9.1");
        assert_eq!(lines[2]["level"], "fatal");
    }

    #[test]
    fn game_analytics_signs_the_body_with_its_secret() {
        // RFC 4231's second test case, as base64.
        assert_eq!(
            game_analytics_auth("Jefe", "what do ya want for nothing?"),
            "W9zBRr9gdU5qBCQmCJV1x1oAPwidJzmDnexYuWTsOEM="
        );
        let mut ga = GameAnalytics::new("key", "secret", "player-1", "0.9.1", 3);
        ga.start();
        ga.design("Harvest:Carrot", Some(3.0));
        let body: Value = serde_json::from_str(&ga.body()).unwrap();
        assert_eq!(body[1]["category"], "design");
        assert_eq!(body[1]["event_id"], "Harvest:Carrot");
        assert_eq!(body[1]["session_num"], 3);
        assert_eq!(ga.queued(), 2);
    }
}
