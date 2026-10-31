//! The game as the window plays it: the screen loads and shows the run,
//! clicks become moves, and a picture of it for a person to look at.

use std::path::Path;
use std::sync::Arc;

use magistral_rules::{Card, Move, Phase};

use crate::front::{Front, Press};
use crate::table::Table;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn new(seed: u64) -> (Front, Table) {
    let front = Front::new(Arc::new(crate::config(root())), seed);
    let mut table = Table::load(root()).expect("the screen reads");
    scrap::html::fit(&mut table.ui, 1280, 720);
    table.show(&front);
    (front, table)
}

#[test]
fn the_screen_reads_and_finds_everything_it_names() {
    let (_, mut table) = new(1);
    assert!(
        table.screen.missing().is_empty(),
        "{:?}",
        table.screen.missing()
    );
    assert!(
        table.screen.style_problems().is_empty(),
        "{:?}",
        table.screen.style_problems()
    );
    let dump = table.ui.dump();
    assert!(dump.contains("Этап 1 из 5"), "{dump}");
    assert!(dump.contains("Конец хода"), "{dump}");
}

#[test]
fn the_end_button_ends_the_turn() {
    let (mut front, mut table) = new(2);
    let end = table.screen.node("end").expect("an end button");
    table.ui.click(end);
    table.frame(&mut front, 0.016);
    assert_eq!(front.state.turn, 1);
    assert!(table.ui.dump().contains("Ход 2 из 8"));
}

#[test]
fn a_path_card_then_a_lit_cell_lays_track() {
    for seed in 0..20 {
        let (mut front, _) = new(seed);
        let Some(i) = front.state.hand.iter().position(|&c| c == Card::Path) else {
            continue;
        };
        front.press(Press::Card(i));
        let view = front.view();
        let lit: Vec<usize> = (0..64).filter(|&c| view.board[c].mark).collect();
        assert!(
            !lit.is_empty(),
            "seed {seed}: a path lights where it can go"
        );
        let before = front.state.cells.iter().filter(|c| c.track).count();
        front.press(Press::Cell(lit[0] as u8));
        let after = front.state.cells.iter().filter(|c| c.track).count();
        assert!(after > before, "seed {seed}: the click laid track");
        return;
    }
    panic!("no hand with a path in 20 seeds");
}

#[test]
fn a_train_is_two_clicks_on_its_stations() {
    // Build a line with the rules, then run a train on it by presses.
    let (mut front, _) = new(3);
    let mut guard = 0;
    while front.state.routes().is_empty() && guard < 200 {
        let mv =
            magistral_rules::ai::Player::choose(&mut magistral_rules::ai::Greedy, &front.state);
        if let Move::Play { card, .. } = mv {
            if card.is_train() {
                break;
            }
        }
        front.apply(mv);
        guard += 1;
    }
    let (a, b) = front.state.routes()[0];
    front.state.hand.push(Card::Loco);
    front.state.energy = 3;
    let i = front.state.hand.len() - 1;
    front.press(Press::Card(i));
    front.press(Press::Cell(b));
    front.press(Press::Cell(a));
    assert_eq!(front.state.trains.len(), 1, "{:?}", front.view().prompt);
}

#[test]
fn the_market_takes_a_card_then_lets_one_go() {
    let (mut front, _) = new(4);
    front.state.money = 500;
    while front.state.phase == Phase::Play {
        front.press(Press::EndTurn);
    }
    let Phase::Pick(offers) = front.state.phase.clone() else {
        panic!("{:?}", front.state.phase)
    };
    let size = front.state.draw.len();
    front.press(Press::Offer(Some(0)));
    assert_eq!(front.state.draw.len(), size + 1);
    assert!(front.state.draw.contains(&offers[0]));
    assert_eq!(front.state.phase, Phase::Remove);
    front.press(Press::Remove(Some(0)));
    assert_eq!(front.state.phase, Phase::Play);
    assert_eq!(front.state.draw.len() + front.state.hand.len(), size);
}

/// What the window shows, as target/shots/*.png — a few turns in, so there
/// is track, a station and a hint to see. Skipped on a machine with no GPU,
/// software or not.
#[test]
fn a_picture_of_the_game() {
    let Ok(gpu) = scrap::Gpu::headless_blocking(true) else {
        eprintln!("no GPU: no picture");
        return;
    };
    let (mut front, mut table) = new(5);
    for _ in 0..14 {
        let mv =
            magistral_rules::ai::Player::choose(&mut magistral_rules::ai::Greedy, &front.state);
        front.apply(mv);
    }
    if let Some(i) = front.state.hand.iter().position(|c| c.is_track()) {
        front.press(Press::Card(i));
    }
    front.press(Press::Hint);
    let target = scrap::OffscreenTarget::new(&gpu, 1280, 720);
    let mut screens =
        scrap::scrap_ui::render::UiRenderer::new(&gpu, target.format().remove_srgb_suffix());
    for (name, front) in [("game", &front)] {
        table.show(front);
        screens.draw(
            &gpu,
            &target.ui_view(),
            1280,
            720,
            &mut table.ui,
            Some(scrap::scrap_ui::Color::rgba(243, 234, 216, 255)),
        );
        let pixels = target.read_rgba(&gpu);
        let dir = root().join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        image::save_buffer(
            dir.join(format!("{name}.png")),
            &pixels,
            1280,
            720,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    // And the market.
    let (mut front, mut table) = new(6);
    front.state.money = 50;
    while front.state.phase == Phase::Play {
        front.press(Press::EndTurn);
    }
    table.show(&front);
    // A renderer of its own: it knows the last tree by its revision only.
    let mut screens =
        scrap::scrap_ui::render::UiRenderer::new(&gpu, target.format().remove_srgb_suffix());
    screens.draw(
        &gpu,
        &target.ui_view(),
        1280,
        720,
        &mut table.ui,
        Some(scrap::scrap_ui::Color::rgba(243, 234, 216, 255)),
    );
    let pixels = target.read_rgba(&gpu);
    image::save_buffer(
        root().join("target/shots/market.png"),
        &pixels,
        1280,
        720,
        image::ColorType::Rgba8,
    )
    .unwrap();
}
