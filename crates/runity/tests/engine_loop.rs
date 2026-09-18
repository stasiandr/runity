//! The whole engine, driven through a headless window: no display required.

use runity::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default, Debug, PartialEq)]
struct Calls {
    start: u32,
    update: u32,
    fixed_update: u32,
    render: u32,
    events: Vec<Event>,
}

struct Counter {
    calls: Rc<RefCell<Calls>>,
    quit_after: Option<u32>,
}

impl Game for Counter {
    fn start(&mut self, engine: &mut Engine) -> std::io::Result<()> {
        self.calls.borrow_mut().start += 1;
        engine.clear_color = Color::rgb(0.0, 0.0, 0.25);
        // A flat background makes "how much of the frame is geometry" easy to
        // count; the sky would fill every pixel.
        engine.renderer.settings.draw_sky = false;
        Ok(())
    }

    fn on_event(&mut self, _engine: &mut Engine, event: &Event) {
        self.calls.borrow_mut().events.push(*event);
    }

    fn update(&mut self, engine: &mut Engine) {
        let mut calls = self.calls.borrow_mut();
        calls.update += 1;
        if Some(calls.update) == self.quit_after {
            engine.quit();
        }
    }

    fn fixed_update(&mut self, _engine: &mut Engine) {
        self.calls.borrow_mut().fixed_update += 1;
    }

    fn render(&mut self, engine: &mut Engine) {
        self.calls.borrow_mut().render += 1;
        engine.draw_pbr(&Mesh::cube(1.0), Mat4::IDENTITY, &Material::default());
    }
}

fn headless(width: u32, height: u32) -> HeadlessWindow {
    HeadlessWindow::new(&WindowConfig::new("test", width, height))
}

#[test]
fn the_loop_calls_every_stage_the_expected_number_of_times() {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let game = Counter {
        calls: Rc::clone(&calls),
        quit_after: None,
    };

    let engine = App::new(WindowConfig::new("test", 64, 48))
        .with_max_frames(5)
        .with_frame_delta(1.0 / 60.0)
        .with_target_fps(None)
        .run_with_window(Box::new(headless(64, 48)), game)
        .expect("the loop runs");

    let calls = calls.borrow();
    assert_eq!(calls.start, 1, "start runs exactly once");
    assert_eq!(calls.update, 5);
    assert_eq!(calls.render, 5);
    // One 1/60s frame is exactly one 1/60s fixed step.
    assert_eq!(calls.fixed_update, 5);
    assert_eq!(engine.time.frame(), 5);
    assert!(!engine.is_running());
}

#[test]
fn a_game_can_quit_before_the_frame_limit() {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let game = Counter {
        calls: Rc::clone(&calls),
        quit_after: Some(2),
    };

    App::new(WindowConfig::new("test", 32, 32))
        .with_max_frames(100)
        .with_frame_delta(1.0 / 60.0)
        .with_target_fps(None)
        .run_with_window(Box::new(headless(32, 32)), game)
        .expect("the loop runs");

    assert_eq!(calls.borrow().update, 2);
    assert_eq!(
        calls.borrow().render,
        2,
        "the frame that quits is still drawn"
    );
}

#[test]
fn a_close_request_stops_the_loop_and_reaches_the_game() {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let game = Counter {
        calls: Rc::clone(&calls),
        quit_after: None,
    };

    let mut window = headless(32, 32);
    window.push_event(Event::CloseRequested);

    App::new(WindowConfig::new("test", 32, 32))
        .with_max_frames(100)
        .with_frame_delta(1.0 / 60.0)
        .with_target_fps(None)
        .run_with_window(Box::new(window), game)
        .expect("the loop runs");

    let calls = calls.borrow();
    assert_eq!(
        calls.update, 1,
        "the loop stops after the frame that saw the close"
    );
    assert_eq!(calls.events, vec![Event::CloseRequested]);
}

#[test]
fn a_resize_event_resizes_the_framebuffer() {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut window = headless(32, 32);
    window.push_event(Event::Resized {
        width: 100,
        height: 40,
    });

    let engine = App::new(WindowConfig::new("test", 32, 32))
        .with_max_frames(1)
        .with_frame_delta(1.0 / 60.0)
        .with_target_fps(None)
        .run_with_window(
            Box::new(window),
            Counter {
                calls: Rc::clone(&calls),
                quit_after: None,
            },
        )
        .expect("the loop runs");

    assert_eq!(
        (engine.framebuffer.width(), engine.framebuffer.height()),
        (100, 40)
    );
    assert!((engine.aspect_ratio() - 2.5).abs() < 1e-6);
}

#[test]
fn rendering_actually_puts_geometry_on_the_screen() {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let engine = App::new(WindowConfig::new("test", 120, 90))
        .with_max_frames(1)
        .with_frame_delta(1.0 / 60.0)
        .with_target_fps(None)
        .run_with_window(
            Box::new(headless(120, 90)),
            Counter {
                calls: Rc::clone(&calls),
                quit_after: None,
            },
        )
        .expect("the loop runs");

    let clear = Color::rgb(0.0, 0.0, 0.25);
    let drawn = engine
        .framebuffer
        .colors()
        .iter()
        .filter(|c| **c != clear)
        .count();
    let total = engine.framebuffer.len();
    assert!(
        drawn > total / 40 && drawn < total,
        "the cube should cover a chunk of the frame, got {drawn} of {total} pixels"
    );

    let stats = engine.frame_stats();
    assert_eq!(stats.triangles_in, 12);
    assert!(stats.triangles_rasterized >= 2 && stats.triangles_rasterized <= 6);
}

#[test]
fn a_slow_frame_cannot_spiral_into_endless_fixed_steps() {
    let calls = Rc::new(RefCell::new(Calls::default()));
    App::new(WindowConfig::new("test", 16, 16))
        .with_max_frames(1)
        // A one-second frame is 60 fixed steps' worth of catch-up...
        .with_frame_delta(1.0)
        .with_target_fps(None)
        .run_with_window(
            Box::new(headless(16, 16)),
            Counter {
                calls: Rc::clone(&calls),
                quit_after: None,
            },
        )
        .expect("the loop runs");

    // ...but `Time::max_delta` clamps the frame and `max_fixed_steps` caps the
    // catch-up, so the loop stays responsive.
    assert!(
        calls.borrow().fixed_update <= 8,
        "{:?}",
        calls.borrow().fixed_update
    );
}
