//! Who holds a lock, seen from the editor.
//!
//! A real Git LFS lock needs an LFS server, which a test machine does not
//! have. So `git` here is a stand-in on the PATH that answers the way
//! git-lfs does — which is what the editor has to read — and records what
//! it was asked. Its own file, so changing the PATH touches no other test.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use scrap_editor::Session;

/// `std::fs::write`, the folders on the way made first: a new project has
/// only the folders its layout needs (docs/layout.md).
#[allow(dead_code)]
fn write_all(path: impl AsRef<std::path::Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

#[test]
fn the_editor_lists_locks_and_takes_and_gives_them_back() {
    let root = std::env::temp_dir().join("scrap-editor-locks");
    let _ = std::fs::remove_dir_all(&root);
    scrap::Project::create(&root, "locks").unwrap();
    let bin = root.join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("git.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{log}'\n\
         if [ \"$1 $2 $3\" = \"lfs locks --verify\" ]; then\n\
         \x20 echo '{{\"ours\":[],\"theirs\":[{{\"id\":\"4\",\"path\":\"materials/clay.scrmat\",\"owner\":{{\"name\":\"Ana\"}},\"locked_at\":\"2026-09-23T11:00:00Z\"}}]}}'\n\
         elif [ \"$1 $2\" = \"lfs locks\" ]; then\n\
         \x20 echo '[{{\"id\":\"3\",\"path\":\"assets/rock.png\",\"owner\":{{\"name\":\"Ada\"}},\"locked_at\":\"2026-09-23T10:00:00Z\"}}]'\n\
         elif [ \"$1\" = \"rev-parse\" ]; then\n\
         \x20 echo '{root}'\n\
         fi\n",
        log = log.display(),
        root = root.display()
    );
    let git = bin.join("git");
    write_all(&git, script).unwrap();
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
    session
        .open_scene(scrap::Project::open(&root).unwrap().scene("main").unwrap())
        .unwrap();
    let locks = session.locks().unwrap();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].path, "assets/rock.png");
    assert_eq!(locks[0].owner, "Ada");

    std::fs::create_dir_all(root.join("assets")).unwrap();
    session
        .set_locked(root.join("assets/tree.png"), true)
        .unwrap();
    session
        .set_locked(root.join("assets/tree.png"), false)
        .unwrap();
    let asked = std::fs::read_to_string(&log).unwrap();
    assert!(asked.contains("lfs lock tree.png"), "{asked}");
    assert!(asked.contains("lfs unlock tree.png"), "{asked}");

    // A material someone else holds is not renamed or deleted from under them.
    write_all(root.join("materials/clay.scrmat"), "(color: \"#b4643c\")\n").unwrap();
    write_all(root.join("materials/moss.scrmat"), "(color: \"#4a5a3c\")\n").unwrap();
    session.reload_assets();
    let e = session
        .rename_asset("materials/clay.scrmat", "materials/terracotta.scrmat")
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("materials/clay.scrmat is locked by Ana"),
        "{e}"
    );
    let e = session.delete_asset("materials/clay.scrmat").unwrap_err();
    assert!(e.to_string().contains("locked by Ana"), "{e}");
    assert!(root.join("materials/clay.scrmat").is_file());
    // One nobody holds moves as before.
    session
        .rename_asset("materials/moss.scrmat", "materials/lichen.scrmat")
        .unwrap();
}
