//! The tree without a GPU: layout, the text dump, input and the events it
//! turns into, and the promise a retained UI makes — nothing changes, nothing
//! is redone.

use scrap_core::input::{InputEvent, Key, MouseButton};
use scrap_ui::{Color, Event, Style, Ui};

const BG: Color = Color::hex(0x161826);
const SURFACE: Color = Color::hex(0x232532);

/// A toolbar with three buttons over a panel with a list — the editor's
/// shape in miniature.
fn editor_like() -> Ui {
    let mut ui = Ui::new();
    ui.set_viewport(400.0, 300.0, 2.0);
    let root = ui.root();
    ui.set_style(root, Style::column().full().background(BG));
    let bar = ui.add(
        root,
        Style::row()
            .height(40.0)
            .gap(4.0)
            .padding(4.0)
            .center_items(),
    );
    ui.set_name(bar, "toolbar");
    for name in ["move", "rotate", "scale"] {
        let b = ui.add(
            bar,
            Style::row()
                .padding_x(8.0)
                .height(26.0)
                .center()
                .radius(8.0)
                .hover(Color::hex(0xe9e9ed).alpha(7)),
        );
        ui.set_name(b, name);
        ui.add_text(b, Style::default().text_size(12.0), name);
    }
    let panel = ui.add(
        root,
        Style::column()
            .fill()
            .background(SURFACE)
            .radius(8.0)
            .clip(),
    );
    ui.set_name(panel, "list");
    for i in 0..40 {
        let line = ui.add(
            panel,
            Style::row().height(24.0).fixed().padding_x(8.0).clickable(),
        );
        ui.set_name(line, format!("line {i}"));
        ui.add_text(line, Style::default(), &format!("entity {i}"));
    }
    ui
}

#[test]
fn layout_places_what_flexbox_says() {
    let mut ui = editor_like();
    ui.paint();
    let bar = ui.find("toolbar").unwrap();
    assert_eq!(ui.rect(bar).height, 40.0);
    assert_eq!(ui.rect(bar).width, 400.0);
    let list = ui.find("list").unwrap();
    let r = ui.rect(list);
    assert_eq!(
        (r.y, r.height),
        (40.0, 260.0),
        "the list fills what the toolbar leaves"
    );
    let (m, s) = (ui.find("move").unwrap(), ui.find("rotate").unwrap());
    assert!(
        ui.rect(s).x > ui.rect(m).x + ui.rect(m).width,
        "side by side, with the gap"
    );
    // A button is as wide as its word plus its padding.
    assert!(
        ui.rect(m).width > 16.0 && ui.rect(m).width < 80.0,
        "{:?}",
        ui.rect(m)
    );
}

#[test]
fn the_dump_reads_as_the_tree() {
    let mut ui = editor_like();
    let dump = ui.dump();
    assert!(dump.contains("#toolbar @0,0 400x40"), "{dump}");
    assert!(dump.contains("\"rotate\""), "{dump}");
    assert!(dump.contains("#line 3 "), "{dump}");
}

#[test]
fn a_frame_where_nothing_changed_redoes_nothing() {
    let mut ui = editor_like();
    ui.paint();
    let before = ui.revision();
    assert!(!ui.is_dirty());
    ui.paint();
    ui.paint();
    assert_eq!(ui.revision(), before, "idle frames paint nothing new");

    // The same text again is not a change.
    let line = ui.find("line 0").unwrap();
    let label = ui.children(line)[0];
    ui.set_text(label, "entity 0");
    assert!(!ui.is_dirty());

    // A colour is a repaint, not a relayout; new text is both.
    ui.restyle(line, |s| s.background(Color::hex(0x2b2741)));
    assert!(ui.is_dirty());
    ui.paint();
    assert_eq!(ui.revision(), before + 1);
    ui.set_text(label, "renamed");
    ui.paint();
    assert_eq!(ui.revision(), before + 2);
}

#[test]
fn a_click_lands_on_the_button_under_it_even_through_its_label() {
    let mut ui = editor_like();
    let rotate = ui.find("rotate").unwrap();
    ui.click(rotate);
    let events = ui.events();
    assert!(
        events.contains(&(
            rotate,
            Event::Click {
                button: MouseButton::Left,
                count: 1
            }
        )),
        "{events:?}"
    );
    // Hovering it painted it differently.
    assert_eq!(ui.hovered(), Some(rotate));
}

#[test]
fn a_press_that_moves_is_a_drag_and_ends_over_what_it_was_dropped_on() {
    let mut ui = editor_like();
    ui.paint();
    let (a, b) = (ui.find("line 1").unwrap(), ui.find("line 4").unwrap());
    let (ax, ay) = ui.rect(a).center();
    let (bx, by) = ui.rect(b).center();
    ui.handle(&InputEvent::MouseMoved { x: ax, y: ay });
    ui.handle(&InputEvent::MouseDown(MouseButton::Left));
    ui.handle(&InputEvent::MouseMoved {
        x: ax,
        y: ay + 10.0,
    });
    ui.handle(&InputEvent::MouseMoved { x: bx, y: by });
    assert_eq!(ui.dragging(), Some(a));
    ui.handle(&InputEvent::MouseUp(MouseButton::Left));
    let events = ui.events();
    assert!(events
        .iter()
        .any(|(n, e)| *n == a && matches!(e, Event::Drag { .. })));
    assert!(
        events.contains(&(a, Event::DragEnd { over: Some(b) })),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|(_, e)| matches!(e, Event::Click { .. })),
        "a drag is not a click"
    );
}

#[test]
fn the_wheel_scrolls_the_list_and_clicks_follow_the_scroll() {
    let mut ui = editor_like();
    let list = ui.find("list").unwrap();
    ui.paint();
    let (x, y) = ui.rect(list).center();
    ui.handle(&InputEvent::MouseMoved { x, y });
    ui.handle(&InputEvent::Scroll { x: 0.0, y: -3.0 });
    assert_eq!(ui.scroll(list), 120.0, "three notches of forty");
    ui.paint();
    // The line under the pointer is five lines further down.
    let under = ui.hit(x, y).unwrap();
    let name = ui.name(under).unwrap().to_string();
    assert!(name.starts_with("line "), "{name}");
    // And it does not scroll past the end: 40 lines of 24 in 260.
    for _ in 0..100 {
        ui.handle(&InputEvent::Scroll { x: 0.0, y: -3.0 });
    }
    assert_eq!(ui.scroll(list), 40.0 * 24.0 - 260.0);
}

#[test]
fn keys_go_to_whatever_was_clicked_last_that_takes_them() {
    let mut ui = Ui::new();
    let root = ui.root();
    let field = ui.add(root, Style::row().size(200.0, 30.0).focusable());
    ui.click(field);
    assert_eq!(ui.focused(), Some(field));
    ui.handle(&InputEvent::Text("ё".into()));
    ui.handle(&InputEvent::KeyDown(Key::Enter));
    let events = ui.events();
    assert!(events.contains(&(field, Event::Focus)));
    assert!(events.contains(&(field, Event::Text("ё".into()))));
    assert!(events.contains(&(field, Event::KeyDown(Key::Enter))));
    // A click on nothing takes the keyboard away.
    ui.handle(&InputEvent::MouseMoved { x: 500.0, y: 500.0 });
    ui.handle(&InputEvent::MouseDown(MouseButton::Left));
    assert_eq!(ui.focused(), None);
    assert!(ui.events().contains(&(field, Event::Blur)));
}

#[test]
fn a_list_from_data_keeps_its_nodes_by_key() {
    let mut ui = Ui::new();
    let root = ui.root();
    let list = ui.add(root, Style::column());
    let mut made = 0;
    let sync = |ui: &mut Ui, keys: &[u64], made: &mut i32| {
        ui.sync_children(
            list,
            keys,
            |ui, parent, key| {
                *made += 1;
                ui.add_text(parent, Style::default(), &key.to_string())
            },
            |_, _, _| {},
        );
    };
    sync(&mut ui, &[1, 2, 3], &mut made);
    let first = ui.children(list);
    sync(&mut ui, &[3, 1, 4], &mut made);
    let second = ui.children(list);
    assert_eq!(made, 4, "only the new key made a node");
    assert_eq!(second[0], first[2], "3 kept its node and moved to the top");
    assert_eq!(second[1], first[0]);
    assert!(!ui.exists(first[1]), "2 is gone, and its node with it");
    assert_eq!(ui.text(second[2]), Some("4"));
}

#[test]
fn a_thousand_lines_lay_out_once_and_then_cost_nothing() {
    let mut ui = Ui::new();
    ui.set_viewport(800.0, 600.0, 1.0);
    let root = ui.root();
    let list = ui.add(root, Style::column().fill().clip());
    for i in 0..1000 {
        let line = ui.add(list, Style::row().height(22.0).fixed());
        ui.add_text(line, Style::default(), &format!("entity number {i}"));
    }
    let start = std::time::Instant::now();
    ui.paint();
    let first = start.elapsed();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        ui.paint();
    }
    let idle = start.elapsed() / 100;
    assert!(idle.as_micros() < 50, "an idle frame took {idle:?}");
    eprintln!("1000 lines: first paint {first:?}, idle {idle:?}");
}

#[test]
fn a_field_types_selects_pastes_and_commits() {
    let mut ui = Ui::new();
    let root = ui.root();
    let field = ui.add_field(root, Style::row().size(200.0, 24.0).padding_x(6.0), "2.5");
    let other = ui.add(root, Style::row().size(50.0, 24.0).focusable());
    ui.click(field);
    assert_eq!(ui.focused(), Some(field));
    ui.events();

    // Select all, type over it: the selection goes, the typing stays.
    ui.handle(&InputEvent::KeyDown(Key::LeftSuper));
    ui.handle(&InputEvent::KeyDown(Key::A));
    ui.handle(&InputEvent::KeyUp(Key::A));
    ui.handle(&InputEvent::KeyUp(Key::LeftSuper));
    assert_eq!(ui.field_selection(field), 0..3);
    ui.handle(&InputEvent::Text("7".into()));
    ui.handle(&InputEvent::Text("ё".into()));
    assert_eq!(ui.text(field), Some("7ё"));
    // Backspace takes one character, not one byte.
    ui.handle(&InputEvent::KeyDown(Key::Backspace));
    assert_eq!(ui.text(field), Some("7"));
    let events = ui.events();
    assert!(
        events.contains(&(field, Event::Changed("7".into()))),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|(_, e)| matches!(e, Event::KeyDown(_))),
        "a field's keys are the field's"
    );

    // Copy, paste twice.
    ui.handle(&InputEvent::KeyDown(Key::LeftSuper));
    for key in [Key::A, Key::C, Key::Right, Key::V, Key::V] {
        ui.handle(&InputEvent::KeyDown(key));
    }
    ui.handle(&InputEvent::KeyUp(Key::LeftSuper));
    assert_eq!(ui.text(field), Some("777"));

    // Enter commits; Escape goes back to what was committed.
    ui.handle(&InputEvent::KeyDown(Key::Enter));
    assert!(ui.events().contains(&(field, Event::Submit("777".into()))));
    ui.handle(&InputEvent::Text("8".into()));
    ui.handle(&InputEvent::KeyDown(Key::Escape));
    assert_eq!(ui.text(field), Some("777"));
    assert_eq!(ui.focused(), None);

    // Leaving it changed is a commit too.
    ui.click(field);
    ui.handle(&InputEvent::Text("9".into()));
    ui.click(other);
    let events = ui.events();
    assert!(
        events
            .iter()
            .any(|(n, e)| *n == field && matches!(e, Event::Submit(t) if t.contains('9'))),
        "{events:?}"
    );
}

#[test]
fn every_icon_has_a_name_and_a_node_shows_one() {
    assert!(Ui::icon_names().count() > 50);
    let mut ui = Ui::new();
    let root = ui.root();
    let play = ui.add_icon(root, Style::default().size(14.0, 14.0), "play");
    let layers = ui.paint().to_vec();
    assert!(layers
        .iter()
        .any(|l| l.icons.iter().any(|i| i.rect.width == 14.0)));
    let _ = play;
}

#[test]
fn a_hover_over_a_long_list_repaints_quickly() {
    let mut ui = Ui::new();
    ui.set_viewport(800.0, 600.0, 1.0);
    let root = ui.root();
    let list = ui.add(root, Style::column().fill().clip());
    for i in 0..2000 {
        let line = ui.add(
            list,
            Style::row()
                .height(22.0)
                .fixed()
                .hover(Color::hex(0x333333)),
        );
        ui.add_text(line, Style::default(), &format!("entity number {i}"));
    }
    ui.paint();
    let t = std::time::Instant::now();
    for i in 0..10 {
        ui.handle(&InputEvent::MouseMoved {
            x: 10.0,
            y: 11.0 + 22.0 * i as f32,
        });
        ui.paint();
    }
    let each = t.elapsed() / 10;
    eprintln!("repaint of 2000 lines on a hover: {each:?}");
    assert!(each.as_millis() < 40, "{each:?}");
}

#[test]
fn text_sits_in_the_middle_of_a_taller_box() {
    let mut ui = Ui::new();
    let root = ui.root();
    let field = ui.add_field(
        root,
        Style::row()
            .size(200.0, 30.0)
            .padding_x(6.0)
            .text_size(12.0),
        "2.6",
    );
    let layers = ui.paint().to_vec();
    let text = layers
        .iter()
        .flat_map(|l| &l.texts)
        .find(|t| t.node == field)
        .unwrap();
    let rect = ui.rect(field);
    // A 12 px line is 16.2 px tall: centred in 30 it starts 6.9 down.
    let line = 12.0 * 1.35;
    assert!(
        (text.y - (rect.y + (30.0 - line) / 2.0)).abs() < 0.5,
        "{} in {rect:?}",
        text.y
    );
}

#[test]
fn a_node_moves_to_another_parent_whole() {
    let mut ui = Ui::new();
    let root = ui.root();
    let a = ui.add(root, Style::column().size(100.0, 100.0));
    let b = ui.add(root, Style::column().size(100.0, 100.0));
    let panel = ui.add(a, Style::column());
    let label = ui.add_text(panel, Style::default(), "inside");
    ui.move_to(panel, b);
    assert_eq!(ui.parent(panel), Some(b));
    assert_eq!(ui.children(a), vec![]);
    assert_eq!(ui.text(label), Some("inside"));
    ui.paint();
    assert_eq!(ui.rect(label).y, ui.rect(b).y);
}

#[test]
fn a_textarea_takes_new_lines_and_moves_between_them() {
    let mut ui = Ui::new();
    let root = ui.root();
    let area = ui.add_textarea(
        root,
        Style::column().width(300.0).padding(6.0),
        "(\n    a: 1,\n)",
    );
    ui.click(area);
    ui.events();
    // To the end of everything, then up one line: inside `a: 1,`.
    ui.handle(&InputEvent::KeyDown(Key::LeftSuper));
    ui.handle(&InputEvent::KeyDown(Key::A));
    ui.handle(&InputEvent::KeyUp(Key::LeftSuper));
    ui.handle(&InputEvent::KeyDown(Key::Right));
    ui.handle(&InputEvent::KeyDown(Key::Up));
    ui.handle(&InputEvent::KeyDown(Key::End));
    ui.handle(&InputEvent::KeyDown(Key::Enter));
    ui.handle(&InputEvent::Text("    b: 2,".into()));
    assert_eq!(ui.text(area), Some("(\n    a: 1,\n    b: 2,\n)"));
    // Enter was a new line, not a commit; Cmd Enter commits.
    assert!(!ui
        .events()
        .iter()
        .any(|(_, e)| matches!(e, Event::Submit(_))));
    ui.handle(&InputEvent::KeyDown(Key::LeftSuper));
    ui.handle(&InputEvent::KeyDown(Key::Enter));
    ui.handle(&InputEvent::KeyUp(Key::LeftSuper));
    assert!(ui
        .events()
        .iter()
        .any(|(_, e)| matches!(e, Event::Submit(t) if t.contains("b: 2"))));
    // It is as tall as its four lines.
    ui.paint();
    assert!(
        ui.rect(area).height > 4.0 * 13.0 * 1.3,
        "{:?}",
        ui.rect(area)
    );
}

#[test]
fn an_input_method_composes_in_place_and_commits_once() {
    let mut ui = Ui::new();
    let root = ui.root();
    let field = ui.add_field(root, Style::row().size(200.0, 24.0), "ab");
    ui.click(field);
    ui.handle(&InputEvent::KeyDown(Key::End));
    ui.events();
    ui.ime_preedit("に");
    ui.ime_preedit("にほ");
    assert_eq!(ui.text(field), Some("abにほ"), "shown while composing");
    assert!(
        !ui.events()
            .iter()
            .any(|(_, e)| matches!(e, Event::Changed(_))),
        "composing is not typing"
    );
    assert!(ui.caret_rect().is_some());
    ui.ime_preedit("");
    assert_eq!(ui.text(field), Some("ab"));
    ui.handle(&InputEvent::Text("日本".into()));
    assert_eq!(ui.text(field), Some("ab日本"));
    assert!(ui
        .events()
        .contains(&(field, Event::Changed("ab日本".into()))));
}

#[test]
fn asking_about_a_removed_node_is_an_answer_not_a_crash() {
    let mut ui = Ui::new();
    let root = ui.root();
    let gone = ui.add_text(root, Style::default(), "here a moment");
    ui.remove(gone);
    assert_eq!(ui.parent(gone), None);
    assert_eq!(ui.name(gone), None);
    assert_eq!(ui.text(gone), None);
    assert!(ui.children(gone).is_empty());
    assert_eq!(ui.rect(gone).width, 0.0);
    assert!(!ui.is_field(gone));
}

#[test]
fn a_palette_recolours_a_built_tree() {
    let mut ui = editor_like();
    let before = ui.revision();
    ui.paint();
    let red = [0xe0, 0x40, 0x40];
    ui.set_palette([([0x23, 0x25, 0x32], red)].into_iter().collect());
    let layers = ui.paint();
    let fills: Vec<Color> = layers
        .iter()
        .flat_map(|l| l.rects.iter().map(|r| r.fill))
        .collect();
    assert!(
        fills.iter().any(|c| [c.r, c.g, c.b] == red),
        "SURFACE drawn red"
    );
    assert!(!fills.iter().any(|c| [c.r, c.g, c.b] == [0x23, 0x25, 0x32]));
    assert!(fills.contains(&BG), "the rest as it was");
    assert!(ui.revision() > before);
    // The same palette again: nothing to redo.
    let r = ui.revision();
    ui.set_palette([([0x23, 0x25, 0x32], red)].into_iter().collect());
    ui.paint();
    assert_eq!(ui.revision(), r);
}
