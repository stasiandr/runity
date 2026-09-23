//! The tools: what each one takes, and the session calls it makes.

use std::fmt::Write as _;
use std::path::PathBuf;

use image::ImageEncoder;
use runity::glam::Vec3;
use runity::scene::{Collider, MaterialRef};
use runity::{Body, EntityDesc, EntityId, Material, Project};
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
        "material": { "type": "string", "description": "a material name: materials/<name>.rmat, or builtin grass, earth, bark, needle, stone, ember, white" },
        "color": { "type": "string", "description": "#rrggbb, sRGB — a colour spelled out instead of a material name" },
        "position": vec3("metres, relative to the parent"),
        "rotation_deg": vec3("Euler degrees, applied Y then X then Z"),
        "scale": vec3("per axis"),
        "body": { "type": "string", "enum": ["None", "Static", "Dynamic", "Kinematic", "Trigger"] },
        "collider": { "type": "string", "description": "RON: None, Box(half: (x, y, z)), Sphere(radius: r), Capsule(half_height: h, radius: r), Cylinder(half_height: h, radius: r), Ramp(half: (x, y, z)), Stairs(half: (x, y, z), steps: n). builtin:cube/cylinder/ramp/stairs fit Box/Cylinder/Ramp/Stairs with half 0.5 (stairs: steps 4)" },
        "camera": { "type": "string", "description": "RON: a camera on this entity, looking along its +z — (fov_deg: 60.0, priority: 0) — or None. Put it on a child of the player and it follows" },
        "layer": { "type": "string", "description": "collision layer by its name in layers.ron; empty is default" },
        "particles": { "type": "string", "description": "RON: particles given off along its up — (rate: 30.0, life: 0.8, speed: 2.0, spread_deg: 20.0, size: 0.06, gravity: -1.0, color: (1.0, 0.6, 0.2)) — sparks, dust, spray; or None" },
        "light": { "type": "string", "description": "RON: a point light at this entity — (color: (1.0, 0.6, 0.3), intensity: 2.0, range: 6.0), colour as a picker says it; add cone_deg: 30.0 for a spot along its +z — or None" },
        "physics": { "type": "string", "description": "RON, only what differs: (friction: 0.5, bounce: 0.0, density: 1.0) — a ball is (bounce: 0.8), iron is (density: 8.0); freeze_turn: \"xz\" keeps it upright, freeze_move: \"y\" at its height" },
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
        "navigation": { "type": "boolean", "description": "show where a walker (0.35 m radius, 40° slope, 0.3 m step) can go, in blue on the ground, until turned off" },
        "view": { "type": "string", "enum": ["top", "bottom", "front", "back", "left", "right", "perspective", "orthographic"], "description": "look along an axis, orthographic, showing as much as before — `top` is the level's plan — or switch projection; stays until changed" },
    });
    let mut simulate = camera.clone();
    simulate["seconds"] = json!({ "type": "number", "description": "simulated time, fixed steps" });
    vec![
        tool("new_project", "Make a runity project (standard layout, starter scene, game crate) and open its scene.", json!({ "path": { "type": "string" }, "name": { "type": "string" } }), &["path"]),
        tool("open_scene", "Open a scene file; its project's prefabs, materials and library come with it.", json!({ "path": { "type": "string" } }), &["path"]),
        tool("new_scene", "Make scenes/<name>.ron in the open project — a ground with the metre grid, solid — and open it. Save the open scene first if it has changes. Lists the project's scenes.", json!({ "name": { "type": "string", "description": "snake_case, like level_2" } }), &["name"]),
        tool("open_prefab", "Prefab Mode: open prefabs/<name>.prefab as the document. Every tool then edits the prefab (a variant's part edits become its overrides) and save_scene writes the prefab file; open_scene goes back.", json!({ "name": { "type": "string" } }), &["name"]),
        tool("save_scene", "Write the scene to its file, or to path.", json!({ "path": { "type": "string" } }), &[]),
        tool("scene_tree", "The open scene as an indented tree: id, name, model or prefab, material, place.", json!({}), &[]),
        tool("find", "Search the scene like the hierarchy's search box: words match names; c:door (has component), m:stone (material), p:campfire (prefab instance), model:pine_large, body:dynamic; quoted \"phrases\"; terms combine with and. Prefab parts included. Returns id and name per line.", json!({ "query": { "type": "string" } }), &["query"]),
        tool("inspect", "The Inspector for an entity: every field as text, and for a prefab's part which ones this instance overrides. With `ids`, for several at once: — where they disagree.", json!({ "id": { "type": "string", "description": ID }, "ids": { "type": "array", "items": { "type": "string" } } }), &[]),
        tool("set_field", "Set one field of an entity from text, as typing into the Inspector: name, model, prefab, position \"(x, y, z)\", rotation, scale, material, body, collider, physics, layer, joint, camera, components.<name>. One undo step; on a prefab's part, an override. With `ids` instead of `id`, the same field of all of them, still one step.", json!({ "id": { "type": "string", "description": ID }, "ids": { "type": "array", "items": { "type": "string" } }, "field": { "type": "string" }, "value": { "type": "string" } }), &["field", "value"]),
        tool("get_entity", "One entity's line in the scene's RON, children included.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("add_entity", "Add an entity, as one undo step. Returns its id.", add, &[]),
        tool("update_entity", "Change any fields of an entity, as one undo step.", update, &["id"]),
        tool("delete_entity", "Delete an entity and everything under it.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("duplicate_entity", "Copy an entity with its children, as its next sibling. Returns the copy's id.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
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
        tool("history", "The commits that touched the open scene's file, newest first: commit, author, date, summary.", json!({}), &[]),
        tool("restore", "Put the open scene back as it was at a commit, as one undo step (render afterwards to look; undo to go back).", json!({ "commit": { "type": "string" } }), &["commit"]),
        tool("conflicts", "While git is merging the open scene with conflicts: each conflict in words, numbered. The file holds ours for each.", json!({}), &[]),
        tool("take_theirs", "Settle one conflict (its number from `conflicts`) theirs' way, as one undo step. Keeping ours needs nothing. Save, then `git add` the file.", json!({ "conflict": { "type": "integer" } }), &["conflict"]),
        tool("copy", "Entities (children included) as RON text, for `paste` here or in another scene.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" } }), &["ids"]),
        tool("paste", "Add entities from RON text — from `copy`, or written by hand, one entity or a list — as new things with new ids, one undo step. Returns their ids.", json!({ "ron": { "type": "string" }, "parent": { "type": "string", "description": ID } }), &["ron"]),
        tool("sculpt", "Shape a terrain with one brush stroke at a world point: raise (by > 0) or lower it, or with flatten: true pull it toward the height `by`. Written as one line in the terrain's .rterrain and rebuilt at once.", json!({
            "id": { "type": "string", "description": "the terrain entity's id" },
            "at": vec3("where the stroke lands"),
            "radius": { "type": "number" },
            "by": { "type": "number", "description": "metres up (or down), or the height to flatten to" },
            "flatten": { "type": "boolean" },
        }), &["id", "at", "radius", "by"]),
        tool("place", "Put entities on whatever a pixel of the last render shows — a table's top, a wall, a slope — by the bottom of their box, keeping their places relative to each other. One undo step. Pixels from the top left, as `pick` takes them.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" }, "x": { "type": "integer" }, "y": { "type": "integer" } }), &["ids", "x", "y"]),
        tool("to_view", "From the render view: `move` puts the entity (and what is under it) on the point the view looks at; `align` stands it where the view is, looking where it looks — frame a shot with render's eye and target, then align the game's camera to it. One undo step.", json!({ "id": { "type": "string", "description": ID }, "how": { "type": "string", "enum": ["move", "align"] } }), &["id", "how"]),
        tool("override_field", "On a prefab's part: `revert` one overridden field to what the prefab says, or `apply` it to the prefab file so every instance has it — the other overrides stay. position, rotation and scale are one override (the transform). One undo step.", json!({ "id": { "type": "string", "description": ID }, "field": { "type": "string" }, "how": { "type": "string", "enum": ["apply", "revert"] } }), &["id", "field", "how"]),
        tool("console", "The editor's Console: what opening, importing and rebuilding said — skipped lines, import warnings, sources that would not rebuild — each once with how many times, oldest first. clear: true empties it after reading.", json!({ "clear": { "type": "boolean" } }), &[]),
        tool("group", "Put entities under a new empty entity named `name`, standing on the ground in the middle of them — Unity's Create Empty Parent. Nothing moves in the world; one undo step; returns the group's id.", json!({ "ids": { "type": "array", "items": { "type": "string" } }, "name": { "type": "string" } }), &["ids", "name"]),
        tool("thumbnail", "A picture of a prefab or a model (by the name scenes use: campfire, builtin:cone, rock) alone, framed whole — the Project window's preview. Changes nothing.", json!({ "what": { "type": "string" }, "size": { "type": "integer", "description": "pixels a side, 16 to 1024; 256 by default" } }), &["what"]),
        tool("drop", "Drop a prefab or a model (by the name scenes use) into the view at a pixel of the last render, standing on whatever is there — Project-window drag and drop. One undo step; returns its id.", json!({ "what": { "type": "string" }, "x": { "type": "integer" }, "y": { "type": "integer" } }), &["what", "x", "y"]),
        tool("add_component", "Put one of the game's components on an entity with a value of its shape to start from — Add Component. Needs library/components.ron, which the game writes when it or `runity test` runs; without `name`, lists the components and what each holds.", json!({ "id": { "type": "string", "description": ID }, "name": { "type": "string" } }), &[]),
        tool("import_settings", "An asset source's import settings (its .rimport), or with `field` and `value` one of them changed — scale, recompute_normals, srgb, origin_to_base — and the asset built again, every scene showing it at once.", json!({ "source": { "type": "string", "description": "project-relative, like assets/rock.obj" }, "field": { "type": "string" }, "value": { "type": "string" } }), &["source"]),
        tool("fit_collider", "Give an entity a box collider that fits its model — size and centre from the model's bounds — as Unity does when a BoxCollider is added. One undo step.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("hide", "Hide entities (and what is under them) from `render`, or with show: true bring them back — the roof off a house to look inside. A view setting: nothing in the scene file, no undo step.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" }, "show": { "type": "boolean" } }), &["ids"]),
        tool("isolate", "Show only these entities (and what is under them) in `render`; an empty list shows everything again, hidden ones too. A view setting, like `hide`.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" } }), &["ids"]),
        tool("drop_to_ground", "Put entities down on whatever is beneath them — the real shape of it: a slope, a terrain — as one undo step.", json!({ "ids": { "type": "array", "items": { "type": "string" }, "description": "entity ids" } }), &["ids"]),
        tool("path", "Can something walk from one point to another in the scene as it stands, and which way? Baked from the static colliders: slope, step height and the walker's radius decide. Returns the corners and the length, or says there is no way.", json!({
            "from": vec3("start, on or above the ground"),
            "to": vec3("goal"),
            "radius": { "type": "number", "description": "the walker's radius, metres (0.35)" },
            "max_step": { "type": "number", "description": "the highest step it climbs, metres (0.3)" },
            "max_slope": { "type": "number", "description": "the steepest slope it walks, degrees (40)" },
        }), &["from", "to"]),
        tool("locks", "Who holds which Git LFS lock in the project: lock binary sources (textures, models, sounds) before editing them.", json!({}), &[]),
        tool("lock", "Take (or with locked: false, give back) the Git LFS lock on a file.", json!({ "path": { "type": "string" }, "locked": { "type": "boolean" } }), &["path"]),
        tool("apply_overrides", "Write a prefab instance's overrides into the prefab file, so every instance gets them, and clear them from this instance.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("unpack_prefab", "Turn a prefab instance into plain entities of the scene, overrides applied, no longer following the prefab file. One undo step; its parts keep their ids.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("replace_with_prefab", "Put a prefab where each thing a search finds is — the greybox cubes named `crate` become the real crate — keeping ids, names and places. One undo step.", json!({ "query": { "type": "string", "description": "what to replace, as find takes it: `crate`, `m:grid model:builtin:cube`" }, "prefab": { "type": "string" } }), &["query", "prefab"]),
        tool("revert_overrides", "Drop a prefab instance's overrides: it is the prefab again. One undo step.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("undo", "Take back the last edit; says what it was (\"move `crate`\").", json!({}), &[]),
        tool("edits", "Every edit undo can take back in this session, oldest first, in words: what has been done since the scene was opened.", json!({}), &[]),
        tool("redo", "Put back the last edit taken back.", json!({}), &[]),
        tool("render", "Draw the view and return it as a PNG. Camera arguments move the view first and are not an edit.", camera, &[]),
        tool("pick", "The entity under a pixel of the last render.", json!({ "x": { "type": "integer" }, "y": { "type": "integer" } }), &["x", "y"]),
        tool("import", "Import a source file (.gltf .glb .obj .png .jpg .tga .bmp .wav .rmat) into the project.", json!({ "source": { "type": "string" } }), &["source"]),
        tool("rename_asset", "Rename or move an asset source (model, texture, sound in assets/, .rmat in materials/, .prefab in prefabs/), its .rimport with it, and rewrite every scene and prefab line that named it. Paths relative to the project root. Refused, with the reason, when the new name already means something.", json!({ "from": { "type": "string" }, "to": { "type": "string" } }), &["from", "to"]),
        tool("assets", "Every asset source in the project — models, textures, sounds, materials, prefabs — with its kind, id, whether it is built, and how many lines use it.", json!({}), &[]),
        tool("delete_asset", "Delete an asset source with its .rimport and built asset. Refused, listing the lines, while any scene or prefab (or the open scene's unsaved edits) still names it.", json!({ "file": { "type": "string" } }), &["file"]),
        tool("duplicate_asset", "Copy an asset source under a new name: a new asset with its own id and the original's import settings.", json!({ "from": { "type": "string" }, "to": { "type": "string" } }), &["from", "to"]),
        tool("usages", "Every scene and prefab line that names an asset file: what a rename would change, and whether it is safe to delete.", json!({ "file": { "type": "string", "description": "relative to the project root, e.g. materials/stone.rmat" } }), &["file"]),
        tool("reload", "Pick up files changed on disk: the scene, prefabs, and assets rebuilt from changed sources.", json!({}), &[]),
        tool("problems", "What is wrong with the open document right now, unsaved edits included: models, materials and prefabs nothing answers to, stale overrides — each with the entity id and the likely intended name. Empty means clean.", json!({}), &[]),
        tool("check", "Everything in the project that does not resolve, with file, entity and the fix.", json!({}), &[]),
        tool("simulate", "Play the scene for some seconds, report where the physics bodies ended up, render, and stop. The document is not changed.", simulate, &["seconds"]),
    ]
}

pub fn call(server: &mut Server, name: &str, args: &Value) -> Answer {
    match name {
        "new_project" => new_project(server, args),
        "open_scene" => open_scene(server, &string(args, "path")?),
        "save_scene" => {
            let path = optional_string(args, "path")?.map(PathBuf::from);
            let session = server.session()?;
            session
                .save_scene(path.as_deref())
                .map_err(|e| e.to_string())?;
            Ok(vec![text("saved")])
        }
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
        "delete_entity" => {
            let id = id(args, "id")?;
            server.session()?.delete(id).map_err(|e| e.to_string())?;
            Ok(vec![text(format!("deleted {id}"))])
        }
        "duplicate_entity" => {
            let id = id(args, "id")?;
            let copy = server.session()?.duplicate(id).map_err(|e| e.to_string())?;
            Ok(vec![text(format!("{copy}"))])
        }
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
                "min" => runity_editor::Align::Min,
                "center" => runity_editor::Align::Center,
                "max" => runity_editor::Align::Max,
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
            let face: runity::edit::Face = string(args, "face")?.parse()?;
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
            let defaults = runity::edit::Scatter::default();
            let layout = runity::edit::Scatter {
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
        "history" => {
            let revisions = server
                .session()?
                .scene_history()
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
            let conflicts = server
                .session()?
                .merge_conflicts()
                .map_err(|e| e.to_string())?;
            if conflicts.is_empty() {
                return Ok(vec![text("no merge conflicts in the open scene")]);
            }
            let lines: Vec<String> = conflicts
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{i}: {c}"))
                .collect();
            Ok(vec![text(lines.join("\n"))])
        }
        "take_theirs" => {
            let index = integer(args, "conflict")? as usize;
            server
                .session()?
                .take_theirs(index)
                .map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "conflict {index} settled theirs' way; not saved"
            ))])
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
                "sculpted; the stroke is a line in the terrain's .rterrain",
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
        "console" => {
            let session = server.session()?;
            let mut out = String::new();
            for line in session.console() {
                let level = match line.level {
                    runity_editor::console::Level::Info => "info",
                    runity_editor::console::Level::Warning => "warning",
                    runity_editor::console::Level::Error => "error",
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
        "add_component" => {
            let session = server.session()?;
            let Some(name) = args.get("name").and_then(Value::as_str) else {
                let shapes = session.component_shapes();
                if shapes.is_empty() {
                    return Ok(vec![text(
                        "no library/components.ron yet: run the game or `runity test` once"
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
        "isolate" => {
            let ids = id_list(args)?;
            let session = server.session()?;
            if ids.is_empty() {
                session.show_all();
                return Ok(vec![text("everything is shown".to_string())]);
            }
            session.isolate(&ids).map_err(|e| e.to_string())?;
            Ok(vec![text(format!("showing {} alone", ids.len()))])
        }
        "drop_to_ground" => {
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
            for (i, id) in ids.iter().enumerate() {
                if i == 0 {
                    session.select(Some(*id)).map_err(|e| e.to_string())?;
                } else {
                    session.add_to_selection(*id).map_err(|e| e.to_string())?;
                }
            }
            let landed = session.drop_to_ground().map_err(|e| e.to_string())?;
            Ok(vec![text(format!(
                "{landed} of {} found ground",
                ids.len()
            ))])
        }
        "path" => {
            let from = optional_vec3(args, "from")?.ok_or("from is required")?;
            let to = optional_vec3(args, "to")?.ok_or("to is required")?;
            let defaults = runity::navigation::NavSettings::default();
            let number = |key: &str, default: f32| {
                args.get(key)
                    .and_then(Value::as_f64)
                    .map_or(default, |n| n as f32)
            };
            let settings = runity::navigation::NavSettings {
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
                            runity::navigation::NavGrid::length(&path),
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
        "undo" => {
            let session = server.session()?;
            let what = session.undo_label();
            let done = session.undo().map_err(|e| e.to_string())?;
            Ok(vec![text(match (done, what) {
                (true, Some(what)) => format!("undone: {what}"),
                (true, None) => "undone".into(),
                (false, _) => "nothing to undo".into(),
            })])
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
        "redo" => {
            let session = server.session()?;
            let what = session.redo_label();
            let done = session.redo().map_err(|e| e.to_string())?;
            Ok(vec![text(match (done, what) {
                (true, Some(what)) => format!("redone: {what}"),
                (true, None) => "redone".into(),
                (false, _) => "nothing to redo".into(),
            })])
        }
        "render" => {
            camera(server, args)?;
            render(server)
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
            let findings = runity_cli::check(project);
            if findings.is_empty() {
                return Ok(vec![text("clean")]);
            }
            let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
            Ok(vec![text(lines.join("\n"))])
        }
        "simulate" => simulate(server, args),
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
                out.push_str("; its library is not built yet, so models from assets/ draw nothing until `reload` or `runity sync`");
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
    fn line(out: &mut String, desc: &EntityDesc, depth: usize, expanded: &runity::Scene) {
        let _ = write!(out, "{}{} {:?}", "  ".repeat(depth), desc.id, desc.name);
        if !desc.prefab.is_empty() {
            let _ = write!(out, " prefab={}", desc.prefab);
        } else if !desc.model.is_empty() {
            let _ = write!(out, " model={}", desc.model);
        }
        match &desc.material {
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
        if desc.body != Body::None {
            let _ = write!(out, " body={:?}", desc.body);
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
            desc.model,
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
                desc.prefab = prefab;
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
        desc.model = model;
    }
    if let Some(material) = optional_string(args, "material")? {
        desc.material = MaterialRef::Named(material);
    }
    if let Some(color) = optional_string(args, "color")? {
        desc.material = MaterialRef::Inline(hex(&color)?);
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
        desc.body = ron::from_str::<Body>(&body).map_err(|_| {
            format!("body is None, Static, Dynamic, Kinematic or Trigger, not {body}")
        })?;
    }
    if let Some(collider) = optional_string(args, "collider")? {
        desc.collider =
            ron::from_str::<Collider>(&collider).map_err(|e| format!("collider: {e}"))?;
    }
    if let Some(camera) = optional_string(args, "camera")? {
        desc.camera = if camera.trim() == "None" {
            None
        } else {
            Some(
                ron::from_str::<runity::scene::Lens>(&camera)
                    .map_err(|e| format!("camera: {e}"))?,
            )
        };
    }
    if let Some(particles) = optional_string(args, "particles")? {
        desc.particles = if particles.trim() == "None" {
            None
        } else {
            Some(
                ron::from_str::<runity::scene::Emitter>(&particles)
                    .map_err(|e| format!("particles: {e}"))?,
            )
        };
    }
    if let Some(light) = optional_string(args, "light")? {
        desc.light = if light.trim() == "None" {
            None
        } else {
            Some(ron::from_str::<runity::scene::Light>(&light).map_err(|e| format!("light: {e}"))?)
        };
    }
    if let Some(layer) = optional_string(args, "layer")? {
        desc.layer = layer;
    }
    if let Some(physics) = optional_string(args, "physics")? {
        desc.physics = ron::from_str::<runity::scene::BodyProps>(&physics)
            .map_err(|e| format!("physics: {e}"))?;
    }
    if let Some(joint) = optional_string(args, "joint")? {
        desc.joint =
            ron::from_str::<runity::scene::Joint>(&joint).map_err(|e| format!("joint: {e}"))?;
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
    if let Some(show) = args.get("navigation").and_then(Value::as_bool) {
        session.set_show_navigation(show.then(runity::navigation::NavSettings::default));
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
        Some(name) => match runity_editor::Side::from_name(name) {
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
        .filter(|(desc, _)| desc.body != Body::None)
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
    let mut out = render(server)?;
    server.session()?.stop();
    out.push(text(report));
    Ok(out)
}

// --- arguments ----------------------------------------------------------

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
