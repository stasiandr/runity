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

/// Click the node with this name, as a pointer would, and let a frame go.
fn click(studio: &mut Studio, name: &str) {
    press(studio, name, MouseButton::Left);
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
    click(&mut s, "menu bar GameObject");
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
    click(&mut s, "view top");
    assert!(s.session.is_orthographic());
    click(&mut s, "view persp");
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
    click(&mut s, "material hex");
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

    // Value to the far left: black, in one step.
    let steps = s.session.undo_steps().len();
    s.ui.paint();
    let track = s.ui.rect(s.ui.find("material value").unwrap());
    s.handle(&InputEvent::MouseMoved {
        x: track.x + 1.0,
        y: track.y + 3.0,
    });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    let m = s.session.material(crate_id).unwrap();
    assert!(m.base_color.iter().all(|c| *c < 0.01), "{m:?}");
    assert_eq!(s.session.undo_steps().len(), steps + 1);
}

#[test]
fn a_prefab_opens_from_the_project_and_back_returns_to_the_scene() {
    let Some((mut s, dir)) = studio() else { return };
    let scene = s.session.scene_path().unwrap().to_path_buf();
    // The first «asset campfire» is the prefab (prefabs come before models).
    click(&mut s, "asset campfire");
    click(&mut s, "asset campfire");
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
fn blockout_from_the_tools_menu() {
    let Some((mut s, _dir)) = studio() else {
        return;
    };
    let count = s.session.entity_count();
    menu(&mut s, "Tools", "Poly Shape: Floor");
    assert_eq!(s.session.entity_count(), count + 1, "a floor");
    let floor = s.session.selected().unwrap();
    let (lo, hi) = s.session.world_bounds(floor).unwrap();
    menu(&mut s, "Tools", "Push Top +0.5");
    let (_, hi2) = s.session.world_bounds(floor).unwrap();
    assert!((hi2.y - hi.y - 0.5).abs() < 0.05, "{hi:?} -> {hi2:?}");
    let _ = lo;
    menu(&mut s, "Tools", "Array: 4 copies along X");
    assert_eq!(s.session.entity_count(), count + 5);
    click(&mut s, "line crate");
    menu(&mut s, "Tools", "Scatter 20 around the view");
    assert!(
        s.session.entity_count() >= count + 5 + 20,
        "scattered crates"
    );
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
    menu(&mut s, "GameObject", "Terrain");
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
    click(&mut s, "pick component");
    click(&mut s, "menu door");
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
    menu(&mut s, "GameObject", "Camera");
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
    click(&mut s, "project pictures");
    for _ in 0..4 {
        s.frame();
    }
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
        "idle": (clip: "idle"), // standing
        "walk": (clip: "walk"),
    },
    transitions: [
        (from: "idle", to: "walk", when: [Above("speed", 0.1)]),
    ],
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

    // A box dragged moves, and the file does not change for it.
    let before = std::fs::read_to_string(&file).unwrap();
    s.ui.paint();
    let walk = s.ui.rect(s.ui.find("state walk").unwrap());
    let (x, y) = walk.center();
    s.handle(&InputEvent::MouseMoved { x, y });
    s.handle(&InputEvent::MouseDown(MouseButton::Left));
    s.frame();
    s.handle(&InputEvent::MouseMoved { x: x + 20.0, y });
    s.frame();
    s.handle(&InputEvent::MouseMoved {
        x: x + 60.0,
        y: y + 80.0,
    });
    s.frame();
    s.handle(&InputEvent::MouseUp(MouseButton::Left));
    s.frame();
    s.ui.paint();
    let moved = s.ui.rect(s.ui.find("state walk").unwrap());
    assert!(
        (moved.x - walk.x - 60.0).abs() < 2.0,
        "{walk:?} -> {moved:?}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    assert!(
        dir.join(".runity/animators.ron").exists(),
        "where boxes stand is kept"
    );

    // Walk is chosen by the press: back to idle, on a condition.
    click(&mut s, "to idle");
    fill(&mut s, "animator field when", r#"Below("speed", 0.1)"#);
    let g = read();
    assert_eq!(g.transitions.len(), 2);
    assert_eq!(g.transitions[1].from, "walk");
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
        r#"(start: "idle", states: {"idle": (clip: "idle"), "walk": (clip: "walk")}, transitions: [])"#,
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
