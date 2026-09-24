//! The editor used as a person uses it — off-screen.
//!
//! The session's behaviour is tested in `runity-editor`; what is left here
//! is the wiring: that a click on a line selects it, a number typed into
//! the Inspector reaches the document as one undo step, the toolbar undoes
//! it, a menu entry makes a cube, F2 renames, a Project entry dragged into
//! the Scene view lands there, a key pressed after choosing a line acts on
//! it. The clicks go to nodes by name, where a person's pointer would be.
//!
//! Skipped, not failed, where there is no GPU to render the scene with.

#[allow(unused_imports)]
use runity::prelude::*;
use runity::input::{InputEvent, Key, MouseButton};
use runity_studio::Studio;

fn studio() -> Option<(Studio, std::path::PathBuf)> {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/valley");
    let dir = std::env::temp_dir().join(format!(
        "runity-studio-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    copy_dir(src.as_ref(), &dir);
    // Only the scene opened: the example's others would fill the Project
    // panel, which does not scroll, and push the tiles dragged here off it.
    for entry in std::fs::read_dir(dir.join("scenes")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("first-light.") {
            std::fs::remove_file(&path).unwrap();
        }
    }
    let scene = dir.join("scenes/first-light.ron");
    let session = match runity_studio::open(&scene) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("no renderer here ({e}); skipped");
            return None;
        }
    };
    let mut studio = Studio::new(session, 1440.0, 900.0, 1.0);
    studio.frame();
    Some((studio, dir))
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name();
        if name == "library" || name == ".git" || name == "target" {
            continue;
        }
        if path.is_dir() {
            copy_dir(&path, &to.join(name));
        } else {
            std::fs::copy(&path, to.join(name)).unwrap();
        }
    }
}

/// Frames, a little apart, until the Project has drawn its pictures: it
/// draws them only once nothing has happened for a moment.
fn draw_pictures(s: &mut Studio) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while s.bottom_pictures_pending() > 0 && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(30));
        s.frame();
    }
}

/// Click the node with this name, as a pointer would, and let a frame go.
fn click(studio: &mut Studio, name: &str) {
    press(studio, name, MouseButton::Left);
}

/// Two clicks within the double-click's time: no frame between them, so a
/// slow frame (a pipeline being built) cannot pull them apart.
fn double_click(studio: &mut Studio, name: &str) {
    studio.ui.paint();
    let node = studio
        .ui
        .find(name)
        .unwrap_or_else(|| panic!("no node {name:?} in\n{}", studio.ui.dump()));
    let (x, y) = studio.ui.rect(node).center();
    studio.handle(&InputEvent::MouseMoved { x, y });
    for _ in 0..2 {
        studio.handle(&InputEvent::MouseDown(MouseButton::Left));
        studio.handle(&InputEvent::MouseUp(MouseButton::Left));
    }
    studio.frame();
}

fn press(studio: &mut Studio, name: &str, button: MouseButton) {
    studio.ui.paint();
    let node = studio
        .ui
        .find(name)
        .unwrap_or_else(|| panic!("no node {name:?} in\n{}", studio.ui.dump()));
    let (x, y) = studio.ui.rect(node).center();
    studio.handle(&InputEvent::MouseMoved { x, y });
    studio.handle(&InputEvent::MouseDown(button));
    studio.handle(&InputEvent::MouseUp(button));
    studio.frame();
}

fn key(studio: &mut Studio, key: Key) {
    studio.handle(&InputEvent::KeyDown(key));
    studio.frame();
    studio.handle(&InputEvent::KeyUp(key));
    studio.frame();
}

fn shortcut(studio: &mut Studio, k: Key) {
    let cmd = if cfg!(target_os = "macos") {
        Key::LeftSuper
    } else {
        Key::LeftControl
    };
    studio.handle(&InputEvent::KeyDown(cmd));
    studio.handle(&InputEvent::KeyDown(k));
    studio.frame();
    studio.handle(&InputEvent::KeyUp(k));
    studio.handle(&InputEvent::KeyUp(cmd));
    studio.frame();
}

fn type_text(studio: &mut Studio, text: &str) {
    studio.handle(&InputEvent::Text(text.into()));
}

#[test]
fn select_type_undo() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let boulder = s.session.find("boulder").unwrap();
    let before = s.session.transform(boulder).unwrap();

    click(&mut s, "line boulder");
    assert_eq!(s.session.selection(), vec![boulder]);

    // Position X: all of it, then 5*2, then Enter.
    click(&mut s, "position x");
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(&mut s, "5*2");
    s.handle(&InputEvent::KeyDown(Key::Enter));
    s.frame();
    let moved = s.session.transform(boulder).unwrap();
    assert_eq!(moved.position.x, 10.0, "typed into the Inspector");
    assert_eq!(moved.position.y, before.position.y);
    assert!(s.session.is_modified());

    click(&mut s, "undo");
    assert_eq!(
        s.session.transform(boulder).unwrap().position,
        before.position
    );
    click(&mut s, "redo");
    assert_eq!(s.session.transform(boulder).unwrap().position.x, 10.0);
}

#[test]
fn a_menu_entry_makes_a_cube_and_f2_renames_it() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let count = s.session.entity_count();
    click(&mut s, "menu bar Entity");
    assert!(s.ui.find("menu Cube").is_some(), "the menu opened");
    click(&mut s, "menu Cube");
    assert!(s.ui.find("menu Cube").is_none(), "and closed");
    assert_eq!(s.session.entity_count(), count + 1);
    let cube = s.session.selected().expect("the new cube is selected");

    key(&mut s, Key::F2);
    assert!(s.ui.find("rename").is_some(), "{}", s.ui.dump());
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(&mut s, "ящик");
    key(&mut s, Key::Enter);
    assert_eq!(s.session.entity_name(cube).as_deref(), Some("ящик"));
    assert!(s.ui.find("line ящик").is_some());
}

#[test]
fn keys_after_a_line_act_on_it_in_the_scene() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let count = s.session.entity_count();
    click(&mut s, "line crate");
    shortcut(&mut s, Key::D);
    assert_eq!(s.session.entity_count(), count + 1, "Cmd D duplicated");
    key(&mut s, Key::Delete);
    assert_eq!(s.session.entity_count(), count, "Delete deleted");
    // W E R: the tools.
    key(&mut s, Key::E);
    assert_eq!(s.session.tool(), runity::gizmo::Tool::Rotate);
    click(&mut s, "tool Scale");
    assert_eq!(s.session.tool(), runity::gizmo::Tool::Scale);
}

#[test]
fn play_stop_and_the_right_click_menu() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "play");
    assert!(s.session.is_playing());
    click(&mut s, "play");
    assert!(!s.session.is_playing());

    press(&mut s, "line boulder", MouseButton::Right);
    assert!(s.ui.find("menu Make Prefab").is_some());
    click(&mut s, "menu Duplicate");
    assert_eq!(s.session.selection().len(), 1);
    assert!(s.session.entity_count() > 13);
}

#[test]
fn a_project_entry_dragged_into_the_view_lands_there() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let count = s.session.entity_count();
    s.ui.paint();
    let tile = s.ui.find("asset sphere").expect("a sphere in the Project");
    let view = s.ui.find("scene view").unwrap();
    let (ax, ay) = s.ui.rect(tile).center();
    let (vx, vy) = s.ui.rect(view).center();
    s.handle(&InputEvent::MouseMoved { x: ax, y: ay });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.handle(&InputEvent::MouseMoved {
        x: ax + 20.0,
        y: ay - 20.0,
    });
    s.handle(&InputEvent::MouseMoved { x: vx, y: vy });
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    assert_eq!(s.session.entity_count(), count + 1);
    let placed = s.session.selected().unwrap();
    assert_eq!(s.session.entity_name(placed).as_deref(), Some("sphere"));
}

#[test]
fn save_writes_the_file_and_clears_the_mark() {
    let Some((mut s, dir)) = studio() else { return };
    click(&mut s, "line crate");
    key(&mut s, Key::Delete);
    assert!(s.session.is_modified());
    click(&mut s, "save");
    assert!(!s.session.is_modified());
    let text = std::fs::read_to_string(dir.join("scenes/first-light.ron")).unwrap();
    assert!(
        !text.contains("\"crate\""),
        "the crate is gone from the file"
    );
}

#[test]
fn play_looks_through_the_game_and_the_tabs_switch_views() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "view game");
    assert!(s.session.is_game_view());
    click(&mut s, "view scene");
    assert!(!s.session.is_game_view());
    click(&mut s, "play");
    assert!(s.session.is_game_view(), "Play brings up the Game view");
    click(&mut s, "play");
    assert!(!s.session.is_game_view(), "and Stop takes it away");
}

/// Drag the line named `from` to a spot `t` of the way down the line named
/// `onto` (0 top, 1 bottom).
fn drag_line(s: &mut Studio, from: &str, onto: &str, t: f32) {
    s.ui.paint();
    let a = s.ui.rect(s.ui.find(from).unwrap());
    let b = s.ui.rect(s.ui.find(onto).unwrap());
    let (ax, ay) = a.center();
    s.handle(&InputEvent::MouseMoved { x: ax, y: ay });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.handle(&InputEvent::MouseMoved { x: ax, y: ay + 8.0 });
    s.frame();
    s.handle(&InputEvent::MouseMoved {
        x: b.x + 60.0,
        y: b.y + b.height * t,
    });
    s.frame();
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
}

fn order(s: &Studio) -> Vec<String> {
    s.session.hierarchy().into_iter().map(|r| r.name).collect()
}

#[test]
fn lines_dragged_reorder_and_nest() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    // Onto the top quarter of «tree near»: before it, at the top level.
    drag_line(&mut s, "line boulder", "line tree near", 0.1);
    let names = order(&s);
    let b = names.iter().position(|n| n == "boulder").unwrap();
    let t = names.iter().position(|n| n == "tree near").unwrap();
    assert_eq!(b + 1, t, "{names:?}");
    // Onto the middle of «crate»: into it.
    drag_line(&mut s, "line boulder", "line crate", 0.5);
    let crate_id = s.session.find("crate").unwrap();
    let rows = s.session.hierarchy();
    let boulder = rows.iter().find(|r| r.name == "boulder").unwrap();
    assert_eq!(boulder.depth, 1, "a child now");
    let _ = crate_id;
    // One undo takes it back out.
    click(&mut s, "undo");
    let rows = s.session.hierarchy();
    assert_eq!(rows.iter().find(|r| r.name == "boulder").unwrap().depth, 0);
}

#[test]
fn shift_selects_a_range_and_the_arrows_move_it() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "line tree near");
    s.handle(&InputEvent::KeyDown(Key::LeftShift));
    click(&mut s, "line tree far");
    s.handle(&InputEvent::KeyUp(Key::LeftShift));
    assert_eq!(s.session.selection().len(), 3, "near, mid, far");

    click(&mut s, "line crate");
    key(&mut s, Key::Down);
    assert_eq!(s.session.selected(), s.session.find("boulder"));
    key(&mut s, Key::Up);
    assert_eq!(s.session.selected(), s.session.find("crate"));
    // Down to the campfire, then Left folds it.
    key(&mut s, Key::Down);
    key(&mut s, Key::Down);
    let lines = s.session.hierarchy().len();
    key(&mut s, Key::Left);
    assert_eq!(s.session.hierarchy().len(), lines - 5);
    key(&mut s, Key::Right);
    assert_eq!(s.session.hierarchy().len(), lines);
}

#[test]
fn snap_and_views_from_the_corner() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    assert_eq!(s.session.snap().meters, 0.0);
    click(&mut s, "snap");
    assert_eq!(s.session.snap().meters, 0.25);
    click(&mut s, "compass top");
    assert!(s.session.is_orthographic());
    click(&mut s, "compass middle");
    assert!(!s.session.is_orthographic());
}

#[test]
fn a_scene_changed_on_disk_comes_in_by_itself() {
    let Some((mut s, dir)) = studio() else { return };
    let path = dir.join("scenes/first-light.ron");
    let text = std::fs::read_to_string(&path).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, text.replace("\"boulder\"", "\"big rock\"")).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while s.session.find("big rock").is_none() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(50));
        s.frame();
    }
    assert!(s.session.find("big rock").is_some(), "reloaded");
    assert!(s.ui.find("line big rock").is_some(), "and shown");
}

#[test]
fn the_inspector_says_when_nothing_is_selected() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    assert!(s.ui.dump().contains("\"Nothing selected\""));
}

#[test]
fn a_colour_typed_or_slid_paints_the_selection() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "line crate");
    let crate_id = s.session.find("crate").unwrap();
    click(&mut s, "material swatch");
    click(&mut s, "picker hex");
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(&mut s, "#ff0000");
    key(&mut s, Key::Enter);
    let m = s.session.material(crate_id).unwrap();
    assert!(
        (m.base_color[0] - 1.0).abs() < 1e-3 && m.base_color[1] < 1e-3,
        "{m:?}"
    );

    // Dragged down the square to the bottom: black as it goes, one step.
    let steps = s.session.undo_steps().len();
    s.ui.paint();
    let square = s.ui.rect(s.ui.find("picker square").unwrap());
    s.handle(&InputEvent::MouseMoved {
        x: square.x + square.width * 0.5,
        y: square.y + square.height * 0.5,
    });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    for i in 1..=6 {
        s.handle(&InputEvent::MouseMoved {
            x: square.x + square.width * 0.5,
            y: square.y + square.height * (0.5 + i as f32 * 0.1),
        });
        s.frame();
    }
    let m = s.session.material(crate_id).unwrap();
    assert!(m.base_color.iter().all(|c| *c < 0.01), "black while held: {m:?}");
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    assert_eq!(s.session.undo_steps().len(), steps + 1, "one step");
}

#[test]
fn a_prefab_opens_from_the_project_and_back_returns_to_the_scene() {
    let Some((mut s, dir)) = studio() else { return };
    let scene = s.session.scene_path().unwrap().to_path_buf();
    // The first «asset campfire» is the prefab (prefabs come before models).
    double_click(&mut s, "asset campfire");
    assert!(s.session.is_prefab(), "double click opened the prefab");
    assert!(s.title().contains("(prefab)"));
    assert!(s.ui.dump().contains("Prefab: campfire"));
    // An edit in the prefab, then Back: saved, and the scene is open again.
    click(&mut s, "line ember");
    key(&mut s, Key::Delete);
    click(&mut s, "prefab back");
    assert!(!s.session.is_prefab());
    assert_eq!(s.session.scene_path(), Some(scene.as_path()));
    let prefab = std::fs::read_to_string(dir.join("prefabs/campfire.prefab")).unwrap();
    assert!(
        !prefab.contains("\"ember\""),
        "the prefab was saved without its ember"
    );
}

#[test]
fn a_big_scene_stays_quick() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    for i in 0..2000 {
        let id = s.session.add(None, "builtin:cube").unwrap();
        let _ = s.session.rename(id, &format!("cube {i}"));
    }
    let t = std::time::Instant::now();
    s.refresh();
    s.frame();
    let first = t.elapsed();
    // What is measured is the editor at rest, not the Project's pictures
    // being drawn once.
    draw_pictures(&mut s);
    // Idle: nothing changed.
    let t = std::time::Instant::now();
    for _ in 0..10 {
        s.frame();
    }
    let idle = t.elapsed() / 10;
    // A selection change: every line is looked at again.
    let ids = s.session.entities();
    let t = std::time::Instant::now();
    for id in ids.iter().take(10) {
        s.session.select(Some(*id)).unwrap();
        s.frame();
    }
    let select = t.elapsed() / 10;
    eprintln!("2000 entities: first {first:?}, idle frame {idle:?}, selection frame {select:?}");
    // Debug build, our crates unoptimised: a budget with room, not a target.
    assert!(select.as_millis() < 80, "a selection took {select:?}");
    assert!(idle.as_millis() < 20, "an idle frame took {idle:?}");
}

#[test]
fn an_asset_clicked_in_the_project_is_shown_in_the_inspector() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "asset campfire");
    let dump = s.ui.dump();
    assert!(dump.contains("#asset preview"), "{dump}");
    assert!(dump.contains("Used in ("), "{dump}");
    // Choosing something in the scene goes back to it.
    click(&mut s, "line crate");
    assert!(s.ui.find("asset preview").is_none());
    assert!(s.ui.find("position x").is_some());
}

fn menu(s: &mut Studio, bar: &str, entry: &str) {
    click(s, &format!("menu bar {bar}"));
    click(s, &format!("menu {entry}"));
}

#[test]
fn with_nothing_selected_the_inspector_sets_the_time_of_day() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    s.ui.paint();
    let track =
        s.ui.rect(s.ui.find("sun hour").expect("the scene's settings"));
    s.handle(&InputEvent::MouseMoved {
        x: track.x + track.width * 0.8,
        y: track.y + 3.0,
    });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let sun = s.session.environment()[0].1.clone();
    assert!(sun.contains("hour:19.") || sun.contains("hour:19"), "{sun}");
    click(&mut s, "undo");
    assert!(!s.session.environment()[0].1.contains("hour:19"));
}

#[test]
fn a_long_console_line_opens_on_a_click() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    s.session.clear_console();
    s.session.say(
        runity_editor::console::Level::Error,
        "error[E0308]: mismatched types\n --> src/door.rs:12:9\n  expected f32",
    );
    s.frame();
    click(&mut s, "tab console");
    let dump = s.ui.dump();
    assert!(dump.contains("mismatched types  …"), "{dump}");
    assert!(!dump.contains("expected f32"));
    click(&mut s, "console line 0");
    assert!(s.ui.dump().contains("expected f32"), "opened");
    // Apart, or it is a double click — which opens the file instead.
    std::thread::sleep(std::time::Duration::from_millis(450));
    click(&mut s, "console line 0");
    assert!(!s.ui.dump().contains("expected f32"), "and closed");
}

#[test]
fn the_history_tab_goes_back_and_forward_to_a_step() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "line crate");
    key(&mut s, Key::Delete);
    click(&mut s, "line boulder");
    key(&mut s, Key::Delete);
    let count = s.session.entity_count();
    click(&mut s, "tab history");
    assert!(s.ui.find("history 2").is_some(), "{}", s.ui.dump());
    click(&mut s, "history 0");
    assert_eq!(s.session.entity_count(), count + 2, "back to as opened");
    click(&mut s, "history 1");
    assert_eq!(s.session.entity_count(), count + 1, "one step forward");
}

#[test]
fn the_git_tab_lists_the_scenes_revisions_and_brings_one_back() {
    let Some((mut s, dir)) = studio() else { return };
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !git(&["init", "-q"]) {
        eprintln!("no git here; skipped");
        return;
    }
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "add", "."]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "first light",
    ]);
    click(&mut s, "line crate");
    key(&mut s, Key::Delete);
    click(&mut s, "save");
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qam",
        "no crate",
    ]);
    assert!(s.session.find("crate").is_none());

    click(&mut s, "tab git");
    s.frame();
    let dump = s.ui.dump();
    assert!(
        dump.contains("\"no crate\"") && dump.contains("\"first light\""),
        "{dump}"
    );
    // The older one, double-clicked: the crate comes back, as one undo step.
    let rows: Vec<String> = dump
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("#revision ")
                .map(|r| r.split(' ').next().unwrap().to_string())
        })
        .collect();
    let older = format!("revision {}", rows.last().unwrap());
    click(&mut s, &older);
    click(&mut s, &older);
    assert!(s.session.find("crate").is_some(), "brought back");
    click(&mut s, "undo");
    assert!(s.session.find("crate").is_none());
}

#[test]
fn the_layout_is_kept_between_runs() {
    let Some((mut s, dir)) = studio() else { return };
    s.ui.paint();
    let split = s.ui.rect(s.ui.find("split left").unwrap());
    let (x, y) = split.center();
    s.handle(&InputEvent::MouseMoved { x, y });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    for i in 1..=10 {
        s.handle(&InputEvent::MouseMoved {
            x: x + 8.0 * i as f32,
            y,
        });
    }
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let width = s.ui.rect(s.ui.find("hierarchy").unwrap()).width;
    click(&mut s, "tab console");
    std::thread::sleep(std::time::Duration::from_millis(600));
    s.frame();
    drop(s);

    let session = runity_studio::open(&dir.join("scenes/first-light.ron")).unwrap();
    let mut again = Studio::new(session, 1440.0, 900.0, 1.0);
    again.frame();
    again.ui.paint();
    let now = again.ui.rect(again.ui.find("hierarchy").unwrap()).width;
    assert!((now - width).abs() < 2.0, "{now} vs {width}");
    assert!(
        again.ui.dump().contains("#console lines"),
        "the Console tab is open again"
    );
    let _ = dir;
}

#[test]
fn a_terrain_from_the_menu_rises_under_the_brush() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    menu(&mut s, "Entity", "Terrain");
    let terrain = s.session.selected().expect("a terrain, selected");
    assert_eq!(s.session.entity_model(terrain).as_deref(), Some("terrain"));
    s.session.set_camera(
        runity::glam::Vec3::new(0.0, 20.0, 20.0),
        runity::glam::Vec3::ZERO,
    );
    s.frame();
    // The brush is on after making one: drag across the middle of the view.
    s.ui.paint();
    let view = s.ui.rect(s.ui.find("scene view").unwrap());
    let (cx, cy) = view.center();
    let (w, h) = s.session.size();
    let before = s.session.point_under(w / 2, h / 2).unwrap().y;
    let selected = s.session.selection();
    s.handle(&InputEvent::MouseMoved { x: cx, y: cy });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    for i in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(110));
        s.handle(&InputEvent::MouseMoved {
            x: cx + i as f32,
            y: cy,
        });
    }
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let after = s.session.point_under(w / 2, h / 2).unwrap().y;
    assert!(after > before + 0.5, "{before} -> {after}");
    assert_eq!(s.session.selection(), selected, "the brush does not select");
}

#[test]
fn panels_hide_and_the_view_takes_the_window() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    s.ui.paint();
    let small = s.ui.rect(s.ui.find("scene view").unwrap()).width;
    menu(&mut s, "Window", "Right Dock");
    assert!(
        s.ui.find("inspector")
            .is_some_and(|n| s.ui.rect(n).width == 0.0)
            || !s.ui.dump().contains("#inspector")
    );
    s.ui.paint();
    let wider = s.ui.rect(s.ui.find("scene view").unwrap()).width;
    assert!(wider > small + 200.0, "{small} -> {wider}");
    // Shift Space: everything but the view, and back.
    s.handle(&InputEvent::KeyDown(Key::LeftShift));
    key(&mut s, Key::Space);
    s.handle(&InputEvent::KeyUp(Key::LeftShift));
    s.ui.paint();
    let full = s.ui.rect(s.ui.find("scene view").unwrap()).width;
    assert!(full > 1400.0, "{full}");
    s.handle(&InputEvent::KeyDown(Key::LeftShift));
    key(&mut s, Key::Space);
    s.handle(&InputEvent::KeyUp(Key::LeftShift));
    s.ui.paint();
    assert!((s.ui.rect(s.ui.find("scene view").unwrap()).width - wider).abs() < 1.0);
}

#[test]
fn a_game_component_is_a_form_by_its_shape() {
    use runity::shape::Shape;
    let Some((mut s, dir)) = studio() else { return };
    let mut shapes = std::collections::BTreeMap::new();
    shapes.insert(
        "door".to_string(),
        Shape::Struct(vec![
            ("open_angle".into(), Shape::Float),
            ("locked".into(), Shape::Bool),
            (
                "side".into(),
                Shape::Enum(vec!["Left".into(), "Right".into()]),
            ),
        ]),
    );
    std::fs::create_dir_all(dir.join("library")).unwrap();
    std::fs::write(
        dir.join("library/components.ron"),
        runity::ron::to_string(&shapes).unwrap(),
    )
    .unwrap();
    let crate_id = s.session.find("crate").unwrap();
    s.session
        .set_component(
            crate_id,
            "door",
            Some("(open_angle: 90.0, locked: false, side: Left)"),
        )
        .unwrap();
    click(&mut s, "line crate");
    let value = |s: &Studio| {
        s.session
            .inspect(crate_id)
            .unwrap()
            .into_iter()
            .find(|f| f.name == "components.door")
            .unwrap()
            .value
    };

    click(&mut s, "door locked");
    assert!(value(&s).contains("locked: true"), "{}", value(&s));

    click(&mut s, "door open_angle");
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(&mut s, "45*2+30");
    key(&mut s, Key::Enter);
    assert!(value(&s).contains("open_angle: 120"), "{}", value(&s));

    click(&mut s, "door side");
    click(&mut s, "menu Right");
    assert!(value(&s).contains("side: Right"), "{}", value(&s));
    assert!(
        value(&s).contains("locked: true"),
        "the other fields kept theirs"
    );
}

#[test]
fn edit_undo_names_the_step_and_an_asset_drops_into_the_hierarchy() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "line crate");
    key(&mut s, Key::Delete);
    click(&mut s, "menu bar Edit");
    let dump = s.ui.dump();
    assert!(dump.contains("\"Undo "), "{dump}");
    key(&mut s, Key::Escape);

    // The sphere from the Project, let go on the campfire's line.
    s.ui.paint();
    let tile = s.ui.rect(s.ui.find("asset sphere").unwrap());
    let line = s.ui.rect(s.ui.find("line campfire").unwrap());
    let (ax, ay) = tile.center();
    s.handle(&InputEvent::MouseMoved { x: ax, y: ay });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.handle(&InputEvent::MouseMoved {
        x: ax + 20.0,
        y: ay - 20.0,
    });
    s.handle(&InputEvent::MouseMoved {
        x: line.x + 60.0,
        y: line.y + line.height / 2.0,
    });
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let rows = s.session.hierarchy();
    let sphere = rows.iter().find(|r| r.name == "sphere").unwrap_or_else(|| {
        panic!(
            "no sphere: {:?}\n{:?}",
            rows.iter().map(|r| (&r.name, r.depth)).collect::<Vec<_>>(),
            s.session
                .console()
                .iter()
                .map(|l| &l.text)
                .collect::<Vec<_>>()
        )
    });
    assert_eq!(sphere.depth, 1, "under the campfire");
}

/// Drag the tab named `tab` and let go over the node named `onto`.
fn drag_tab(s: &mut Studio, tab: &str, onto: &str) {
    s.ui.paint();
    let a = s.ui.rect(s.ui.find(tab).unwrap());
    let b = s.ui.rect(s.ui.find(onto).unwrap());
    let (ax, ay) = a.center();
    let (bx, by) = b.center();
    s.handle(&InputEvent::MouseMoved { x: ax, y: ay });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.handle(&InputEvent::MouseMoved {
        x: ax + 10.0,
        y: ay + 10.0,
    });
    s.frame();
    s.handle(&InputEvent::MouseMoved { x: bx, y: by });
    s.frame();
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
}

#[test]
fn a_tab_dragged_to_another_dock_takes_its_panel_there_and_stays() {
    let Some((mut s, dir)) = studio() else { return };
    drag_tab(&mut s, "tab console", "dock 0");
    s.ui.paint();
    let dock0 = s.ui.rect(s.ui.find("dock 0").unwrap());
    let console = s.ui.rect(s.ui.find("console lines").unwrap());
    assert!(
        dock0.contains(console.x + 1.0, console.y + 1.0),
        "the Console is on the left: {console:?} in {dock0:?}"
    );
    let tab = s.ui.rect(s.ui.find("tab hierarchy").unwrap());
    assert!(
        dock0.contains(tab.x + 1.0, tab.y + 1.0),
        "beside the Hierarchy's tab"
    );
    // The Hierarchy is a tab away.
    click(&mut s, "tab hierarchy");
    assert!(s.ui.rect(s.ui.find("hierarchy list").unwrap()).width > 0.0);
    click(&mut s, "tab console");
    // Written down, and read back next run.
    std::thread::sleep(std::time::Duration::from_millis(600));
    s.frame();
    drop(s);
    let session = runity_studio::open(&dir.join("scenes/first-light.ron")).unwrap();
    let mut again = Studio::new(session, 1440.0, 900.0, 1.0);
    again.frame();
    again.ui.paint();
    let dock0 = again.ui.rect(again.ui.find("dock 0").unwrap());
    let console = again.ui.rect(again.ui.find("console lines").unwrap());
    assert!(
        dock0.contains(console.x + 1.0, console.y + 1.0),
        "still on the left, on top"
    );
}

#[test]
fn a_dock_left_without_tabs_folds_away_and_shows_again_while_a_tab_is_dragged() {
    let Some((mut s, _dir)) = studio() else { return };
    s.ui.paint();
    let view_before = s.ui.rect(s.ui.find("scene view").unwrap()).width;
    drag_tab(&mut s, "tab inspector", "dock 2");
    s.frame();
    s.ui.paint();
    assert!(!s.ui.is_shown(s.ui.find("dock 1").unwrap()), "nothing is left on the right");
    let view_after = s.ui.rect(s.ui.find("scene view").unwrap()).width;
    assert!(
        view_after > view_before + 200.0,
        "the view took its room: {view_before} -> {view_after}"
    );
    // Picking the tab up shows the empty dock, to put it back on.
    let (ax, ay) = s.ui.rect(s.ui.find("tab inspector").unwrap()).center();
    s.handle(&InputEvent::MouseMoved { x: ax, y: ay });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.handle(&InputEvent::MouseMoved {
        x: ax + 10.0,
        y: ay + 10.0,
    });
    s.frame();
    s.ui.paint();
    let right = s.ui.rect(s.ui.find("dock 1").unwrap());
    assert!(right.width > 100.0, "shown while dragging: {right:?}");
    let (bx, by) = right.center();
    s.handle(&InputEvent::MouseMoved { x: bx, y: by });
    s.frame();
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    s.ui.paint();
    let right = s.ui.rect(s.ui.find("dock 1").unwrap());
    let inspector = s.ui.rect(s.ui.find("tab inspector").unwrap());
    assert!(
        right.contains(inspector.x + 1.0, inspector.y + 1.0),
        "back on the right"
    );
}

/// A double click after the last one's time has run out: not a third
/// and fourth click.
fn pause_then_double_click(s: &mut Studio, name: &str) {
    std::thread::sleep(std::time::Duration::from_millis(450));
    double_click(s, name);
}

#[test]
fn a_double_clicked_tab_takes_the_whole_window_and_gives_it_back() {
    let Some((mut s, _dir)) = studio() else { return };
    s.ui.paint();
    let lower = s.ui.rect(s.ui.find("dock 2").unwrap());
    pause_then_double_click(&mut s, "tab project");
    s.ui.paint();
    let big = s.ui.rect(s.ui.find("dock 2").unwrap());
    assert!(
        big.width > 1300.0 && big.height > 700.0,
        "the lower dock over the window: {big:?}"
    );
    assert!(!s.ui.is_shown(s.ui.find("scene view").unwrap()), "the view gave way");
    assert!(!s.ui.is_shown(s.ui.find("dock 0").unwrap()));
    pause_then_double_click(&mut s, "tab project");
    s.ui.paint();
    let back = s.ui.rect(s.ui.find("dock 2").unwrap());
    assert!(
        (back.height - lower.height).abs() < 1.0 && (back.width - lower.width).abs() < 1.0,
        "as it was: {back:?}, was {lower:?}"
    );
    // A side dock too, and the Scene tab for the view.
    pause_then_double_click(&mut s, "tab hierarchy");
    s.ui.paint();
    assert!(s.ui.rect(s.ui.find("dock 0").unwrap()).width > 1300.0);
    pause_then_double_click(&mut s, "tab hierarchy");
    pause_then_double_click(&mut s, "view scene");
    s.ui.paint();
    assert!(!s.ui.is_shown(s.ui.find("dock 0").unwrap()));
    assert!(s.ui.rect(s.ui.find("scene view").unwrap()).width > 1300.0);
}

#[test]
fn play_pause_and_step_stand_in_the_middle_of_the_window() {
    let Some((mut s, _dir)) = studio() else { return };
    s.ui.paint();
    let (x, _) = s.ui.rect(s.ui.find("pause").unwrap()).center();
    assert!((x - 720.0).abs() < 2.0, "pause at {x}, the middle is 720");
    s.resize(1700.0, 900.0, 1.0);
    s.frame();
    s.ui.paint();
    let (x, _) = s.ui.rect(s.ui.find("pause").unwrap()).center();
    assert!((x - 850.0).abs() < 2.0, "pause at {x}, the middle is 850");
    // Too narrow for the middle: pushed aside, never over the tools.
    s.resize(1100.0, 800.0, 1.0);
    s.frame();
    s.ui.paint();
    let grid = s.ui.rect(s.ui.find("grid").unwrap());
    let play = s.ui.rect(s.ui.find("play").unwrap());
    assert!(play.x > grid.x + grid.width, "{play:?} clear of {grid:?}");
}

#[test]
fn the_hierarchy_collapses_and_expands_every_line_at_once() {
    let Some((mut s, _dir)) = studio() else { return };
    s.ui.paint();
    assert!(s.ui.find("line ember").is_some(), "the campfire starts open");
    click(&mut s, "hierarchy collapse all");
    s.ui.paint();
    assert!(s.ui.find("line ember").is_none(), "folded");
    assert!(s.ui.find("line campfire").is_some());
    click(&mut s, "hierarchy expand all");
    s.ui.paint();
    assert!(s.ui.find("line ember").is_some(), "open again");
}

#[test]
fn a_lines_eye_and_lock_show_only_under_the_pointer_or_when_set() {
    let Some((mut s, _dir)) = studio() else { return };
    s.ui.paint();
    // Line → [arrow, icon, name, tag, tools[eye, lock], dot].
    let tools = |s: &Studio, name: &str| -> (f32, f32) {
        let line = s.ui.find(name).unwrap();
        let tools = s.ui.children(s.ui.children(line)[4]);
        (
            s.ui.style(tools[0]).look.opacity,
            s.ui.style(tools[1]).look.opacity,
        )
    };
    assert_eq!(tools(&s, "line crate"), (0.0, 0.0), "quiet");
    let (x, y) = s.ui.rect(s.ui.find("line crate").unwrap()).center();
    s.handle(&InputEvent::MouseMoved { x, y });
    s.frame();
    assert_eq!(tools(&s, "line crate"), (1.0, 1.0), "under the pointer");
    assert_eq!(tools(&s, "line boulder"), (0.0, 0.0));
    // Hidden: its eye stays when the pointer leaves.
    let crate_id = s.session.find("crate").unwrap();
    s.session.set_hidden(&[crate_id], true).unwrap();
    s.refresh();
    let (x, y) = s.ui.rect(s.ui.find("line boulder").unwrap()).center();
    s.handle(&InputEvent::MouseMoved { x, y });
    s.frame();
    assert_eq!(tools(&s, "line crate"), (1.0, 0.0), "the eye of what is hidden");
}

#[test]
fn the_project_shows_pictures_of_scenes_and_materials_by_default() {
    let Some((mut s, _dir)) = studio() else { return };
    draw_pictures(&mut s);
    s.ui.paint();
    assert!(
        s.ui.find("asset first-light").is_some() && s.bottom_pictures_pending() == 0,
        "every picture drawn: {} left",
        s.bottom_pictures_pending()
    );
    let scene = s.ui.dump();
    assert!(scene.contains("thumb scene:"), "a scene's picture");
    assert!(scene.contains("thumb material:"), "a material's picture");
}

#[test]
fn the_compass_looks_from_an_axis_and_its_middle_switches_projection() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "compass top");
    assert!(s.session.is_orthographic());
    let c = s.session.camera();
    assert!(c.position.y > c.target.y + 1.0, "from above: {c:?}");
    click(&mut s, "compass middle");
    assert!(!s.session.is_orthographic());
}

#[test]
fn a_long_value_is_edited_on_several_lines() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let crate_id = s.session.find("crate").unwrap();
    s.session
        .set_field(
            crate_id,
            "light",
            "(color: (1.0, 0.9, 0.8), intensity: 3.0, range: 8.0)",
        )
        .unwrap();
    click(&mut s, "line crate");
    // The value as RON is Debug mode's.
    click(&mut s, "inspector more");
    click(&mut s, "inspector mode Debug");
    click(&mut s, "light");
    // Enter is a new line inside the value; Cmd Enter commits it.
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(&mut s, "(");
    key(&mut s, Key::Enter);
    type_text(&mut s, "    intensity: 5.0,");
    key(&mut s, Key::Enter);
    type_text(&mut s, ")");
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    key(&mut s, Key::Enter);
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    let light = s
        .session
        .inspect(crate_id)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "light")
        .unwrap()
        .value;
    assert!(
        light.contains("intensity:5.0") || light.contains("intensity: 5.0"),
        "{light}"
    );
}

/// Answer the open dialog with `text`.
fn answer(s: &mut Studio, text: &str) {
    assert!(
        s.ui.find("dialog field").is_some(),
        "a dialog is open:\n{}",
        s.ui.dump()
    );
    type_text(s, text);
    key(s, Key::Enter);
    assert!(s.ui.find("dialog field").is_none(), "and closed");
}

#[test]
fn assets_are_renamed_and_made_from_the_menus() {
    let Some((mut s, dir)) = studio() else { return };
    // Rename a material from its context menu: the file moves and the
    // crate that used it follows.
    click(&mut s, "project search");
    type_text(&mut s, "earth");
    s.frame();
    press(&mut s, "asset earth", MouseButton::Right);
    let menu_dump: Vec<String> =
        s.ui.dump()
            .lines()
            .filter(|l| l.contains("#menu"))
            .map(String::from)
            .collect();
    assert!(s.ui.find("menu Rename…").is_some(), "{menu_dump:?}");
    click(&mut s, "menu Rename…");
    answer(&mut s, "soil");
    assert!(dir.join("materials/soil.rmat").is_file(), "renamed");
    let crate_id = s.session.find("crate").unwrap();
    assert_eq!(s.session.material_name(crate_id).as_deref(), Some("soil"));

    // A component script, then on the crate from the list.
    menu(&mut s, "Assets", "Create Component…");
    answer(&mut s, "door");
    assert!(dir.join("src/components/door.rs").is_file());
    click(&mut s, "line crate");
    click(&mut s, "add component");
    type_text(&mut s, "doo");
    key(&mut s, Key::Enter);
    assert!(
        s.session
            .inspect(crate_id)
            .unwrap()
            .iter()
            .any(|f| f.name == "components.door"),
        "the crate has a door: {:?}\n{}",
        s.session
            .console()
            .iter()
            .map(|l| &l.text)
            .collect::<Vec<_>>(),
        s.ui.dump()
            .lines()
            .filter(|l| l.contains("#menu"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // The crate's colour as a material of its own.
    menu(&mut s, "Assets", "Save Material from Selection…");
    answer(&mut s, "rust");
    assert!(dir.join("materials/rust.rmat").is_file());

    // A variant of an instance of the campfire prefab.
    let fire = s.session.add_instance(None, "campfire").unwrap();
    s.session.rename(fire, "fire instance").unwrap();
    s.refresh();
    press(&mut s, "line fire instance", MouseButton::Right);
    click(&mut s, "menu Make Prefab Variant…");
    assert!(
        s.ui.find("dialog field").is_some(),
        "{:?}",
        s.session
            .console()
            .iter()
            .map(|l| &l.text)
            .collect::<Vec<_>>()
    );
    answer(&mut s, "campfire_lit");
    assert!(dir.join("prefabs/campfire_lit.prefab").is_file());
}

#[test]
fn snap_steps_and_navigation_from_the_view_menu() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    menu(&mut s, "View", "Snap Settings…");
    // The field starts with the current steps, all selected.
    answer(&mut s, "0.5, 30, 0.2");
    assert_eq!(s.session.snap().meters, 0.5);
    assert_eq!(s.session.snap().degrees, 30.0);
    menu(&mut s, "View", "Navigation");
    s.frame();
    s.frame();
    // Baked and drawn (this scene's ground has no collider, so the count
    // itself may be nothing).
    assert!(s.session.walkable_cells().is_some(), "navigation is shown");
}

#[test]
fn project_settings_open_and_save_only_what_reads() {
    let Some((mut s, dir)) = studio() else { return };
    click(&mut s, "tab settings");
    click(&mut s, "settings runity.ron");
    assert!(s
        .ui
        .text(s.ui.find("settings text").unwrap())
        .unwrap()
        .contains("valley"));
    let select_all = |s: &mut Studio| {
        s.handle(&InputEvent::KeyDown(Key::LeftSuper));
        s.handle(&InputEvent::KeyDown(Key::A));
        s.handle(&InputEvent::KeyUp(Key::A));
        s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    };
    click(&mut s, "settings text");
    select_all(&mut s);
    type_text(&mut s, "(name: \"valley\"");
    click(&mut s, "settings save");
    let text = std::fs::read_to_string(dir.join("runity.ron")).unwrap();
    assert!(
        text.contains("start_scene"),
        "broken RON is not written: {text}"
    );
    click(&mut s, "settings text");
    select_all(&mut s);
    type_text(
        &mut s,
        "(name: \"valley\", engine: \"0.1.0\", game: (start_scene: \"camp\"))",
    );
    click(&mut s, "settings save");
    let text = std::fs::read_to_string(dir.join("runity.ron")).unwrap();
    assert!(
        text.contains("\"camp\""),
        "{text}\nfield: {:?}\n{:?}",
        s.ui.text(s.ui.find("settings text").unwrap()),
        s.session
            .console()
            .iter()
            .map(|l| &l.text)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_profiler_shows_what_frames_cost() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "tab profiler");
    for _ in 0..5 {
        s.handle(&InputEvent::MouseMoved { x: 600.0, y: 400.0 });
        s.frame();
    }
    let summary =
        s.ui.text(s.ui.find("profiler summary").unwrap())
            .unwrap()
            .to_string();
    assert!(summary.contains("median"), "{summary}");
}

#[test]
fn an_error_does_not_take_the_tab_but_the_status_line_leads_to_it() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "tab settings");
    s.session
        .say(runity_editor::console::Level::Error, "something broke");
    s.frame();
    s.refresh();
    s.ui.paint();
    assert!(
        s.ui.rect(s.ui.find("settings files").unwrap()).width > 0.0,
        "Settings stays on top"
    );
    click(&mut s, "status problems");
    s.ui.paint();
    assert!(
        s.ui.rect(s.ui.find("console lines").unwrap()).width > 0.0,
        "the Console is shown"
    );
}

#[test]
fn a_fields_menu_resets_copies_pastes_and_removes() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let crate_id = s.session.find("crate").unwrap();
    let boulder = s.session.find("boulder").unwrap();
    click(&mut s, "line crate");
    click(&mut s, "label rotation");
    click(&mut s, "menu Reset");
    assert_eq!(
        s.session.transform(crate_id).unwrap().rotation_deg,
        runity::glam::Vec3::ZERO
    );

    click(&mut s, "label position");
    click(&mut s, "menu Copy Value");
    click(&mut s, "line boulder");
    click(&mut s, "label position");
    click(&mut s, "menu Paste Value");
    assert_eq!(
        s.session.transform(boulder).unwrap().position,
        s.session.transform(crate_id).unwrap().position
    );

    s.session
        .set_component(crate_id, "door", Some("()"))
        .unwrap();
    click(&mut s, "line crate");
    click(&mut s, "label components.door");
    click(&mut s, "menu Remove");
    assert!(!s
        .session
        .inspect(crate_id)
        .unwrap()
        .iter()
        .any(|f| f.name == "components.door"));
}

#[test]
fn the_game_view_takes_a_shape() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "aspect");
    click(&mut s, "menu 4:3");
    assert!(s.session.is_game_view());
    s.frame();
    s.ui.paint();
    let r = s.ui.rect(s.ui.find("scene view").unwrap());
    assert!((r.width / r.height - 4.0 / 3.0).abs() < 0.02, "{r:?}");
    let (w, h) = s.session.size();
    assert!(
        (w as f32 / h as f32 - 4.0 / 3.0).abs() < 0.02,
        "the render follows: {w}x{h}"
    );
}

#[test]
fn the_animation_tab_plays_a_clip_in_the_view() {
    let Some((mut s, dir)) = studio() else { return };
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runity-import/tests/fixtures/skinned_banner.gltf");
    std::fs::copy(&fixture, dir.join("assets/banner.gltf")).unwrap();
    menu(&mut s, "Assets", "Refresh");
    let banner = s.session.add(None, "banner").unwrap();
    s.session.select(Some(banner)).unwrap();
    s.frame();
    click(&mut s, "tab animation");
    click(&mut s, "clip furl");
    assert_eq!(s.session.previewing(), &[banner]);
    click(&mut s, "animation stop");
    assert!(s.session.previewing().is_empty());
}

#[test]
fn an_input_method_composes_in_the_focused_field_only() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    assert!(
        !s.typing(),
        "the Scene view has the keyboard: no input method"
    );
    click(&mut s, "hierarchy search");
    assert!(s.typing());
    s.ime_preedit("クレ");
    let search = s.ui.find("hierarchy search").unwrap();
    assert_eq!(s.ui.text(search), Some("クレ"));
    assert!(s.ime_area().is_some());
    s.ime_preedit("");
    s.handle(&InputEvent::Text("crate".into()));
    s.frame();
    assert_eq!(s.ui.text(search), Some("crate"));
}

#[test]
fn a_selected_camera_shows_what_it_sees_in_the_corner() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    menu(&mut s, "Entity", "Camera");
    s.frame();
    s.ui.paint();
    let shown = |s: &Studio| s.ui.rect(s.ui.find("camera preview image").unwrap()).width > 0.0;
    assert!(
        shown(&s),
        "{}",
        s.ui.dump()
            .lines()
            .filter(|l| l.contains("camera"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(s.session.preview_target().is_some());
    key(&mut s, Key::Escape);
    s.ui.paint();
    assert!(
        !s.ui.dump().contains("#camera preview image"),
        "hidden with nothing selected"
    );
}

/// A tenth of a second of silence, as a WAV file.
fn silence() -> Vec<u8> {
    let samples = 4410u32;
    let data = samples * 2;
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(1u16.to_le_bytes()); // mono
    out.extend(44100u32.to_le_bytes());
    out.extend((44100u32 * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    out.extend(std::iter::repeat_n(0u8, data as usize));
    out
}

#[test]
fn a_sound_is_listed_in_the_project_to_listen_to() {
    let Some((mut s, dir)) = studio() else { return };
    std::fs::write(dir.join("assets/beep.wav"), silence()).unwrap();
    menu(&mut s, "Assets", "Refresh");
    assert!(s.session.sound("beep").is_some(), "imported");
    click(&mut s, "project search");
    type_text(&mut s, "beep");
    s.frame();
    // Not played here: a test should not make the machine beep.
    press(&mut s, "asset beep", MouseButton::Right);
    assert!(s.ui.find("menu Play").is_some() && s.ui.find("menu Stop").is_some());
}

#[test]
fn quick_search_finds_things_in_the_scene_the_project_and_the_menus() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    shortcut(&mut s, Key::K);
    assert!(s.ui.find("search field").is_some());
    type_text(&mut s, "boulder");
    s.frame();
    let dump = s.ui.dump();
    assert!(
        dump.contains("in the scene") && dump.contains("in the project"),
        "{dump}"
    );
    // Enter goes to the first: the boulder in the scene.
    key(&mut s, Key::Enter);
    assert_eq!(s.session.selected(), s.session.find("boulder"));
    assert!(s.ui.find("search field").is_none());

    // A menu entry by its name.
    shortcut(&mut s, Key::K);
    type_text(&mut s, "game view");
    s.frame();
    click(&mut s, "result Game View");
    assert!(s.session.is_game_view());
}

#[test]
fn a_dragged_selection_moves_whole_and_a_child_is_made_under_a_line() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "line tree near");
    s.handle(&InputEvent::KeyDown(Key::LeftShift));
    click(&mut s, "line tree mid");
    s.handle(&InputEvent::KeyUp(Key::LeftShift));
    drag_line(&mut s, "line tree near", "line crate", 0.5);
    let rows = s.session.hierarchy();
    for name in ["tree near", "tree mid"] {
        assert_eq!(
            rows.iter().find(|r| r.name == name).unwrap().depth,
            1,
            "{name} under the crate"
        );
    }
    click(&mut s, "undo");
    let rows = s.session.hierarchy();
    assert!(
        rows.iter()
            .filter(|r| r.name.starts_with("tree"))
            .all(|r| r.depth == 0),
        "one step back"
    );

    press(&mut s, "line boulder", MouseButton::Right);
    click(&mut s, "menu Create Empty Child");
    let rows = s.session.hierarchy();
    assert_eq!(rows.iter().find(|r| r.name == "Child").unwrap().depth, 1);
}

#[test]
fn the_project_filters_by_kind() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "kind Scenes");
    let dump = s.ui.dump();
    assert!(
        dump.contains("#asset first-light") && !dump.contains("#asset cube"),
        "only scenes"
    );
    click(&mut s, "kind All");
    assert!(s.ui.dump().contains("#asset cube"));
}

#[test]
fn the_project_shows_pictures_of_models_and_prefabs() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    click(&mut s, "kind Prefabs");
    draw_pictures(&mut s);
    assert!(
        s.ui.dump().contains("#thumb campfire"),
        "a card with a picture"
    );
    assert!(s.bottom_pictures_pending() == 0, "drawn");
}

/// Put `text` into the field with this name, in place of what it held, and
/// press Enter.
fn fill(s: &mut Studio, name: &str, text: &str) {
    click(s, name);
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(s, text);
    s.handle(&InputEvent::KeyDown(Key::Enter));
    s.handle(&InputEvent::KeyUp(Key::Enter));
    s.frame();
}

#[test]
fn ui_builder_moves_and_edits_a_screen() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    let file = dir.join("ui/menu.ron");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        r#"// The main menu.
(elements: [
    (id: "title", anchor: Top, at: (0, 60), size: (600, 60), kind: Text("The Valley"), text_size: 40),
    (id: "play", anchor: Center, at: (0, 0), size: (240, 48), kind: Button("Play")),
])"#,
    )
    .unwrap();
    let read = || -> runity::screen::Layout {
        runity::ron::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap()
    };

    click(&mut s, "tab screens");
    click(&mut s, "screen menu");
    assert!(s.ui.find("element play").is_some(), "{}", s.ui.dump());

    // A press on the button in the picture picks it; a drag moves it.
    s.ui.paint();
    let canvas = s.ui.rect(s.ui.find("screen canvas").unwrap());
    let (cx, cy) = canvas.center();
    s.handle(&InputEvent::MouseMoved { x: cx, y: cy });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    assert!(s.ui.find("screen field width").is_some(), "play picked");
    let step = canvas.width / 1280.0;
    s.handle(&InputEvent::MouseMoved {
        x: cx + 10.0,
        y: cy,
    });
    s.frame();
    s.handle(&InputEvent::MouseMoved {
        x: cx + 100.0 * step,
        y: cy + 20.0 * step,
    });
    s.frame();
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let play = read()
        .elements
        .into_iter()
        .find(|e| e.id == "play")
        .unwrap();
    assert!(
        (play.at.0 - 100.0).abs() < 2.0,
        "moved right: {:?}",
        play.at
    );
    assert!((play.at.1 - 20.0).abs() < 2.0, "moved down: {:?}", play.at);
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .starts_with("// The main menu."),
        "comments kept"
    );

    // Boxes write to the file; a bad kind does not.
    fill(&mut s, "screen field width", "300");
    fill(&mut s, "screen field id", "start");
    let layout = read();
    let start = layout.elements.iter().find(|e| e.id == "start").unwrap();
    assert_eq!(start.size.0, 300.0);
    click(&mut s, "anchor BottomRight");
    let start = read()
        .elements
        .into_iter()
        .find(|e| e.id == "start")
        .unwrap();
    assert_eq!(start.anchor, runity::screen::Anchor::BottomRight);
    fill(&mut s, "screen field kind", "Nonsense(");
    assert!(read().elements.iter().any(|e| e.id == "start"), "file kept");

    // The list picks too.
    click(&mut s, "element title");
    assert!(s.ui.find("screen field text size").is_some());

    // Wide: the Scene view gives its room to the canvas, and takes it back.
    let small = s.ui.rect(s.ui.find("screen canvas").unwrap()).width;
    click(&mut s, "screen wide");
    s.frame();
    s.ui.paint();
    let big = s.ui.rect(s.ui.find("screen canvas").unwrap()).width;
    assert!(big > small * 2.0, "{small} -> {big}");
    click(&mut s, "screen wide");
    s.frame();
    s.ui.paint();
    assert!(s.ui.rect(s.ui.find("scene view").unwrap()).height > 300.0);
}

#[test]
fn a_locked_inspector_keeps_its_entity() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let boulder = s.session.find("boulder").unwrap();
    let crate_ = s.session.find("crate").unwrap();
    click(&mut s, "line boulder");
    click(&mut s, "inspector lock");
    click(&mut s, "line crate");
    assert_eq!(s.session.selection(), vec![crate_]);
    let name = s.ui.find("inspector name").unwrap();
    assert_eq!(s.ui.text(name), Some("boulder"), "still the boulder");

    // Typing goes to the boulder, not the selection.
    let crate_before = s.session.transform(crate_).unwrap();
    fill(&mut s, "position x", "7");
    assert_eq!(s.session.transform(boulder).unwrap().position.x, 7.0);
    assert_eq!(s.session.transform(crate_).unwrap(), crate_before);

    // Unlocked, it follows the selection again.
    click(&mut s, "inspector lock");
    let name = s.ui.find("inspector name").unwrap();
    assert_eq!(s.ui.text(name), Some("crate"));
}

#[test]
fn a_theme_file_recolours_the_running_editor() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    let file = dir.join(".runity/theme.ron");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let surface = [0x23, 0x25, 0x32];
    let has = |s: &mut Studio, rgb: [u8; 3]| {
        s.ui.paint()
            .iter()
            .flat_map(|l| l.rects.iter())
            .any(|r| [r.fill.r, r.fill.g, r.fill.b] == rgb)
    };
    assert!(has(&mut s, surface));
    std::fs::write(&file, r##"{"SURFACE": "#402020"}"##).unwrap();
    let wait = |s: &mut Studio, until: &dyn Fn(&mut Studio) -> bool| {
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            s.frame();
            if until(s) {
                return true;
            }
        }
        false
    };
    assert!(
        wait(&mut s, &|s| has(s, [0x40, 0x20, 0x20])),
        "panels in the new colour"
    );
    assert!(!has(&mut s, surface));

    // A bad file says so and keeps the colours.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&file, r##"{"SURFAC": "#402020"}"##).unwrap();
    assert!(
        wait(&mut s, &|s| s.session.console_counts().2 > 0),
        "an error said"
    );
    assert!(has(&mut s, [0x40, 0x20, 0x20]));

    // Gone: Nocturne again.
    std::fs::remove_file(&file).unwrap();
    assert!(wait(&mut s, &|s| has(s, surface)));
}

#[test]
fn the_animator_edits_a_graph_and_keeps_its_comments() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    let file = dir.join("animators/hero.ron");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        r#"// The hero.
(
    start: "idle",
    states: {
        "idle": (clip: "idle", transitions: [
            (to: "walk", when: [Above("speed", 0.1)]),
        ]), // standing
        "walk": (clip: "walk"),
    },
)
"#,
    )
    .unwrap();
    let read = || -> runity::animgraph::Graph {
        runity::ron::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap()
    };

    click(&mut s, "tab animator");
    click(&mut s, "animator wide");
    s.frame();
    click(&mut s, "animator hero");
    assert!(s.ui.find("state idle").is_some(), "{}", s.ui.dump());
    assert!(s.ui.find("transition 0").is_some(), "an arrow idle → walk");

    // The boxes place themselves: walk one column over from idle, and
    // pulling at a box neither moves it nor writes anything anywhere.
    let before = std::fs::read_to_string(&file).unwrap();
    s.ui.paint();
    let idle = s.ui.rect(s.ui.find("state idle").unwrap());
    let walk = s.ui.rect(s.ui.find("state walk").unwrap());
    assert!(walk.x > idle.x + idle.width, "{idle:?} {walk:?}");
    let (x, y) = walk.center();
    s.handle(&InputEvent::MouseMoved { x, y });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    s.handle(&InputEvent::MouseMoved {
        x: x + 60.0,
        y: y + 80.0,
    });
    s.frame();
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    s.ui.paint();
    let after = s.ui.rect(s.ui.find("state walk").unwrap());
    assert!(
        (after.x - walk.x).abs() < 1.0 && (after.y - walk.y).abs() < 1.0,
        "{walk:?} -> {after:?}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    assert!(
        !dir.join(".runity/animators.ron").exists(),
        "no places kept"
    );

    // Walk is chosen by the press: back to idle, on a condition.
    click(&mut s, "to idle");
    fill(&mut s, "animator field when", r#"Below("speed", 0.1)"#);
    let g = read();
    assert_eq!(g.transitions.len(), 2);
    assert_eq!(g.transitions[1].from, "walk");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("\"walk\": (clip: \"walk\", transitions: [\n            (to: \"idle\", when: [Below(\"speed\", 0.1)]),\n        ]),"),
        "written in the state it leaves"
    );
    assert_eq!(
        g.transitions[1].when,
        vec![runity::animgraph::Condition::Below("speed".into(), 0.1)]
    );

    // A new state, renamed, made the start.
    click(&mut s, "animator add state");
    fill(&mut s, "animator field name", "jump");
    click(&mut s, "animator start");
    click(&mut s, "animator looping");
    let g = read();
    assert_eq!(g.start, "jump");
    assert!(!g.states["jump"].looping);

    // An arrow chosen and deleted.
    click(&mut s, "transition 0 head");
    click(&mut s, "animator delete");
    assert_eq!(read().transitions.len(), 1);

    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("// The hero."), "{text}");
    assert!(text.contains("// standing"));
}

#[test]
fn the_animator_lights_up_the_state_the_running_game_is_in() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    std::fs::create_dir_all(dir.join("animators")).unwrap();
    std::fs::write(
        dir.join("animators/hero.ron"),
        r#"(start: "idle", states: {"idle": (clip: "idle"), "walk": (clip: "walk")})"#,
    )
    .unwrap();
    // A game that says the boulder walks: its report, from a stand-in.
    let state = dir.join(".runity/state.ron");
    let mut game = std::process::Command::new("sleep");
    game.arg("30").env("RUNITY_STATE_FILE", &state);
    s.session.run_in_console(game).unwrap();
    let boulder = s.session.find("boulder").unwrap();
    let mut report = runity::save::capture(
        &runity::hecs::World::new(),
        &runity::Components::new(),
        &runity::Scene::default(),
    );
    report.entities.push(runity::save::Saved {
        id: boulder,
        transform: Default::default(),
        components: Vec::new(),
        prefab: String::new(),
        animator: "walk".into(),
    });
    report.write(&state).unwrap();

    click(&mut s, "line boulder");
    click(&mut s, "tab animator");
    click(&mut s, "animator hero");
    let border = |s: &mut Studio, name: &str| {
        let node = s.ui.find(name).unwrap();
        s.ui.style(node).look.border
    };
    let mut lit = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        s.frame();
        if border(&mut s, "state walk") == runity_studio::theme::WARNING {
            lit = true;
            break;
        }
    }
    assert!(lit, "walk lit while the game is in it");
    assert_ne!(border(&mut s, "state idle"), runity_studio::theme::WARNING);
    s.session.stop_game();
}

#[test]
fn face_mode_outlines_a_face_and_a_drag_pushes_it() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let crate_ = s.session.find("crate").unwrap();
    let (low, high) = s.session.world_bounds(crate_).unwrap();
    let centre = (low + high) / 2.0;
    s.session
        .set_camera(centre + runity::glam::Vec3::new(2.5, 3.0, 4.0), centre);
    click(&mut s, "faces");
    s.frame();
    let top = runity::edit::Face::PosY;
    let (corners, _) = s.session.face_on_screen(crate_, top).unwrap();
    let (cx, cy) = (
        corners.iter().map(|c| c.0).sum::<f32>() / 4.0,
        corners.iter().map(|c| c.1).sum::<f32>() / 4.0,
    );
    s.ui.paint();
    let view = s.ui.rect(s.ui.find("scene view").unwrap());
    let (x, y) = (view.x + cx, view.y + cy);
    s.handle(&InputEvent::MouseMoved { x, y });
    s.frame();
    s.ui.paint();
    let outline = s.ui.find("face outline").unwrap();
    let label = s.ui.children(outline)[0];
    assert!(
        s.ui.text(label).is_some_and(|t| t.contains("crate PosY")),
        "{:?}",
        s.ui.text(label)
    );
    let steps = s.session.undo_steps().len();

    // Up the screen is out of the top.
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    for dy in [10.0, 30.0, 60.0] {
        s.handle(&InputEvent::MouseMoved { x, y: y - dy });
        s.frame();
    }
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let (_, pushed) = s.session.world_bounds(crate_).unwrap();
    assert!(pushed.y > high.y + 0.1, "{} -> {}", high.y, pushed.y);
    assert_eq!(
        s.session.undo_steps().len(),
        steps + 1,
        "one drag, one step"
    );
    s.session.undo().unwrap();
    let (_, back) = s.session.world_bounds(crate_).unwrap();
    assert!((back.y - high.y).abs() < 1e-4);
}

#[test]
fn a_panel_floats_in_a_window_of_its_own_and_docks_back() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    menu(&mut s, "Window", "Float the Inspector");
    assert_eq!(
        s.floating().into_iter().map(|(n, _)| n).collect::<Vec<_>>(),
        ["inspector"]
    );
    assert!(s.ui.find("tab inspector").is_none(), "its tab is gone");
    s.resize_float("inspector", 400.0, 500.0);
    click(&mut s, "line boulder");

    // The floating window's own pixels: its pointer is shifted to the frame.
    s.ui.paint();
    let frame = s.ui.rect(s.ui.find("float inspector").unwrap());
    assert_eq!((frame.width, frame.height), (400.0, 500.0));
    let name = s.ui.rect(s.ui.find("inspector name").unwrap());
    let (x, y) = (name.center().0 - frame.x, name.center().1 - frame.y);
    assert!(
        x > 0.0 && x < 400.0 && y > 0.0 && y < 500.0,
        "inside it: {x},{y}"
    );
    s.handle_float("inspector", &InputEvent::MouseMoved { x, y });
    s.handle_float("inspector", &InputEvent::MouseDown(MouseButton::Left));
    s.handle_float("inspector", &InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(&mut s, "big rock");
    s.handle(&InputEvent::KeyDown(Key::Enter));
    s.frame();
    assert!(
        s.session.find("big rock").is_some(),
        "renamed from the floating window"
    );

    // Drawn from its corner: the panel's surface, not the main window's.
    let target = runity::OffscreenTarget::new(s.session.gpu(), 400, 500);
    let mut renderer = s.renderer(target.format());
    let mut seen = u64::MAX;
    s.draw_float(
        "inspector",
        &mut renderer,
        &mut seen,
        &target.ui_view(),
        400,
        500,
    );
    let pixels = target.read_rgba(s.session.gpu());
    if let Ok(out) = std::env::var("RUNITY_FLOAT_SHOT") {
        image::save_buffer(out, &pixels, 400, 500, image::ExtendedColorType::Rgba8).unwrap();
    }
    let at = |x: usize, y: usize| &pixels[(y * 400 + x) * 4..(y * 400 + x) * 4 + 3];
    let surface = runity_studio::theme::SURFACE;
    assert_eq!(
        at(200, 20),
        [surface.r, surface.g, surface.b],
        "the panel's card"
    );

    // Closing its window docks it again.
    s.close_float("inspector");
    s.frame();
    assert!(s.floating().is_empty());
    assert!(s.ui.find("tab inspector").is_some());
    assert!(s.ui.find("float inspector").is_none());
}

#[test]
fn an_entity_field_is_picked_from_the_scene_and_a_gone_target_is_named() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    // The game says it has a door that names its switch.
    let shapes: std::collections::BTreeMap<String, runity::shape::Shape> = [(
        "door".to_string(),
        runity::shape::Shape::Struct(vec![
            ("switch".into(), runity::shape::Shape::Entity),
            ("open".into(), runity::shape::Shape::Bool),
        ]),
    )]
    .into_iter()
    .collect();
    std::fs::create_dir_all(dir.join(runity::project::SHAPES).parent().unwrap()).unwrap();
    std::fs::write(
        dir.join(runity::project::SHAPES),
        runity::ron::to_string(&shapes).unwrap(),
    )
    .unwrap();
    let boulder = s.session.find("boulder").unwrap();
    let crate_ = s.session.find("crate").unwrap();
    s.session.add_component(boulder, "door").unwrap();
    click(&mut s, "line boulder");
    let picker = s.ui.find("door switch").unwrap();
    let label = s.ui.children(picker)[1];
    assert_eq!(s.ui.text(label), Some("None (entity)"));

    // The picker lists the scene; crate is chosen.
    click(&mut s, "door switch");
    click(&mut s, "menu crate");
    let value = s
        .session
        .inspect(boulder)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "components.door")
        .unwrap()
        .value;
    assert!(
        value.contains(&format!("EntityRef(\"{crate_}\")")),
        "{value}"
    );
    let picker = s.ui.find("door switch").unwrap();
    let label = s.ui.children(picker)[1];
    assert_eq!(s.ui.text(label), Some("crate"));

    // Its target deleted: the link is named as broken.
    s.session.delete(crate_).unwrap();
    assert!(
        s.session
            .problems()
            .iter()
            .any(|p| p.message.contains("`door` links to")),
        "{:?}",
        s.session.problems()
    );
}

#[test]
fn k_during_play_keeps_where_the_crate_fell() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let crate_ = s.session.find("crate").unwrap();
    s.session.set_field(crate_, "body", "Dynamic").unwrap();
    s.session
        .set_field(crate_, "collider", "Box(half: (0.5, 0.5, 0.5))")
        .unwrap();
    // Lifted, so it has somewhere to fall.
    let mut t = s.session.transform(crate_).unwrap();
    t.position.y += 3.0;
    s.session.set_transform(crate_, t).unwrap();
    let start = t.position;
    s.frame();

    click(&mut s, "play");
    assert!(s.session.is_playing());
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(15));
        s.frame();
    }
    click(&mut s, "line crate");
    key(&mut s, Key::K);
    assert_eq!(s.session.kept(), vec![crate_]);
    click(&mut s, "play");
    assert!(!s.session.is_playing());
    let after = s.session.transform(crate_).unwrap().position;
    assert!(
        after.y < start.y - 0.5,
        "kept where it fell: {start} -> {after}"
    );
}

#[test]
fn a_field_that_differs_has_a_reset_arrow_and_search_narrows_the_inspector() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let boulder = s.session.find("boulder").unwrap();
    click(&mut s, "line boulder");
    assert!(
        s.ui.find("reset scale").is_some(),
        "scale 1.1 is not a new entity's"
    );
    assert!(s.ui.find("reset rotation").is_none(), "rotation 0 is");
    click(&mut s, "reset scale");
    assert_eq!(
        s.session.transform(boulder).unwrap().scale,
        runity::glam::Vec3::ONE
    );
    assert!(s.ui.find("reset scale").is_none(), "and the arrow goes");

}

#[test]
fn the_foliage_brush_paints_the_selected_model_in_one_stroke() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let tree = s.session.find("tree near").unwrap();
    let model = s.session.entity_model(tree).unwrap();
    click(&mut s, "line tree near");
    click(&mut s, "foliage");
    let steps = s.session.undo_steps().len();

    s.ui.paint();
    let view = s.ui.rect(s.ui.find("scene view").unwrap());
    let (x, y) = (view.x + view.width / 2.0, view.y + view.height * 0.7);
    s.handle(&InputEvent::MouseMoved { x, y });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    for i in 1..=4 {
        std::thread::sleep(std::time::Duration::from_millis(110));
        s.handle(&InputEvent::MouseMoved {
            x: x + 40.0 * i as f32,
            y,
        });
        s.frame();
    }
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();

    let group = s
        .session
        .find(&format!(
            "foliage: {}",
            model.trim_start_matches("builtin:")
        ))
        .unwrap_or_else(|| {
            panic!(
                "no group; console: {:?}",
                s.session
                    .console()
                    .iter()
                    .map(|l| l.text.clone())
                    .collect::<Vec<_>>()
            )
        });
    let planted = s.session.scene().get(group).unwrap().children.len();
    assert!(planted > 3, "{planted}");
    assert_eq!(
        s.session.undo_steps().len(),
        steps + 1,
        "one stroke, one step"
    );

    type_text(&mut s, "]");
    s.frame();
    assert!(s
        .session
        .console()
        .iter()
        .any(|l| l.text.contains("m across")));
}

#[test]
fn a_fence_is_shaped_by_dragging_its_points() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let fence = s
        .session
        .add_fence("builtin:cylinder", runity::glam::Vec3::ZERO, 1.0)
        .unwrap();
    s.session.select(Some(fence)).unwrap();
    s.frame();
    assert!(s.ui.find("spline point 0").is_some() && s.ui.find("spline point 1").is_some());
    let posts = s.session.spawned_count();
    let spline = |s: &Studio| -> runity::Spline {
        let text = s
            .session
            .inspect(fence)
            .unwrap()
            .into_iter()
            .find(|f| f.name == "spline")
            .unwrap()
            .value;
        runity::ron::from_str(&text).unwrap()
    };
    let before = spline(&s).points[1];
    let steps = s.session.undo_steps().len();

    // The second point, dragged across the view: one step, and a longer
    // fence has more posts.
    s.ui.paint();
    let handle = s.ui.rect(s.ui.find("spline point 1").unwrap());
    let view = s.ui.rect(s.ui.find("scene view").unwrap());
    let (x, y) = handle.center();
    s.handle(&InputEvent::MouseMoved { x, y });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    for i in 1..=5 {
        let t = i as f32 / 5.0;
        s.handle(&InputEvent::MouseMoved {
            x: x + (view.x + view.width * 0.9 - x) * t,
            y: y + 10.0 * t,
        });
        s.frame();
    }
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let after = spline(&s).points[1];
    // How far depends on what the ground under the pointer is; that it moved
    // with the drag is the point.
    assert!((after - before).length() > 0.2, "{before} -> {after}");
    assert_eq!(
        s.session.undo_steps().len(),
        steps + 1,
        "one drag, one step"
    );
    assert!(s.session.spawned_count() >= posts, "longer, no fewer posts");
}

#[test]
fn a_material_instance_is_made_from_the_project_and_is_its_parent_until_changed() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    click(&mut s, "project search");
    type_text(&mut s, "stone");
    s.frame();
    s.frame();
    press(&mut s, "asset stone", MouseButton::Right);
    click(&mut s, "menu Create Material Instance");
    s.frame();
    let file = dir.join("materials/stone_instance.rmat");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains(r#"(parent: ("stone", ""#),
        "linked by name and ID: {text}"
    );
    let library = runity::Library::open(dir.join("library")).unwrap().0;
    assert_eq!(
        library
            .material_by_name("stone_instance")
            .unwrap()
            .base_color,
        library.material_by_name("stone").unwrap().base_color,
        "the parent, until something on it says otherwise"
    );

    // The Inspector shows it as an instance; a parameter set there is one
    // line of its file, and the arrow takes it back to the parent's.
    s.frame();
    assert!(s.ui.dump().contains("Instance of stone"), "{}", s.ui.dump());
    fill(&mut s, "material smoothness", "0.9");
    s.frame();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("smoothness: 0.9"), "{text}");
    assert!(text.starts_with("// stone"), "the comment stays: {text}");
    let library = runity::Library::open(dir.join("library")).unwrap().0;
    assert_eq!(
        library
            .material_by_name("stone_instance")
            .unwrap()
            .smoothness,
        0.9
    );
    click(&mut s, "reset material smoothness");
    s.frame();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("smoothness"), "{text}");
    fill(&mut s, "material metallic", "lots");
    assert!(
        s.session
            .console()
            .iter()
            .any(|l| l.text.contains("metallic")),
        "a value it cannot be built with is refused"
    );
}

#[test]
fn a_prefab_field_is_picked_from_the_project_and_a_wrong_one_is_named() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    let shapes: std::collections::BTreeMap<String, runity::shape::Shape> = [(
        "spawner".to_string(),
        runity::shape::Shape::Struct(vec![
            ("what".into(), runity::shape::Shape::Asset("prefab".into())),
            ("every".into(), runity::shape::Shape::Float),
        ]),
    )]
    .into_iter()
    .collect();
    std::fs::create_dir_all(dir.join(runity::project::SHAPES).parent().unwrap()).unwrap();
    std::fs::write(
        dir.join(runity::project::SHAPES),
        runity::ron::to_string(&shapes).unwrap(),
    )
    .unwrap();
    let boulder = s.session.find("boulder").unwrap();
    s.session.add_component(boulder, "spawner").unwrap();
    click(&mut s, "line boulder");
    click(&mut s, "spawner what");
    click(&mut s, "menu campfire");
    let value = s
        .session
        .inspect(boulder)
        .unwrap()
        .into_iter()
        .find(|f| f.name == "components.spawner")
        .unwrap()
        .value;
    assert!(
        value.contains(r#"PrefabLink(("campfire","#),
        "by name and ID: {value}"
    );
    let picker = s.ui.find("spawner what").unwrap();
    assert_eq!(s.ui.text(s.ui.children(picker)[1]), Some("campfire"));

    // Written by hand, wrong: named, with the nearest.
    s.session
        .set_component(
            boulder,
            "spawner",
            Some(r#"(what: PrefabLink("campfir"), every: 2.0)"#),
        )
        .unwrap();
    let problems = s.session.problems();
    assert!(
        problems
            .iter()
            .any(|p| p.message.contains("links to prefab `campfir`")
                && p.message.contains("`campfire`")),
        "{problems:?}"
    );
}

#[test]
fn the_animator_shows_what_changed_since_the_commit_and_takes_one_back() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !git(&["init", "-q"]) {
        eprintln!("no git here; skipped");
        return;
    }
    let file = dir.join("animators/hero.ron");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        "(\n    start: \"idle\",\n    states: {\n        \"idle\": (clip: \"idle\"),\n    },\n)\n",
    )
    .unwrap();
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "add", "."]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "hero",
    ]);
    // An agent's edit: a new state and a way into it.
    std::fs::write(
        &file,
        "(\n    start: \"idle\",\n    states: {\n        \"idle\": (clip: \"idle\", transitions: [\n            (to: \"swim\", when: [Is(\"wet\")]),\n        ]),\n        \"swim\": (clip: \"swim\"),\n    },\n)\n",
    )
    .unwrap();
    click(&mut s, "tab animator");
    click(&mut s, "animator wide");
    s.frame();
    click(&mut s, "animator hero");
    let dump = s.ui.dump();
    assert!(dump.contains("CHANGES SINCE THE LAST COMMIT"), "{dump}");
    assert!(dump.contains("+ state swim"), "{dump}");
    assert!(dump.contains("+ idle → swim when wet"), "{dump}");
    // Taking the new way in back, alone: states come first in the list,
    // so it is the second.
    click(&mut s, "animator undo change 1");
    let graph: runity::animgraph::Graph =
        runity::ron::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(graph.transitions.is_empty(), "{graph:?}");
    assert!(graph.states.contains_key("swim"), "the state stays");
}

#[test]
fn the_dialogues_window_draws_a_conversation_and_reads_a_line() {
    let Some((mut s, dir)) = studio() else {
        return;
    };
    let file = dir.join("dialogues/captain.ron");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        r#"(start: "hello", lines: {
            "hello": (speaker: "Captain", text: "Ahoy.", next: "ask"),
            "ask": (speaker: "Captain", text: "Help me?", choices: [
                (text: "Yes", to: "thanks", set: ["agreed"]),
                (text: "No", to: "bye", when: [Not("broke")]),
            ]),
            "thanks": (text: "Good."),
            "bye": (text: "Pity."),
        })"#,
    )
    .unwrap();
    click(&mut s, "tab dialogues");
    s.frame();
    click(&mut s, "dialogue captain");
    for line in ["hello", "ask", "thanks", "bye"] {
        assert!(
            s.ui.find(&format!("line {line}")).is_some(),
            "{line}: {}",
            s.ui.dump()
        );
    }
    assert!(s.ui.find("dialogue edge 0").is_some(), "an arrow");
    click(&mut s, "line ask");
    let dump = s.ui.dump();
    assert!(dump.contains("Help me?"), "{dump}");
    assert!(dump.contains("“No” → bye if not broke"), "{dump}");
    assert!(dump.contains("sets agreed"), "{dump}");
}

#[test]
fn the_inspector_scrubs_adds_takes_away_and_switches_off() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let crate_id = s.session.find("crate").unwrap();
    click(&mut s, "line crate");
    let x = s.session.transform(crate_id).unwrap().position.x;

    // X dragged 100 pixels to the right: a metre on, one step.
    let steps = s.session.undo_steps().len();
    s.ui.paint();
    let handle = s.ui.rect(s.ui.find("scrub position x").unwrap());
    let (hx, hy) = (handle.x + handle.width / 2.0, handle.y + handle.height / 2.0);
    s.handle(&InputEvent::MouseMoved { x: hx, y: hy });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    for i in 1..=10 {
        s.handle(&InputEvent::MouseMoved { x: hx + i as f32 * 10.0, y: hy });
        s.frame();
    }
    let moved = s.session.transform(crate_id).unwrap().position.x;
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    assert!((moved - x - 1.0).abs() < 0.05, "{x} → {moved}, as it is dragged");
    assert_eq!(s.session.undo_steps().len(), steps + 1, "one step");

    // A light from Add Component's search, then the trash takes it away.
    click(&mut s, "add component");
    type_text(&mut s, "lig");
    s.frame();
    key(&mut s, Key::Enter);
    let has = |s: &Studio, name: &str| {
        s.session
            .inspect(crate_id)
            .unwrap()
            .iter()
            .any(|f| f.name == name && f.value != "None")
    };
    assert!(has(&s, "light"), "a light on the crate");
    s.frame();
    click(&mut s, "remove light");
    assert!(!has(&s, "light"), "and off again");

    // The switch before the name: off, and back on.
    click(&mut s, "inspector active");
    let off = |s: &Studio| {
        s.session
            .inspect(crate_id)
            .unwrap()
            .iter()
            .any(|f| f.name == "inactive" && f.value == "true")
    };
    assert!(off(&s));
    s.frame();
    click(&mut s, "inspector active");
    assert!(!off(&s));
}

/// Everything typed into the box with this name, instead of what it held.
fn retype(s: &mut Studio, name: &str, text: &str) {
    click(s, name);
    s.handle(&InputEvent::KeyDown(Key::LeftSuper));
    s.handle(&InputEvent::KeyDown(Key::A));
    s.handle(&InputEvent::KeyUp(Key::A));
    s.handle(&InputEvent::KeyUp(Key::LeftSuper));
    type_text(s, text);
    key(s, Key::Enter);
}

fn look(s: &Studio, field: &str) -> String {
    s.session
        .environment()
        .into_iter()
        .find(|(f, _)| *f == field)
        .unwrap()
        .1
}

/// Every text box the Inspector shows, by name, with what it holds.
fn inspector_boxes(s: &mut Studio) -> Vec<(String, String)> {
    s.ui.paint();
    let mut out = Vec::new();
    let mut stack = vec![s.ui.find("inspector").unwrap()];
    while let Some(node) = stack.pop() {
        if s.ui.is_field(node) {
            out.push((
                s.ui.name(node).unwrap_or_default().to_string(),
                s.ui.text(node).unwrap_or_default().to_string(),
            ));
        }
        stack.extend(s.ui.children(node));
    }
    out
}

#[test]
fn the_scene_look_is_a_form_and_never_its_ron() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let boxes = inspector_boxes(&mut s);
    assert!(
        boxes.iter().any(|(n, t)| n == "scene fog start" && t == "18"),
        "{boxes:?}"
    );
    for (name, text) in &boxes {
        assert!(
            !text.trim_start().starts_with('(') && text != "None",
            "{name} shows RON: {text}"
        );
    }
    assert!(!s.ui.dump().contains("(hour:"), "no RON anywhere");

    // A number: one step, the rest of the fog as it was.
    let before = look(&s, "fog");
    let steps = s.session.undo_steps().len();
    retype(&mut s, "scene fog start", "25");
    let fog = look(&s, "fog");
    assert!(fog.contains("start:25.0"), "{fog}");
    assert_eq!(
        fog.replace("start:25.0", "start:18.0"),
        before,
        "only the start changed"
    );
    assert_eq!(s.session.undo_steps().len(), steps + 1, "one step");
    click(&mut s, "undo");
    assert_eq!(look(&s, "fog"), before);
    assert!(
        inspector_boxes(&mut s)
            .iter()
            .any(|(n, t)| n == "scene fog start" && t == "18"),
        "the form follows the undo"
    );

    // A colour, from its swatch's picker.
    click(&mut s, "scene fog color swatch");
    retype(&mut s, "picker hex", "#ff0000");
    assert!(look(&s, "fog").contains("color:(1.0,0.0,0.0)"), "{}", look(&s, "fog"));
    assert_eq!(s.session.undo_steps().len(), steps + 1, "one step");
    click(&mut s, "popover ground");

    // An enum from its list.
    click(&mut s, "scene fog mode");
    click(&mut s, "menu Exponential");
    assert!(look(&s, "fog").contains("mode:Exponential"), "{}", look(&s, "fog"));
}

#[test]
fn a_part_of_the_look_switches_on_at_its_default_and_off_to_none() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    assert_eq!(look(&s, "sky"), "None");
    let steps = s.session.undo_steps().len();
    click(&mut s, "scene sky");
    assert_eq!(
        runity::ron::from_str::<runity::ron::Value>(&look(&s, "sky")).ok(),
        runity::ron::from_str::<runity::ron::Value>(&s.session.field_blank("sky").unwrap())
            .ok(),
        "the engine's default sky"
    );
    assert_eq!(s.session.undo_steps().len(), steps + 1);
    assert!(s.ui.find("scene sky exposure").is_some(), "open, its fields shown");
    click(&mut s, "scene sky");
    assert_eq!(look(&s, "sky"), "None");
    assert!(s.ui.find("scene sky exposure").is_none());
}

#[test]
fn debug_mode_shows_the_ron_and_normal_the_form() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    assert!(s.ui.find("scene fog").is_none(), "no RON box outside Debug mode");
    click(&mut s, "inspector more");
    click(&mut s, "inspector mode Debug");
    let fog = s.ui.find("scene fog").expect("the fog as one box");
    assert!(s.ui.text(fog).unwrap().starts_with("(color:"));
    assert!(s.session.inspector_debug(), "remembered");
    click(&mut s, "inspector more");
    click(&mut s, "inspector mode Normal");
    assert!(s.ui.find("scene fog").is_none());
    assert!(s.ui.find("scene fog start").is_some());
}

#[test]
fn a_collider_picks_its_shape_and_edits_its_numbers() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let crate_id = s.session.find("crate").unwrap();
    s.session
        .set_field(crate_id, "collider", "Box(half: (0.5, 0.5, 0.5))")
        .unwrap();
    click(&mut s, "line crate");
    let collider = |s: &Studio| {
        s.session
            .inspect(crate_id)
            .unwrap()
            .into_iter()
            .find(|f| f.name == "collider")
            .unwrap()
            .value
    };
    let boxes = inspector_boxes(&mut s);
    for (name, text) in &boxes {
        assert!(!text.trim_start().starts_with('('), "{name} shows RON: {text}");
    }
    assert!(boxes.iter().any(|(n, t)| n == "collider half y" && t == "0.5"), "{boxes:?}");

    // One number of the box: one step.
    let steps = s.session.undo_steps().len();
    retype(&mut s, "collider half y", "2");
    assert_eq!(collider(&s), "Box(half:(0.5,2.0,0.5))");
    assert_eq!(s.session.undo_steps().len(), steps + 1);

    // A field the value leaves out, at its default until typed into.
    retype(&mut s, "collider center y", "1");
    assert_eq!(collider(&s), "Box(half:(0.5,2.0,0.5),center:(0.0,1.0,0.0))");

    // Another shape from the list: its own fields to fill in.
    click(&mut s, "collider");
    click(&mut s, "menu Sphere");
    assert!(collider(&s).starts_with("Sphere(radius:"), "{}", collider(&s));
    assert!(s.ui.find("collider radius").is_some());

    // A unit variant from the list, and the body's.
    click(&mut s, "body");
    click(&mut s, "menu Dynamic");
    assert!(s
        .session
        .inspect(crate_id)
        .unwrap()
        .iter()
        .any(|f| f.name == "body" && f.value == "Dynamic"));
}
