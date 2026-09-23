//! `runity check` on a project with every kind of mistake in it.
//!
//! Each finding has to name the file and the entity and say what would fix
//! it: that is what makes it something an agent can act on rather than a
//! line to read and guess from.

use std::path::Path;

use runity::Project;
use runity_cli::{check, Finding, Severity};

const CUBE: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn project(name: &str) -> Project {
    let root = std::env::temp_dir().join(format!("runity-check-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    Project::create(&root, name).unwrap()
}

fn errors(findings: &[Finding]) -> Vec<String> {
    findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .map(ToString::to_string)
        .collect()
}

fn one_containing<'a>(lines: &'a [String], needle: &str) -> &'a String {
    let found: Vec<_> = lines.iter().filter(|l| l.contains(needle)).collect();
    assert_eq!(found.len(), 1, "one line with {needle:?} in {lines:#?}");
    found[0]
}

#[test]
fn a_fresh_project_is_clean() {
    let project = project("fresh");
    let findings = check(&project);
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn every_kind_of_dangling_name_is_found_with_a_fix() {
    let project = project("dangling");
    let root = project.root();
    write(&root.join("assets/rock.obj"), CUBE);
    write(&root.join("materials/moss.rmat"), r##"(color: "#4a5a3c")"##);
    write(
        &root.join("prefabs/campfire.prefab"),
        r#"(id: "c1", name: "campfire", model: "builtin:cube", material: "embr")"#,
    );
    runity_import::sync(&project);
    write(
        &root.join("scenes/main.ron"),
        r#"(entities: [
            (id: "a1", name: "boulder", model: "rok", material: "mos"),
            (id: "a2", name: "ball", model: "builtin:sphre"),
            (id: "a3", name: "fire", prefab: "campfir"),
            (id: "a1", name: "pasted", model: "rock", material: "moss"),
            (name: "loose", model: "rock", material: "builtin:stone"),
        ])"#,
    );

    let lines = errors(&check(&project));
    assert_eq!(lines.len(), 6, "{lines:#?}");
    let model = one_containing(&lines, "no model named `rok`");
    assert!(
        model.starts_with("error: scenes/main.ron: `boulder` (00000000000000a1)"),
        "{model}"
    );
    assert!(model.contains("did you mean `rock`?"), "{model}");
    assert!(one_containing(&lines, "`mos`").contains("did you mean `moss`?"));
    assert!(one_containing(&lines, "builtin:sphre").contains("`builtin:sphere`"));
    assert!(one_containing(&lines, "campfir`").contains("did you mean `campfire`?"));
    assert!(one_containing(&lines, "used twice").contains("`pasted`"));
    let prefab = one_containing(&lines, "`embr`");
    assert!(prefab.contains("prefabs/campfire.prefab"), "{prefab}");
    assert!(
        prefab.contains("did you mean `ember`?"),
        "a builtin counts: {prefab}"
    );

    let warnings: Vec<String> = check(&project)
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .map(ToString::to_string)
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert!(
        warnings[0].contains("1 entities have no id"),
        "{warnings:?}"
    );
}

#[test]
fn a_scene_that_does_not_parse_says_where() {
    let project = project("parse");
    write(
        &project.scenes().join("broken.ron"),
        "(entities: [(name: \"a\",",
    );
    let lines = errors(&check(&project));
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert!(
        lines[0].starts_with("error: scenes/broken.ron: 1:"),
        "line and column: {}",
        lines[0]
    );
}

#[test]
fn two_sources_with_one_name_are_one_name_too_many() {
    let project = project("clash");
    write(&project.assets().join("trees/pine.obj"), CUBE);
    write(&project.assets().join("rocks/pine.obj"), CUBE);
    let lines = errors(&check(&project));
    let clash = one_containing(&lines, "model name `pine`");
    assert!(
        clash.contains("assets/rocks/pine.obj, assets/trees/pine.obj"),
        "{clash}"
    );
}

#[test]
fn a_source_without_a_current_sidecar_is_a_warning_until_synced() {
    let project = project("sidecar");
    let source = project.assets().join("rock.obj");
    write(&source, CUBE);
    let warnings = check(&project);
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert!(warnings[0]
        .to_string()
        .contains("has no .rimport — run `runity sync`"));

    runity_import::sync(&project);
    assert!(check(&project).is_empty());

    write(&source, &CUBE.replace("v 1 0 0", "v 2 0 0"));
    let warnings = check(&project);
    assert!(
        warnings[0].to_string().contains("changed since"),
        "{warnings:?}"
    );
}
