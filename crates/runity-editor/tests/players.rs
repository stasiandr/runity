//! Playing with several players from the editor: Unity's Multiplayer Play
//! Mode.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use runity::project::Engine;
use runity::Project;
use runity_editor::Session;

/// A project on this checkout's engine, with its start scene open.
fn project(name: &str) -> Option<(Session, Project)> {
    let root = std::env::temp_dir().join(format!("runity-players-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runity");
    let project = Project::create_with(
        &root,
        name,
        &Engine::Path(std::path::absolute(engine).unwrap()),
    )
    .unwrap();
    let mut session = match Session::offscreen(64, 64) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("skipping: {e}");
            return None;
        }
    };
    session
        .open_scene(project.scenes().join("main.ron"))
        .unwrap();
    Some((session, project))
}

#[test]
fn how_many_play_is_this_persons_choice_and_kept() {
    let Some((mut session, project)) = project("count") else {
        return;
    };
    assert_eq!(session.players(), 1, "one, until chosen");
    assert_eq!(session.set_players(3), 3);
    assert_eq!(session.set_players(9), runity_editor::MAX_PLAYERS);
    assert_eq!(session.set_players(0), 1);
    session.set_players(2);
    assert!(session.set_link("sluggish").is_err());
    session.set_link("poor").unwrap();

    let mut again = Session::offscreen(64, 64).unwrap();
    again.open_scene(project.scenes().join("main.ron")).unwrap();
    assert_eq!(again.players(), 2, "kept in .runity/, with the view");
    assert_eq!(again.link(), "poor");
}

/// The whole thing: the editor builds the project's game, starts it as the
/// host, then a second player from the same build, and each says it sees
/// the other. Compiles a game, so it is not in every run:
/// `cargo test -p runity-editor --test players -- --ignored`.
#[test]
#[ignore]
fn two_players_play_together_from_the_editor() {
    let Some((mut session, project)) = project("together") else {
        return;
    };
    // The game's build shares one folder between runs of this test.
    let target: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/players-game");
    std::env::set_var("CARGO_TARGET_DIR", std::path::absolute(target).unwrap());
    session.set_players(2);
    // Player 2 over a poor link: loss and jitter on real UDP.
    session.set_link("poor").unwrap();
    session.start_game().unwrap();

    let said =
        |session: &Session, text: &str| session.console().iter().any(|l| l.text.contains(text));
    let deadline = Instant::now() + Duration::from_secs(900);
    while !(said(&session, "player 1: Player 2 joined")
        && said(&session, "player 2: in the game as Player 2"))
    {
        assert!(session.game_running(), "{:#?}", session.console());
        assert!(Instant::now() < deadline, "{:#?}", session.console());
        std::thread::sleep(Duration::from_millis(200));
    }
    // Player 2 says where its world is, beside the host's report.
    let reports = project.root().join(".runity/live/main.state.player2.ron");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !reports.is_file() {
        assert!(
            Instant::now() < deadline,
            "player 2 never said where things are"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(session.stop_game());
    assert!(!session.game_running());
}
