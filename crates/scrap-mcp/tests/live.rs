//! The live studio, driven the way an agent drives it: a session with the
//! studio's socket open, as a window has it, and an agent at the other end
//! — over the socket itself, and through the `scrap-mcp` binary, which
//! passes its stdio on to the socket when a studio serves the project and
//! is its own headless editor when none does.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use scrap_editor::Session;
use scrap_mcp::live;
use serde_json::{json, Value};

/// A fresh folder of its own for each test: the editor's config folder and
/// a project in it.
fn place(name: &str) -> (PathBuf, PathBuf) {
    // Short: a Unix socket's path has room for about a hundred bytes.
    let base = std::env::temp_dir().join(format!("sl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let config = base.join("cfg");
    std::fs::create_dir_all(&config).unwrap();
    let project = scrap::Project::create(base.join("game"), "live").unwrap();
    (config, project.root().to_path_buf())
}

/// A session with the project's main scene open, as the studio opens it;
/// `None` on a machine without a GPU.
fn studio_session(root: &Path) -> Option<Session> {
    let mut session = match Session::offscreen(320, 180) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let project = scrap::Project::open(root).unwrap();
    let scene = project
        .scene("main")
        .expect("a new project has a main scene");
    session.open_scene(&scene).unwrap();
    Some(session)
}

/// Serve the studio's frame loop until `done` says the agent is finished,
/// as the window does between frames.
fn pump(listener: &mut live::Listener, session: &mut Session, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done() {
        listener.serve(session);
        assert!(Instant::now() < deadline, "the agent never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    listener.serve(session);
}

fn requests() -> Vec<Value> {
    vec![
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "add_entity",
            "arguments": { "name": "from-agent", "model": "builtin:cube", "position": [0.0, 1.0, 0.0] },
        }}),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "attached", "arguments": {} } }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
            "name": "render", "arguments": { "width": 100 },
        }}),
        json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "game_view", "arguments": {} } }),
    ]
}

/// The text of a tool call's reply.
fn said(reply: &Value) -> String {
    reply["result"]["content"]
        .as_array()
        .unwrap_or_else(|| panic!("not a tool's reply: {reply}"))
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_image(reply: &Value) -> bool {
    reply["result"]["content"]
        .as_array()
        .is_some_and(|c| c.iter().any(|c| c["type"] == "image"))
}

/// What every agent below checks of the replies to [`requests`].
fn check_replies(replies: &[Value]) {
    assert_eq!(
        replies.len(),
        5,
        "a reply per request, none for the notification"
    );
    assert_eq!(
        replies[0]["result"]["serverInfo"]["name"],
        "scrap (live studio)"
    );
    let id = said(&replies[1]);
    assert_eq!(id.len(), 16, "an entity id: {id}");
    assert!(
        said(&replies[2]).starts_with("attached: the live scrap studio"),
        "{}",
        said(&replies[2])
    );
    assert!(has_image(&replies[3]), "{}", replies[3]);
    let render = said(&replies[3]);
    assert!(
        render.starts_with("100x56 (the studio's Scene view, 320x180)"),
        "{render}"
    );
    assert!(has_image(&replies[4]), "{}", replies[4]);
    assert!(
        said(&replies[4]).contains("no game is playing"),
        "{}",
        said(&replies[4])
    );
}

/// The edit the agent made is in the session's own document — the one the
/// window shows — and one undo takes it back, as the person would.
fn check_edit_is_the_person_s(session: &mut Session) {
    let named = |s: &Session| s.scene().entities.iter().any(|e| e.name == "from-agent");
    assert!(
        named(session),
        "the agent's cube is in the studio's document"
    );
    assert!(session.is_modified());
    session.undo().unwrap();
    assert!(!named(session), "and the person's undo takes it back");
    // The view the studio draws is the size it was: the picture was
    // scaled, the view not resized.
    assert_eq!(session.size(), (320, 180));
}

#[test]
fn an_edit_over_the_studio_s_socket_is_in_its_document_and_undoable() {
    let (config, root) = place("socket");
    let Some(mut session) = studio_session(&root) else {
        return;
    };
    let mut listener = live::Listener::open(&config, &root, || {}).unwrap();
    let socket = listener.socket().to_path_buf();
    assert_eq!(socket, live::socket_path(&config, &root));
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "only its owner may talk to the editor");
    // The registry says which project, and scrap-mcp finds it by it — from
    // the project, and from a folder holding it.
    let found = live::find(&config, None, &root).unwrap();
    assert_eq!(found.project, std::fs::canonicalize(&root).unwrap());
    assert_eq!(
        live::find(&config, None, root.parent().unwrap()).unwrap(),
        found
    );

    let agent = std::thread::spawn(move || {
        let stream = UnixStream::connect(&socket).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut replies = Vec::new();
        for request in requests() {
            writeln!(writer, "{request}").unwrap();
            if request.get("id").is_none() {
                continue;
            }
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(reply["id"], request["id"]);
            replies.push(reply);
        }
        replies
    });
    pump(&mut listener, &mut session, || agent.is_finished());
    check_replies(&agent.join().unwrap());
    check_edit_is_the_person_s(&mut session);

    // Closed, it leaves nothing behind to be mistaken for a studio.
    drop(listener);
    assert!(!live::socket_path(&config, &root).exists());
    assert!(!live::registry_path(&config, &root).exists());
    assert!(live::studios(&config).is_empty());
}

#[test]
fn scrap_mcp_passes_its_lines_to_a_studio_serving_the_project() {
    let (config, root) = place("proxy");
    let Some(mut session) = studio_session(&root) else {
        return;
    };
    let mut listener = live::Listener::open(&config, &root, || {}).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_scrap-mcp"))
        .env("SCRAP_CONFIG_DIR", &config)
        .env_remove("SCRAP_PROJECT")
        .env_remove("SCRAP_MCP_HEADLESS")
        // Started in the project, as `claude mcp add scrap -- scrap-mcp`
        // is from the game's folder.
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let agent = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut replies = Vec::new();
        for request in requests() {
            writeln!(stdin, "{request}").unwrap();
            stdin.flush().unwrap();
            if request.get("id").is_none() {
                continue;
            }
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            replies.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        drop(stdin);
        replies
    });
    pump(&mut listener, &mut session, || agent.is_finished());
    check_replies(&agent.join().unwrap());
    check_edit_is_the_person_s(&mut session);
    let _ = child.wait();
}

#[test]
fn scrap_mcp_is_its_own_editor_when_no_studio_serves_the_project() {
    let (config, root) = place("alone");
    // A studio that died without tidying up: its socket and registry are
    // still there, and nobody answers.
    let stale = live::socket_path(&config, &root);
    drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
    std::fs::write(
        live::registry_path(&config, &root),
        json!({ "project": root.to_string_lossy(), "pid": 1, "socket": stale.to_string_lossy() })
            .to_string(),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_scrap-mcp"))
        .env("SCRAP_CONFIG_DIR", &config)
        .arg("--project")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |request: Value| {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    let hello = ask(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
    assert_eq!(hello["result"]["serverInfo"]["name"], "scrap");
    let attached = said(&ask(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "attached", "arguments": {} } }),
    ));
    assert!(attached.starts_with("headless:"), "{attached}");
    assert!(attached.contains("no scrap studio is open"), "{attached}");
    // The stale studio was tidied away on the way.
    assert!(!stale.exists());
    assert!(!live::registry_path(&config, &root).exists());
    drop(stdin);
    let _ = child.wait();
}
