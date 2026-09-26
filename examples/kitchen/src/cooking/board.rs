//! The order board on the back wall: a screen in the world (`WorldUi`) the
//! game draws on every step, on every peer — the clock, the score and the
//! orders, each a coloured card with a bar of how long it will wait.

use scrap::glam::Vec4;
use scrap::hecs::{Entity, World};
use scrap::ui::{Quad, TextRun};
use scrap::world::WorldUi;

use crate::components::item::{Dish, Food};
use crate::components::Mark;
use crate::state::Round;

/// The board's picture, in pixels: as wide as it is on the wall, twice.
pub const SIZE: (u32, u32) = (720, 240);

pub fn run(world: &mut World, _seconds: f32) {
    let round = world.query::<&Round>().iter().next().cloned();
    let boards: Vec<Entity> = world
        .query::<(Entity, &Mark)>()
        .iter()
        .filter(|(_, m)| m.name == "board")
        .map(|(e, _)| e)
        .collect();
    for board in boards {
        if world.get::<&WorldUi>(board).is_err() {
            let mut screen = WorldUi::new("orders", SIZE);
            screen.background = Vec4::new(0.12, 0.13, 0.15, 1.0);
            let _ = world.insert_one(board, screen);
        }
        let Ok(mut screen) = world.get::<&mut WorldUi>(board) else {
            continue;
        };
        draw(&mut screen.ui, round.as_ref());
    }
}

/// The board as it is now: nothing before a round.
pub fn draw(ui: &mut scrap::ui::Ui, round: Option<&Round>) {
    ui.clear();
    let white = Vec4::ONE;
    let Some(round) = round else {
        ui.text(TextRun::new(24.0, 90.0, 40.0, white, "Kitchen Rush"));
        return;
    };
    let seconds = round.time_left.ceil() as u32;
    ui.text(TextRun::new(24.0, 14.0, 34.0, white, format!("{}:{:02}", seconds / 60, seconds % 60)));
    ui.text(TextRun::new(SIZE.0 as f32 - 220.0, 14.0, 34.0, white, format!("{:>5} pts", round.score)));
    if round.orders.is_empty() {
        ui.text(TextRun::new(24.0, 110.0, 28.0, Vec4::new(1.0, 1.0, 1.0, 0.5), "No orders yet"));
    }
    for (i, order) in round.orders.iter().enumerate() {
        let x = 24.0 + i as f32 * 172.0;
        let (colour, name) = match order.dish {
            Dish::Soup(Food::Onion) => (Vec4::new(0.9, 0.8, 0.45, 1.0), "ONION"),
            Dish::Soup(_) => (Vec4::new(0.85, 0.26, 0.18, 1.0), "TOMATO"),
            Dish::Salad => (Vec4::new(0.35, 0.72, 0.35, 1.0), "SALAD"),
            Dish::Burger => (Vec4::new(0.72, 0.45, 0.22, 1.0), "BURGER"),
        };
        ui.quad(Quad::new(x, 70.0, 160.0, 140.0, Vec4::new(1.0, 1.0, 1.0, 0.08)));
        ui.quad(Quad::new(x + 50.0, 84.0, 60.0, 60.0, colour));
        ui.text(TextRun::new(x + 12.0, 152.0, 24.0, white, name));
        let left = (order.left / order.total).clamp(0.0, 1.0);
        let bar = if left < 0.3 { Vec4::new(0.9, 0.35, 0.25, 1.0) } else { Vec4::new(0.5, 0.83, 0.42, 1.0) };
        ui.quad(Quad::new(x + 12.0, 190.0, 136.0 * left, 10.0, bar));
    }
}
