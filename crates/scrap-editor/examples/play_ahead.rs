//! How long Play takes, measured as a window's session sees it: the scene
//! opened, the game started ahead (`Session::set_play_ahead`), Play
//! pressed, and the time until the game's first frame is in the Game view
//! — then the same started cold, for comparison.
//!
//! `cargo run -p scrap-editor --example play_ahead -- <project>/scenes/X.scene.ron`

use std::time::{Duration, Instant};

use scrap_editor::Session;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scene = std::env::args().nth(1).ok_or("give a scene to play")?;
    let mut session = Session::offscreen(1600, 900)?;
    session.draw_while_building();
    session.open_scene(std::path::Path::new(&scene))?;
    // Alone, so no other player's window opens; the person's own choice
    // is put back at the end.
    let players = session.players();
    session.set_players(1);
    let measured = measure(&mut session);
    session.set_players(players);
    measured
}

fn measure(session: &mut Session) -> Result<(), Box<dyn std::error::Error>> {
    let input = scrap::input::Input::new();

    let first_frame = |session: &mut Session, pressed: Instant| -> Result<Duration, Box<dyn std::error::Error>> {
        while !session.game_draws_in_view() {
            session.scene_view(&input, 1.0 / 60.0)?;
            session.render();
            if !session.game_running() {
                return Err("the game ended before it drew".into());
            }
            if pressed.elapsed() > Duration::from_secs(300) {
                return Err("no frame in five minutes".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(pressed.elapsed())
    };

    session.set_play_ahead(true);
    let started = Instant::now();
    while !session.is_warm_ready() {
        session.poll_game();
        if !session.is_warm() && started.elapsed() > Duration::from_secs(5) {
            for line in session.console() {
                eprintln!("console: {}", line.text);
            }
            return Err("the game could not be started ahead".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("started ahead, ready in {:.2} s", started.elapsed().as_secs_f32());
    std::thread::sleep(Duration::from_secs(1));
    session.set_game_view(true);
    let pressed = Instant::now();
    session.start_game()?;
    let ahead = first_frame(session, pressed)?;
    println!("Play → first frame, started ahead: {:.0} ms", ahead.as_secs_f32() * 1000.0);
    session.stop_game();

    session.set_play_ahead(false);
    let pressed = Instant::now();
    session.start_game()?;
    let cold = first_frame(session, pressed)?;
    println!("Play → first frame, cold: {:.0} ms", cold.as_secs_f32() * 1000.0);
    session.stop_game();
    for line in session.console().iter().filter(|l| l.text.contains("playing")) {
        println!("console: {}", line.text);
    }
    Ok(())
}
