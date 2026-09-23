//! Who holds a lock, seen from the editor.
//!
//! A real Git LFS lock needs an LFS server, which a test machine does not
//! have. So `git` here is a stand-in on the PATH that answers the way
//! git-lfs does — which is what the editor has to read — and records what
//! it was asked. Its own file, so changing the PATH touches no other test.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use runity_editor::Session;

#[test]
fn the_editor_lists_locks_and_takes_and_gives_them_back() {
    let root = std::env::temp_dir().join("runity-editor-locks");
    let _ = std::fs::remove_dir_all(&root);
    runity::Project::create(&root, "locks").unwrap();
    let bin = root.join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("git.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{}'\nif [ \"$1 $2\" = \"lfs locks\" ]; then\n  echo '[{{\"id\":\"3\",\"path\":\"assets/rock.png\",\"owner\":{{\"name\":\"Ada\"}},\"locked_at\":\"2026-09-23T10:00:00Z\"}}]'\nfi\n",
        log.display()
    );
    let git = bin.join("git");
    std::fs::write(&git, script).unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", bin.display()));

    let mut session = match Session::offscreen(64, 64) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    session.open_scene(root.join("scenes/main.ron")).unwrap();
    let locks = session.locks().unwrap();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].path, "assets/rock.png");
    assert_eq!(locks[0].owner, "Ada");

    session
        .set_locked(root.join("assets/tree.png"), true)
        .unwrap();
    session
        .set_locked(root.join("assets/tree.png"), false)
        .unwrap();
    let asked = std::fs::read_to_string(&log).unwrap();
    assert!(asked.contains("lfs lock tree.png"), "{asked}");
    assert!(asked.contains("lfs unlock tree.png"), "{asked}");
}
