//! The window, used with a mouse and a keyboard — off-screen.
//!
//! The session's behaviour is tested in `runity-editor`, without a window.
//! What is left to test here is the wiring: that a click on a line of the
//! Hierarchy selects that entity, that a number typed into the Inspector
//! reaches the document as one undo step, that the toolbar's undo takes it
//! back, that a line's arrow folds it. The window is opened off-screen and
//! drawn by the real renderer (GPUI's `VisualTestAppContext`), and the
//! clicks land where a person's would — so the positions below are the
//! layout's, at the default 1440 × 900.
//!
//! Skipped, not failed, where there is no GPU to draw with. Its own
//! `main` rather than libtest's: AppKit wants the main thread, and libtest
//! runs every test on another.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    mac::the_panels_select_edit_undo_and_fold();
    println!("test the_panels_select_edit_undo_and_fold ... ok");
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui::{point, px, size, AnyWindowHandle, Entity, Modifiers, VisualTestAppContext};
    use gpui_kit::component::Root;
    use runity_editor::Session;
    use runity_studio::Studio;

    const SCENE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/valley/scenes/first-light.ron"
    );

    /// Let frames go by: the Scene view draws, the panels hear about it, the
    /// layout settles where a click can find it.
    fn settle(cx: &mut VisualTestAppContext) {
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    fn click(cx: &mut VisualTestAppContext, window: AnyWindowHandle, x: f32, y: f32) {
        cx.simulate_click(window, point(px(x), px(y)), Modifiers::default());
        settle(cx);
    }

    /// The session behind the window.
    fn session(cx: &mut VisualTestAppContext, window: AnyWindowHandle) -> Entity<Session> {
        cx.update_window(window, |view, _, cx| {
            let root = view.downcast::<Root>().unwrap();
            let studio = root.read(cx).view().clone().downcast::<Studio>().unwrap();
            studio.read(cx).session().clone()
        })
        .unwrap()
    }

    pub fn the_panels_select_edit_undo_and_fold() {
        // A copy, so that nothing here can write over the example.
        let dir = std::env::temp_dir().join(format!("runity-studio-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let scene = dir.join("first-light.ron");
        std::fs::copy(SCENE, &scene).unwrap();

        let Ok(opened) = runity_studio::open(&scene) else {
            eprintln!("no renderer here; skipped");
            return;
        };
        let platform = gpui_platform::current_platform(false);
        let mut cx = VisualTestAppContext::with_asset_source(
            platform,
            Arc::new(gpui_kit::assets::AllAssets),
        );
        cx.update(runity_studio::install);
        let window = cx
            .open_offscreen_window(size(px(1440.0), px(900.0)), |window, cx| {
                runity_studio::window_root(opened, window, cx)
            })
            .unwrap()
            .into();
        settle(&mut cx);
        let doc = session(&mut cx, window);
        let boulder = cx.read(|cx| doc.read(cx).find("boulder").unwrap());
        let before = cx.read(|cx| doc.read(cx).transform(boulder).unwrap());

        // The seventh line of the Hierarchy.
        click(&mut cx, window, 70.0, 261.0);
        assert_eq!(cx.read(|cx| doc.read(cx).selection()), vec![boulder]);

        // Position X, in the Inspector: all of it, then 5, then Enter.
        click(&mut cx, window, 1243.0, 150.0);
        cx.simulate_keystrokes(window, "cmd-a");
        cx.simulate_input(window, "5");
        cx.simulate_keystrokes(window, "enter");
        settle(&mut cx);
        let moved = cx.read(|cx| doc.read(cx).transform(boulder).unwrap());
        assert_eq!(moved.position.x, 5.0, "typed into the Inspector");
        assert_eq!(
            moved.position.y, before.position.y,
            "the other boxes kept theirs"
        );
        assert!(cx.read(|cx| doc.read(cx).is_modified()));

        // The toolbar's undo.
        click(&mut cx, window, 1301.0, 20.0);
        let back = cx.read(|cx| doc.read(cx).transform(boulder).unwrap());
        assert_eq!(back.position, before.position, "one step, undone");

        // The campfire's arrow folds its five stones away.
        let lines = cx.read(|cx| doc.read(cx).hierarchy().len());
        click(&mut cx, window, 22.0, 285.0);
        assert_eq!(cx.read(|cx| doc.read(cx).hierarchy().len()), lines - 5);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
