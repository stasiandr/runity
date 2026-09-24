//! Two branches edit one scene, and git merges them through `scrap merge`.
//!
//! The whole path, with real git: a project, two branches, the driver
//! configured the way `scrap git-setup` configures it (pointing at this
//! build of the command), and `git merge`. Edits to different trees on one
//! line merge; edits to the same field conflict in words, and the file
//! still loads.

use std::path::{Path, PathBuf};
use std::process::Command;

use scrap::{Project, Scene};

const SCENE: &str = "\
(
    entities: [
        // Trees, one per line.
        (id: \"00000000000000a1\", name: \"pine\",  model: \"builtin:cone\", transform: (position: (0.0, 0.0, 0.0))),
        (id: \"00000000000000b2\", name: \"birch\", model: \"builtin:cone\", transform: (position: (4.0, 0.0, 0.0))),
        (id: \"00000000000000c3\", name: \"rock\",  model: \"builtin:sphere\", material: \"stone\"),
    ],
)
";

fn git(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git runs")
}

fn ok(root: &Path, args: &[&str]) {
    let out = git(root, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with a project in it, one commit, and the driver on.
fn repository(name: &str) -> Option<(PathBuf, PathBuf)> {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: no git");
        return None;
    }
    let root = std::env::temp_dir().join(format!("scrap-merge-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, name).unwrap();
    let scene = project.scenes().join("main.ron");
    std::fs::write(&scene, SCENE).unwrap();

    ok(&root, &["init", "-q", "-b", "main"]);
    ok(&root, &["config", "user.email", "test@scrap"]);
    ok(&root, &["config", "user.name", "test"]);
    ok(&root, &["config", "merge.scrap.name", "scrap"]);
    let driver = format!("{} merge %O %A %B %P", env!("CARGO_BIN_EXE_scrap"));
    ok(&root, &["config", "merge.scrap.driver", &driver]);
    ok(&root, &["add", "-A"]);
    ok(&root, &["commit", "-q", "-m", "start"]);
    Some((root, scene))
}

fn edit(root: &Path, scene: &Path, branch: &str, from: &str, to: &str) {
    ok(root, &["checkout", "-q", branch]);
    let text = std::fs::read_to_string(scene).unwrap();
    assert!(text.contains(from), "{from} in {text}");
    std::fs::write(scene, text.replacen(from, to, 1)).unwrap();
    ok(root, &["commit", "-q", "-am", branch]);
}

#[test]
fn edits_to_different_things_merge_even_on_neighbouring_lines() {
    let Some((root, scene)) = repository("clean") else {
        return;
    };
    ok(&root, &["branch", "theirs"]);
    edit(&root, &scene, "main", "(0.0, 0.0, 0.0)", "(1.0, 0.0, 0.0)");
    edit(
        &root,
        &scene,
        "theirs",
        "(4.0, 0.0, 0.0)",
        "(4.0, 0.0, 7.0)",
    );

    ok(&root, &["checkout", "-q", "main"]);
    let out = git(&root, &["merge", "-q", "--no-edit", "theirs"]);
    assert!(
        out.status.success(),
        "a line merge would conflict here; this must not: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let merged = Scene::load(&scene).unwrap();
    assert_eq!(merged.find("pine").unwrap().transform.position.x, 1.0);
    assert_eq!(merged.find("birch").unwrap().transform.position.z, 7.0);
    let text = std::fs::read_to_string(&scene).unwrap();
    assert!(text.contains("// Trees, one per line."), "{text}");
    assert_eq!(text.lines().count(), SCENE.lines().count(), "{text}");
}

#[test]
fn the_same_field_changed_twice_conflicts_in_words_and_still_loads() {
    let Some((root, scene)) = repository("conflict") else {
        return;
    };
    ok(&root, &["branch", "theirs"]);
    edit(&root, &scene, "main", "(0.0, 0.0, 0.0)", "(1.0, 0.0, 0.0)");
    edit(
        &root,
        &scene,
        "theirs",
        "(0.0, 0.0, 0.0)",
        "(2.0, 0.0, 0.0)",
    );

    ok(&root, &["checkout", "-q", "main"]);
    let out = git(&root, &["merge", "--no-edit", "theirs"]);
    assert!(!out.status.success(), "a real conflict");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        said.contains("`pine` (00000000000000a1): both changed its position"),
        "{said}"
    );

    let text = std::fs::read_to_string(&scene).unwrap();
    assert!(!text.contains("<<<<<<<"), "{text}");
    let merged = Scene::load(&scene).expect("the conflicted file still loads");
    assert_eq!(
        merged.find("pine").unwrap().transform.position.x,
        1.0,
        "ours kept"
    );
}

#[test]
fn git_setup_turns_the_driver_on_in_a_clone() {
    let Some((root, _)) = repository("setup") else {
        return;
    };
    ok(&root, &["config", "--unset", "merge.scrap.driver"]);
    let out = Command::new(env!("CARGO_BIN_EXE_scrap"))
        .args(["git-setup"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let driver = git(&root, &["config", "merge.scrap.driver"]);
    assert_eq!(
        String::from_utf8_lossy(&driver.stdout).trim(),
        "scrap merge %O %A %B %P"
    );
    let attributes = std::fs::read_to_string(root.join(".gitattributes")).unwrap();
    assert_eq!(
        attributes.matches("merge=scrap").count(),
        2,
        "not added twice"
    );
}
