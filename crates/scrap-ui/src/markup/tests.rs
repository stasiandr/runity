use serde::Serialize;

use super::*;

const HUD: &str = r#"<!doctype html>
<html>
<head>
  <link rel="stylesheet" href="hud.css">
  <style>
    .score { color: var(--accent); }
  </style>
</head>
<body>
  <div class="top">
    <span id="clock">{{round.clock}}</span>
    <span class="score">Score: {{round.score}}</span>
  </div>
  <div class="bar"><div id="fill" class="fill" data-bind-style-width="{{player.hp / player.max * 100}}%"></div></div>
  <div id="orders" class="orders">
    <div class="order" data-bind-for="i, order:{{orders}}" data-bind-class-toggle="late:{{order.left < 10}}">
      <span>{{i + 1}}. {{order.dish}}</span>
      <button id="cancel">Cancel</button>
    </div>
  </div>
  <input id="name" placeholder="Your name">
  <button id="leave" data-bind-if="{{round.over}}">@hud.leave</button>
</body>
</html>"#;

const CSS: &str = r#"
:root { --accent: #e07a5f; }
body { padding: 20px; gap: 8px; }
.top { flex-direction: row; justify-content: space-between; height: 40px; }
.bar { width: 200px; height: 10px; background: #333333; }
.fill { height: 100%; background: #4caf50; }
.orders { gap: 4px; }
.order { flex-direction: row; gap: 8px; height: 30px; background: #222222; }
.order.late { background: #aa0000; }
"#;

#[derive(Serialize)]
struct Order {
    dish: &'static str,
    left: u32,
}

fn hud() -> (Screen, Ui) {
    let mut screen = Screen::from_source("hud.screen.html", HUD, &[("hud.css", CSS)]).unwrap();
    screen.set(
        "round",
        serde_json::json!({"clock": "1:30", "score": 40, "over": false}),
    );
    screen.set("player", serde_json::json!({"hp": 30, "max": 120}));
    screen.set(
        "orders",
        vec![
            Order {
                dish: "Soup",
                left: 50,
            },
            Order {
                dish: "Salad",
                left: 5,
            },
        ],
    );
    let mut ui = Ui::new();
    ui.set_viewport(1280.0, 720.0, 1.0);
    let root = ui.root();
    screen.mount(&mut ui, root);
    (screen, ui)
}

#[test]
fn a_screen_is_built_from_its_html_and_css_and_shows_the_model() {
    let (screen, mut ui) = hud();
    let dump = ui.dump();
    assert_eq!(screen.text(&ui, "clock").as_deref(), Some("1:30"), "{dump}");
    assert!(dump.contains("\"Score: 40\""), "{dump}");
    assert!(dump.contains("\"1. Soup\""), "{dump}");
    assert!(dump.contains("\"2. Salad\""), "{dump}");
    assert!(
        !dump.contains("#leave"),
        "hidden until the round is over: {dump}"
    );
    // 30 of 120: a quarter of the bar.
    let fill = screen.node("fill").unwrap();
    assert_eq!(ui.rect(fill).width, 50.0);
    // The second order is late, and its style says so.
    let orders = screen.node("orders").unwrap();
    let rows = ui.children(orders);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        ui.style(rows[0]).look.background,
        crate::Color::hex(0x222222)
    );
    assert_eq!(
        ui.style(rows[1]).look.background,
        crate::Color::hex(0xaa0000)
    );
    // `var()` from `:root`, through a `<style>` in the page.
    let top = ui.find("clock").and_then(|c| ui.parent(c)).unwrap();
    let score = ui.children(top)[1];
    assert_eq!(ui.style(score).text.color, crate::Color::hex(0xe07a5f));
    assert!(screen.missing().is_empty(), "{:?}", screen.missing());
}

#[test]
fn changing_the_model_changes_only_what_it_shows() {
    let (mut screen, mut ui) = hud();
    ui.paint();
    let clock = screen.node("clock").unwrap();
    let before = ui.revision();
    screen.update(&mut ui);
    ui.paint();
    assert_eq!(ui.revision(), before, "nothing changed, nothing painted");

    screen.set("round.clock", "1:29");
    screen.set("round.over", true);
    screen.update(&mut ui);
    assert_eq!(screen.text(&ui, "clock").as_deref(), Some("1:29"));
    assert_eq!(
        screen.node("clock"),
        Some(clock),
        "the same node, not a new one"
    );
    assert_eq!(screen.text(&ui, "leave").as_deref(), Some("@hud.leave"));
}

#[test]
fn a_list_grows_and_shrinks_with_the_model_and_keeps_its_rows() {
    let (mut screen, mut ui) = hud();
    let orders = screen.node("orders").unwrap();
    let first = ui.children(orders)[0];
    screen.set(
        "orders",
        vec![
            Order {
                dish: "Soup",
                left: 50,
            },
            Order {
                dish: "Salad",
                left: 5,
            },
            Order {
                dish: "Burger",
                left: 40,
            },
        ],
    );
    screen.update(&mut ui);
    let rows = ui.children(orders);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], first, "the row already built stays");
    assert!(ui.dump().contains("\"3. Burger\""));
    screen.set(
        "orders",
        vec![Order {
            dish: "Burger",
            left: 40,
        }],
    );
    screen.update(&mut ui);
    let rows = ui.children(orders);
    assert_eq!(rows.len(), 1);
    let dump = ui.dump();
    assert!(
        dump.contains("\"1. Burger\"") && !dump.contains("Soup"),
        "{dump}"
    );
    // The input after the list is still after it.
    let body = ui.parent(orders).unwrap();
    let kids = ui.children(body);
    let at = kids.iter().position(|k| *k == orders).unwrap();
    assert_eq!(Some(kids[at + 1]), screen.node("name"));
}

#[test]
fn clicks_come_back_by_id_and_row() {
    let (mut screen, mut ui) = hud();
    let orders = screen.node("orders").unwrap();
    let second = ui.children(orders)[1];
    let cancel = ui
        .children(second)
        .into_iter()
        .find(|n| ui.name(*n) == Some("cancel"))
        .unwrap();
    ui.click(cancel);
    let done = screen.read(&ui.events());
    assert!(done.clicked("cancel"));
    assert_eq!(done.clicked_row("cancel"), Some(1));

    screen.set("round.over", true);
    screen.update(&mut ui);
    let leave = screen.node("leave").unwrap();
    ui.click(leave);
    let done = screen.read(&ui.events());
    assert_eq!(
        done.clicked,
        [Target {
            id: "leave".into(),
            index: None
        }]
    );
}

#[test]
fn typing_into_a_field_is_reported_by_its_id() {
    let (screen, mut ui) = hud();
    let name = screen.node("name").unwrap();
    ui.click(name);
    ui.handle(&scrap_core::input::InputEvent::Text("Ann".into()));
    ui.handle(&scrap_core::input::InputEvent::KeyDown(
        scrap_core::input::Key::Enter,
    ));
    let done = screen.read(&ui.events());
    assert_eq!(done.changed("name"), Some("Ann"));
    assert_eq!(done.submitted("name"), Some("Ann"));
}

#[test]
fn what_the_ui_cannot_do_is_said_at_its_line() {
    let page = "<div>\n  <script>go()</script>\n  <img src=a.png>\n  <marquee>hi</marquee>\n  <button onclick=\"x()\">a</button>\n  <p id=a></p><p id=a></p>\n  <span data-bind-value=\"{{a = 1}}\"></span>\n</div>";
    let problems = Screen::from_source("bad.html", page, &[]).err().unwrap();
    let text = problems.to_string();
    assert!(text.contains("bad.html:2:3: no scripts"), "{text}");
    assert!(
        text.contains("bad.html:3:3: `<img>` is not read yet"),
        "{text}"
    );
    assert!(
        text.contains("bad.html:4:3: `<marquee>` is not an element"),
        "{text}"
    );
    assert!(text.contains("bad.html:5:11: no scripts"), "{text}");
    assert!(
        text.contains("bad.html:6:15: `a` is the id of another element too (line 6)"),
        "{text}"
    );
    assert!(text.contains("only reads"), "{text}");

    let bad_css = Screen::from_source(
        "x.html",
        "<link rel=stylesheet href=x.css><div></div>",
        &[("x.css", "div {\n  float: left;\n}")],
    )
    .err()
    .unwrap();
    assert_eq!(
        bad_css.to_string(),
        "x.css:2:3: `float` is not a property this UI reads"
    );
}

#[test]
fn a_path_the_game_never_set_is_named() {
    let mut screen = Screen::from_source("m.html", "<span>{{player.nmae}}</span>", &[]).unwrap();
    screen.set("player.name", "Ann");
    let mut ui = Ui::new();
    let root = ui.root();
    screen.mount(&mut ui, root);
    assert_eq!(screen.missing(), ["player.nmae"]);
}

#[test]
fn a_saved_file_rebuilds_the_screen_and_a_broken_one_keeps_the_last() {
    let dir = std::env::temp_dir().join(format!("scrap-markup-reload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("menu.screen.html");
    let css = dir.join("menu.css");
    std::fs::write(
        &html,
        "<link rel=stylesheet href=menu.css><button id=play>{{label}}</button>",
    )
    .unwrap();
    std::fs::write(&css, "button { width: 100px; }").unwrap();
    let mut screen = Screen::load(&html).unwrap();
    screen.set("label", "Play");
    let mut ui = Ui::new();
    let root = ui.root();
    screen.mount(&mut ui, root);
    ui.paint();
    assert_eq!(ui.rect(screen.node("play").unwrap()).width, 100.0);

    let later = |path: &Path, secs: u64| {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() + std::time::Duration::from_secs(secs))
            .unwrap();
    };
    std::fs::write(&css, "button { width: 300px; }").unwrap();
    later(&css, 2);
    assert_eq!(screen.poll(&mut ui, 1.0), Some(Ok(())));
    ui.paint();
    let play = screen.node("play").unwrap();
    assert_eq!(ui.rect(play).width, 300.0);
    assert_eq!(
        screen.text(&ui, "play").as_deref(),
        Some("Play"),
        "the model outlives the file"
    );

    std::fs::write(&css, "button { width: lots; }").unwrap();
    later(&css, 4);
    let err = screen.poll(&mut ui, 1.0).unwrap().unwrap_err();
    assert!(
        err.to_string()
            .contains("menu.css:1:10: `lots` is not a length"),
        "{err}"
    );
    ui.paint();
    assert_eq!(
        ui.rect(screen.node("play").unwrap()).width,
        300.0,
        "the last good one"
    );
    assert_eq!(screen.poll(&mut ui, 1.0), None, "said once");
}

#[test]
fn hover_and_press_change_the_look_and_text_is_inherited() {
    let mut screen = Screen::from_source(
        "s.html",
        "<style>.menu { color: #ff0000; font-size: 20px } .menu button:hover { background: #00ff00 }</style><div class=menu><button id=b>Go</button><span id=s>x</span></div>",
        &[],
    )
    .unwrap();
    let mut ui = Ui::new();
    let root = ui.root();
    screen.mount(&mut ui, root);
    let b = screen.node("b").unwrap();
    assert_eq!(
        ui.style(b).look.hover_background,
        Some(crate::Color::hex(0x00ff00))
    );
    assert_eq!(
        ui.style(b).look.press_background,
        Some(crate::Color::hex(0x2c2f3d)),
        "from the UA sheet"
    );
    let s = screen.node("s").unwrap();
    assert_eq!(ui.style(s).text.color, crate::Color::hex(0xff0000));
    assert_eq!(ui.style(s).text.size, 20.0);
}

#[test]
fn words_starting_with_at_are_in_the_current_language() {
    let dir = std::env::temp_dir().join(format!("scrap-markup-strings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ru.ron"), r#"{"hud.leave": "Выйти"}"#).unwrap();
    let mut screen =
        Screen::from_source("s.html", "<button id=leave>@hud.leave</button>", &[]).unwrap();
    screen.set_strings(&scrap_core::strings::Strings::load(&dir, "ru").unwrap());
    let mut ui = Ui::new();
    let root = ui.root();
    screen.mount(&mut ui, root);
    assert_eq!(screen.text(&ui, "leave").as_deref(), Some("Выйти"));
}

#[test]
fn the_words_a_screen_asks_a_language_for_are_listed() {
    let (screen, _) = hud();
    assert_eq!(screen.keys(), ["hud.leave"]);
}

#[test]
fn a_screen_is_fitted_to_the_window_as_a_canvas_scaler_does() {
    let mut ui = Ui::new();
    // Twice the reference: everything twice as big.
    assert_eq!(fit(&mut ui, 2560, 1440), 2.0);
    assert_eq!(ui.viewport(), (1280.0, 720.0, 2.0));
    // Wider than 16:9: by the height, more room across.
    assert_eq!(fit(&mut ui, 3440, 1440), 2.0);
    assert_eq!(ui.viewport(), (1720.0, 720.0, 2.0));
    // A phone held upright: by the width.
    let s = fit(&mut ui, 640, 1280);
    assert_eq!(s, 0.5);
    // A click at physical (200, 100) is at CSS (400, 200).
    let moved = pointer(
        &scrap_core::input::InputEvent::MouseMoved { x: 200.0, y: 100.0 },
        s,
    );
    assert_eq!(
        moved,
        [scrap_core::input::InputEvent::MouseMoved { x: 400.0, y: 200.0 }]
    );
}
