//! The editor driven the way an agent drives it: JSON in, JSON out.
//!
//! A project is made, a scene edited, looked at, checked, simulated and
//! saved, and the file on disk is what the edits said — all through the
//! protocol, with nothing but the messages an MCP client would send.

use scrap_mcp::Server;
use serde_json::{json, Value};

struct Agent {
    server: Server,
    next: u64,
}

impl Agent {
    fn new() -> Self {
        Self {
            server: Server::new(),
            next: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let reply = self
            .server
            .handle(
                &json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params }),
            )
            .expect("a request gets a reply");
        assert_eq!(reply["id"], self.next);
        reply
    }

    /// Call a tool; `Err` with its text when it reports an error.
    fn call(&mut self, name: &str, arguments: Value) -> Result<Vec<Value>, String> {
        let reply = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = &reply["result"];
        let content = result["content"].as_array().unwrap().clone();
        if result["isError"] == true {
            return Err(content[0]["text"].as_str().unwrap().to_string());
        }
        Ok(content)
    }

    fn text(&mut self, name: &str, arguments: Value) -> String {
        let content = self
            .call(name, arguments)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        content
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn decode_base64(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            _ => 63,
        }
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let n = chunk
            .iter()
            .map(|&c| if c == b'=' { 0 } else { value(c) })
            .fold(0u32, |acc, v| (acc << 6) | v);
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    out
}

#[test]
fn the_handshake_lists_the_tools_without_needing_a_gpu() {
    let mut agent = Agent::new();
    let reply = agent.request("initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert!(reply["result"]["capabilities"]["tools"].is_object());
    let tools = agent.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "open_scene",
        "add_entity",
        "render",
        "check",
        "simulate",
        "undo",
        "make_variant",
        "rename_asset",
        "usages",
        "assets",
        "delete_asset",
        "duplicate_asset",
        "find",
        "open_prefab",
        "problems",
        "push_face",
        "paint_foliage",
        "fence",
        "wire",
        "array",
        "edits",
        "measure",
        "align",
        "replace_with_prefab",
        "inspect",
        "set_field",
        "hide",
        "isolate",
        "place",
        "to_view",
        "override_field",
        "console",
        "game_console",
        "group",
        "thumbnail",
        "drop",
        "add_component",
        "import_settings",
        "fit_collider",
        "table",
        "set_cell",
        "undo_cell",
        "configs",
        "config_set",
        "config_undo",
    ] {
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
    for tool in tools["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    }
}

#[test]
fn an_agent_builds_a_scene_looks_at_it_checks_it_and_saves_it() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-agent");
    let _ = std::fs::remove_dir_all(&root);

    let made = match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(content) => content,
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    assert!(made[0]["text"].as_str().unwrap().contains("made project"));

    // A tower of one crate, placed and coloured in one step.
    let crate_id = agent.text(
        "add_entity",
        json!({
            "name": "tower",
            "model": "builtin:cube",
            "position": [0.0, 4.0, 0.0],
            "material": "bark",
            "body": "Dynamic",
            "collider": "Box(half: (0.5, 0.5, 0.5))",
        }),
    );
    assert_eq!(crate_id.len(), 16, "an id: {crate_id}");
    let tree = agent.text("scene_tree", json!({}));
    let line = tree.lines().find(|l| l.contains("\"tower\"")).unwrap();
    assert!(line.starts_with(&crate_id), "{line}");
    assert!(
        line.contains("material=bark") && line.contains("(0.00, 4.00, 0.00)"),
        "{line}"
    );

    // One step, so one undo takes all of it back, and redo returns it.
    assert_eq!(agent.text("undo", json!({})), "undone: add `tower`");
    assert!(!agent.text("scene_tree", json!({})).contains("\"tower\""));
    assert_eq!(agent.text("redo", json!({})), "redone: add `tower`");

    // A material name nothing answers to is legal — it draws grey — and
    // `check` is where it is caught, with the fix.
    agent.text(
        "update_entity",
        json!({ "id": crate_id, "material": "bakr" }),
    );
    let now = agent.text("problems", json!({}));
    assert!(
        now.contains("no material named `bakr`") && now.contains("did you mean `bark`?"),
        "known before saving: {now}"
    );
    agent.text("save_scene", json!({}));
    let findings = agent.text("check", json!({}));
    assert!(findings.contains("no material named `bakr`"), "{findings}");
    assert!(findings.contains("did you mean `bark`?"), "{findings}");
    agent.text(
        "update_entity",
        json!({ "id": crate_id, "material": "bark" }),
    );

    // Bad arguments are answered in words, and leave no step behind.
    let err = agent
        .call(
            "update_entity",
            json!({ "id": crate_id, "position": [1, 2] }),
        )
        .unwrap_err();
    assert!(err.contains("position is [x, y, z]"), "{err}");
    let err = agent
        .call("delete_entity", json!({ "id": "nope" }))
        .unwrap_err();
    assert!(err.contains("16 hex digits"), "{err}");

    // It looks at what it made.
    let content = agent
        .call(
            "render",
            json!({ "focus": crate_id, "width": 160, "height": 90 }),
        )
        .unwrap();
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["mimeType"], "image/png");
    let png = decode_base64(content[0]["data"].as_str().unwrap());
    let decoded = image::load_from_memory(&png).expect("a PNG");
    assert_eq!((decoded.width(), decoded.height()), (160, 90));

    // The crate falls when played, and the document does not change.
    let ground = agent
        .text("scene_tree", json!({}))
        .lines()
        .find(|l| l.contains("\"ground\""))
        .unwrap()[..16]
        .to_string();
    agent.text(
        "update_entity",
        json!({ "id": ground, "body": "Static", "collider": "Box(half: (20.0, 0.05, 20.0))" }),
    );
    let report = agent.text("simulate", json!({ "seconds": 2.0 }));
    let fell = report
        .lines()
        .find(|l| l.contains("\"tower\""))
        .unwrap_or_else(|| panic!("{report}"));
    assert!(!fell.contains("4.00, 0.00)"), "it moved: {fell}");
    assert!(agent
        .text("scene_tree", json!({}))
        .contains("(0.00, 4.00, 0.00)"));
    // Kept: the tower stays where it fell, and undo takes it back up.
    let tower = fell[..16].to_string();
    let report = agent.text("simulate", json!({ "seconds": 2.0, "keep": [tower] }));
    assert!(report.contains("kept where they fell: 1"), "{report}");
    assert!(!agent
        .text("scene_tree", json!({}))
        .contains("(0.00, 4.00, 0.00)"));
    agent.text("undo", json!({}));
    assert!(agent
        .text("scene_tree", json!({}))
        .contains("(0.00, 4.00, 0.00)"));

    // The game's own components go on as RON text, and show in the tree.
    agent.text(
        "update_entity",
        json!({ "id": crate_id, "components": { "loot": "(table: \"chest\")" } }),
    );
    assert!(agent
        .text("scene_tree", json!({}))
        .contains("loot=(table: \"chest\")"));
    let err = agent
        .call(
            "update_entity",
            json!({ "id": crate_id, "components": { "loot": "(table: " } }),
        )
        .unwrap_err();
    assert!(err.contains("component loot"), "{err}");

    // And the file says what the edits said.
    agent.text("save_scene", json!({}));
    let saved = std::fs::read_to_string(root.join("scenes/main.ron")).unwrap();
    assert!(saved.contains(&format!("id: \"{crate_id}\"")), "{saved}");
    assert!(saved.contains("Dynamic"), "{saved}");
    // A component is a file of the game's: until there is one, check says so.
    let findings = agent.text("check", json!({}));
    assert!(findings.contains("no component `loot`"), "{findings}");
    std::fs::write(
        root.join("src/components/loot.rs"),
        "#[derive(serde::Deserialize)]\npub struct Loot { pub table: String }\n",
    )
    .unwrap();
    assert_eq!(agent.text("check", json!({})), "clean");
}

#[test]
fn an_agent_renames_a_material_and_the_scene_follows() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-rename");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    std::fs::write(root.join("materials/clay.scrmat"), "(color: \"#b4643c\")\n").unwrap();
    agent.text("reload", json!({}));
    let id = agent.text(
        "add_entity",
        json!({ "name": "pot", "model": "builtin:sphere", "material": "clay" }),
    );
    agent.text("save_scene", json!({}));

    let used = agent.text("usages", json!({ "file": "materials/clay.scrmat" }));
    assert!(used.contains(&format!("`pot` ({id}) material")), "{used}");

    let said = agent.text(
        "rename_asset",
        json!({ "from": "materials/clay.scrmat", "to": "materials/terracotta.scrmat" }),
    );
    assert!(
        said.contains("scenes said material `clay`, now material `terracotta`"),
        "{said}"
    );
    let tree = agent.text("scene_tree", json!({}));
    assert!(tree.contains("material=terracotta"), "{tree}");
    let wall = agent.text(
        "add_entity",
        json!({ "name": "wall", "model": "builtin:cube", "position": [0.0, 1.5, 0.0], "scale": [8.0, 3.0, 0.3], "material": "grid" }),
    );
    let said = agent.text(
        "push_face",
        json!({ "id": wall, "face": "+x", "metres": 2.0 }),
    );
    assert!(
        said.contains("(1.00, 1.50, 0.00) scale (10.00, 3.00, 0.30)"),
        "{said}"
    );
    let err = agent
        .call(
            "push_face",
            json!({ "id": wall, "face": "north", "metres": 1.0 }),
        )
        .unwrap_err();
    assert!(err.contains("+x, -x"), "{err}");
    // The foliage brush, as the agent holds it: cones on top of the wall.
    let said = agent.text(
        "paint_foliage",
        json!({ "what": "builtin:cone", "centre": [0.0, 3.0, 0.0], "radius": 3.0, "density": 4.0 }),
    );
    assert!(said.ends_with("planted"), "{said}");
    assert!(!said.starts_with("0 "), "{said}");
    let said = agent.text(
        "fence",
        json!({ "what": "builtin:cylinder", "points": [[0.0, 0.0, 5.0], [6.0, 0.0, 5.0]], "spacing": 2.0 }),
    );
    assert!(said.contains("fence of builtin:cylinder"), "{said}");
    // A porch wired to a lamp: the lamp goes out when the last one leaves.
    let porch = agent.text(
        "add_entity",
        json!({ "name": "porch", "body": "Trigger", "collider": "Box(half: (1.0, 1.0, 1.0))" }),
    );
    let lamp = agent.text(
        "add_entity",
        json!({ "name": "lamp", "model": "builtin:sphere" }),
    );
    let said = agent.text(
        "wire",
        json!({ "from": porch, "to": lamp, "do": "Deactivate", "on": "Empty" }),
    );
    assert!(
        said.contains("on:Empty") && said.contains("do:Deactivate"),
        "{said}"
    );
    let err = agent
        .call(
            "wire",
            json!({ "from": porch, "to": "123", "do": "Activate" }),
        )
        .unwrap_err();
    assert!(err.contains("0000000000000123"), "{err}");
    let err = agent
        .call("render", json!({ "from_game": true }))
        .unwrap_err();
    assert!(err.contains("no entity has a camera"), "{err}");
    let eye = agent.text(
        "add_entity",
        json!({ "name": "eye", "position": [0.0, 2.0, -8.0], "camera": "(fov_deg: 50.0)" }),
    );
    let seen = agent.call("render", json!({ "from_game": true })).unwrap();
    assert_eq!(seen[0]["type"], "image");
    agent.text("delete_entity", json!({ "id": eye }));
    let plan = agent.text(
        "render",
        json!({ "view": "top", "width": 64, "height": 64 }),
    );
    assert!(
        plan.contains("eye (") && plan.contains("looking at"),
        "{plan}"
    );
    let placed = agent.text("place", json!({ "ids": [wall], "x": 32, "y": 32 }));
    assert_eq!(placed, "placed 1");
    agent.text("undo", json!({}));
    let err = agent
        .call("render", json!({ "view": "sideways" }))
        .unwrap_err();
    assert!(err.contains("view is top, bottom"), "{err}");
    agent.text("render", json!({ "view": "perspective" }));
    agent.text(
        "set_field",
        json!({ "id": wall, "field": "layer", "value": "player" }),
    );
    assert_eq!(
        agent.text(
            "set_field",
            json!({ "ids": [wall], "field": "layer", "value": "player" })
        ),
        "1 layer = player"
    );
    let both = agent.text("inspect", json!({ "ids": [wall, wall] }));
    assert!(both.contains("layer: player"), "{both}");
    let fields = agent.text("inspect", json!({ "id": wall }));
    assert!(
        fields.contains("layer: player") && fields.contains("model: builtin:cube"),
        "{fields}"
    );
    let size = agent.text("measure", json!({ "id": wall }));
    assert!(size.contains("size (10.00, 3.00, 0.30)"), "{size}");
    let post = agent.text(
        "add_entity",
        json!({ "name": "post", "model": "builtin:cube", "position": [9.0, 3.0, 2.0] }),
    );
    let apart = agent.text("measure", json!({ "id": wall, "to": post }));
    assert!(
        apart.contains("gap per axis (2.50, -0.50, 1.35)"),
        "{apart}"
    );
    assert_eq!(
        agent.text(
            "align",
            json!({ "ids": [wall, post], "axis": "y", "to": "min" })
        ),
        "1 moved"
    );
    let post_now = agent.text("measure", json!({ "id": post }));
    assert!(
        post_now.contains("from (8.50, 0.00, 1.50)"),
        "down on the wall's floor: {post_now}"
    );
    assert_eq!(
        agent.text("hide", json!({ "ids": [post] })),
        "hidden 1; hidden now: 1"
    );
    assert_eq!(
        agent.text("hide", json!({ "ids": [post], "show": true })),
        "shown 1; hidden now: 0"
    );
    assert_eq!(
        agent.text("isolate", json!({ "ids": [wall] })),
        "showing 1 alone"
    );
    assert_eq!(
        agent.text("isolate", json!({ "ids": [] })),
        "everything is shown"
    );
    agent.text("delete_entity", json!({ "id": post }));
    let copies = agent.text(
        "array",
        json!({ "id": wall, "count": 2, "step": [0.0, 0.0, 4.0] }),
    );
    assert_eq!(copies.lines().count(), 2, "{copies}");
    let tree = agent.text("scene_tree", json!({}));
    assert!(tree.contains("(1.00, 1.50, 8.00)"), "{tree}");
    agent.text("undo", json!({}));
    assert!(!agent
        .text("scene_tree", json!({}))
        .contains("(1.00, 1.50, 8.00)"));
    agent.text("undo", json!({}));
    agent.text("delete_entity", json!({ "id": wall }));
    let found = agent.text("find", json!({ "query": "m:terracotta" }));
    assert_eq!(found, format!("{id} \"pot\""));
    let err = agent.call("find", json!({ "query": "t:Pot" })).unwrap_err();
    assert!(err.contains("no filter `t:`"), "{err}");
    let findings = agent.text("check", json!({}));
    assert!(!findings.contains("terracotta"), "{findings}");

    // Onto a name the builtins already answer to, with a line using it.
    agent.text(
        "add_entity",
        json!({ "name": "ember", "model": "builtin:cube", "material": "ember" }),
    );
    agent.text("save_scene", json!({}));
    let refused = agent
        .call(
            "rename_asset",
            json!({ "from": "materials/terracotta.scrmat", "to": "materials/ember.scrmat" }),
        )
        .unwrap_err();
    assert!(
        refused.contains("material `ember` is already named by 1 line(s)"),
        "{refused}"
    );

    let listed = agent.text("assets", json!({}));
    assert!(
        listed.contains("materials/terracotta.scrmat material `terracotta` used 1"),
        "{listed}"
    );
    let refused = agent
        .call(
            "delete_asset",
            json!({ "file": "materials/terracotta.scrmat" }),
        )
        .unwrap_err();
    assert!(refused.contains("`pot`"), "{refused}");
}

#[test]
fn the_scene_and_the_project_files_are_readable_resources() {
    let mut agent = Agent::new();
    let listed = agent.request("resources/list", json!({}));
    assert_eq!(
        listed["result"]["resources"].as_array().unwrap().len(),
        0,
        "nothing open, nothing listed — and no GPU asked for"
    );
    let root = std::env::temp_dir().join("scrap-mcp-resources");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    agent.text(
        "add_entity",
        json!({ "name": "unsaved stone", "model": "builtin:sphere" }),
    );

    let listed = agent.request("resources/list", json!({}));
    let resources = listed["result"]["resources"].as_array().unwrap();
    let names: Vec<&str> = resources
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"scenes/main.ron"), "{names:?}");
    assert!(names.contains(&"configs/world.ron"), "{names:?}");

    let document = agent.request("resources/read", json!({ "uri": "scrap://document" }));
    let text = document["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("unsaved stone"),
        "the document as it stands: {text}"
    );

    let uri = resources
        .iter()
        .find(|r| r["name"] == "scenes/main.ron")
        .unwrap()["uri"]
        .as_str()
        .unwrap()
        .to_string();
    let file = agent.request("resources/read", json!({ "uri": uri }));
    let text = file["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(!text.contains("unsaved stone"), "the file, as saved");

    let secret = std::env::temp_dir().join("scrap-mcp-outside.txt");
    std::fs::write(&secret, "not the project's").unwrap();
    let outside = agent.request(
        "resources/read",
        json!({ "uri": format!("file://{}", secret.display()) }),
    );
    let message = outside["error"]["message"].as_str().unwrap_or("");
    assert!(message.contains("outside the project"), "{outside}");
}

#[test]
fn an_agent_starts_a_new_level_in_the_same_project() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-new-scene");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    let opened = agent.text("new_scene", json!({ "name": "cave" }));
    let said = agent.text("console", json!({ "clear": true }));
    assert!(
        said.contains("info: opened") && said.contains("cave.ron"),
        "{said}"
    );
    assert_eq!(agent.text("console", json!({})), "nothing said");
    assert!(
        opened.contains("cave.ron") && opened.contains("scenes: cave, main"),
        "{opened}"
    );
    let tree = agent.text("scene_tree", json!({}));
    assert!(
        tree.contains("\"ground\"") && !tree.contains("\"cube\""),
        "{tree}"
    );
    let err = agent
        .call("new_scene", json!({ "name": "main" }))
        .unwrap_err();
    assert!(err.contains("already there"), "{err}");
    // No game running: the game's console says what to do first.
    let err = agent
        .call("game_console", json!({ "command": "help" }))
        .unwrap_err();
    assert!(err.contains("start_game first"), "{err}");
}

#[test]
fn an_agent_sees_the_player_s_size_and_is_told_where_it_cannot_start() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-player");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    // The capsule and the jump stand where the view looks: a picture.
    let shot = agent
        .call(
            "render",
            json!({ "eye": [0.0, 3.0, 6.0], "target": [0.0, 0.5, 0.0], "player": true }),
        )
        .unwrap();
    assert!(shot.iter().any(|c| c["type"] == "image"), "{shot:?}");
    // A walk across the ground, for the project's player.
    let way = agent.text(
        "path",
        json!({ "from": [-5.0, 0.0, 0.0], "to": [5.0, 0.0, 0.0] }),
    );
    assert!(way.starts_with("a way"), "{way}");
    // Looking at the sky from over nothing: nowhere to stand, said so, and
    // nothing started.
    agent
        .call(
            "render",
            json!({ "eye": [100.0, 3.0, 0.0], "target": [110.0, 3.0, 0.0] }),
        )
        .unwrap();
    let e = agent
        .call("start_game", json!({ "from_here": true }))
        .unwrap_err();
    assert!(e.contains("nothing to stand on"), "{e}");
}

#[test]
fn an_agent_sets_a_mood_and_tunes_the_look() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-mood");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    let moods = agent.text("mood", json!({}));
    assert!(
        moods.contains("golden hour") && moods.contains("storm"),
        "{moods}"
    );

    let said = agent
        .call("mood", json!({ "name": "golden hour" }))
        .unwrap();
    assert!(
        said[0]["text"].as_str().unwrap().contains("hour: 17.6"),
        "{said:?}"
    );
    assert_eq!(said[1]["type"], "image", "and a picture of it");

    let look = agent.text(
        "look",
        json!({ "weather": "(rain: 1.0, wetness: 1.0)", "volumetric_fog": "None" }),
    );
    assert!(look.contains("weather: (rain:1.0"), "{look}");
    assert!(look.contains("volumetric_fog: None"), "{look}");
    assert!(
        look.contains("sun: (hour:17.6"),
        "the mood's sun stays: {look}"
    );
    let err = agent
        .call("look", json!({ "sky": "(mode: Sepia)" }))
        .unwrap_err();
    assert!(err.contains("sky"), "{err}");
    let err = agent
        .call("mood", json!({ "name": "tuesday" }))
        .unwrap_err();
    assert!(err.contains("golden hour"), "{err}");
}

#[test]
fn an_agent_reads_connects_and_renames_in_an_animator_graph() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-graph");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    let dir = root.join("animators");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("hero.ron"),
        "(\n    start: \"idle\",\n    states: {\n        \"idle\": (clip: \"idle\"),\n        \"walk\": (clip: \"walk\"),\n    },\n)\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("hero.cases.ron"),
        "(cases: [(name: \"walks\", steps: [(set: {\"speed\": 1.0}, expect: \"walk\")])])\n",
    )
    .unwrap();
    let read = agent.text("graph", json!({ "name": "hero" }));
    assert!(read.contains("`walk` is never reached"), "{read}");
    agent.text(
        "graph_connect",
        json!({ "name": "hero", "from": "idle", "to": "walk", "when": "[Above(\"speed\", 0.1)]" }),
    );
    let read = agent.text("graph", json!({ "name": "hero" }));
    assert!(read.contains("idle → walk when speed > 0.1"), "{read}");
    assert!(!read.contains("never reached"), "{read}");
    let err = agent
        .call(
            "graph_connect",
            json!({ "name": "hero", "from": "idel", "to": "walk" }),
        )
        .unwrap_err();
    assert!(err.contains("did you mean `idle`"), "{err}");
    agent.text(
        "graph_rename",
        json!({ "name": "hero", "from": "walk", "to": "stroll" }),
    );
    let file = std::fs::read_to_string(dir.join("hero.ron")).unwrap();
    assert!(
        file.contains("\"stroll\"") && !file.contains("\"walk\":"),
        "{file}"
    );
    assert!(
        file.contains("to: \"stroll\""),
        "the transition follows the state: {file}"
    );
    let cases = std::fs::read_to_string(dir.join("hero.cases.ron")).unwrap();
    assert!(cases.contains("expect: \"stroll\""), "{cases}");
    agent.text(
        "graph_disconnect",
        json!({ "name": "hero", "from": "idle", "to": "stroll" }),
    );
    assert!(agent
        .text("graph", json!({ "name": "hero" }))
        .contains("never reached"));
}

#[test]
fn an_agent_balances_records_in_a_table() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-table");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    let file = root.join("configs/enemies.ron");
    let text = "{\n    \"goblin\": (hp: 10, speed: 2.5),\n    \"orc\": (hp: 30), // slow\n}\n";
    std::fs::write(&file, text).unwrap();

    let sources = agent.text("table", json!({}));
    assert!(sources.contains("configs/enemies.ron"), "{sources}");
    let table = agent.text("table", json!({ "source": "configs/enemies.ron" }));
    assert!(table.starts_with("key\tlabel\thp\tspeed"), "{table}");
    assert!(table.contains("orc\torc\t30\t"), "{table}");

    agent.text(
        "set_cell",
        json!({ "source": "configs/enemies.ron", "row": "orc", "column": "speed", "value": "1.5" }),
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        text.replace("(hp: 30)", "(hp: 30, speed: 1.5)")
    );
    let e = agent
        .call(
            "set_cell",
            json!({ "source": "configs/enemies.ron", "row": "orcc", "column": "hp", "value": "1" }),
        )
        .unwrap_err();
    assert!(e.contains("did you mean `orc`?"), "{e}");
    assert!(agent.text("undo_cell", json!({})).contains("orc.speed"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);

    // The world's numbers: one struct, one row.
    agent.text(
        "set_cell",
        json!({ "source": "configs/world.ron", "column": "gravity", "value": "-3.7" }),
    );
    let world = std::fs::read_to_string(root.join("configs/world.ron")).unwrap();
    assert!(world.contains("gravity: -3.7"), "{world}");
    assert!(
        world.starts_with("// The world's numbers."),
        "comments stay: {world}"
    );
}

#[test]
fn an_agent_writes_plays_and_renames_in_a_dialogue() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-dialogue");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    // A dialogue made a line at a time; the first is its start.
    agent.text(
        "dialogue_line",
        json!({ "name": "trader", "line": "hello", "entry": "(speaker: \"Trader\", text: \"Buying?\", choices: [(text: \"Yes\", to: \"sold\", when: [Var(\"coins\", Ge, 3)], add: {\"coins\": -3}), (text: \"No\", to: \"bye\")])" }),
    );
    let said = agent.text(
        "dialogue_line",
        json!({ "name": "trader", "line": "sold", "entry": "(text: \"Done.\", event: \"sold\")" }),
    );
    assert!(said.contains("leads to `bye`"), "{said}");
    agent.text(
        "dialogue_line",
        json!({ "name": "trader", "line": "bye", "entry": "(text: \"Bye.\")" }),
    );
    let file = root.join("dialogues/trader.ron");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("start: \"hello\""), "{text}");
    assert!(
        text.contains("        \"bye\": (text: \"Bye.\"),"),
        "a line a line: {text}"
    );
    let read = agent.text("dialogue", json!({ "name": "trader" }));
    assert!(
        read.contains("answer 1: “Yes” → sold if coins ≥ 3"),
        "{read}"
    );
    assert!(!read.contains("problem"), "{read}");

    // Played without the game: poor, then with coins.
    let poor = agent.text(
        "dialogue_play",
        json!({ "name": "trader", "answers": ["Yes"] }),
    );
    assert!(poor.contains("answers on offer: “No”"), "{poor}");
    assert!(poor.contains("“Yes” is not on offer"), "{poor}");
    let rich = agent.text(
        "dialogue_play",
        json!({ "name": "trader", "answers": ["Yes"], "vars": { "coins": 5 } }),
    );
    assert!(
        rich.contains("tells `sold`") && rich.contains("coins = 2"),
        "{rich}"
    );

    // Cases follow a rename; check plays them.
    std::fs::write(
        root.join("dialogues/trader.cases.ron"),
        "(cases: [(name: \"buys\", vars: {\"coins\": 3}, steps: [Choose(\"Yes\", \"sold\"), Told(\"sold\")])])\n",
    )
    .unwrap();
    agent.text(
        "dialogue_rename",
        json!({ "name": "trader", "from": "sold", "to": "done" }),
    );
    let cases = std::fs::read_to_string(root.join("dialogues/trader.cases.ron")).unwrap();
    assert!(cases.contains("Choose(\"Yes\", \"done\")"), "{cases}");
    assert!(agent
        .text("dialogue", json!({ "name": "trader" }))
        .contains("cases: 1 of 1 pass"));
    let err = agent
        .call("dialogue", json!({ "name": "tradr" }))
        .unwrap_err();
    assert!(err.contains("did you mean `trader`"), "{err}");

    let wrote = agent.text("export_lines", json!({}));
    assert!(wrote.contains("build/lines/en.csv"), "{wrote}");
    let sheet = std::fs::read_to_string(root.join("build/lines/en.csv")).unwrap();
    assert!(sheet.contains("trader/hello,Trader,,Buying?"), "{sheet}");
    assert!(sheet.contains("trader/hello/1,,,Yes"), "{sheet}");
}

#[test]
fn an_agent_reads_finds_and_sets_a_table_s_records() {
    let mut agent = Agent::new();
    let root = std::env::temp_dir().join("scrap-mcp-configs");
    let _ = std::fs::remove_dir_all(&root);
    match agent.call("new_project", json!({ "path": root.to_string_lossy() })) {
        Ok(_) => {}
        Err(e) if e.contains("GPU") => {
            eprintln!("skipping: {e}");
            return;
        }
        Err(e) => panic!("{e}"),
    }
    std::fs::write(
        root.join("configs/tools.ron"),
        "{\n    \"Палка\": (id: \"4c1e\", weight: 1.0),\n    \"Посох\": (base: \"Палка\", weight: 2.0),\n}\n",
    )
    .unwrap();
    let listed = agent.text("configs", json!({}));
    assert!(listed.contains("configs/tools.ron") && listed.contains("configs/world.ron"), "{listed}");
    let found = agent.text("configs", json!({ "find": "по" }));
    assert_eq!(found.trim(), "configs/tools.ron: Посох", "{found}");
    let shown = agent.text("configs", json!({ "file": "configs/tools.ron" }));
    assert!(shown.contains("name | base | weight") && shown.contains("Посох | \"Палка\" | 2.0"), "{shown}");

    let set = agent.text(
        "config_set",
        json!({ "file": "configs/tools.ron", "record": "Палка", "field": "weight", "value": "1.5" }),
    );
    assert_eq!(set, "set tools.ron: Палка.weight");
    let text = std::fs::read_to_string(root.join("configs/tools.ron")).unwrap();
    assert!(text.contains("(id: \"4c1e\", weight: 1.5)"), "{text}");
    assert!(text.contains("\"Посох\": (id: \""), "its id written: {text}");
    // A top-level field of a struct of numbers.
    agent.text(
        "config_set",
        json!({ "file": "configs/world.ron", "field": "gravity", "value": "-3.7" }),
    );
    assert!(std::fs::read_to_string(root.join("configs/world.ron")).unwrap().contains("-3.7"));
    assert_eq!(agent.text("config_undo", json!({})), "undone: world.ron: gravity");
    let err = agent
        .call("config_set", json!({ "file": "configs/tools.ron", "record": "Кость", "field": "weight", "value": "1" }))
        .unwrap_err();
    assert!(err.contains("no record `Кость`"), "{err}");
}
