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
    size: (u32, u32),
    /// Draft models from a neural network: a module, made on first use.
    #[cfg(feature = "gen")]
    generator: Option<scrap_gen::Generator>,
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
            size: (640, 360),
            #[cfg(feature = "gen")]
            generator: None,
        }
    }

    /// Answer one message. `None` for a notification, which gets no reply.
    pub fn handle(&mut self, message: &Value) -> Option<Value> {
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
                "serverInfo": { "name": "scrap", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": Self::tools() })),
            "tools/call" => Ok(self.call(&params)),
            "resources/list" => Ok(json!({ "resources": self.resources() })),
            "resources/read" => self
                .read(&params)
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
    fn call(&mut self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let empty = json!({});
        let arguments = params.get("arguments").unwrap_or(&empty);
        #[cfg(feature = "gen")]
        if let (Some(generator), Some(session)) = (&mut self.generator, &mut self.session) {
            generator.poll(session);
        }
        #[cfg(feature = "gen")]
        if let Some(answer) = self.call_gen(name, arguments) {
            return match answer {
                Ok(line) => json!({ "content": [text(line)], "isError": false }),
                Err(message) => json!({ "content": [text(message)], "isError": true }),
            };
        }
        match tools::call(self, name, arguments) {
            Ok(content) => json!({ "content": content, "isError": false }),
            Err(message) => json!({ "content": [text(message)], "isError": true }),
        }
    }

    /// The engine's tools, and those of the modules built in.
    fn tools() -> Vec<Value> {
        #[allow(unused_mut)]
        let mut all = tools::list();
        #[cfg(feature = "gen")]
        all.extend(scrap_gen::mcp::list());
        all
    }

    /// A tool of the `gen` module, or `None` for one of anybody else's.
    #[cfg(feature = "gen")]
    fn call_gen(&mut self, name: &str, arguments: &Value) -> Option<Result<String, String>> {
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
        if let Err(e) = self.session() {
            return Some(Err(e));
        }
        let session = self.session.as_mut().expect("made just above");
        let generator = self
            .generator
            .as_mut()
            .ok_or_else(|| "no generator: FAL_KEY is not set".to_string());
        scrap_gen::mcp::call(generator, session, name, arguments)
    }

    /// What there is to read: the open document as it stands — unsaved
    /// edits included — and the project's scenes, prefabs and materials as
    /// files. Empty until something is open; listing never makes a GPU.
    fn resources(&self) -> Vec<Value> {
        let Some(session) = &self.session else {
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
        for (dir, extension) in [
            (project.scenes(), "ron"),
            (project.prefabs(), "prefab"),
            (project.materials(), "scrmat"),
        ] {
            scrap_import::walk(&dir, &mut |path| {
                if path.extension().is_some_and(|e| e == extension) {
                    files.push(path.to_path_buf());
                }
            });
        }
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
    fn read(&self, params: &Value) -> Result<Value, (i64, String)> {
        let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
        let session = self
            .session
            .as_ref()
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
