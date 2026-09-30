//! What changes when the session is the live studio's ([`crate::live`]),
//! and the tools that say where the agent is and show the Game view.
//!
//! The tools are the same functions either way; the studio's window only
//! asks for manners. Its view is not resized for a picture (the window
//! owns its size: the picture is scaled instead), Play shows the game in
//! the Game view as the button does, and the person's unsaved work is not
//! thrown away by opening another scene over it.

use serde_json::{json, Value};

use crate::{image, text, Call, Mode};

type Answer = Result<Vec<Value>, String>;

/// The biggest picture of the live view given when no size is asked for:
/// a Retina window's view is several megabytes of PNG.
const LIVE_PICTURE: (u32, u32) = (1280, 800);

pub fn list() -> Vec<Value> {
    vec![
        json!({
            "name": "attached",
            "description": "Whose editor these tools drive: the live scrap studio a person has open (what they see changes as you edit, and they can undo it), or this server's own headless session, and why no studio was attached to.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "game_view",
            "description": "What the Game view shows right now, as a PNG: the last frame of the game start_game is playing there, or — before it draws, or with no game — the scene through the game's camera. Says which. width and height scale the picture to fit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "width": { "type": "integer" },
                    "height": { "type": "integer" },
                },
            },
        }),
    ]
}

/// Tools that write no file of the project: they look, play, or edit the
/// open document (which is written only by save_scene).
pub fn writes_no_files(name: &str) -> bool {
    matches!(
        name,
        "attached"
            | "game_view"
            | "render"
            | "scene_tree"
            | "find"
            | "inspect"
            | "console"
            | "game_state"
            | "stop_game"
            | "problems"
            | "check"
            | "edits"
            | "pick"
            | "assets"
            | "usages"
            | "simulate"
            | "add_entity"
            | "update_entity"
            | "delete_entity"
            | "undo"
            | "redo"
            | "set_field"
            | "look"
            | "mood"
            | "path"
            | "graph"
            | "dialogue"
            | "dialogue_play"
            | "shader_graph"
            | "shader_graph_preview"
            | "git_status"
            | "changes"
            | "history"
            | "locks"
    )
}

/// In the live studio, `width` and `height` size the picture, not the view:
/// taken out of the arguments before the tool sees them.
pub fn take_size(arguments: &mut Value) -> Option<(u32, u32)> {
    let object = arguments.as_object_mut()?;
    let width = object.remove("width").and_then(|v| v.as_u64());
    let height = object.remove("height").and_then(|v| v.as_u64());
    match (width, height) {
        (None, None) => None,
        (w, h) => Some((
            w.unwrap_or(u64::from(u32::MAX)).clamp(16, 4096) as u32,
            h.unwrap_or(u64::from(u32::MAX)).clamp(16, 4096) as u32,
        )),
    }
}

/// A tool answered here, or `None` for the engine's own.
pub fn call(call: &mut Call<'_>, mode: &Mode, name: &str, args: &Value) -> Option<Answer> {
    let live = call.live_size().is_some();
    match name {
        "attached" => Some(Ok(vec![text(attached(mode))])),
        "game_view" => Some(game_view(call, args)),
        "new_project" if live => Some(Err(
            "this is the studio a person has open: a new project would take their window \
             somewhere else. Make it with `scrap new`, or ask them to."
                .into(),
        )),
        "open_scene" if live => Some(open_scene(call, mode, args)),
        "simulate" if live => {
            let session = match call.session() {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
            if session.is_playing() {
                return Some(Err(
                    "the person is simulating in the view: stop that first (Play → Simulate)"
                        .into(),
                ));
            }
            None
        }
        // Play, as the button: the game shows in the Game view.
        "start_game" if live => Some(crate::tools::call(call, name, args).inspect(|_| {
            if let Ok(session) = call.session() {
                session.set_game_view(true);
            }
        })),
        // And stop goes back to the Scene view.
        "stop_game" if live => Some(crate::tools::call(call, name, args).inspect(|_| {
            if let Ok(session) = call.session() {
                session.set_game_view(false);
            }
        })),
        _ => None,
    }
}

fn attached(mode: &Mode) -> String {
    match mode {
        Mode::Live { project } => format!(
            "attached: the live scrap studio on {} (process {}). Every tool acts on the window the \
             person sees — the same document, undo history, Game view and Console; they see each \
             edit at once and can undo it. Nothing is written to disk until save_scene.",
            project.display(),
            std::process::id()
        ),
        Mode::Headless(why) => format!(
            "headless: this server's own editor session, not a studio window — {why}. To drive \
             the studio a person has open, open the project there and restart this MCP server \
             (in Claude Code: /mcp); `scrap-mcp --project PATH` names the project."
        ),
    }
}

/// Opening a scene in the person's window: not over their unsaved work,
/// and not in another project than the one this studio serves.
fn open_scene(call: &mut Call<'_>, mode: &Mode, args: &Value) -> Answer {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("path is a string")?;
    let session = call.session()?;
    let discard = args.get("discard").and_then(Value::as_bool) == Some(true);
    if session.is_modified() && !discard {
        return Err(
            "the open scene has unsaved changes the person may want: save_scene first, \
                    or open_scene with discard: true to throw them away"
                .into(),
        );
    }
    let mut args = args.clone();
    if let Mode::Live { project } = mode {
        // A relative path is the project's: the studio's working folder
        // is not the agent's.
        let path = if std::path::Path::new(path).is_relative() {
            project.join(path)
        } else {
            std::path::PathBuf::from(path)
        };
        args["path"] = Value::String(path.to_string_lossy().into_owned());
        let here = scrap::Project::find(&path)
            .ok()
            .and_then(|p| std::fs::canonicalize(p.root()).ok());
        if here.as_deref() != Some(project.as_path()) {
            return Err(format!(
                "{} is not in {}, the project this studio serves",
                path.display(),
                project.display()
            ));
        }
    }
    if let Some(object) = args.as_object_mut() {
        object.remove("discard");
    }
    crate::tools::call(call, "open_scene", &args)
}

/// The live studio's Scene view as the person has it — handles, grid and
/// outlines — at the window's size, scaled to `wanted`.
pub fn render(session: &mut scrap_editor::Session, wanted: Option<(u32, u32)>) -> Answer {
    let was = session.is_game_view();
    session.set_game_view(false);
    session.render();
    let pixels = session.frame_target().read_rgba(session.gpu());
    session.set_game_view(was);
    let (width, height) = session.size();
    let (png, (w, h)) = png(pixels, (width, height), wanted.unwrap_or(LIVE_PICTURE))?;
    let camera = session.camera();
    Ok(vec![
        image(&png),
        text(format!(
            "{w}x{h} (the studio's Scene view, {width}x{height}), eye {} looking at {}",
            crate::tools::triple(camera.position),
            crate::tools::triple(camera.target)
        )),
    ])
}

/// What the Game view shows now.
fn game_view(call: &mut Call<'_>, args: &Value) -> Answer {
    let wanted = match call.live_size() {
        Some(wanted) => wanted,
        None => {
            let mut args = args.clone();
            take_size(&mut args)
        }
    };
    let session = call.session()?;
    session.poll_game();
    let was = session.is_game_view();
    session.set_game_view(true);
    session.render();
    let pixels = session.frame_target().read_rgba(session.gpu());
    session.set_game_view(was);
    let size = session.size();
    let (png, (w, h)) = png(pixels, size, wanted.unwrap_or(LIVE_PICTURE))?;
    let what = if session.game_draws_in_view() {
        "the game's last frame".to_string()
    } else if session.is_game_in_view() {
        "the game is still building or starting, so this is the scene through its camera; \
         console says how far it got"
            .to_string()
    } else {
        "no game is playing (start_game plays it), so this is the scene through the game's camera"
            .to_string()
    };
    Ok(vec![image(&png), text(format!("{w}x{h}: {what}"))])
}

/// RGBA8 as a PNG, scaled down to fit `fit` when bigger, keeping its shape.
fn png(
    pixels: Vec<u8>,
    size: (u32, u32),
    fit: (u32, u32),
) -> Result<(Vec<u8>, (u32, u32)), String> {
    use image::ImageEncoder;
    let (width, height) = size;
    let mut picture = image::RgbaImage::from_raw(width, height, pixels)
        .ok_or("the view's picture is not its size")?;
    let scale = (fit.0 as f32 / width as f32)
        .min(fit.1 as f32 / height as f32)
        .min(1.0);
    if scale < 1.0 {
        let w = ((width as f32 * scale).round() as u32).max(1);
        let h = ((height as f32 * scale).round() as u32).max(1);
        picture = image::imageops::resize(&picture, w, h, image::imageops::FilterType::Triangle);
    }
    let (w, h) = picture.dimensions();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(picture.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok((out, (w, h)))
}
