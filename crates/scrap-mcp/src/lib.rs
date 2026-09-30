//! The editor, for an agent: an MCP server over [`scrap_editor::Session`].
//!
//! DNA, postulate 5: everything the editor can do, an agent can do, because
//! the editor's operations are engine functions and this hands them over.
//! Nothing here edits a scene itself — each tool is a call into the session
//! the GPUI editor will drive, so an agent's edit and a person's are the
//! same edit: one undo step, the same file on save, the same errors.
//!
//! The protocol is JSON-RPC 2.0, a message per line, over stdio.
//! [`Server::handle`] takes one message and returns the reply, so tests
//! drive it without a pipe.

pub mod live;
mod live_tools;
mod tools;

use scrap_editor::Session;
use serde_json::{json, Value};

/// The open document, as a resource.
const DOCUMENT: &str = "scrap://document";

/// The protocol version answered when the client does not name one.
pub const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "\
Edit scrap scenes headlessly. Start with open_scene (or new_project). \
Entities are addressed by the 16-hex-digit ids that scene_tree shows. \
Every edit is one undo step and nothing is written until save_scene. \
render returns a PNG of the view; look at it after changes that matter. \
Run check after editing: it names every model, material or prefab a scene \
refers to that does not exist, with the closest real name.";

pub struct Server {
    session: Option<Session>,
    /// The configs this agent edited, a step each: `config_undo`.
    configs: scrap_editor::configs::Edits,
    size: (u32, u32),
    /// Whose session the tools act on: this server's own, or the live
    /// studio's it is lent ([`Server::handle_on`]).
    mode: Mode,
    /// Draft models from a neural network: a module, made on first use.
    #[cfg(feature = "gen")]
    generator: Option<scrap_gen::Generator>,
}

/// Whose session a server's tools act on, as the `attached` tool says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Its own editor, headless; why no studio was attached to.
    Headless(String),
    /// The session of the studio window a person has open on `project`:
    /// the tools act on what they see ([`live`]).
    Live { project: std::path::PathBuf },
}

/// One tool call's view of the server: the session it acts on — the
/// server's own, made on first use, or one lent by the studio — and what
/// the tools keep between calls.
pub(crate) struct Call<'a> {
    slot: Slot<'a>,
    pub(crate) configs: &'a mut scrap_editor::configs::Edits,
    /// In the live studio: the picture size asked for (`width`, `height`),
    /// which scales the picture rather than resizing the person's view.
    live: Option<Option<(u32, u32)>>,
}

enum Slot<'a> {
    Own(&'a mut Option<Session>, (u32, u32)),
    Lent(&'a mut Session),
}

impl Call<'_> {
    /// The session, made on first use: a GPU is found only when something
    /// needs one, so `tools/list` works on a machine without.
    pub(crate) fn session(&mut self) -> Result<&mut Session, String> {
        match &mut self.slot {
            Slot::Lent(session) => Ok(session),
            Slot::Own(slot, (width, height)) => {
                if slot.is_none() {
                    **slot = Some(Session::offscreen(*width, *height).map_err(|e| e.to_string())?);
                }
                Ok(slot.as_mut().expect("made a line above"))
            }
        }
    }

    /// The session if there is one yet: nothing is made to answer.
    fn session_if_open(&mut self) -> Result<&mut Session, ()> {
        match &mut self.slot {
            Slot::Lent(session) => Ok(session),
            Slot::Own(slot, _) => slot.as_mut().ok_or(()),
        }
    }

    /// `Some` in the live studio, with the picture size asked for.
    pub(crate) fn live_size(&self) -> Option<Option<(u32, u32)>> {
        self.live
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new() -> Self {
        Self {
            session: None,
            configs: scrap_editor::configs::Edits::new(),
            size: (640, 360),
            mode: Mode::Headless("started headless".into()),
            #[cfg(feature = "gen")]
            generator: None,
        }
    }

    /// A server for the live studio: its tools act on the session lent
    /// with each message ([`Server::handle_on`]), which is the one the
    /// person sees, on `project`.
    pub fn live(project: impl Into<std::path::PathBuf>) -> Self {
        Self {
            mode: Mode::Live {
                project: project.into(),
            },
            ..Self::new()
        }
    }

    /// Say why this headless server is not attached to a studio: what
    /// the `attached` tool answers.
    pub fn set_headless_reason(&mut self, why: impl Into<String>) {
        self.mode = Mode::Headless(why.into());
    }

    /// Whether this server has an editor session of its own yet: made by
    /// the first tool that needs one.
    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// `scrap`, or `scrap (live studio)` when the tools act on a window
    /// a person has open: the client sees which in the handshake.
    fn server_name(&self) -> &'static str {
        match self.mode {
            Mode::Headless(_) => "scrap",
            Mode::Live { .. } => "scrap (live studio)",
        }
    }

    fn instructions(&self) -> String {
        match &self.mode {
            Mode::Headless(_) => INSTRUCTIONS.to_string(),
            Mode::Live { project } => format!(
                "{INSTRUCTIONS} This server is the live scrap studio a person has open on {}: \
                 every tool acts on the window they see — the same document, undo \
                 history, Game view and Console — and they see each edit at once and \
                 can undo it. The scene is already open; do not save unless asked. \
                 start_game is the Play button, game_view shows what the Game view \
                 shows, stop_game stops it.",
                project.display()
            ),
        }
    }

    /// Answer one message. `None` for a notification, which gets no reply.
    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        self.handle_with(None, message)
    }

    /// Answer one message on `session`, lent for it: the live studio's,
    /// the one the person sees and edits. What the tools do is done to it
    /// — the same document, undo history, Game view and Console.
    pub fn handle_on(&mut self, session: &mut Session, message: &Value) -> Option<Value> {
        self.handle_with(Some(session), message)
    }

    fn handle_with(&mut self, mut lent: Option<&mut Session>, message: &Value) -> Option<Value> {
        let id = message.get("id")?.clone();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL),
                "capabilities": { "tools": {}, "resources": {} },
                "serverInfo": { "name": self.server_name(), "version": env!("CARGO_PKG_VERSION") },
                "instructions": self.instructions(),
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": Self::tools() })),
            "tools/call" => Ok(self.call(lent.as_deref_mut(), &params)),
            "resources/list" => Ok(json!({ "resources": self.resources(lent.as_deref()) })),
            "resources/read" => self
                .read(lent.as_deref(), &params)
                .map(|contents| json!({ "contents": [contents] })),
            other => Err((-32601, format!("no method `{other}`"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": message },
            }),
        })
    }

    /// Run a tool. A tool that fails is still an answer — `isError` with a
    /// sentence saying why — because the agent is the one who can fix it.
    fn call(&mut self, mut lent: Option<&mut Session>, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let empty = json!({});
        let mut arguments = params.get("arguments").unwrap_or(&empty).clone();
        #[cfg(feature = "gen")]
        {
            let session = match lent.as_deref_mut() {
                Some(session) => Some(session),
                None => self.session.as_mut(),
            };
            if let (Some(generator), Some(session)) = (&mut self.generator, session) {
                generator.poll(session);
            }
        }
        #[cfg(feature = "gen")]
        if let Some(answer) = self.call_gen(lent.as_deref_mut(), name, &arguments) {
            return match answer {
                Ok(line) => json!({ "content": [text(line)], "isError": false }),
                Err(message) => json!({ "content": [text(message)], "isError": true }),
            };
        }
        let live = lent.is_some();
        let mut call = Call {
            slot: match lent {
                Some(session) => Slot::Lent(session),
                None => Slot::Own(&mut self.session, self.size),
            },
            configs: &mut self.configs,
            live: live.then(|| live_tools::take_size(&mut arguments)),
        };
        let answer = match live_tools::call(&mut call, &self.mode, name, &arguments) {
            Some(answer) => answer,
            None => tools::call(&mut call, name, &arguments),
        };
        // A tool may have written files: what the session read from the
        // project is read again at the next ask. Not after one that only
        // looks or edits the document — in the studio the next frame would
        // read the whole project's listing again for nothing.
        if !(live && live_tools::writes_no_files(name)) {
            if let Ok(session) = call.session_if_open() {
                session.wrote();
            }
        }
        match answer {
            Ok(content) => json!({ "content": content, "isError": false }),
            Err(message) => json!({ "content": [text(message)], "isError": true }),
        }
    }

    /// The engine's tools, and those of the modules built in.
    fn tools() -> Vec<Value> {
        #[allow(unused_mut)]
        let mut all = tools::list();
        all.extend(live_tools::list());
        #[cfg(feature = "gen")]
        all.extend(scrap_gen::mcp::list());
        all
    }

    /// A tool of the `gen` module, or `None` for one of anybody else's.
    #[cfg(feature = "gen")]
    fn call_gen(
        &mut self,
        lent: Option<&mut Session>,
        name: &str,
        arguments: &Value,
    ) -> Option<Result<String, String>> {
        if !scrap_gen::mcp::list().iter().any(|t| t["name"] == name) {
            return None;
        }
        if self.generator.is_none() {
            match scrap_gen::Generator::from_env() {
                Ok(generator) => self.generator = Some(generator),
                Err(e) if name == "generate" => return Some(Err(format!("{e:#}"))),
                Err(_) => {}
            }
        }
        let session = match lent {
            Some(session) => session,
            None => {
                if let Err(e) = self.session() {
                    return Some(Err(e));
                }
                self.session.as_mut().expect("made just above")
            }
        };
        let generator = self
            .generator
            .as_mut()
            .ok_or_else(|| "no generator: FAL_KEY is not set".to_string());
        scrap_gen::mcp::call(generator, session, name, arguments)
    }

    /// What there is to read: the open document as it stands — unsaved
    /// edits included — and the project's scenes, prefabs, materials and
    /// tuning numbers as files. Empty until something is open; listing
    /// never makes a GPU.
    fn resources(&self, lent: Option<&Session>) -> Vec<Value> {
        let Some(session) = lent.or(self.session.as_ref()) else {
            return Vec::new();
        };
        let mut out = vec![json!({
            "uri": DOCUMENT,
            "name": "the open document",
            "description": "the scene or prefab being edited, as RON, with edits not yet saved",
            "mimeType": "text/plain",
        })];
        let Some(project) = session.project() else {
            return out;
        };
        let mut files = Vec::new();
        for kind in [
            scrap::layout::Kind::Scene,
            scrap::layout::Kind::Prefab,
            scrap::layout::Kind::Material,
        ] {
            files.extend(project.files(kind));
        }
        files.extend(scrap::layout::data_files(project.root()));
        files.sort();
        for path in files {
            let name = project.relative(&path).unwrap_or_default();
            out.push(json!({
                "uri": format!("file://{}", path.display()),
                "name": name,
                "mimeType": "text/plain",
            }));
        }
        out
    }

    /// One resource's text. Files only from inside the open project.
    fn read(&self, lent: Option<&Session>, params: &Value) -> Result<Value, (i64, String)> {
        let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
        let session = lent
            .or(self.session.as_ref())
            .ok_or((-32002, "nothing is open yet — open_scene first".to_string()))?;
        let text = if uri == DOCUMENT {
            let pretty = scrap::ron::ser::PrettyConfig::new().depth_limit(4);
            let scene = session.scene();
            let written = if session.is_prefab() && scene.entities.len() == 1 {
                scrap::ron::ser::to_string_pretty(&scene.entities[0], pretty)
            } else {
                scrap::ron::ser::to_string_pretty(scene, pretty)
            };
            written.map_err(|e| (-32603, e.to_string()))?
        } else {
            let path = uri
                .strip_prefix("file://")
                .ok_or((-32002, format!("no resource {uri}")))?;
            let root = session
                .project()
                .and_then(|p| std::fs::canonicalize(p.root()).ok())
                .ok_or((-32002, "the open scene is in no project".to_string()))?;
            let path = std::fs::canonicalize(path).map_err(|e| (-32002, format!("{path}: {e}")))?;
            if !path.starts_with(&root) {
                return Err((-32002, format!("{} is outside the project", path.display())));
            }
            std::fs::read_to_string(&path)
                .map_err(|e| (-32002, format!("{}: {e}", path.display())))?
        };
        Ok(json!({ "uri": uri, "mimeType": "text/plain", "text": text }))
    }

    /// The session, made on first use: a GPU is found only when something
    /// needs one, so `tools/list` works on a machine without.
    #[cfg(feature = "gen")]
    fn session(&mut self) -> Result<&mut Session, String> {
        if self.session.is_none() {
            let (width, height) = self.size;
            self.session = Some(Session::offscreen(width, height).map_err(|e| e.to_string())?);
        }
        Ok(self.session.as_mut().expect("made a line above"))
    }
}

fn text(message: impl Into<String>) -> Value {
    json!({ "type": "text", "text": message.into() })
}

fn image(png: &[u8]) -> Value {
    json!({ "type": "image", "data": base64(png), "mimeType": "image/png" })
}

/// Standard base64 with padding, for image content.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> shift) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_notification_gets_no_reply_and_an_unknown_method_an_error() {
        let mut server = Server::new();
        let notification = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(server.handle(&notification).is_none());
        let reply = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 7, "method": "prompts/list" }))
            .unwrap();
        assert_eq!(reply["error"]["code"], -32601);
        assert_eq!(reply["id"], 7);
    }
}
