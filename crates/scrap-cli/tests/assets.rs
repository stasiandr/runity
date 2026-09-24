//! `scrap rename`, seen the way a reviewer sees it: in `git diff`.
//!
//! The file shows up as a rename (same bytes, so git pairs the two), its
//! sidecar as a rename with the source line changed, and each scene line
//! that named it as a one-line change — nothing else in the project moves.

use std::path::Path;
use std::process::Command;

use scrap::Project;

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn scrap(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_scrap"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn a_rename_is_a_move_and_one_line_per_use_in_the_diff() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: no git");
        return;
    }
    let root = std::env::temp_dir().join("scrap-cli-rename");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "rename").unwrap();
    std::fs::write(
        project.materials().join("clay.scrmat"),
        "(color: \"#b4643c\")\n",
    )
    .unwrap();
    std::fs::write(
        project.scenes().join("main.ron"),
        "(\n    entities: [\n        // Pots along the wall.\n        (id: \"00000000000000a1\", name: \"pot\",  model: \"builtin:sphere\", material: \"clay\"),\n        (id: \"00000000000000a2\", name: \"lid\",  model: \"builtin:cube\",   material: \"clay\"),\n        (id: \"00000000000000a3\", name: \"wall\", model: \"builtin:cube\",   material: \"stone\"),\n    ],\n)\n",
    )
    .unwrap();
    assert!(scrap(&root, &["sync"]).status.success());
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "test@scrap"]);
    git(&root, &["config", "user.name", "test"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "start"]);

    let used = scrap(&root, &["uses", "materials/clay.scrmat"]);
    let said = String::from_utf8_lossy(&used.stdout);
    assert!(
        said.contains("scenes/main.ron: `pot` (00000000000000a1) material"),
        "{said}"
    );
    assert!(said.contains("named 2 times"), "{said}");

    let out = scrap(
        &root,
        &["rename", "materials/clay.scrmat", "materials/terracotta.scrmat"],
    );
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    git(&root, &["add", "-A"]);
    let status = git(&root, &["diff", "--cached", "-M", "--name-status"]);
    let lines: Vec<&str> = status.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "the file, its sidecar, the scene:\n{status}"
    );
    assert!(lines.contains(&"M\tscenes/main.ron"), "{status}");
    assert!(
        lines.contains(&"R100\tmaterials/clay.scrmat\tmaterials/terracotta.scrmat"),
        "{status}"
    );
    // The sidecar's `source:` line changed, so git pairs it by similarity.
    assert!(
        lines.iter().any(|l| l.starts_with('R')
            && l.ends_with("\tmaterials/clay.scrmat.scrimport\tmaterials/terracotta.scrmat.scrimport")),
        "{status}"
    );

    let diff = git(&root, &["diff", "--cached", "-U0", "--", "scenes/main.ron"]);
    let changed: Vec<&str> = diff
        .lines()
        .filter(|l| {
            (l.starts_with('+') || l.starts_with('-'))
                && !l.starts_with("+++")
                && !l.starts_with("---")
        })
        .collect();
    assert_eq!(changed.len(), 4, "two lines out, two in:\n{diff}");
    assert!(
        changed
            .iter()
            .all(|l| l.contains("\"pot\"") || l.contains("\"lid\"")),
        "{diff}"
    );

    let listed = scrap(&root, &["assets"]);
    let listed = String::from_utf8_lossy(&listed.stdout);
    assert!(
        listed.contains("material    2 used  materials/terracotta.scrmat"),
        "{listed}"
    );
    let refused = scrap(&root, &["delete", "materials/terracotta.scrmat"]);
    assert!(!refused.status.success());
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(said.contains("still used — 2 place(s)"), "{said}");
    assert!(root.join("materials/terracotta.scrmat").is_file());

    let check = scrap(&root, &["check"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
}

#[test]
fn add_writes_a_component_and_a_system_where_they_go() {
    let root = std::env::temp_dir().join("scrap-cli-add");
    let _ = std::fs::remove_dir_all(&root);
    Project::create(&root, "add").unwrap();
    let out = scrap(&root, &["add", "component", "front_door"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(root.join("src/components/front_door.rs")).unwrap();
    assert!(text.contains("pub struct FrontDoor {}"), "{text}");

    let out = scrap(&root, &["add", "system", "patrol"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("src/systems/patrol.rs").is_file());
    let main = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
    let spin = main.find("systems::spin::run(").unwrap();
    let patrol = main
        .find("profile.time(\"patrol\", || systems::patrol::run(world, seconds));")
        .unwrap();
    assert!(spin < patrol, "last in the order:\n{main}");

    let refused = scrap(&root, &["add", "component", "Front Door"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("snake_case"));
    let refused = scrap(&root, &["add", "system", "patrol"]);
    assert!(String::from_utf8_lossy(&refused.stderr).contains("already there"));
}
