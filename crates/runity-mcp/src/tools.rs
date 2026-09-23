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
        "body": { "type": "string", "enum": ["None", "Static", "Dynamic"] },
        "collider": { "type": "string", "description": "RON: None, Box(half: (x, y, z)), Sphere(radius: r), Capsule(half_height: h, radius: r), Cylinder(half_height: h, radius: r), Ramp(half: (x, y, z)), Stairs(half: (x, y, z), steps: n). builtin:cube/cylinder/ramp/stairs fit Box/Cylinder/Ramp/Stairs with half 0.5 (stairs: steps 4)" },
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
    });
    let mut simulate = camera.clone();
    simulate["seconds"] = json!({ "type": "number", "description": "simulated time, fixed steps" });
    vec![
        tool("new_project", "Make a runity project (standard layout, starter scene, game crate) and open its scene.", json!({ "path": { "type": "string" }, "name": { "type": "string" } }), &["path"]),
        tool("open_scene", "Open a scene file; its project's prefabs, materials and library come with it.", json!({ "path": { "type": "string" } }), &["path"]),
        tool("save_scene", "Write the scene to its file, or to path.", json!({ "path": { "type": "string" } }), &[]),
        tool("scene_tree", "The open scene as an indented tree: id, name, model or prefab, material, place.", json!({}), &[]),
        tool("get_entity", "One entity's line in the scene's RON, children included.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("add_entity", "Add an entity, as one undo step. Returns its id.", add, &[]),
        tool("update_entity", "Change any fields of an entity, as one undo step.", update, &["id"]),
        tool("delete_entity", "Delete an entity and everything under it.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("duplicate_entity", "Copy an entity with its children, as its next sibling. Returns the copy's id.", json!({ "id": { "type": "string", "description": ID } }), &["id"]),
        tool("reparent", "Move an entity under another, or to the top without parent. Refuses loops.", json!({ "id": { "type": "string", "description": ID }, "parent": { "type": "string", "description": ID } }), &["id"]),
        tool("make_prefab", "Turn an entity into prefabs/<name>.prefab and leave an instance in its place.", json!({ "id": { "type": "string", "description": ID }, "name": { "type": "string" } }), &["id", "name"]),
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
        tool("undo", "Take back the last edit.", json!({}), &[]),
        tool("redo", "Put back the last edit taken back.", json!({}), &[]),
        tool("render", "Draw the view and return it as a PNG. Camera arguments move the view first and are not an edit.", camera, &[]),
        tool("pick", "The entity under a pixel of the last render.", json!({ "x": { "type": "integer" }, "y": { "type": "integer" } }), &["x", "y"]),
        tool("import", "Import a source file (.gltf .glb .obj .png .jpg .tga .bmp .wav .rmat) into the project.", json!({ "source": { "type": "string" } }), &["source"]),
        tool("reload", "Pick up files changed on disk: the scene, prefabs, and assets rebuilt from changed sources.", json!({}), &[]),
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
        "scene_tree" => Ok(vec![text(tree(server)?)]),
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
            let moved = server
                .session()?
                .reparent(id, parent)
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
        "undo" => {
            let done = server.session()?.undo().map_err(|e| e.to_string())?;
            Ok(vec![text(if done { "undone" } else { "nothing to undo" })])
        }
        "redo" => {
            let done = server.session()?.redo().map_err(|e| e.to_string())?;
            Ok(vec![text(if done { "redone" } else { "nothing to redo" })])
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
        desc.body = ron::from_str::<Body>(&body)
            .map_err(|_| format!("body is None, Static or Dynamic, not {body}"))?;
    }
    if let Some(collider) = optional_string(args, "collider")? {
        desc.collider =
            ron::from_str::<Collider>(&collider).map_err(|e| format!("collider: {e}"))?;
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
    let session = server.session()?;
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
