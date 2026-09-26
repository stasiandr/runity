//! The tools: what each one takes, and the session calls it makes.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::fmt::Write as _;
use std::path::PathBuf;

use image::ImageEncoder;
use scrap::glam::Vec3;
use scrap::scene::{Collider, MaterialRef};
use scrap::{Body, EntityDesc, EntityId, Material, Project};
use serde_json::{json, Value};

use crate::{image, text, Server};

type Answer = Result<Vec<Value>, String>;

const ID: &str = "an entity id: 16 hex digits, as scene_tree shows";

fn vec3(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": description })
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

/// The fields an entity's line has, shared by add and update.
fn entity_fields() -> Value {
    json!({
        "name": { "type": "string" },
        "model": { "type": "string", "description": "builtin:plane|cube|cone|sphere|cylinder|ramp|stairs (blockout shapes, one unit, centred), or a model's file name without extension from assets/" },
        "material": { "type": "string", "description": "a material name: materials/<name>.scrmat, or builtin grass, earth, bark, needle, stone, ember, white" },
        "color": { "type": "string", "description": "#rrggbb, sRGB — a colour spelled out instead of a material name" },
        "position": vec3("metres, relative to the parent"),
        "rotation_deg": vec3("Euler degrees, applied Y then X then Z"),
        "scale": vec3("per axis"),
        "body": { "type": "string", "enum": ["None", "Static", "Dynamic", "Kinematic", "Trigger"] },
        "collider": { "type": "string", "description": "RON: None, Box(half: (x, y, z)), Sphere(radius: r), Capsule(half_height: h, radius: r), Cylinder(half_height: h, radius: r), Ramp(half: (x, y, z)), Stairs(half: (x, y, z), steps: n). builtin:cube/cylinder/ramp/stairs fit Box/Cylinder/Ramp/Stairs with half 0.5 (stairs: steps 4)" },
        "camera": { "type": "string", "description": "RON: a camera on this entity, looking along its +z — (fov_deg: 60.0, priority: 0) — or None. Put it on a child of the player and it follows" },
        "layer": { "type": "string", "description": "collision layer by its name in layers.ron; empty is default" },
        "route": { "type": "string", "description": "RON: travels by itself — (points: [(0.0, 0.0, 0.0), (0.0, 4.0, 0.0)], speed: 1.5, ends: Back|Loop|Stop, smooth: true, pause: 1.0), points from where it stands; with a Kinematic body it carries what stands on it (a lift, a moving platform); or None" },
        "decal": { "type": "string", "description": "RON: a decal's box, centred here and pressed down its -y — (size: (2.0, 1.0, 2.0)); the entity's material is the picture (base map, alpha, normal map) — or None" },
        "footprints": { "type": "string", "description": "RON: leaves prints in the ground as it walks and kicks up dust at each step — (stride: 0.75, stance: 0.12, size: 0.28, depth: 0.03, lasts: 60.0, dust: 1.0, feet: 0.9, color: (0.62, 0.46, 0.3)); feet is how far below its origin its feet are; or None" },
        "terrain": { "type": "string", "description": "RON: ground shaped from numbers, centred here, wind along its +x — (size: 400.0, cells: 256, dunes: (height: 8.0, wavelength: 60.0, sinuosity: 0.5, barchans: 0.4, seed: 1)); give it material (shading: Sand) and collider: Model to walk on it; or None" },
        "reflection_probe": { "type": "string", "description": "RON: a reflection probe's box, centred here — (size: (8.0, 4.0, 8.0)); box_projection: false, blend_distance: 1.0 — what polished things in it reflect instead of the sky; or None" },
        "sound": { "type": "string", "description": "RON: a sound it makes — (clip: \"radio\", looped: true); volume: 1.0, on_start: true, spatial: true (false: everywhere, music), near: 1.0, far: 40.0, group: \"music\" — or None" },
        "animator": { "type": "string", "description": "the graph in animators/ that moves it and what is under it, with clips from clips/ (Unity's Animator); empty for none" },
        "inactive": { "type": "boolean", "description": "switched off: it and everything under it is not drawn, has no body, makes no sound" },
        "particles": { "type": "string", "description": "RON: particles given off along its up — (rate: 30.0, life: 0.8, speed: 2.0, spread_deg: 20.0, size: 0.06, gravity: -1.0, color: (1.0, 0.6, 0.2)) — sparks, dust, spray; or None" },
        "light": { "type": "string", "description": "RON: a point light at this entity — (color: (1.0, 0.6, 0.3), intensity: 2.0, range: 6.0), colour as a picker says it; add cone_deg: 30.0 for a spot along its −z — or None" },
        "physics": { "type": "string", "description": "RON, only what differs: (friction: 0.5, bounce: 0.0, density: 1.0) — a ball is (bounce: 0.8), iron is (density: 8.0); freeze_turn: \"xz\" keeps it upright, freeze_move: \"y\" at its height" },
        "wires": { "type": "string", "description": "RON, on a Trigger (or any body): what it does to other things when something comes in or goes out — [(on: Enter|Leave|Empty, to: \"<id>\", do: Trigger(\"open\")|Set(\"lit\", true)|Activate|Deactivate|Toggle|Spawn(prefab: \"crate\"), only: \"player\", once: true)]; Empty is when the last one left; Trigger and Set pull the target's animator; Spawn puts the prefab where the target stands; only is a layer; or None. No conditions or sequences: logic is the game's code" },
        "joint": { "type": "string", "description": "RON, on the body that moves: None, Hinge(to: \"<id>\", anchor: (x, y, z), axis: (x, y, z), limits_deg: (min, max)), Ball(to: \"<id>\", anchor: (x, y, z)), Fixed(to: \"<id>\"), Slider(to: \"<id>\", axis: (x, y, z), limits: (min, max)). Anchor and axis in its own space; leave out `to` to hang from the world" },
        "components": { "type": "object", "additionalProperties": { "type": ["string", "null"] }, "description": "the game's components by registered name, each value in RON, e.g. {\"door\": \"(open_angle: 90.0)\"}; null removes one" },
    })
}

pub fn list() -> Vec<Value> {
    let mut add = entity_fields();
    add["parent"] = json!({ "type": "string", "description": ID });
    add["prefab"] = json!({ "type": "string", "description": "place an instance of prefabs/<name>.prefab instead of a model" });
    add["ron"] = json!({ "type": "string", "description": "a whole entity in the scene's RON, children included; other fields are then ignored" });
    let mut update = entity_fields();
    update["id"] = json!({ "type": "string", "description": ID });
    let camera = json!({
        "eye": vec3("where the camera is"),
        "target": vec3("what it looks at"),
        "focus": { "type": "string", "description": "an entity id to frame instead of eye/target" },
        "width": { "type": "integer" },
        "height": { "type": "integer" },
        "from_game": { "type": "boolean", "description": "look through the game's camera — the entity with `camera` — instead of the editor's" },
        "colliders": { "type": "boolean", "description": "draw every collider as an outline — green static, blue dynamic, orange kinematic, yellow trigger — until turned off" },
        "navigation": { "type": "boolean", "description": "show where the project's player (scrap.ron, game.player: radius, slope, step) can go, in blue on the ground, until turned off" },
        "player": { "type": "boolean", "description": "draw the project's player (scrap.ron, game.player) in green — a capsule its height and width, and the arc its feet take in a running jump — standing where start_game's from_here would start it, until turned off: is this door tall enough, this gap jumpable" },
        "grid": { "type": "boolean", "description": "the Scene view's grid on the ground (on the wall in a side view), spaced as drags snap; on by default, stays until changed" },
        "view": { "type": "string", "enum": ["top", "bottom", "front", "back", "left", "right", "perspective", "orthographic"], "description": "look along an axis, orthographic, showing as much as before — `top` is the level's plan — or switch projection; stays until changed" },
    });
    let mut simulate = camera.clone();
    simulate["seconds"] = json!({ "type": "number", "description": "simulated time, fixed steps" });
    simulate["keep"] = json!({ "type": "array", "items": { "type": "string" }, "description": "entity ids to leave where the simulation put them — Unreal's Keep Simulation Changes; one undo step" });
    vec![
        tool("new_project", "Make a scrap project (standard layout, starter scene, game crate) and open its scene.", json!({ "path": { "type": "string" }, "name": { "type": "string" } }), &["path"]),
        tool("open_scene", "Open a scene file; its project's prefabs, materials and library come with it.", json!({ "path": { "type": "string" } }), &["path"]),
        tool("new_scene", "Make scenes/<name>.ron in the open project — a ground with the metre grid, solid — and open it. Save the open scene first if it has changes. Lists the project's scenes.", json!({ "name": { "type": "string", "description": "snake_case, like level_2" } }), &["name"]),
        tool("open_prefab", "Prefab Mode: open prefabs/<name>.prefab as the document. Every tool then edits the prefab (a variant's part edits become its overrides) and save_scene writes the prefab file; open_scene goes back.", json!({ "name": { "type": "string" } }), &["name"]),
        tool("scene_tree", "The open scene as an indented tree: id, name, model or prefab, material, place.", json!({}), &[]),
        tool("find", "Search the scene like the hierarchy's search box: words match names; c:door (has component), m:stone (material), p:campfire (prefab instance), model:pine_large, body:dynamic, has:light|camera|particles|probe|decal|route|joint|collider, layer:debris; quoted \"phrases\"; terms combine with and. Prefab parts included. Returns id and name per line.", json!({ "query": { "type": "string" } }), &["query"]),
        tool("inspect", "The Inspector for an entity: every field as text, and for a prefab's part which ones this instance overrides. With `ids`, for several at once: — where they disagree.", json!({ "id": { "type": "string", "description": ID }, "ids": { "type": "array", "items": { "type": "string" } } }), &[]),
        tool("set_field", "Set one field of an entity from text, as typing into the Inspector: name, model, prefab, position \"(x, y, z)\", rotation, scale, material, body, collider, physics, layer, joint, wires, camera, components.<name>. One undo step; on a prefab's part, an override. With `ids` instead of `id`, the same field of all of them, still one step.", json!({ "id": { "type": "string", "description": ID }, "ids": { "type": "array", "items": { "type": "string" } }, "field": { "type": "string" }, "value": { "type": "string" } }), &["field", "value"]),
        tool("get_entity", "One entity's line in the scene's RON, children included.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("add_entity", "Add an entity, as one undo step. Returns its id.", add, &[]),
        tool("update_entity", "Change any fields of an entity, as one undo step.", update, &["id"]),
        tool("reparent", "Move an entity under another, or to the top without parent. Refuses loops. Its transform stays as written (now relative to the new parent) unless `stay` is true, which keeps it where it is in the world as a Hierarchy drag does; `index` puts it at that place among its new siblings (0 first) — the same parent reorders.", json!({ "id": { "type": "string", "description": ID }, "parent": { "type": "string", "description": ID }, "index": { "type": "integer" }, "stay": { "type": "boolean" } }), &["id"]),
        tool("make_prefab", "Turn an entity into prefabs/<name>.prefab and leave an instance in its place.", json!({ "id": { "type": "string", "description": ID }, "name": { "type": "string" } }), &["id", "name"]),
        tool("make_variant", "Save a prefab instance, with its overrides, material, components and children, as prefabs/<name>.prefab — a variant of its prefab — and make it an instance of that. Later changes to the base still reach the variant where it said nothing.", json!({ "id": { "type": "string", "description": ID }, "name": { "type": "string" } }), &["id", "name"]),
        tool("measure", "How big a thing is in the world, in metres — its box, prefab parts and children included — and, given `to`, how far it is from another and the gap between them on each axis.", json!({ "id": { "type": "string", "description": ID }, "to": { "type": "string", "description": ID } }), &["id"]),
        tool("align", "Line things up on one axis: their min sides, centres or max sides on one plane. One undo step.", json!({ "ids": { "type": "array", "items": { "type": "string" } }, "axis": { "type": "string", "enum": ["x", "y", "z"] }, "to": { "type": "string", "enum": ["min", "center", "max"] } }), &["ids", "axis", "to"]),
        tool("array", "Greybox: count copies of an entity in a row, each step further (in its parent's space) than the last — posts, pillars, steps. One undo step; returns the copies' ids.", json!({ "id": { "type": "string", "description": ID }, "count": { "type": "integer" }, "step": vec3("from one copy to the next, metres") }), &["id", "count", "step"]),
        tool("push_face", "Greybox: move one face of a thing by some metres (negative pulls it in), the opposite face staying where it is — make a wall 2 m longer at its +x end. Faces in the thing's own axes: +x -x +y (top) -y (bottom) +z -z. One undo step.", json!({ "id": { "type": "string", "description": ID }, "face": { "type": "string" }, "metres": { "type": "number" } }), &["id", "face", "metres"]),
        tool("scatter", "Scatter copies of a model, or instances of a prefab, over a disc — trees, rocks, grass — as one group and one undo step. The same seed gives the same layout. Returns the group's id.", json!({
            "what": { "type": "string", "description": "a model (builtin:cone, or a name from assets/) or a prefab name" },
            "centre": vec3("the middle of the disc"),
            "radius": { "type": "number" },
            "count": { "type": "integer" },
            "spacing": { "type": "number", "description": "least distance between two; fewer come out if the disc is too full" },
            "seed": { "type": "integer" },
            "scale_min": { "type": "number" },
            "scale_max": { "type": "number" },
            "parent": { "type": "string", "description": ID },
        }), &["what", "count"]),
        tool("paint_foliage", "The foliage brush, one dab: copies of a model (or instances of a prefab) planted on whatever is under a disc — trees on a hillside, rocks along a path — about `density` per square metre, none closer than that allows, turned and sized a little at random. They go into one group per model, `foliage: <name>`. `erase` takes this model's copies inside the disc away instead. One undo step. Returns how many were planted or taken.", json!({
            "what": { "type": "string", "description": "a model (builtin:cone, or a name from assets/) or a prefab name" },
            "centre": vec3("the middle of the disc; the ground under it is found by a ray from above"),
            "radius": { "type": "number" },
            "density": { "type": "number", "description": "per square metre; 0.4 by default" },
            "align": { "type": "boolean", "description": "lean each with the slope; off by default (trees grow up)" },
            "erase": { "type": "boolean" },
            "seed": { "type": "integer" },
        }), &["what", "centre", "radius"]),
        tool("table", "Many things of one kind as a table — what a designer balances in. `source` is a tuning file, tuning/enemies.ron (a map { \"goblin\": (hp: 10), … } is a row a record; a struct is one row), or a scene search like find's, c:enemy (a row an entity; with c:<component> the columns are its fields, else name, model, material, position…). Without `source`, lists the sources there are. Returns tab-separated rows under a header: key, label, then a cell per column as the file writes it (empty: left at its default), with problems on the lines after a record that does not fit the game's type.", json!({ "source": { "type": "string" } }), &[]),
        tool("set_cell", "Set one cell of a table from RON text (35, 2.5, true, \"fast\") — only that field changes in the file; empty `value` takes the field out, back to its default. `row` is the key table shows (a record's name, an entity's id; empty for a tuning file that is one struct); `rows` for the same value in several at once. Scene cells are one undo step; a tuning file is written at once (the running game reloads it) and undo_cell takes it back. A value that does not fit what the game reads changes nothing and says why.", json!({ "source": { "type": "string" }, "row": { "type": "string" }, "rows": { "type": "array", "items": { "type": "string" } }, "column": { "type": "string" }, "value": { "type": "string" } }), &["source", "column", "value"]),
        tool("undo_cell", "Take back the last cell set in a tuning file: the file as it was before. Refused when the file changed since.", json!({}), &[]),
        tool("wire", "Wire a trigger to what it does, without code (docs/wires.md): when something comes into `from`'s body (on: Enter), goes out (Leave) or the last one has gone (Empty), `to` gets `do` — Trigger(\"open\") or Set(\"lit\", true) on its animator, Activate, Deactivate, Toggle, or Spawn(prefab: \"crate\") where it stands. only: a layer (the player, not a crate); once: the first time only. Appended to `from`'s `wires`, one undo step; `problems` then says if its animator lacks that trigger or it has no body. Play (simulate) runs wires, animators and physics without the game.", json!({
            "from": { "type": "string", "description": "the trigger's id" },
            "to": { "type": "string", "description": "the id of what it acts on" },
            "do": { "type": "string", "description": "RON: Trigger(\"open\"), Set(\"lit\", true), Activate, Deactivate, Toggle, Spawn(prefab: \"crate\")" },
            "on": { "type": "string", "enum": ["Enter", "Leave", "Empty"] },
            "only": { "type": "string", "description": "a collision layer from layers.ron; empty for anything" },
            "once": { "type": "boolean" },
        }), &["from", "to", "do"]),
        tool("fence", "Copies of a model along a line through points — a fence, a row of lamps, a colonnade — as one entity with a spline and a spacing. The copies are built from those two and rebuilt when either changes (set_field `spline` or `along`); the file keeps only the line. One undo step; returns the entity's id.", json!({
            "what": { "type": "string", "description": "a model; builtin:cylinder makes posts" },
            "points": { "type": "array", "items": { "type": "array", "items": { "type": "number" } }, "description": "world points the line goes through, at least two" },
            "spacing": { "type": "number", "description": "metres between copies; 1 by default" },
        }), &["what", "points"]),
        tool("history", "The commits that touched the open scene's file — or with project: true, anything in the project — newest first: commit, author, date, summary.", json!({ "project": { "type": "boolean" } }), &[]),
        tool("changes", "What one commit changed: the project's files it touched and, when it touched the open scene, each thing added or removed and each field changed, before → after.", json!({ "commit": { "type": "string" } }), &["commit"]),
        tool("git_status", "Where the project's repository stands: branch, commits ahead of and behind the branch it follows, whether a merge is under way, and each file not as committed, with a letter: M changed, A new, D deleted, R renamed, U in conflict.", json!({}), &[]),
        tool("commit", "Commit files of the project — all that `git_status` lists when paths is left out — with a message. Save the scene first: unsaved edits are refused, not left out silently.", json!({ "message": { "type": "string" }, "paths": { "type": "array", "items": { "type": "string" }, "description": "relative to the project, or absolute" } }), &["message"]),
        tool("restore", "Put the open scene back as it was at a commit, as one undo step (render afterwards to look; undo to go back).", json!({ "commit": { "type": "string" } }), &["commit"]),
        tool("conflicts", "While git is merging the open scene with conflicts: each conflict in words, numbered, and which side the document holds for it now. The merge holds ours for each.", json!({}), &[]),
        tool("take_theirs", "Settle one conflict (its number from `conflicts`) theirs' way, as one undo step. Keeping ours needs nothing. Then save and `resolved`.", json!({ "conflict": { "type": "integer" } }), &["conflict"]),
        tool("take_ours", "Settle one conflict ours' way again after taking theirs or editing it, as one undo step.", json!({ "conflict": { "type": "integer" } }), &["conflict"]),
        tool("resolved", "Tell git the open scene's conflicts are settled (`git add`), so the merge can be committed. Save first.", json!({}), &[]),
        tool("copy", "Entities (children included) as RON text, for `paste` here or in another scene.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" } }), &["ids"]),
        tool("paste", "Add entities from RON text — from `copy`, or written by hand, one entity or a list — as new things with new ids, one undo step. Returns their ids.", json!({ "ron": { "type": "string" }, "parent": { "type": "string", "description": ID } }), &["ron"]),
        tool("sculpt", "Shape a terrain with one brush stroke at a world point: raise (by > 0) or lower it, or with flatten: true pull it toward the height `by`. Written as one line in the terrain's .scrterrain and rebuilt at once.", json!({
            "id": { "type": "string", "description": "the terrain entity's id" },
            "at": vec3("where the stroke lands"),
            "radius": { "type": "number" },
            "by": { "type": "number", "description": "metres up (or down), or the height to flatten to" },
            "flatten": { "type": "boolean" },
        }), &["id", "at", "radius", "by"]),
        tool("place", "Put entities on whatever a pixel of the last render shows — a table's top, a wall, a slope — by the bottom of their box, keeping their places relative to each other. One undo step. Pixels from the top left, as `pick` takes them.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" }, "x": { "type": "integer" }, "y": { "type": "integer" } }), &["ids", "x", "y"]),
        tool("to_view", "From the render view: `move` puts the entity (and what is under it) on the point the view looks at; `align` stands it where the view is, looking where it looks — frame a shot with render's eye and target, then align the game's camera to it. One undo step.", json!({ "id": { "type": "string", "description": ID }, "how": { "type": "string", "enum": ["move", "align"] } }), &["id", "how"]),
        tool("override_field", "On a prefab's part: `revert` one overridden field to what the prefab says, or `apply` it to the prefab file so every instance has it — the other overrides stay. position, rotation and scale are one override (the transform). One undo step.", json!({ "id": { "type": "string", "description": ID }, "field": { "type": "string" }, "how": { "type": "string", "enum": ["apply", "revert"] } }), &["id", "field", "how"]),
        tool("console", "The editor's Console: what opening, importing and rebuilding said — skipped lines, import warnings, sources that would not rebuild — and what a game started with start_game printed (cargo's compile errors, the game's own lines, a panic), each once with how many times, oldest first. clear: true empties it after reading.", json!({ "clear": { "type": "boolean" } }), &[]),
        tool("start_game", "Play with the game's own code: save the open scene and run the project's game on it (cargo run, SCRAP_SCENE), drawing in the editor's Game view (the other players, when several, in windows of their own). What it prints goes to the console; read it with console. One game at a time — starting again stops the one running. players: 2 to 4 opens that many windows playing together on this machine (Unity's Multiplayer Play Mode): player 1 hosts, the others join once it is up, and console lines start with whose they are (\"player 2: ...\"). The count is remembered for the next start; players: 1 goes back to one. link makes the other players' connection bad on purpose — \"poor\", \"awful\", \"latency=80,jitter=10,loss=3,dup=1\" (round-trip ms, percent), or \"\" for a perfect one — also remembered.", json!({ "players": { "type": "integer", "minimum": 1, "maximum": 4 }, "link": { "type": "string" }, "from_here": { "type": "boolean", "description": "Play from Here: the player starts on the ground the view looks at (render's eye and target), facing its way — the game moves the lines its scene marks player_start: true there (SCRAP_START). Refused when there is nothing to stand on." } }), &[]),
        tool("game_state", "What the game started with start_game says its world is like now, a few times a second: every entity that is not where the scene puts it (id, name, position), the saved components, the scene's entities that are gone, what was spawned at run time, and each animator's last transitions (from → to, and the conditions that held). The Inspector shows the same as game.* fields.", json!({}), &[]),
        tool("stop_game", "Stop the game start_game started; says whether one was running and whether it had ended by itself.", json!({}), &[]),
        tool("game_console", "Type a line into the running game's own console (Unreal's ~ console), as a player would over the game, and get its answer: `help` lists the commands; `get world.gravity` reads tuning/world.ron; `set world.gravity -3` changes that value in the file itself (only its text, comments kept) and the game reloads it — the file is the truth, so keep or git-checkout the change; everything else is the game's own cheats (`spin 90` in the template). The answer is the console entry: `> line`, then what the game said, `error: …` when it could not. Needs a game started with start_game that has drawn (built and running); the console is on in debug builds.", json!({ "command": { "type": "string" }, "wait_seconds": { "type": "number", "description": "How long to wait for the answer; 5 by default." } }), &["command"]),
        tool("group", "Put entities under a new empty entity named `name`, standing on the ground in the middle of them — Unity's Create Empty Parent. Nothing moves in the world; one undo step; returns the group's id.", json!({ "ids": { "type": "array", "items": { "type": "string" } }, "name": { "type": "string" } }), &["ids", "name"]),
        tool("thumbnail", "A picture of a prefab or a model (by the name scenes use: campfire, builtin:cone, rock) alone, framed whole — the Project window's preview. Changes nothing.", json!({ "what": { "type": "string" }, "size": { "type": "integer", "description": "pixels a side, 16 to 1024; 256 by default" } }), &["what"]),
        tool("drop", "Drop a prefab or a model (by the name scenes use) into the view at a pixel of the last render, standing on whatever is there — Project-window drag and drop. One undo step; returns its id.", json!({ "what": { "type": "string" }, "x": { "type": "integer" }, "y": { "type": "integer" } }), &["what", "x", "y"]),
        tool("configs", "The project's configs/ (docs/data.md): without `file`, the files, each table with the type of its records; with `file` (project-relative, like configs/materials.ron), its fields and grids as text — a table's records by name, what a record takes from its base marked ^; with `find`, the records and rows whose name has it, in every file. Changes nothing.", json!({ "file": { "type": "string" }, "find": { "type": "string" } }), &[]),
        tool("config_set", "Set one value in a config as RON text — the Configs window's cell: `record` and `field` in a table of records (field `name` renames it, `base` names its base), or `field` alone for a top-level field, or `grid`, `row` and `field` for a list's item under a field. An empty value takes the field away (a record then takes it from its base). Checked before it is written — it has to read, and fit the type the game said the table holds — and every record's id is written; the rest of the file is kept as it was. One step of config_undo.", json!({ "file": { "type": "string" }, "record": { "type": "string" }, "grid": { "type": "string" }, "row": { "type": "integer" }, "field": { "type": "string" }, "value": { "type": "string" } }), &["file", "field", "value"]),
        tool("config_undo", "Put back the config this agent last changed with config_set, as it was — refused when the file was changed since by anything else.", json!({}), &[]),
        tool("add_component", "Put one of the game's components on an entity with a value of its shape to start from — Add Component. Needs library/components.ron, which the game writes when it or `scrap test` runs; without `name`, lists the components and what each holds.", json!({ "id": { "type": "string", "description": ID }, "name": { "type": "string" } }), &[]),
        tool("import_settings", "An asset source's import settings (its .scrimport), or with `field` and `value` one of them changed — scale, recompute_normals, srgb, origin_to_base — and the asset built again, every scene showing it at once.", json!({ "source": { "type": "string", "description": "project-relative, like assets/rock.obj" }, "field": { "type": "string" }, "value": { "type": "string" } }), &["source"]),
        tool("poly_shape", "Greybox a floor plan: an outline of points (x, z metres around the shape's origin, either way round, not crossing itself) pulled up `height` metres into a solid — an L-shaped room, a platform, a plinth; ProBuilder's Poly Shape. Writes assets/<name>.scrpoly, imports it, places it at `at` as a static body with a collider of its own shape, selected. One undo step; returns its id.", json!({ "name": { "type": "string", "description": "snake_case, becomes the model's name" }, "points": { "type": "array", "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }, "description": "[[x, z], ...], at least three" }, "height": { "type": "number" }, "holes": { "type": "array", "items": { "type": "array", "items": { "type": "array", "items": { "type": "number" } } }, "description": "outlines cut all the way through, inside points: a window in a standing wall, a well in a floor" }, "standing": { "type": "boolean", "description": "stand it up: points are [x, y], a wall seen from the front, and height is its thickness along z — a U of points is a wall with a doorway" }, "at": vec3("where its origin goes") }), &["name", "points", "height"]),
        tool("set_poly", "Change a Poly Shape's outline and/or height: its .scrpoly is rewritten and rebuilt, and every placement of it changes. Omitted parts stay. Refused, with why, for an outline that cannot be a floor.", json!({ "name": { "type": "string" }, "points": { "type": "array", "items": { "type": "array", "items": { "type": "number" } } }, "height": { "type": "number" }, "standing": { "type": "boolean" }, "holes": { "type": "array", "items": { "type": "array", "items": { "type": "array", "items": { "type": "number" } } }, "description": "outlines cut all the way through, inside points: a window in a standing wall, a well in a floor" } }), &["name"]),
        tool("push_poly_edge", "Push one wall of a Poly Shape out by `metres` (negative pulls it in): both ends of edge `edge` — from point `edge` to the next — move along its outward normal; the walls beside it follow. push_face for an outline.", json!({ "name": { "type": "string" }, "edge": { "type": "integer" }, "metres": { "type": "number" } }), &["name", "edge", "metres"]),
        tool("brush_shape", "Greybox with brushes, as in Hammer or TrenchBroom: boxes, ramps, cylinders and stairs, each added or cut out (op: subtract) of everything before it, in order — a wall with a doorway, a room with a corridor through it. Writes assets/<name>.scrbrush (a line a brush), imports it, places it at `at` as a static body with a collider of its own shape, selected. One undo step; returns its id. To cut grey shapes already in the scene out of another, use `carve`.", json!({ "name": { "type": "string", "description": "snake_case, becomes the model's name" }, "brushes": { "type": "array", "items": { "type": "object", "properties": { "op": { "type": "string", "enum": ["add", "subtract"] } }, "required": ["shape"], "description": "a brush: op, shape, position, rotation_deg, scale, sides" }, "description": "in order; the first is usually an add" }, "at": vec3("where its origin goes") }), &["name", "brushes"]),
        tool("brush_add", "Add a brush to a brush solid (assets/<name>.scrbrush): what it covers becomes solid, even where earlier brushes were cut away. One line added to the file, rebuilt at once; every placement changes.", { let mut p = brush_fields(); p["name"] = json!({ "type": "string" }); p }, &["name", "shape"]),
        tool("brush_subtract", "Cut a brush out of a brush solid (assets/<name>.scrbrush): everything added before it loses what it covers — a doorway, a window, a tunnel. In the solid's own space. One line added to the file, rebuilt at once; every placement changes. Refused, with why, when nothing would be left.", { let mut p = brush_fields(); p["name"] = json!({ "type": "string" }); p }, &["name", "shape"]),
        tool("set_brushes", "Replace a brush solid's whole list of brushes (to move, change or take one away): the .scrbrush is rewritten and rebuilt, and every placement changes. Refused, with why, when nothing would be left.", json!({ "name": { "type": "string" }, "brushes": { "type": "array", "items": { "type": "object", "required": ["shape"], "description": "a brush: op (add|subtract), shape, position, rotation_deg, scale, sides" } } }), &["name", "brushes"]),
        tool("snap_selection", "Put the selection on the grid: positions to the nearest snap step (a metre when snapping is off), turns to the nearest angle step when there is one — Unity's Snap All Axes. One undo step; says how many moved.", json!({}), &[]),
        tool("fit_collider", "Give an entity a box collider that fits its model — size and centre from the model's bounds — as Unity does when a BoxCollider is added. One undo step.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("hide", "Hide entities (and what is under them) from `render`, or with show: true bring them back — the roof off a house to look inside. A view setting: nothing in the scene file, no undo step.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" }, "show": { "type": "boolean" } }), &["ids"]),
        tool("path", "Can something walk from one point to another in the scene as it stands, and which way? Baked from the static colliders: slope, step height and the walker's radius decide. Returns the corners and the length, or says there is no way.", json!({
            "from": vec3("start, on or above the ground"),
            "to": vec3("goal"),
            "radius": { "type": "number", "description": "the walker's radius, metres (the project player's: scrap.ron game.player.radius)" },
            "max_step": { "type": "number", "description": "the highest step it climbs, metres (the player's step)" },
            "max_slope": { "type": "number", "description": "the steepest slope it walks, degrees (the player's slope)" },
        }), &["from", "to"]),
        tool("locks", "Who holds which Git LFS lock in the project: lock binary sources (textures, models, sounds) before editing them.", json!({}), &[]),
        tool("lock", "Take (or with locked: false, give back) the Git LFS lock on a file.", json!({ "path": { "type": "string" }, "locked": { "type": "boolean" } }), &["path"]),
        tool("apply_overrides", "Write a prefab instance's overrides into the prefab file, so every instance gets them, and clear them from this instance.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("unpack_prefab", "Turn a prefab instance into plain entities of the scene, overrides applied, no longer following the prefab file. One undo step; its parts keep their ids.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("replace_with_prefab", "Put a prefab where each thing a search finds is — the greybox cubes named `crate` become the real crate — keeping ids, names and places. One undo step.", json!({ "query": { "type": "string", "description": "what to replace, as find takes it: `crate`, `m:grid model:builtin:cube`" }, "prefab": { "type": "string" } }), &["query", "prefab"]),
        tool("revert_overrides", "Drop a prefab instance's overrides: it is the prefab again. One undo step.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("edits", "Every edit undo can take back in this session, oldest first, in words: what has been done since the scene was opened.", json!({}), &[]),
        tool("render", "Draw the view and return it as a PNG. Camera arguments move the view first and are not an edit.", camera, &[]),
        tool("look", "The scene's look — sun, fog, sky (mode: Procedural|Physical, clouds), post (bloom, grading, depth_of_field, lens_flare...), ambient_occlusion, volumetric_fog, weather (rain, wetness, puddles, snow, snowfall), wind, screen_space_reflections, ray_tracing — as the file writes them. Give any of them as RON to set them, all in one undo step; \"None\" clears an optional one back to the engine's default. Returns the look after; render to see it.", json!({
            "sun": { "type": "string", "description": "(hour: 17.5, intensity: 1.2)" },
            "fog": { "type": "string", "description": "(color: (0.62, 0.68, 0.74), start: 30.0, end: 180.0)" },
            "sky": { "type": "string", "description": "(mode: Physical, clouds: (coverage: 0.4)) or None" },
            "post": { "type": "string", "description": "(bloom: (intensity: 0.4), temperature: 10.0, depth_of_field: (mode: Bokeh, focus_distance: 6.0)) or None" },
            "ambient_occlusion": { "type": "string" },
            "volumetric_fog": { "type": "string", "description": "(enabled: true, density: 0.04) or None" },
            "weather": { "type": "string", "description": "(rain: 1.0, wetness: 1.0, puddles: 0.5) or None" },
            "wind": { "type": "string", "description": "(direction: (1.0, 0.0, 0.3), strength: 1.5) or None" },
            "screen_space_reflections": { "type": "string", "description": "(enabled: true) or None" },
            "ray_tracing": { "type": "string" }
        }), &[]),
        tool("mood", "Give the scene a mood in one step — its sun, sky, fog, weather, wind and grading together: clear noon, golden hour, overcast, misty morning, rainy, snowy, night, storm, desert noon, sandstorm. One undo step; returns what it set and a render of the result. Without `name`, lists the moods and what each is for. Tune further with look.", json!({ "name": { "type": "string" } }), &[]),
        tool("pick", "The entity under a pixel of the last render.", json!({ "x": { "type": "integer" }, "y": { "type": "integer" } }), &["x", "y"]),
        tool("import", "Import a source file (.gltf .glb .obj .png .jpg .tga .bmp .wav .scrmat) into the project.", json!({ "source": { "type": "string" } }), &["source"]),
        tool("rename_asset", "Rename or move an asset source (model, texture, sound in assets/, .scrmat in materials/, .prefab in prefabs/), its .scrimport with it, and rewrite every scene and prefab line that named it. Paths relative to the project root. Refused, with the reason, when the new name already means something.", json!({ "from": { "type": "string" }, "to": { "type": "string" } }), &["from", "to"]),
        tool("assets", "Every asset source in the project — models, textures, sounds, materials, prefabs — with its kind, id, whether it is built, and how many lines use it.", json!({}), &[]),
        tool("delete_asset", "Delete an asset source with its .scrimport and built asset. Refused, listing the lines, while any scene or prefab (or the open scene's unsaved edits) still names it.", json!({ "file": { "type": "string" } }), &["file"]),
        tool("duplicate_asset", "Copy an asset source under a new name: a new asset with its own id and the original's import settings.", json!({ "from": { "type": "string" }, "to": { "type": "string" } }), &["from", "to"]),
        tool("usages", "Every scene and prefab line that names an asset file: what a rename would change, and whether it is safe to delete.", json!({ "file": { "type": "string", "description": "relative to the project root, e.g. materials/stone.scrmat" } }), &["file"]),
        tool("reload", "Pick up files changed on disk: the scene, prefabs, and assets rebuilt from changed sources.", json!({}), &[]),
        tool("problems", "What is wrong with the open document right now, unsaved edits included: models, materials and prefabs nothing answers to, stale overrides — each with the entity id and the likely intended name. Empty means clean.", json!({}), &[]),
        tool("check", "Everything in the project that does not resolve, with file, entity and the fix.", json!({}), &[]),
        tool("graph", "An animator graph (animators/<name>.ron): its start, every state with what it plays, every transition with its conditions, each layer (mask, blend, weight) with its own states and transitions, the parameters it reads, what is wrong with its shape, and what changed since the last commit.", json!({ "name": { "type": "string", "description": "the graph's file name without .ron" } }), &["name"]),
        tool("graph_connect", "Add a transition to an animator graph, written into the file where it goes (in the state it leaves). `from` is a state, or \"*\" for any state.", json!({
            "name": { "type": "string" },
            "from": { "type": "string" },
            "to": { "type": "string" },
            "when": { "type": "string", "description": "conditions as RON, e.g. [Above(\"speed\", 0.1), Trigger(\"jump\")]; Is, Not, Below, Finished too. Empty: always." },
            "fade": { "type": "number", "description": "seconds; 0.2 when not given" },
        }), &["name", "from", "to"]),
        tool("graph_disconnect", "Remove the transitions from one state to another in an animator graph.", json!({ "name": { "type": "string" }, "from": { "type": "string" }, "to": { "type": "string" } }), &["name", "from", "to"]),
        tool("graph_rename", "Rename a state in an animator graph, and everything that names it: the start, the transitions, and the graph's cases (animators/<name>.cases.ron).", json!({ "name": { "type": "string" }, "from": { "type": "string" }, "to": { "type": "string" } }), &["name", "from", "to"]),
        tool("dialogue", "A dialogue (dialogues/<name>.ron): its start, every line with who says it, what, when, and where it goes, every answer, what is wrong with it, its cases (dialogues/<name>.cases.ron) played, and what changed since the last commit. An empty name lists the dialogues.", json!({ "name": { "type": "string", "description": "the dialogue's path in dialogues/ without .ron" } }), &["name"]),
        tool("dialogue_line", "Write one line of a dialogue, into the file where it goes (the rest of the file as it was). `entry` is the line as RON — (speaker: \"@chef\", text: \"@chef.hi\", next: \"ask\"), with when: [Is(\"f\"), Not(\"f\"), Var(\"n\", Ge, 3)], else, set: [\"f\"], add: {\"n\": 1}, put: {\"n\": 0}, event, choices: [(text, to, when, once: true, set, add, put, event)]; an empty entry removes the line. A dialogue that is not there is made, starting at this line.", json!({
            "name": { "type": "string" },
            "line": { "type": "string" },
            "entry": { "type": "string" },
            "start": { "type": "boolean", "description": "make it the line the dialogue starts at" },
        }), &["name", "line", "entry"]),
        tool("dialogue_rename", "Rename a line of a dialogue, and everything that names it: the start, next, else, answers' to, and the dialogue's cases.", json!({ "name": { "type": "string" }, "from": { "type": "string" }, "to": { "type": "string" } }), &["name", "from", "to"]),
        tool("dialogue_play", "Play a dialogue without the game: from the start (or `from`), on through lines, answering with `answers` (by the answer's text) in turn, until an answer is wanted and none is left, or it is over. Says every line, the answers on offer, the events told, and the flags and numbers at the end.", json!({
            "name": { "type": "string" },
            "answers": { "type": "array", "items": { "type": "string" } },
            "flags": { "type": "array", "items": { "type": "string" }, "description": "flags set before it begins" },
            "vars": { "type": "object", "description": "numbers before it begins, {\"coins\": 3}" },
            "from": { "type": "string", "description": "a line to begin at instead of the start" },
        }), &["name"]),
        tool("export_lines", "Everything the dialogues say as a CSV sheet per language of strings/ (id <dialogue>/<line>, speaker, key, text), for recording voices and for translators — what `scrap lines` writes.", json!({ "out": { "type": "string", "description": "folder relative to the project; build/lines by default" } }), &[]),
        tool("simulate", "Play the scene for some seconds, report where the physics bodies ended up, render, and stop. The document is not changed, except the entities in `keep`, which stay where they fell (one undo step).", simulate, &["seconds"]),
    ]
    .into_iter()
    .chain(scrap_editor::actions::registry().iter().map(action_tool))
    .collect()
}

/// An editor action as a tool: the same name, sentence and arguments the
/// menu's item has (`scrap_editor::actions`).
fn action_tool(action: &scrap_editor::actions::EditorAction) -> Value {
    use scrap_editor::actions::Kind;
    let mut properties = serde_json::Map::new();
    for param in action.params {
        let schema = match param.kind {
            Kind::Id | Kind::Text => json!({ "type": "string", "description": param.about }),
            Kind::Ids => json!({ "type": "array", "items": { "type": "string" }, "description": param.about }),
            Kind::Flag => json!({ "type": "boolean", "description": param.about }),
        };
        properties.insert(param.name.to_string(), schema);
    }
    tool(action.name, action.about, Value::Object(properties), &[])
}

/// A tool call's arguments as an editor action takes them.
fn action_args(action: &scrap_editor::actions::EditorAction, args: &Value) -> Result<scrap_editor::actions::Args, String> {
    use scrap_editor::actions::{Arg, Args, Kind};
    let mut out = Args::default();
    for param in action.params {
        let Some(value) = args.get(param.name).filter(|v| !v.is_null()) else {
            continue;
        };
        let parse = |v: &Value| {
            v.as_str()
                .and_then(|s| s.parse::<EntityId>().ok())
                .ok_or_else(|| format!("{} is {ID}, not {v}", param.name))
        };
        let arg = match param.kind {
            Kind::Id => Arg::Id(parse(value)?),
            Kind::Ids => Arg::Ids(
                value
                    .as_array()
                    .ok_or_else(|| format!("{} is a list of entity ids", param.name))?
                    .iter()
                    .map(parse)
                    .collect::<Result<_, _>>()?,
            ),
            Kind::Text => Arg::Text(value.as_str().ok_or_else(|| format!("{} is text", param.name))?.to_string()),
            Kind::Flag => Arg::Flag(value.as_bool().ok_or_else(|| format!("{} is true or false", param.name))?),
        };
        out = out.with(param.name, arg);
    }
    Ok(out)
}

pub fn call(server: &mut Server, name: &str, args: &Value) -> Answer {
    // An editor action: the same one the menu runs.
    if let Some(action) = scrap_editor::actions::find(name) {
        let args = action_args(&action, args)?;
        return Ok(vec![text((action.run)(server.session()?, &args)?)]);
    }
    match name {
        "new_project" => new_project(server, args),
        "open_scene" => open_scene(server, &string(args, "path")?),
        "open_prefab" => {
            let name = string(args, "name")?;
            let skipped = server
                .session()?
                .open_prefab(&name)
                .map_err(|e| e.to_string())?;
            let mut out = format!("editing prefab {name}; save_scene writes prefabs/{name}.prefab");
            for line in skipped {
                let _ = write!(out, "\nskipped: {line}");
            }
            Ok(vec![text(out)])
        }
        "scene_tree" => Ok(vec![text(tree(server)?)]),
        "find" => {
            let query = string(args, "query")?;
            let session = server.session()?;
            let found = session.search(&query).map_err(|e| e.to_string())?;
            Ok(vec![text(if found.is_empty() {
                format!("nothing matches {query}")
            } else {
                found
                    .iter()
                    .map(|id| format!("{id} {:?}", session.entity_name(*id).unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })])
        }
        "inspect" => {
            let ids = match args.get("ids") {
                Some(_) => id_list(args)?,
                None => vec![id(args, "id")?],
            };
            let fields = server
                .session()?
                .inspect_all(&ids)
                .ok_or("no entity with one of those ids".to_string())?;
            Ok(vec![text(
                fields
                    .iter()
                    .map(|f| {
                        format!(
                            "{}{}: {}{}",
                            f.name,
                            if f.overridden { " (overridden)" } else { "" },
                            f.value,
                            if f.shape.is_empty() {
                                String::new()
                            } else {
                                format!("    — {}", f.shape)
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            )])
        }
        "set_field" => {
            let (field, value) = (string(args, "field")?, string(args, "value")?);
            if args.get("ids").is_some() {
                let ids = id_list(args)?;
                server
                    .session()?
                    .set_field_all(&ids, &field, &value)
                    .map_err(|e| e.to_string())?;
                return Ok(vec![text(format!("{} {field} = {value}", ids.len()))]);
            }
            let id = id(args, "id")?;
            server
                .session()?
                .set_field(id, &field, &value)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{id} {field} = {value}"))])
        }
        "table" => {
            let session = server.session()?;
            let Some(source) = optional_string(args, "source")? else {
                let sources = session.table_sources();
                return Ok(vec![text(if sources.is_empty() {
                    "no tables: no tuning/*.ron and no components in the scene".to_string()
                } else {
                    sources.join("\n")
                })]);
            };
            let table = session.table(&source).map_err(|e| e.to_string())?;
            let mut out = String::new();
            for problem in &table.problems {
                let _ = writeln!(out, "problem: {problem}");
            }
            let header: Vec<String> = table
                .columns
                .iter()
                .map(|c| {
                    if c.shape.is_empty() {
                        c.name.clone()
                    } else {
                        format!("{} ({})", c.name, c.shape)
                    }
                })
                .collect();
            let _ = writeln!(out, "key\tlabel\t{}", header.join("\t"));
            for row in &table.rows {
                let _ = writeln!(out, "{}\t{}\t{}", row.key, row.label, row.cells.join("\t"));
                for problem in &row.problems {
                    let _ = writeln!(out, "  problem: {problem}");
                }
            }
            if table.rows.is_empty() {
                out.push_str("no rows\n");
            }
            Ok(vec![text(out.trim_end_matches('\n').to_string())])
        }
        "set_cell" => {
            let (source, column, value) = (
                string(args, "source")?,
                string(args, "column")?,
                string(args, "value")?,
            );
            let rows: Vec<String> = match args.get("rows").and_then(Value::as_array) {
                Some(rows) => rows
                    .iter()
                    .map(|r| {
                        r.as_str()
                            .map(str::to_string)
                            .ok_or("rows are strings".to_string())
                    })
                    .collect::<Result<_, _>>()?,
                None => vec![optional_string(args, "row")?.unwrap_or_default()],
            };
            server
                .session()?
                .set_cells(&source, &rows, &column, &value)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "{} {column} = {value}",
                rows.join(", ")
            ))])
        }
        "undo_cell" => {
            let undone = server.session()?.undo_cell().map_err(|e| e.to_string())?;
            Ok(vec![text(match undone {
                Some(what) => format!("undone: {what}"),
                None => "no cell to take back".to_string(),
            })])
        }
        "get_entity" => {
            let id = id(args, "id")?;
            let session = server.session()?;
            let desc = session
                .scene()
                .get(id)
                .ok_or_else(|| format!("no entity {id} in the scene"))?;
            let pretty = ron::ser::PrettyConfig::new().depth_limit(4);
            Ok(vec![text(
                ron::ser::to_string_pretty(desc, pretty).map_err(|e| e.to_string())?,
            )])
        }
        "add_entity" => add_entity(server, args),
        "update_entity" => update_entity(server, args),
        "reparent" => {
            let id = id(args, "id")?;
            let parent = optional_id(args, "parent")?;
            let index = args
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i as usize);
            let stay = args.get("stay").and_then(Value::as_bool).unwrap_or(false);
            let session = server.session()?;
            let moved = if stay || index.is_some() {
                session.move_in_hierarchy(id, parent, index)
            } else {
                session.reparent(id, parent)
            }
            .map_err(|e| e.to_string())?;
            Ok(vec![text(if moved {
                "moved"
            } else {
                "not moved: the new parent is inside the entity, or is the entity"
            })])
        }
        "make_prefab" => {
            let id = id(args, "id")?;
            let name = string(args, "name")?;
            server
                .session()?
                .make_prefab(id, &name)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "prefabs/{name}.prefab written; {id} is now an instance"
            ))])
        }
        "make_variant" => {
            let id = id(args, "id")?;
            let name = string(args, "name")?;
            server
                .session()?
                .make_variant(id, &name)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "prefabs/{name}.prefab written as a variant; {id} is now an instance of it"
            ))])
        }
        "measure" => {
            let id = id(args, "id")?;
            let other = optional_id(args, "to")?;
            let session = server.session()?;
            let (a, b) = session
                .world_bounds(id)
                .ok_or(format!("{id} draws nothing to measure"))?;
            let mut out = format!(
                "{id}: size {} from {} to {}",
                triple(b - a),
                triple(a),
                triple(b)
            );
            if let Some(other) = other {
                let (c, d) = session
                    .world_bounds(other)
                    .ok_or(format!("{other} draws nothing to measure"))?;
                let centres = ((a + b) * 0.5).distance((c + d) * 0.5);
                // The space between the two boxes on each axis; negative is
                // how far they overlap.
                let gap = (c - b).max(a - d);
                let _ = write!(
                    out,
                    "\n{other}: centres {centres:.2} m apart, gap per axis {}",
                    triple(gap)
                );
            }
            Ok(vec![text(out)])
        }
        "align" => {
            let ids: Vec<EntityId> = args
                .get("ids")
                .and_then(Value::as_array)
                .ok_or("ids is a list of entity ids")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .ok_or("an id is a string".to_string())?
                        .parse::<EntityId>()
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<_, String>>()?;
            let axis = match string(args, "axis")?.as_str() {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                other => return Err(format!("axis is x, y or z, not {other}")),
            };
            let to = match string(args, "to")?.as_str() {
                "min" => scrap_editor::Align::Min,
                "center" => scrap_editor::Align::Center,
                "max" => scrap_editor::Align::Max,
                other => return Err(format!("to is min, center or max, not {other}")),
            };
            let session = server.session()?;
            let mut first = true;
            for id in &ids {
                if first {
                    session.select(Some(*id)).map_err(|e| e.to_string())?;
                    first = false;
                } else {
                    session.add_to_selection(*id).map_err(|e| e.to_string())?;
                }
            }
            let moved = session
                .align_selection(axis, to)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{moved} moved"))])
        }
        "array" => {
            let id = id(args, "id")?;
            let count = args
                .get("count")
                .and_then(Value::as_u64)
                .ok_or("count is a whole number")? as usize;
            let step = optional_vec3(args, "step")?.ok_or("step is [x, y, z]")?;
            let copies = server
                .session()?
                .array(id, count, step)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                copies
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )])
        }
        "push_face" => {
            let id = id(args, "id")?;
            let face: scrap::edit::Face = string(args, "face")?.parse()?;
            let metres = args
                .get("metres")
                .and_then(Value::as_f64)
                .ok_or("metres is a number")? as f32;
            let session = server.session()?;
            session
                .push_face(id, face, metres)
                .map_err(|e| e.to_string())?;
            let t = session.transform(id).unwrap_or_default();
            Ok(vec![text(format!(
                "{id} now at {} scale {}",
                triple(t.position),
                triple(t.scale)
            ))])
        }
        "scatter" => {
            let what = string(args, "what")?;
            let number = |key: &str, default: f32| -> Result<f32, String> {
                match args.get(key) {
                    None | Some(Value::Null) => Ok(default),
                    Some(v) => v
                        .as_f64()
                        .map(|n| n as f32)
                        .ok_or_else(|| format!("{key} is a number, not {v}")),
                }
            };
            let defaults = scrap::edit::Scatter::default();
            let layout = scrap::edit::Scatter {
                radius: number("radius", defaults.radius)?,
                count: integer(args, "count")?.min(10_000),
                spacing: number("spacing", defaults.spacing)?,
                seed: optional_integer(args, "seed")?.map_or(defaults.seed, u64::from),
                scale: (
                    number("scale_min", defaults.scale.0)?,
                    number("scale_max", defaults.scale.1)?,
                ),
                turn: true,
            };
            let centre = optional_vec3(args, "centre")?.unwrap_or(Vec3::ZERO);
            let parent = optional_id(args, "parent")?;
            let group = server
                .session()?
                .scatter(parent, &what, centre, &layout)
                .map_err(|e| e.to_string())?;
            let placed = server
                .session()?
                .scene()
                .get(group)
                .map_or(0, |g| g.children.len());
            Ok(vec![text(format!("{group}: {placed} placed"))])
        }
        "wire" => {
            let from = id(args, "from")?;
            let act: scrap::scene::Act =
                ron::from_str(&string(args, "do")?).map_err(|e| format!("do: {e}"))?;
            let on = match optional_string(args, "on")? {
                Some(on) => ron::from_str(&on)
                    .map_err(|_| format!("on is Enter, Leave or Empty, not {on}"))?,
                None => scrap::scene::On::Enter,
            };
            let wire = scrap::scene::Wire {
                on,
                only: optional_string(args, "only")?.unwrap_or_default(),
                to: id(args, "to")?,
                act,
                once: args.get("once").and_then(Value::as_bool).unwrap_or(false),
            };
            let session = server.session()?;
            session.wire(from, wire).map_err(|e| e.to_string())?;
            let problems: Vec<String> = session
                .problems()
                .into_iter()
                .filter(|d| d.entity == Some(from))
                .map(|d| d.message)
                .collect();
            let wires = session
                .inspect(from)
                .and_then(|f| f.into_iter().find(|f| f.name == "wires"))
                .map(|f| f.value)
                .unwrap_or_default();
            let mut said = format!("{from} wires = {wires}");
            for problem in problems {
                said.push_str(&format!("\n{problem}"));
            }
            Ok(vec![text(said)])
        }
        "fence" => {
            let what = string(args, "what")?;
            let points: Vec<Vec3> = args
                .get("points")
                .and_then(Value::as_array)
                .ok_or("points is a list of [x, y, z]")?
                .iter()
                .map(|p| {
                    let n: Vec<f32> = p
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_f64)
                                .map(|v| v as f32)
                                .collect()
                        })
                        .unwrap_or_default();
                    match n.as_slice() {
                        [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
                        _ => Err(format!("a point is [x, y, z], not {p}")),
                    }
                })
                .collect::<Result<_, _>>()?;
            if points.len() < 2 {
                return Err("a fence needs at least two points".into());
            }
            let spacing = args.get("spacing").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let session = server.session()?;
            let fence = session
                .add_fence(&what, points[0], spacing)
                .map_err(|e| e.to_string())?;
            let local: Vec<Vec3> = points.iter().map(|p| *p - points[0]).collect();
            let spline = scrap::Spline {
                points: local,
                closed: false,
            };
            session
                .set_field(
                    fence,
                    "spline",
                    &scrap::ron::to_string(&spline).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            session.squash_last(2);
            let copies = session.spawned_count();
            Ok(vec![text(format!(
                "{fence}: fence of {what}, {copies} in the world now"
            ))])
        }
        "paint_foliage" => {
            let what = string(args, "what")?;
            let centre = optional_vec3(args, "centre")?.unwrap_or(Vec3::ZERO);
            let number = |key: &str, default: f32| -> Result<f32, String> {
                match args.get(key) {
                    None | Some(Value::Null) => Ok(default),
                    Some(v) => v
                        .as_f64()
                        .map(|n| n as f32)
                        .ok_or_else(|| format!("{key} is a number, not {v}")),
                }
            };
            let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
            let count = server
                .session()?
                .paint_foliage(
                    &what,
                    centre,
                    number("radius", 4.0)?,
                    number("density", 0.4)?,
                    flag("align"),
                    flag("erase"),
                    optional_integer(args, "seed")?.map_or(1, u64::from),
                )
                .map_err(|e| e.to_string())?;
            let verb = if flag("erase") {
                "taken away"
            } else {
                "planted"
            };
            Ok(vec![text(format!("{count} {verb}"))])
        }
        "history" => {
            let session = server.session()?;
            let revisions = if args.get("project").and_then(Value::as_bool) == Some(true) {
                session.project_history(200)
            } else {
                session.scene_history()
            }
            .map_err(|e| e.to_string())?;
            if revisions.is_empty() {
                return Ok(vec![text("not in any commit yet")]);
            }
            let lines: Vec<String> = revisions
                .iter()
                .map(|r| {
                    format!(
                        "{} {} {} — {}",
                        &r.commit[..r.commit.len().min(10)],
                        r.date,
                        r.author,
                        r.summary
                    )
                })
                .collect();
            Ok(vec![text(lines.join("\n"))])
        }
        "changes" => {
            let commit = string(args, "commit")?;
            let changes = server
                .session()?
                .commit_changes(&commit)
                .map_err(|e| e.to_string())?;
            let mut lines: Vec<String> = changes
                .files
                .iter()
                .map(|(letter, name)| format!("{letter} {name}"))
                .collect();
            if !changes.scene.is_empty() {
                lines.push(String::new());
                lines.push("in the open scene:".into());
                lines.extend(changes.scene.iter().map(|c| format!("  {c}")));
            }
            if lines.is_empty() {
                return Ok(vec![text("it changed nothing in the project")]);
            }
            Ok(vec![text(lines.join("\n"))])
        }
        "git_status" => {
            let session = server.session()?;
            let status = session.git_status().map_err(|e| e.to_string())?;
            let mut lines = vec![git_branch_line(&status)];
            if session.is_modified() {
                lines.push("the open scene has unsaved edits".into());
            }
            if status.files.is_empty() {
                lines.push("nothing to commit".into());
            }
            lines.extend(
                status
                    .files
                    .iter()
                    .map(|f| format!("{} {}", f.letter(), f.name)),
            );
            Ok(vec![text(lines.join("\n"))])
        }
        "commit" => {
            let message = string(args, "message")?;
            let session = server.session()?;
            let paths: Vec<std::path::PathBuf> = match args.get("paths") {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| v.as_str().map(Into::into).ok_or("paths are strings"))
                    .collect::<Result<_, _>>()?,
                _ => session
                    .git_status()
                    .map_err(|e| e.to_string())?
                    .files
                    .into_iter()
                    .map(|f| f.path)
                    .collect(),
            };
            let commit = session
                .commit(&paths, &message)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "committed {commit}: {} file{}",
                paths.len(),
                if paths.len() == 1 { "" } else { "s" }
            ))])
        }
        "restore" => {
            let commit = string(args, "commit")?;
            server
                .session()?
                .restore_revision(&commit)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "the scene as it was at {commit}; not saved"
            ))])
        }
        "conflicts" => {
            let session = server.session()?;
            let conflicts = session.merge_conflicts().map_err(|e| e.to_string())?;
            if conflicts.is_empty() {
                return Ok(vec![text("no merge conflicts in the open scene")]);
            }
            let sides = session.conflict_sides();
            let lines: Vec<String> = conflicts
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let holds = match sides.get(i).copied().flatten() {
                        Some(scrap::merge::Side::Ours) => " [holds ours]",
                        Some(scrap::merge::Side::Theirs) => " [holds theirs]",
                        None => "",
                    };
                    format!("{i}: {c}{holds}")
                })
                .collect();
            Ok(vec![text(lines.join("\n"))])
        }
        "take_theirs" | "take_ours" => {
            let index = integer(args, "conflict")? as usize;
            let session = server.session()?;
            let side = if name == "take_ours" {
                session.take_ours(index)
            } else {
                session.take_theirs(index)
            };
            side.map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "conflict {index} settled {} way; not saved",
                if name == "take_ours" { "ours'" } else { "theirs'" }
            ))])
        }
        "resolved" => {
            server
                .session()?
                .mark_resolved()
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                "the scene's conflicts are settled for git; commit to finish the merge",
            )])
        }
        "copy" => {
            let ids = match args.get("ids") {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .and_then(|s| s.parse::<EntityId>().ok())
                            .ok_or_else(|| format!("ids are {ID}, not {v}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err("ids is a list of entity ids".into()),
            };
            let session = server.session()?;
            let mut first = true;
            for id in ids {
                if first {
                    session.select(Some(id)).map_err(|e| e.to_string())?;
                    first = false;
                } else {
                    session.add_to_selection(id).map_err(|e| e.to_string())?;
                }
            }
            Ok(vec![text(session.copy_selection())])
        }
        "paste" => {
            let ron = string(args, "ron")?;
            let parent = optional_id(args, "parent")?;
            let pasted = server
                .session()?
                .paste(&ron, parent)
                .map_err(|e| e.to_string())?;
            let ids: Vec<String> = pasted.iter().map(ToString::to_string).collect();
            Ok(vec![text(ids.join("\n"))])
        }
        "sculpt" => {
            let terrain = id(args, "id")?;
            let at = optional_vec3(args, "at")?.ok_or("at is required")?;
            let number = |key: &str| {
                args.get(key)
                    .and_then(Value::as_f64)
                    .map(|n| n as f32)
                    .ok_or_else(|| format!("{key} is a number"))
            };
            let flatten = args
                .get("flatten")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            server
                .session()?
                .sculpt(terrain, at, number("radius")?, number("by")?, flatten)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                "sculpted; the stroke is a line in the terrain's .scrterrain",
            )])
        }
        "place" => {
            let ids = id_list(args)?;
            let (x, y) = (integer(args, "x")?, integer(args, "y")?);
            let session = server.session()?;
            for (i, id) in ids.iter().enumerate() {
                if i == 0 {
                    session.select(Some(*id)).map_err(|e| e.to_string())?;
                } else {
                    session.add_to_selection(*id).map_err(|e| e.to_string())?;
                }
            }
            if !session.place_on_surface(x, y).map_err(|e| e.to_string())? {
                return Err(format!("nothing to stand on at ({x}, {y})"));
            }
            Ok(vec![text(format!("placed {}", ids.len()))])
        }
        "new_scene" => {
            let name = string(args, "name")?;
            let session = server.session()?;
            let path = session.new_scene(&name).map_err(|e| e.to_string())?;
            let names = session
                .project()
                .map(|p| p.scene_names().join(", "))
                .unwrap_or_default();
            Ok(vec![text(format!(
                "opened {}; scenes: {names}",
                path.display()
            ))])
        }
        "to_view" => {
            let id = id(args, "id")?;
            let how = string(args, "how")?;
            let session = server.session()?;
            session.select(Some(id)).map_err(|e| e.to_string())?;
            let done = match how.as_str() {
                "move" => session.move_to_view(),
                "align" => session.align_with_view(),
                other => return Err(format!("how is move or align, not {other}")),
            }
            .map_err(|e| e.to_string())?;
            let at = session.world_position(id).unwrap_or_default();
            Ok(vec![text(if done {
                format!("at ({:.2}, {:.2}, {:.2})", at.x, at.y, at.z)
            } else {
                "not moved".to_string()
            })])
        }
        "override_field" => {
            let id = id(args, "id")?;
            let (field, how) = (string(args, "field")?, string(args, "how")?);
            let session = server.session()?;
            let done = match how.as_str() {
                "apply" => session.apply_field(id, &field),
                "revert" => session.revert_field(id, &field),
                other => return Err(format!("how is apply or revert, not {other}")),
            }
            .map_err(|e| e.to_string())?;
            Ok(vec![text(if done {
                format!("{how} {field}")
            } else {
                format!("{field} is not overridden there")
            })])
        }
        "start_game" => {
            let session = server.session()?;
            if let Some(count) = args.get("players").and_then(|v| v.as_u64()) {
                session.set_players(count as u32);
            }
            if let Some(link) = args.get("link").and_then(|v| v.as_str()) {
                session.set_link(link).map_err(|e| e.to_string())?;
            }
            let from_here = args
                .get("from_here")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let start = if from_here {
                Some(session.start_game_from_here().map_err(|e| e.to_string())?)
            } else {
                session.start_game().map_err(|e| e.to_string())?;
                None
            };
            let players = session.players();
            let link = session.link();
            let over = if link.is_empty() {
                String::new()
            } else {
                format!(", the others over a `{link}` link")
            };
            let here = match start {
                Some(start) => {
                    let marked = scrap::player::player_starts(session.scene()).len();
                    format!(
                        " from {} facing {:.0}° ({marked} lines marked player_start)",
                        triple(start.position),
                        start.yaw_deg
                    )
                }
                None => String::new(),
            };
            Ok(vec![text(if players > 1 {
                format!("started with {players} players{over}{here}; their output goes to the console as it comes")
            } else {
                format!("started{here}; its output goes to the console as it comes")
            })])
        }
        "game_state" => {
            let session = server.session()?;
            session.poll_game();
            let state = session
                .game_state()
                .ok_or("no game started with start_game is running, or it has said nothing yet")?;
            let mut out = String::new();
            let mut unmoved = 0;
            for saved in &state.entities {
                let name = session.entity_name(saved.id).unwrap_or_default();
                let moved = session
                    .transform(saved.id)
                    .is_none_or(|t| t != saved.transform);
                if !saved.prefab.is_empty() {
                    let _ = write!(out, "spawned {} from {}", saved.id, saved.prefab);
                } else if moved || !saved.components.is_empty() {
                    let _ = write!(out, "{} {name:?}", saved.id);
                } else {
                    unmoved += 1;
                    continue;
                }
                let _ = write!(out, " at {}", triple(saved.transform.position));
                for (component, value) in &saved.components {
                    let _ = write!(out, " {component}: {value}");
                }
                out.push('\n');
            }
            for id in &state.gone {
                let _ = writeln!(
                    out,
                    "gone {id} {:?}",
                    session.entity_name(*id).unwrap_or_default()
                );
            }
            let _ = write!(out, "{unmoved} more where the scene puts them");
            // How each animated thing came to be in its state.
            for (id, trail) in state
                .diagnostics
                .as_ref()
                .map(|d| d.animators.as_slice())
                .unwrap_or(&[])
            {
                let name = session.entity_name(*id).unwrap_or_default();
                let _ = write!(out, "\nanimator of {id} {name:?}:");
                for passage in trail {
                    let _ = write!(out, "\n  {passage}");
                }
            }
            Ok(vec![text(out)])
        }
        "stop_game" => {
            let session = server.session()?;
            let answer = if let Some(code) = session.poll_game() {
                format!("it had already ended, with code {code}")
            } else if session.stop_game() {
                "stopped".to_string()
            } else {
                "no game was running".to_string()
            };
            Ok(vec![text(answer)])
        }
        "game_console" => {
            let command = string(args, "command")?;
            let wait = args
                .get("wait_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(5.0)
                .clamp(0.1, 60.0);
            let session = server.session()?;
            let answer = session
                .game_console(&command, std::time::Duration::from_secs_f64(wait))
                .map_err(|e| e.to_string())?;
            Ok(vec![text(answer)])
        }
        "console" => {
            let session = server.session()?;
            session.poll_game();
            let mut out = String::new();
            for line in session.console() {
                let level = match line.level {
                    scrap_editor::console::Level::Info => "info",
                    scrap_editor::console::Level::Warning => "warning",
                    scrap_editor::console::Level::Error => "error",
                };
                let times = if line.count > 1 {
                    format!(" (x{})", line.count)
                } else {
                    String::new()
                };
                let _ = writeln!(out, "{level}: {}{times}", line.text);
            }
            if args.get("clear").and_then(Value::as_bool).unwrap_or(false) {
                session.clear_console();
            }
            Ok(vec![text(if out.is_empty() {
                "nothing said".to_string()
            } else {
                out
            })])
        }
        "group" => {
            let ids = id_list(args)?;
            let name = string(args, "name")?;
            let session = server.session()?;
            session.select(None).map_err(|e| e.to_string())?;
            for id in &ids {
                session.add_to_selection(*id).map_err(|e| e.to_string())?;
            }
            let group = session.group_selection(&name).map_err(|e| e.to_string())?;
            Ok(vec![text(group.to_string())])
        }
        "thumbnail" => {
            let what = string(args, "what")?;
            let size = optional_integer(args, "size")?
                .unwrap_or(256)
                .clamp(16, 1024);
            let pixels = server
                .session()?
                .thumbnail(&what, size)
                .map_err(|e| e.to_string())?;
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&pixels, size, size, image::ExtendedColorType::Rgba8)
                .map_err(|e| e.to_string())?;
            Ok(vec![image(&png), text(format!("{what}, {size}x{size}"))])
        }
        "drop" => {
            let what = string(args, "what")?;
            let (x, y) = (integer(args, "x")?, integer(args, "y")?);
            let id = server
                .session()?
                .drop_asset(&what, x, y)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(id.to_string())])
        }
        "configs" => {
            let root = project_root(server)?;
            if let Some(find) = optional_string(args, "find")? {
                let needle = find.to_lowercase();
                let mut out = String::new();
                for file in scrap_editor::configs::files(&root) {
                    let Ok(text) = std::fs::read_to_string(&file) else { continue };
                    let Ok(config) = scrap_editor::configs::read(&text) else { continue };
                    let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
                    for table in &config.tables {
                        for row in &table.rows {
                            if row[0].to_lowercase().contains(&needle) {
                                let under = table.name.as_deref().map(|n| format!(" {n}")).unwrap_or_default();
                                let _ = writeln!(out, "{rel}{under}: {}", row[0]);
                            }
                        }
                    }
                }
                return Ok(vec![text(if out.is_empty() { format!("nothing called like {find:?}") } else { out })]);
            }
            let Some(file) = optional_string(args, "file")? else {
                let mut out = String::new();
                for file in scrap_editor::configs::files(&root) {
                    let rel = file.strip_prefix(&root).unwrap_or(&file).display().to_string();
                    match scrap_editor::configs::shape_of(&root, &file) {
                        Some(shape) => {
                            let _ = writeln!(out, "{rel}: a table of {}: {}", shape.record, shape.shape);
                        }
                        None => {
                            let _ = writeln!(out, "{rel}");
                        }
                    }
                }
                return Ok(vec![text(if out.is_empty() { "no configs/ files".into() } else { out })]);
            };
            let path = root.join(&file);
            let body = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
            let shape = scrap_editor::configs::shape_of(&root, &path);
            let config = scrap_editor::configs::read_with(&body, shape.as_ref())?;
            let mut out = String::new();
            if let Some(shape) = &shape {
                let _ = writeln!(out, "records of {}", shape.record);
            }
            for (key, value) in &config.fields {
                let _ = writeln!(out, "{key}: {value}");
            }
            for table in &config.tables {
                if let Some(name) = &table.name {
                    let _ = writeln!(out, "\n{name}:");
                }
                let _ = writeln!(out, "{}", table.columns.join(" | "));
                for (row, taken) in table.rows.iter().zip(&table.inherited) {
                    let cells: Vec<String> = row
                        .iter()
                        .zip(taken)
                        .map(|(cell, taken)| if *taken { format!("^{cell}") } else { cell.clone() })
                        .collect();
                    let _ = writeln!(out, "{}", cells.join(" | "));
                }
            }
            Ok(vec![text(out)])
        }
        "config_set" => {
            let root = project_root(server)?;
            let file = string(args, "file")?;
            let path = root.join(&file);
            let field = string(args, "field")?;
            let value = string(args, "value")?;
            let place = match (optional_string(args, "record")?, optional_string(args, "grid")?) {
                (Some(record), _) => {
                    let body = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
                    let config = scrap_editor::configs::read(&body)?;
                    let row = config
                        .tables
                        .iter()
                        .find(|t| t.records)
                        .and_then(|t| t.rows.iter().position(|r| r[0] == record))
                        .ok_or_else(|| format!("{file}: no record `{record}`"))?;
                    scrap_editor::configs::Place::Cell { grid: None, row, column: field }
                }
                (None, Some(grid)) => scrap_editor::configs::Place::Cell {
                    grid: Some(grid),
                    row: integer(args, "row")? as usize,
                    column: field,
                },
                (None, None) => scrap_editor::configs::Place::Field(field),
            };
            let label = server.configs.set(&root, &path, &place, &value)?;
            Ok(vec![text(if label.is_empty() { "already so".into() } else { format!("set {label}") })])
        }
        "config_undo" => Ok(vec![text(match server.configs.undo()? {
            Some(label) => format!("undone: {label}"),
            None => "nothing to undo".into(),
        })]),
        "add_component" => {
            let session = server.session()?;
            let Some(name) = args.get("name").and_then(Value::as_str) else {
                let shapes = session.component_shapes();
                if shapes.is_empty() {
                    return Ok(vec![text(
                        "no library/components.ron yet: run the game or `scrap test` once"
                            .to_string(),
                    )]);
                }
                let mut out = String::new();
                for (name, shape) in shapes {
                    let _ = writeln!(out, "{name}: {shape}");
                }
                return Ok(vec![text(out)]);
            };
            let id = id(args, "id")?;
            let value = session.add_component(id, name).map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{name}: {value}"))])
        }
        "import_settings" => {
            let source = string(args, "source")?;
            let session = server.session()?;
            if let (Some(field), Some(value)) = (
                args.get("field").and_then(Value::as_str),
                args.get("value").and_then(Value::as_str),
            ) {
                session
                    .set_import_setting(&source, field, value)
                    .map_err(|e| e.to_string())?;
            }
            let s = session
                .import_settings(&source)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "{source}: scale {}, recompute_normals {}, srgb {}, origin_to_base {}",
                s.scale, s.recompute_normals, s.srgb, s.origin_to_base
            ))])
        }
        "brush_shape" | "set_brushes" => {
            let making = name == "brush_shape";
            let name = string(args, "name")?;
            let brushes = args
                .get("brushes")
                .and_then(Value::as_array)
                .ok_or("brushes is a list of {shape, op, position, rotation_deg, scale}")?
                .iter()
                .map(|b| brush_of(b, None))
                .collect::<Result<Vec<_>, _>>()?;
            let source = scrap_import::brush::BrushSource { brushes };
            let session = server.session()?;
            if making {
                let at = optional_vec3(args, "at")?.unwrap_or(Vec3::ZERO);
                let id = session
                    .brush_shape(&name, &source, at)
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(format!("{id} {name:?}, assets/{name}.scrbrush"))])
            } else {
                session
                    .set_brushes(&name, &source)
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(format!(
                    "{name}: {} brushes",
                    source.brushes.len()
                ))])
            }
        }
        "brush_add" | "brush_subtract" => {
            let op = if name == "brush_add" {
                scrap_import::brush::Op::Add
            } else {
                scrap_import::brush::Op::Subtract
            };
            let name = string(args, "name")?;
            let brush = brush_of(args, Some(op))?;
            let session = server.session()?;
            session
                .add_brush(&name, &brush)
                .map_err(|e| e.to_string())?;
            let count = session
                .brushes(&name)
                .map_err(|e| e.to_string())?
                .brushes
                .len();
            Ok(vec![text(format!(
                "{name}: {} as brush {}",
                brush.to_line(),
                count - 1
            ))])
        }
        "poly_shape" | "set_poly" => {
            let making = name == "poly_shape";
            let name = string(args, "name")?;
            let ring = |list: &Value| -> Result<Vec<(f32, f32)>, String> {
                list.as_array()
                    .ok_or("points is [[x, z], ...]")?
                    .iter()
                    .map(|p| match p.as_array().map(Vec::as_slice) {
                        Some([x, z]) => Ok((
                            x.as_f64().ok_or("a point is two numbers")? as f32,
                            z.as_f64().ok_or("a point is two numbers")? as f32,
                        )),
                        _ => Err("a point is [x, z]".to_string()),
                    })
                    .collect()
            };
            let points = args.get("points").map(ring).transpose()?;
            let holes = match args.get("holes") {
                None => None,
                Some(list) => Some(
                    list.as_array()
                        .ok_or("holes is [[[x, z], ...], ...]")?
                        .iter()
                        .map(ring)
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            };
            let height = args.get("height").and_then(Value::as_f64).map(|h| h as f32);
            let session = server.session()?;
            if making {
                let at = optional_vec3(args, "at")?.unwrap_or(Vec3::ZERO);
                let source = scrap_import::poly::PolySource {
                    points: points.ok_or("points is [[x, z], ...]")?,
                    height: height.ok_or("height is a number of metres")?,
                    standing: args
                        .get("standing")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    holes: holes.unwrap_or_default(),
                };
                let id = session
                    .poly_shape(&name, &source, at)
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(format!("{id} {name:?}, assets/{name}.scrpoly"))])
            } else {
                let mut source = session.poly(&name).map_err(|e| e.to_string())?;
                if let Some(points) = points {
                    source.points = points;
                }
                if let Some(height) = height {
                    source.height = height;
                }
                if let Some(standing) = args.get("standing").and_then(Value::as_bool) {
                    source.standing = standing;
                }
                if let Some(holes) = holes {
                    source.holes = holes;
                }
                session
                    .set_poly(&name, &source)
                    .map_err(|e| e.to_string())?;
                Ok(vec![text(format!(
                    "{name}: {} points, {} m high",
                    source.points.len(),
                    source.height
                ))])
            }
        }
        "push_poly_edge" => {
            let name = string(args, "name")?;
            let edge = args
                .get("edge")
                .and_then(Value::as_u64)
                .ok_or("edge is a whole number")? as usize;
            let metres = args
                .get("metres")
                .and_then(Value::as_f64)
                .ok_or("metres is a number")? as f32;
            let session = server.session()?;
            session
                .push_poly_edge(&name, edge, metres)
                .map_err(|e| e.to_string())?;
            let source = session.poly(&name).map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{name}: points {:?}", source.points))])
        }
        "snap_selection" => {
            let moved = server
                .session()?
                .snap_selection()
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{moved} moved onto the grid"))])
        }
        "fit_collider" => {
            let id = id(args, "id")?;
            let fitted = server
                .session()?
                .fit_collider(id)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(
                if fitted { "fitted" } else { "no model to fit" }.to_string(),
            )])
        }
        "hide" => {
            let ids = id_list(args)?;
            let show = args.get("show").and_then(Value::as_bool).unwrap_or(false);
            let session = server.session()?;
            session.set_hidden(&ids, !show).map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "{} {}; hidden now: {}",
                if show { "shown" } else { "hidden" },
                ids.len(),
                session.hidden().len()
            ))])
        }
        "path" => {
            let from = optional_vec3(args, "from")?.ok_or("from is required")?;
            let to = optional_vec3(args, "to")?.ok_or("to is required")?;
            // The project's player, unless the question names another.
            let defaults = server.session()?.walker();
            let number = |key: &str, default: f32| {
                args.get(key)
                    .and_then(Value::as_f64)
                    .map_or(default, |n| n as f32)
            };
            let settings = scrap::navigation::NavSettings {
                radius: number("radius", defaults.radius),
                max_step: number("max_step", defaults.max_step),
                max_slope: number("max_slope", defaults.max_slope),
                ..defaults
            };
            Ok(vec![text(
                match server.session()?.find_path(from, to, settings) {
                    Some(path) => {
                        let corners: Vec<String> = path.iter().map(|p| triple(*p)).collect();
                        format!(
                            "a way, {:.1} m: {}",
                            scrap::navigation::NavGrid::length(&path),
                            corners.join(" → ")
                        )
                    }
                    None => "no way: nothing walkable connects them".to_string(),
                },
            )])
        }
        "locks" => {
            let locks = server.session()?.locks().map_err(|e| e.to_string())?;
            if locks.is_empty() {
                return Ok(vec![text("nothing is locked")]);
            }
            let lines: Vec<String> = locks
                .iter()
                .map(|l| format!("{} — {} since {}", l.path, l.owner, l.locked_at))
                .collect();
            Ok(vec![text(lines.join("\n"))])
        }
        "lock" => {
            let path = string(args, "path")?;
            let locked = args.get("locked").and_then(Value::as_bool).unwrap_or(true);
            server
                .session()?
                .set_locked(&path, locked)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "{path} {}",
                if locked { "locked" } else { "unlocked" }
            ))])
        }
        "apply_overrides" => {
            let id = id(args, "id")?;
            let parts = server
                .session()?
                .apply_overrides(id)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{parts} parts written into the prefab"))])
        }
        "unpack_prefab" => {
            let id = id(args, "id")?;
            server
                .session()?
                .unpack_prefab(id)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{id} is plain entities now"))])
        }
        "replace_with_prefab" => {
            let (query, prefab) = (string(args, "query")?, string(args, "prefab")?);
            let session = server.session()?;
            let found = session.select_matching(&query).map_err(|e| e.to_string())?;
            if found == 0 {
                return Err(format!("nothing matches {query}"));
            }
            let replaced = session
                .replace_with_prefab(&prefab)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{replaced} replaced with {prefab}"))])
        }
        "revert_overrides" => {
            let id = id(args, "id")?;
            server
                .session()?
                .revert_overrides(id)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{id} is the prefab again"))])
        }
        "edits" => {
            let steps = server.session()?.undo_steps();
            Ok(vec![text(if steps.is_empty() {
                "no edits yet".to_string()
            } else {
                steps
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("{}. {s}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n")
            })])
        }
        "render" => {
            camera(server, args)?;
            render(server)
        }
        "look" => {
            let mut changes = Vec::new();
            for field in scrap::moods::LOOK_FIELDS {
                if let Some(value) = optional_string(args, field)? {
                    changes.push((field, value));
                }
            }
            let session = server.session()?;
            if !changes.is_empty() {
                let borrowed: Vec<(&str, &str)> =
                    changes.iter().map(|(f, v)| (*f, v.as_str())).collect();
                session.set_look(&borrowed).map_err(|e| e.to_string())?;
            }
            let mut out = String::new();
            for (field, value) in session.environment() {
                let _ = writeln!(out, "{field}: {value}");
            }
            Ok(vec![text(out)])
        }
        "mood" => {
            let Some(name) = optional_string(args, "name")? else {
                let mut out = String::new();
                for mood in scrap::moods::MOODS {
                    let _ = writeln!(out, "{} — {}", mood.name, mood.about);
                }
                return Ok(vec![text(out)]);
            };
            server
                .session()?
                .apply_mood(&name)
                .map_err(|e| e.to_string())?;
            let mood = scrap::moods::mood(&name).expect("applied above");
            let mut out = format!("{}: {}\n", mood.name, mood.about);
            for (field, value) in mood.fields {
                let _ = writeln!(out, "  {field}: {value}");
            }
            let mut result = vec![text(out)];
            result.extend(render(server)?);
            Ok(result)
        }
        "pick" => {
            let (x, y) = (integer(args, "x")?, integer(args, "y")?);
            let hit = server.session()?.pick(x, y);
            Ok(vec![text(match hit {
                Some(id) => id.to_string(),
                None => "nothing there".into(),
            })])
        }
        "import" => {
            let source = string(args, "source")?;
            let warnings = server
                .session()?
                .import(&source)
                .map_err(|e| e.to_string())?;
            let mut out = format!("imported {source}");
            for warning in warnings {
                let _ = write!(out, "\nwarning: {warning}");
            }
            Ok(vec![text(out)])
        }
        "rename_asset" => {
            let (from, to) = (string(args, "from")?, string(args, "to")?);
            let done = server
                .session()?
                .rename_asset(&from, &to)
                .map_err(|e| e.to_string())?;
            let mut out = format!("{} -> {}", done.from, done.to);
            match &done.reference {
                Some((old, new)) => {
                    let _ = write!(out, "\nscenes said {old}, now {new}");
                }
                None => out.push_str("\nno scene names it by a different name now"),
            }
            for (file, count) in &done.rewritten {
                let _ = write!(out, "\n  {file}: {count} rewritten");
            }
            for failed in done.synced.iter().filter_map(|r| r.result.as_ref().err()) {
                let _ = write!(out, "\nwarning: {failed}");
            }
            Ok(vec![text(out)])
        }
        "assets" => {
            let entries = server.session()?.assets().map_err(|e| e.to_string())?;
            let lines: Vec<String> = entries
                .iter()
                .map(|e| {
                    format!(
                        "{} {} `{}` used {}{}",
                        e.file,
                        e.kind,
                        e.name,
                        e.uses,
                        if e.built {
                            ""
                        } else {
                            " (not built — reload)"
                        }
                    )
                })
                .collect();
            Ok(vec![text(if lines.is_empty() {
                "no assets yet".to_string()
            } else {
                lines.join("\n")
            })])
        }
        "delete_asset" => {
            let file = string(args, "file")?;
            server
                .session()?
                .delete_asset(&file)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{file} deleted"))])
        }
        "duplicate_asset" => {
            let (from, to) = (string(args, "from")?, string(args, "to")?);
            server
                .session()?
                .duplicate_asset(&from, &to)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{from} copied to {to}"))])
        }
        "usages" => {
            let file = string(args, "file")?;
            let found = server
                .session()?
                .asset_usages(&file)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(if found.is_empty() {
                format!("nothing names {file}")
            } else {
                found
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            })])
        }
        "problems" => {
            let found = server.session()?.problems();
            Ok(vec![text(if found.is_empty() {
                "no problems".to_string()
            } else {
                found
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            })])
        }
        "reload" => {
            let session = server.session()?;
            let rebuilt = session.reload_assets();
            let scene = session.reload_scene().map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "scene: {scene:?}; assets rebuilt: {rebuilt}"
            ))])
        }
        "check" => {
            let session = server.session()?;
            let project = session.project().ok_or("the open scene is in no project")?;
            let findings = scrap_cli::check(project);
            if findings.is_empty() {
                return Ok(vec![text("clean")]);
            }
            let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
            Ok(vec![text(lines.join("\n"))])
        }
        "simulate" => simulate(server, args),
        "graph" | "graph_connect" | "graph_disconnect" | "graph_rename" => {
            graph_tool(server, name, args)
        }
        "dialogue" | "dialogue_line" | "dialogue_rename" | "dialogue_play" | "export_lines" => {
            dialogue_tool(server, name, args)
        }
        other => Err(format!("no tool `{other}`")),
    }
}

fn new_project(server: &mut Server, args: &Value) -> Answer {
    let path = PathBuf::from(string(args, "path")?);
    let name = match optional_string(args, "name")? {
        Some(name) => name,
        None => std::path::absolute(&path)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .ok_or("the path has no folder name; pass name")?,
    };
    let project = Project::create(&path, &name).map_err(|e| e.to_string())?;
    let scene = project.scenes().join("main.ron");
    let mut out = open_scene(server, &scene.to_string_lossy())?;
    out.insert(
        0,
        text(format!(
            "made project {name} at {}",
            project.root().display()
        )),
    );
    Ok(out)
}

fn open_scene(server: &mut Server, path: &str) -> Answer {
    let session = server.session()?;
    let problems = session.open_scene(path).map_err(|e| e.to_string())?;
    let mut out = format!("opened {path}: {} entities", session.entity_count());
    match session.project() {
        Some(project) => {
            let _ = write!(out, " in project {}", project.name());
            let has_sources = std::fs::read_dir(project.assets())
                .map(|entries| {
                    entries
                        .flatten()
                        .any(|e| !e.file_name().to_string_lossy().starts_with('.'))
                })
                .unwrap_or(false);
            if has_sources && !project.library().is_dir() {
                out.push_str("; its library is not built yet, so models from assets/ draw nothing until `reload` or `scrap sync`");
            }
        }
        None => out.push_str(", in no project: builtins only"),
    }
    for problem in problems {
        let _ = write!(out, "\nskipped {problem}");
    }
    Ok(vec![text(out)])
}

fn tree(server: &mut Server) -> Result<String, String> {
    fn line(out: &mut String, desc: &EntityDesc, depth: usize, expanded: &scrap::Scene) {
        let _ = write!(out, "{}{} {:?}", "  ".repeat(depth), desc.id, desc.name);
        if !desc.prefab.is_empty() {
            let _ = write!(out, " prefab={}", desc.prefab);
        } else if !desc.model().is_empty() {
            let _ = write!(out, " model={}", desc.model());
        }
        match &desc.material_ref() {
            MaterialRef::Named(name) => {
                let _ = write!(out, " material={name}");
            }
            MaterialRef::Inline(material) if *material != Material::default() => {
                let [r, g, b] = material.base_color;
                let _ = write!(out, " color=({r:.2}, {g:.2}, {b:.2}) linear");
            }
            MaterialRef::Inline(_) => {}
        }
        let t = desc.transform;
        let _ = write!(out, " at {}", triple(t.position));
        if t.rotation_deg != Vec3::ZERO {
            let _ = write!(out, " rot {}", triple(t.rotation_deg));
        }
        if t.scale != Vec3::ONE {
            let _ = write!(out, " scale {}", triple(t.scale));
        }
        if desc.body() != Body::None {
            let _ = write!(out, " body={:?}", desc.body());
        }
        for (name, value) in &desc.components {
            let _ = write!(out, " {name}={}", value.get_ron());
        }
        if !desc.overrides.is_empty() {
            let _ = write!(out, " overrides={}", desc.overrides.len());
        }
        out.push('\n');
        // An instance's parts, as the expanded scene has them: addressable
        // by these ids, and an edit to one is an override on the instance.
        if !desc.prefab.is_empty() {
            if let Some(expanded) = expanded.get(desc.id) {
                for part in &expanded.children {
                    part_line(out, part, depth + 1);
                }
            }
        }
        for child in &desc.children {
            line(out, child, depth + 1, expanded);
        }
    }
    fn part_line(out: &mut String, desc: &EntityDesc, depth: usize) {
        let _ = writeln!(
            out,
            "{}{} {:?} (part) model={} at {}",
            "  ".repeat(depth),
            desc.id,
            desc.name,
            desc.model(),
            triple(desc.transform.position)
        );
        for child in &desc.children {
            part_line(out, child, depth + 1);
        }
    }
    let session = server.session()?;
    let expanded = session.expanded();
    let mut out = String::new();
    for desc in &session.scene().entities {
        line(&mut out, desc, 0, expanded);
    }
    if out.is_empty() {
        out.push_str("(empty scene)");
    }
    Ok(out)
}

fn triple(v: Vec3) -> String {
    format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z)
}

fn add_entity(server: &mut Server, args: &Value) -> Answer {
    let parent = optional_id(args, "parent")?;
    let desc = match optional_string(args, "ron")? {
        Some(text) => ron::from_str::<EntityDesc>(&text).map_err(|e| format!("ron: {e}"))?,
        None => {
            let mut desc = EntityDesc {
                name: "entity".into(),
                ..EntityDesc::default()
            };
            if let Some(prefab) = optional_string(args, "prefab")? {
                let session = server.session()?;
                if !session.prefab_names().contains(&prefab) {
                    return Err(format!(
                        "no prefab named {prefab}; there are: {}",
                        session.prefab_names().join(", ")
                    ));
                }
                desc.name = prefab.clone();
                desc.prefab = prefab.into();
            }
            apply(&mut desc, args)?;
            desc
        }
    };
    let id = server
        .session()?
        .add_entity(parent, desc)
        .map_err(|e| e.to_string())?;
    Ok(vec![text(id.to_string())])
}

fn update_entity(server: &mut Server, args: &Value) -> Answer {
    let id = id(args, "id")?;
    // Parsed before the session is touched, so a bad argument does not
    // leave an empty step on the undo stack.
    let mut probe = EntityDesc::default();
    apply(&mut probe, args)?;
    server
        .session()?
        .update(id, |desc| {
            apply(desc, args).expect("parsed a line above");
        })
        .map_err(|e| e.to_string())?;
    Ok(vec![text(format!("updated {id}"))])
}

/// Write the fields present in `args` onto an entity.
fn apply(desc: &mut EntityDesc, args: &Value) -> Result<(), String> {
    if let Some(name) = optional_string(args, "name")? {
        desc.name = name;
    }
    if let Some(model) = optional_string(args, "model")? {
        desc.set_part(&scrap::scene::ModelRef(model.into()));
    }
    if let Some(material) = optional_string(args, "material")? {
        desc.set_part(&MaterialRef::Named(material.into()));
    }
    if let Some(color) = optional_string(args, "color")? {
        desc.set_part(&MaterialRef::Inline(hex(&color)?));
    }
    if let Some(v) = optional_vec3(args, "position")? {
        desc.transform.position = v;
    }
    if let Some(v) = optional_vec3(args, "rotation_deg")? {
        desc.transform.rotation_deg = v;
    }
    if let Some(v) = optional_vec3(args, "scale")? {
        desc.transform.scale = v;
    }
    if let Some(body) = optional_string(args, "body")? {
        desc.set_part(&ron::from_str::<Body>(&body).map_err(|_| {
            format!("body is None, Static, Dynamic, Kinematic or Trigger, not {body}")
        })?);
    }
    if let Some(collider) = optional_string(args, "collider")? {
        desc.set_part(&ron::from_str::<Collider>(&collider).map_err(|e| format!("collider: {e}"))?);
    }
    if let Some(camera) = optional_string(args, "camera")? {
        desc.set_part_opt(
            (if camera.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::Lens>(&camera)
                        .map_err(|e| format!("camera: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(wires) = optional_string(args, "wires")? {
        desc.set_part_opt(
            (if wires.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::Wires>(&wires)
                        .map_err(|e| format!("wires: {e}"))?,
                )
            })
            .filter(|w| !w.0.is_empty())
            .as_ref(),
        );
    }
    if let Some(route) = optional_string(args, "route")? {
        desc.set_part_opt(
            (if route.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::Route>(&route)
                        .map_err(|e| format!("route: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(decal) = optional_string(args, "decal")? {
        desc.set_part_opt(
            (if decal.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::Decal>(&decal)
                        .map_err(|e| format!("decal: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(prints) = optional_string(args, "footprints")? {
        desc.set_part_opt(
            (if prints.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::footprints::Footprints>(&prints)
                        .map_err(|e| format!("footprints: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(ground) = optional_string(args, "terrain")? {
        desc.set_part_opt(
            (if ground.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::terrain::Terrain>(&ground)
                        .map_err(|e| format!("terrain: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(probe) = optional_string(args, "reflection_probe")? {
        desc.set_part_opt(
            (if probe.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::Probe>(&probe)
                        .map_err(|e| format!("reflection_probe: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(sound) = optional_string(args, "sound")? {
        desc.set_part_opt(
            (if sound.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::SoundSource>(&sound)
                        .map_err(|e| format!("sound: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(animator) = optional_string(args, "animator")? {
        desc.set_part(&scrap::scene::AnimatorRef(animator.trim().to_string()));
    }
    if let Some(inactive) = args.get("inactive") {
        desc.inactive = inactive
            .as_bool()
            .ok_or_else(|| "inactive is true or false".to_string())?;
    }
    if let Some(particles) = optional_string(args, "particles")? {
        desc.set_part_opt(
            (if particles.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::Emitter>(&particles)
                        .map_err(|e| format!("particles: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(light) = optional_string(args, "light")? {
        desc.set_part_opt(
            (if light.trim() == "None" {
                None
            } else {
                Some(
                    ron::from_str::<scrap::scene::Light>(&light)
                        .map_err(|e| format!("light: {e}"))?,
                )
            })
            .as_ref(),
        );
    }
    if let Some(layer) = optional_string(args, "layer")? {
        desc.set_part(&scrap::scene::LayerName(layer));
    }
    if let Some(physics) = optional_string(args, "physics")? {
        desc.set_part(
            &ron::from_str::<scrap::scene::BodyProps>(&physics)
                .map_err(|e| format!("physics: {e}"))?,
        );
    }
    if let Some(joint) = optional_string(args, "joint")? {
        desc.set_part(
            &ron::from_str::<scrap::scene::Joint>(&joint).map_err(|e| format!("joint: {e}"))?,
        );
    }
    match args.get("components") {
        None | Some(Value::Null) => {}
        Some(Value::Object(components)) => {
            for (name, value) in components {
                match value {
                    Value::Null => {
                        desc.components.remove(name);
                    }
                    Value::String(ron) => desc.set_component(name, ron)?,
                    other => {
                        return Err(format!(
                            "components.{name} is RON text in a string, or null, not {other}"
                        ))
                    }
                }
            }
        }
        Some(other) => return Err(format!("components is an object, not {other}")),
    }
    Ok(())
}

fn hex(color: &str) -> Result<Material, String> {
    let digits = color.trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    match (digits.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => Ok(Material::from_srgb(r, g, b)),
        _ => Err(format!("color is #rrggbb in hex, not {color}")),
    }
}

/// Move the view as asked. Not an edit: the camera is the editor's, not
/// the document's.
fn camera(server: &mut Server, args: &Value) -> Result<(), String> {
    let width = optional_integer(args, "width")?;
    let height = optional_integer(args, "height")?;
    let focus = optional_id(args, "focus")?;
    let eye = optional_vec3(args, "eye")?;
    let target = optional_vec3(args, "target")?;
    let colliders = args.get("colliders").and_then(Value::as_bool);
    let from_game = args
        .get("from_game")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let session = server.session()?;
    if from_game {
        let camera = session
            .game_camera()
            .ok_or("no entity has a camera; the game looks from the scene's view")?;
        session.look_through(camera);
    }
    if let Some(show) = colliders {
        session.set_show_colliders(show);
    }
    if let Some(show) = args.get("grid").and_then(Value::as_bool) {
        session.set_show_grid(show);
    }
    if let Some(show) = args.get("navigation").and_then(Value::as_bool) {
        let walker = session.walker();
        session.set_show_navigation(show.then_some(walker));
    }
    if let Some(show) = args.get("player").and_then(Value::as_bool) {
        session.set_show_player(show);
    }
    if width.is_some() || height.is_some() {
        let (w, h) = session.size();
        session.resize(
            width.unwrap_or(w).clamp(16, 2048),
            height.unwrap_or(h).clamp(16, 2048),
        );
    }
    if let Some(id) = focus {
        session.select(Some(id)).map_err(|e| e.to_string())?;
        session.focus_selected();
    }
    if eye.is_some() || target.is_some() {
        let now = session.camera();
        session.set_camera(eye.unwrap_or(now.position), target.unwrap_or(now.target));
    }
    match args.get("view").and_then(Value::as_str) {
        None => {}
        Some("perspective") => session.set_orthographic(false),
        Some("orthographic") => session.set_orthographic(true),
        Some(name) => match scrap_editor::Side::from_name(name) {
            Some(side) => session.look_from(side),
            None => {
                return Err(format!(
                    "view is top, bottom, front, back, left, right, perspective or orthographic, not {name}"
                ))
            }
        },
    }
    Ok(())
}

fn render(server: &mut Server) -> Answer {
    let session = server.session()?;
    session.render();
    let (width, height) = session.size();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            session.frame_pixels(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    let camera = session.camera();
    Ok(vec![
        image(&png),
        text(format!(
            "{width}x{height}, eye {} looking at {}",
            triple(camera.position),
            triple(camera.target)
        )),
    ])
}

fn simulate(server: &mut Server, args: &Value) -> Answer {
    let seconds = args
        .get("seconds")
        .and_then(Value::as_f64)
        .ok_or("seconds is a number")? as f32;
    if !(0.0..=60.0).contains(&seconds) {
        return Err("seconds is between 0 and 60".into());
    }
    camera(server, args)?;
    let session = server.session()?;
    let bodies: Vec<(EntityId, String)> = session
        .scene()
        .flatten()
        .into_iter()
        .filter(|(desc, _)| desc.body() != Body::None)
        .map(|(desc, _)| (desc.id, desc.name.clone()))
        .collect();
    session.play();
    // In frame-sized slices: the clock caps how many steps one call may
    // catch up, so a single call for three seconds would simulate a few
    // steps of them and call it done.
    let mut steps = 0;
    let slice: f32 = 1.0 / 60.0;
    let mut left = seconds;
    while left > 0.0 {
        steps += session.step(slice.min(left));
        left -= slice;
    }
    let mut report = format!("{steps} steps, {seconds}s:");
    for (id, name) in &bodies {
        if let Some(at) = session.world_position(*id) {
            let _ = write!(report, "\n{id} {name:?} at {}", triple(at));
        }
    }
    if bodies.is_empty() {
        report.push_str("\nno entity has a body, so nothing moves");
    }
    let keep: Vec<EntityId> = match args.get("keep").and_then(Value::as_array) {
        Some(ids) => ids
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| format!("keep holds entity ids, not {v}"))?
                    .parse::<EntityId>()
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let mut out = render(server)?;
    let session = server.session()?;
    if !keep.is_empty() {
        session.select(None).map_err(|e| e.to_string())?;
        for id in &keep {
            session.add_to_selection(*id).map_err(|e| e.to_string())?;
        }
        session.keep_simulation();
        let _ = write!(
            report,
            "
kept where they fell: {}",
            keep.len()
        );
    }
    session.stop();
    out.push(text(report));
    Ok(out)
}

// --- arguments ----------------------------------------------------------

/// The animator graph tools: read one, connect, disconnect, rename a state.
fn graph_tool(server: &mut Server, tool: &str, args: &Value) -> Result<Vec<Value>, String> {
    use scrap::animgraph::{Condition, Graph, Transition, ANY};
    let session = server.session()?;
    let project = session.project().ok_or("the open scene is in no project")?;
    let name = string(args, "name")?;
    let dir = project.root().join(scrap::project::ANIMATORS);
    let path = dir.join(format!("{name}.ron"));
    let old = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut graph: Graph =
        scrap::ron::from_str(&old).map_err(|e| format!("{}: {e}", path.display()))?;
    let state_named = |graph: &Graph, state: &str| -> Result<(), String> {
        if state == ANY || graph.states.contains_key(state) {
            Ok(())
        } else {
            let near = scrap::spelling::closest(state, graph.states.keys().map(String::as_str))
                .map(|n| format!(" — did you mean `{n}`?"))
                .unwrap_or_default();
            Err(format!("`{state}` is not a state of {name}{near}"))
        }
    };
    let save = |graph: &Graph| -> Result<(), String> {
        std::fs::write(&path, scrap::graph_text::write(&old, graph)).map_err(|e| e.to_string())
    };
    match tool {
        "graph" => {
            let mut out = format!("start: {}\n", graph.start);
            for (state_name, state) in &graph.states {
                let plays = if !state.directional.is_empty() {
                    format!("2D blend by {}, {}", state.blend_by, state.blend_by_y)
                } else if !state.blend.is_empty() {
                    format!("blend by {}", state.blend_by)
                } else {
                    state.clip.clone()
                };
                out.push_str(&format!("state {state_name}: {plays}\n"));
            }
            for t in &graph.transitions {
                out.push_str(&format!(
                    "{} → {} when {} (fade {})\n",
                    t.from,
                    t.to,
                    scrap::animgraph::describe(&t.when),
                    t.fade
                ));
            }
            // Each layer: how it lays on the body, then its own graph.
            for layer in &graph.layers {
                let mask = if layer.mask.is_empty() {
                    "the whole body".to_string()
                } else {
                    layer.mask.join(", ")
                };
                let weight = match &layer.weight_from {
                    Some(p) => format!("{} × {p}", layer.weight),
                    None => layer.weight.to_string(),
                };
                out.push_str(&format!(
                    "layer {}: {:?} over {mask}, weight {weight}, start {}\n",
                    layer.name, layer.blend, layer.graph.start
                ));
                for (state_name, state) in &layer.graph.states {
                    let plays = if state.clip.is_empty()
                        && state.blend.is_empty()
                        && state.directional.is_empty()
                    {
                        "nothing (the body below shows)".to_string()
                    } else if state.blend.is_empty() && state.directional.is_empty() {
                        state.clip.clone()
                    } else {
                        format!("blend by {}", state.blend_by)
                    };
                    out.push_str(&format!("  state {state_name}: {plays}\n"));
                }
                for t in &layer.graph.transitions {
                    out.push_str(&format!(
                        "  {} → {} when {} (fade {})\n",
                        t.from,
                        t.to,
                        scrap::animgraph::describe(&t.when),
                        t.fade
                    ));
                }
            }
            let parameters: Vec<String> = graph.parameters().into_iter().collect();
            out.push_str(&format!("parameters: {}\n", parameters.join(", ")));
            for problem in graph.shape_problems() {
                out.push_str(&format!("problem: {problem}\n"));
            }
            // What changed since the last commit, as the Animator window
            // shows it.
            if let Some(head) = scrap_editor::history::show(&path, "HEAD")
                .ok()
                .and_then(|t| scrap::ron::from_str::<Graph>(&t).ok())
            {
                for change in scrap::animgraph::diff(&head, &graph) {
                    out.push_str(&format!("since the last commit: {change}\n"));
                }
            }
            Ok(vec![text(out)])
        }
        "graph_connect" => {
            let (from, to) = (string(args, "from")?, string(args, "to")?);
            state_named(&graph, &from)?;
            state_named(&graph, &to)?;
            let when: Vec<Condition> = match optional_string(args, "when")? {
                Some(w) if !w.trim().is_empty() => {
                    scrap::ron::from_str(&w).map_err(|e| format!("when: {e}"))?
                }
                _ => Vec::new(),
            };
            let fade = args.get("fade").and_then(Value::as_f64).unwrap_or(0.2) as f32;
            graph.transitions.push(Transition {
                from: from.clone(),
                to: to.clone(),
                when,
                fade,
            });
            save(&graph)?;
            Ok(vec![text(format!("{from} → {to} added to {name}"))])
        }
        "graph_disconnect" => {
            let (from, to) = (string(args, "from")?, string(args, "to")?);
            let before = graph.transitions.len();
            graph
                .transitions
                .retain(|t| !(t.from == from && t.to == to));
            if graph.transitions.len() == before {
                return Err(format!("{name} has no transition {from} → {to}"));
            }
            save(&graph)?;
            Ok(vec![text(format!("{from} → {to} removed from {name}"))])
        }
        _ => {
            let (from, to) = (string(args, "from")?, string(args, "to")?);
            state_named(&graph, &from)?;
            if graph.states.contains_key(&to) {
                return Err(format!("{name} already has a state `{to}`"));
            }
            let state = graph.states.remove(&from).expect("checked");
            graph.states.insert(to.clone(), state);
            if graph.start == from {
                graph.start = to.clone();
            }
            for t in &mut graph.transitions {
                for end in [&mut t.from, &mut t.to] {
                    if *end == from {
                        *end = to.clone();
                    }
                }
            }
            save(&graph)?;
            // The graph's cases name states too.
            let cases = dir.join(format!("{name}.cases.ron"));
            let mut also = String::new();
            if let Ok(text_of) = std::fs::read_to_string(&cases) {
                let renamed =
                    text_of.replace(&format!("expect: {from:?}"), &format!("expect: {to:?}"));
                if renamed != text_of {
                    std::fs::write(&cases, renamed).map_err(|e| e.to_string())?;
                    also = format!(", and in {name}.cases.ron");
                }
            }
            Ok(vec![text(format!("`{from}` is `{to}` in {name}{also}"))])
        }
    }
}

/// The dialogue tools: read one, write a line, rename a line, play it,
/// and the sheets of what they all say.
fn dialogue_tool(server: &mut Server, tool: &str, args: &Value) -> Result<Vec<Value>, String> {
    use scrap::dialogue::{describe, named_twice, Cases, Conversation, Dialogue, State};
    let session = server.session()?;
    let project = session.project().ok_or("the open scene is in no project")?;
    if tool == "export_lines" {
        let out = optional_string(args, "out")?.unwrap_or_else(|| "build/lines".into());
        let written = scrap_cli::lines::export(project, &project.root().join(out))?;
        let files: Vec<String> = written
            .iter()
            .map(|p| {
                project
                    .relative(p)
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect();
        return Ok(vec![text(format!("wrote {}", files.join(", ")))]);
    }
    let dir = project.root().join(scrap::dialogue::DIR);
    let name = string(args, "name")?;
    let (all, _) = scrap_cli::lines::dialogues(project);
    if tool == "dialogue" && name.is_empty() {
        let names: Vec<String> = all.iter().map(|(_, d)| d.name.clone()).collect();
        return Ok(vec![text(if names.is_empty() {
            "no dialogues yet: dialogue_line makes one".to_string()
        } else {
            names.join("\n")
        })]);
    }
    let path = dir.join(format!("{name}.ron"));
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut dialogue: Dialogue = if old.is_empty() && tool == "dialogue_line" {
        Dialogue::default()
    } else if old.is_empty() {
        let near = scrap::spelling::closest(&name, all.iter().map(|(_, d)| d.name.as_str()))
            .map(|n| format!(" — did you mean `{n}`?"))
            .unwrap_or_default();
        return Err(format!("no dialogue `{name}` in dialogues/{near}"));
    } else {
        scrap::ron::from_str(&old).map_err(|e| format!("{}: {e}", path.display()))?
    };
    dialogue.name = name.clone();
    let save = |d: &Dialogue| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, scrap::dialogue_text::write(&old, d)).map_err(|e| e.to_string())
    };
    let cases_path = path.with_extension("cases.ron");
    match tool {
        "dialogue" => {
            let mut out = format!("start: {}\n", dialogue.start);
            for (line_name, line) in &dialogue.lines {
                let who = if line.speaker.is_empty() {
                    String::new()
                } else {
                    format!("{}: ", line.speaker)
                };
                out.push_str(&format!("line {line_name}: {who}{}", line.text));
                if !line.when.is_empty() {
                    let otherwise = if line.otherwise.is_empty() {
                        &line.next
                    } else {
                        &line.otherwise
                    };
                    out.push_str(&format!(
                        " (said if {}, else → {otherwise})",
                        describe(&line.when)
                    ));
                }
                if !line.next.is_empty() {
                    out.push_str(&format!(" → {}", line.next));
                }
                if !line.set.is_empty() {
                    out.push_str(&format!(", sets {}", line.set.join(", ")));
                }
                if !line.event.is_empty() {
                    out.push_str(&format!(", tells `{}`", line.event));
                }
                out.push('\n');
                for (i, c) in line.choices.iter().enumerate() {
                    out.push_str(&format!("  answer {}: “{}” → {}", i + 1, c.text, c.to));
                    if !c.when.is_empty() {
                        out.push_str(&format!(" if {}", describe(&c.when)));
                    }
                    if c.once {
                        out.push_str(", once");
                    }
                    if !c.set.is_empty() {
                        out.push_str(&format!(", sets {}", c.set.join(", ")));
                    }
                    if !c.event.is_empty() {
                        out.push_str(&format!(", tells `{}`", c.event));
                    }
                    out.push('\n');
                }
            }
            for problem in named_twice(&old).into_iter().chain(dialogue.problems()) {
                out.push_str(&format!("problem: {problem}\n"));
            }
            if let Ok(cases_text) = std::fs::read_to_string(&cases_path) {
                match scrap::ron::from_str::<Cases>(&cases_text) {
                    Ok(cases) => {
                        let failed = cases.run(&dialogue);
                        out.push_str(&format!(
                            "cases: {} of {} pass\n",
                            cases.cases.len().saturating_sub(failed.len()),
                            cases.cases.len()
                        ));
                        for f in failed {
                            out.push_str(&format!("case failed: {f}\n"));
                        }
                    }
                    Err(e) => out.push_str(&format!("cases: {e}\n")),
                }
            }
            if let Some(head) = scrap_editor::history::show(&path, "HEAD")
                .ok()
                .and_then(|t| scrap::ron::from_str::<Dialogue>(&t).ok())
            {
                let mut head = head;
                head.name = name.clone();
                for change in scrap::dialogue::diff(&head, &dialogue) {
                    out.push_str(&format!("since the last commit: {change}\n"));
                }
            }
            Ok(vec![text(out)])
        }
        "dialogue_line" => {
            let line = string(args, "line")?;
            let entry = string(args, "entry")?;
            if line.is_empty() {
                return Err("a line needs a name".into());
            }
            let said = if entry.trim().is_empty() {
                if dialogue.lines.remove(&line).is_none() {
                    return Err(format!("{name} has no line `{line}`"));
                }
                format!("`{line}` removed from {name}")
            } else {
                let parsed: scrap::dialogue::Line =
                    scrap::ron::from_str(&entry).map_err(|e| format!("entry: {e}"))?;
                let was = dialogue.lines.insert(line.clone(), parsed).is_some();
                if dialogue.start.is_empty() {
                    dialogue.start = line.clone();
                }
                format!(
                    "`{line}` {} in {name}",
                    if was { "written" } else { "added" }
                )
            };
            if args.get("start").and_then(Value::as_bool) == Some(true) {
                dialogue.start = line.clone();
            }
            save(&dialogue)?;
            let problems = dialogue.problems();
            Ok(vec![text(if problems.is_empty() {
                said
            } else {
                format!("{said}; now: {}", problems.join("; "))
            })])
        }
        "dialogue_rename" => {
            let (from, to) = (string(args, "from")?, string(args, "to")?);
            scrap::dialogue::rename(&mut dialogue, &from, &to)
                .map_err(|e| format!("{name}: {e}"))?;
            save(&dialogue)?;
            // The dialogue's cases name lines too.
            let mut also = String::new();
            if let Ok(cases) = std::fs::read_to_string(&cases_path) {
                let renamed = cases
                    .replace(&format!("At({from:?})"), &format!("At({to:?})"))
                    .replace(&format!("Next({from:?})"), &format!("Next({to:?})"))
                    .replace(&format!(", {from:?})"), &format!(", {to:?})"));
                if renamed != cases {
                    std::fs::write(&cases_path, renamed).map_err(|e| e.to_string())?;
                    also = format!(", and in {name}.cases.ron");
                }
            }
            Ok(vec![text(format!("`{from}` is `{to}` in {name}{also}"))])
        }
        _ => {
            let mut state = State::default();
            if let Some(flags) = args.get("flags").and_then(Value::as_array) {
                state
                    .flags
                    .extend(flags.iter().filter_map(Value::as_str).map(str::to_string));
            }
            if let Some(vars) = args.get("vars").and_then(Value::as_object) {
                for (k, v) in vars {
                    let n = v
                        .as_i64()
                        .ok_or_else(|| format!("vars: {k} is a whole number, not {v}"))?;
                    state.vars.insert(k.clone(), n);
                }
            }
            let mut answers: std::collections::VecDeque<String> = args
                .get("answers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let mut talk = match optional_string(args, "from")? {
                Some(from) => Conversation::begin_at(dialogue.clone(), &from, &mut state),
                None => Conversation::begin(dialogue.clone(), &mut state),
            };
            let mut out = String::new();
            for _ in 0..1000 {
                let Some(line) = talk.line().cloned() else {
                    out.push_str("— over\n");
                    break;
                };
                let who = if line.speaker.is_empty() {
                    String::new()
                } else {
                    format!("{}: ", line.speaker)
                };
                out.push_str(&format!(
                    "{} — {who}{}\n",
                    talk.at().unwrap_or(""),
                    line.text
                ));
                for event in talk.events() {
                    out.push_str(&format!("  tells `{event}`\n"));
                }
                if line.choices.is_empty() {
                    talk.next(&mut state);
                    continue;
                }
                let offered: Vec<(usize, String)> = talk
                    .choices(&state)
                    .map(|(i, c)| (i, c.text.clone()))
                    .collect();
                let words: Vec<String> = offered.iter().map(|(_, t)| format!("“{t}”")).collect();
                out.push_str(&format!("  answers on offer: {}\n", words.join(", ")));
                let Some(answer) = answers.pop_front() else {
                    out.push_str("— waits for an answer\n");
                    break;
                };
                let Some((index, _)) = offered.iter().find(|(_, t)| *t == answer) else {
                    out.push_str(&format!("— “{answer}” is not on offer\n"));
                    break;
                };
                out.push_str(&format!("  answers “{answer}”\n"));
                talk.choose(*index, &mut state);
            }
            let flags: Vec<&str> = state.flags.iter().map(String::as_str).collect();
            let vars: Vec<String> = state
                .vars
                .iter()
                .map(|(k, v)| format!("{k} = {v}"))
                .collect();
            out.push_str(&format!(
                "flags: {}\nnumbers: {}\n",
                flags.join(", "),
                vars.join(", ")
            ));
            Ok(vec![text(out)])
        }
    }
}

/// `main → origin/main, 2 ahead, 1 behind`, and the merge under way.
fn git_branch_line(status: &scrap_editor::Status) -> String {
    let mut line = status
        .branch
        .clone()
        .unwrap_or_else(|| "no branch (detached HEAD)".into());
    if let Some(upstream) = &status.upstream {
        line += &format!(
            " → {upstream}, {} ahead, {} behind",
            status.ahead, status.behind
        );
    }
    if status.merging {
        line += "; merging — committing finishes it";
    }
    line
}

/// The open project's folder: where configs/ is.
fn project_root(server: &mut Server) -> Result<PathBuf, String> {
    server
        .session()?
        .project()
        .map(|p| p.root().to_path_buf())
        .ok_or_else(|| "no project open: open_scene first".to_string())
}

fn string(args: &Value, key: &str) -> Result<String, String> {
    optional_string(args, key)?.ok_or_else(|| format!("{key} is required"))
}

fn optional_string(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(format!("{key} is a string, not {other}")),
    }
}

fn id(args: &Value, key: &str) -> Result<EntityId, String> {
    optional_id(args, key)?.ok_or_else(|| format!("{key} is required: {ID}"))
}

fn optional_id(args: &Value, key: &str) -> Result<Option<EntityId>, String> {
    optional_string(args, key)?
        .map(|s| {
            s.parse::<EntityId>()
                .map_err(|_| format!("{key} is {ID}, not {s:?}"))
        })
        .transpose()
}

fn integer(args: &Value, key: &str) -> Result<u32, String> {
    optional_integer(args, key)?.ok_or_else(|| format!("{key} is required"))
}

fn optional_integer(args: &Value, key: &str) -> Result<Option<u32>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("{key} is a whole number, not {v}")),
    }
}

/// A brush's fields, for the tools that take one or a list.
fn brush_fields() -> Value {
    json!({
        "shape": { "type": "string", "enum": ["box", "ramp", "cylinder", "stairs"], "description": "a unit in size and centred, like builtin:cube|ramp|cylinder|stairs; a ramp is high at the back (-z)" },
        "position": vec3("metres, in the solid's own space"),
        "rotation_deg": vec3("Euler degrees, applied Y then X then Z"),
        "scale": vec3("its size in metres along each axis; [1, 1, 1] when left out"),
        "sides": { "type": "integer", "description": "a cylinder's sides, 3 to 64; 24 when left out" },
    })
}

/// A brush from a tool's arguments; `op` given by the tool, or read from
/// an `op` field of a list's item.
fn brush_of(
    value: &Value,
    op: Option<scrap_import::brush::Op>,
) -> Result<scrap_import::brush::Brush, String> {
    use scrap_import::brush::{Brush, Op, Shape};
    let shape = match string(value, "shape")?.to_ascii_lowercase().as_str() {
        "box" | "cube" => Shape::Box,
        "ramp" => Shape::Ramp,
        "cylinder" => Shape::Cylinder,
        "stairs" => Shape::Stairs,
        other => {
            return Err(format!(
                "shape is box, ramp, cylinder or stairs, not {other:?}"
            ))
        }
    };
    let op = match op {
        Some(op) => op,
        None => match optional_string(value, "op")?.as_deref() {
            None | Some("add") | Some("Add") => Op::Add,
            Some("subtract") | Some("Subtract") => Op::Subtract,
            Some(other) => return Err(format!("op is add or subtract, not {other:?}")),
        },
    };
    Ok(Brush {
        op,
        shape,
        position: optional_vec3(value, "position")?.unwrap_or(Vec3::ZERO),
        rotation_deg: optional_vec3(value, "rotation_deg")?.unwrap_or(Vec3::ZERO),
        scale: optional_vec3(value, "scale")?.unwrap_or(Vec3::ONE),
        sides: optional_integer(value, "sides")?.unwrap_or(scrap_import::brush::CYLINDER_SIDES),
    })
}

fn optional_vec3(args: &Value, key: &str) -> Result<Option<Vec3>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.len() == 3 => {
            let mut out = [0.0f32; 3];
            for (slot, item) in out.iter_mut().zip(items) {
                *slot = item
                    .as_f64()
                    .ok_or_else(|| format!("{key} is three numbers, not {item}"))?
                    as f32;
            }
            Ok(Some(Vec3::from_array(out)))
        }
        Some(other) => Err(format!("{key} is [x, y, z], not {other}")),
    }
}

/// An `ids` argument: a list of entity ids.
fn id_list(args: &Value) -> Result<Vec<EntityId>, String> {
    match args.get("ids") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<EntityId>().ok())
                    .ok_or_else(|| format!("ids are {ID}, not {v}"))
            })
            .collect(),
        _ => Err("ids is a list of entity ids".into()),
    }
}
