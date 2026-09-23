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
    let Some((mut s, _dir)) = studio() else { return };
    assert!(s.ui.dump().contains("\"Nothing selected\""));
}
