//! `scrap check` on a project with every kind of mistake in it.
//!
//! Each finding has to name the file and the entity and say what would fix
//! it: that is what makes it something an agent can act on rather than a
//! line to read and guess from.

use std::path::Path;

use scrap::Project;
use scrap_cli::{check, Finding, Severity};

const CUBE: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn project(name: &str) -> Project {
    let root = std::env::temp_dir().join(format!("scrap-check-{name}"));
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
    write(&root.join("materials/moss.scrmat"), r##"(color: "#4a5a3c")"##);
    write(
        &root.join("prefabs/campfire.prefab"),
        r#"(id: "c1", name: "campfire", model: "builtin:cube", material: "embr")"#,
    );
    scrap_import::sync(&project);
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
fn two_sources_with_one_name_are_fine_until_a_line_names_them_without_an_id() {
    let project = project("clash");
    write(&project.assets().join("trees/pine.obj"), CUBE);
    write(&project.assets().join("rocks/pine.obj"), CUBE);
    scrap_import::sync(&project);
    assert!(
        !errors(&check(&project)).iter().any(|l| l.contains("pine")),
        "two assets, two IDs: nothing wrong yet"
    );

    // A line that says only `pine` cannot tell which.
    let scene = project.scenes().join("forest.ron");
    write(&scene, r#"(entities: [(name: "tree", model: "pine")])"#);
    let lines = errors(&check(&project));
    let clash = one_containing(&lines, "`pine` is the name of");
    assert!(
        clash.contains("assets/rocks/pine.obj, assets/trees/pine.obj"),
        "{clash}"
    );

    // With the ID, it can.
    let id = scrap_import::ImportSettings::load(scrap_import::sidecar_for(
        &project.assets().join("trees/pine.obj"),
    ))
    .unwrap()
    .asset_id();
    write(
        &scene,
        &format!(r#"(entities: [(name: "tree", model: ("pine", "{id}"))])"#),
    );
    assert!(!errors(&check(&project)).iter().any(|l| l.contains("pine")));
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
        .contains("has no .scrimport — run `scrap sync`"));

    scrap_import::sync(&project);
    assert!(check(&project).is_empty());

    write(&source, &CUBE.replace("v 1 0 0", "v 2 0 0"));
    let warnings = check(&project);
    assert!(
        warnings[0].to_string().contains("changed since"),
        "{warnings:?}"
    );
}

#[test]
fn a_stale_override_and_a_variant_of_itself_are_found_once_each() {
    let project = project("variants");
    let root = project.root();
    write(
        &root.join("prefabs/campfire.prefab"),
        r#"(id: "c1", name: "campfire", model: "builtin:cube",
            children: [(id: "c2", name: "ember", model: "builtin:sphere")])"#,
    );
    // A variant overriding a part the base no longer has.
    write(
        &root.join("prefabs/mossy.prefab"),
        r#"(id: "d1", name: "mossy", prefab: "campfire", overrides: { "c9": (material: "moss") })"#,
    );
    write(
        &root.join("prefabs/loop.prefab"),
        r#"(id: "e1", name: "loop", prefab: "loop")"#,
    );
    write(
        &root.join("scenes/main.ron"),
        r#"(entities: [
            (id: "a1", name: "west", prefab: "mossy"),
            (id: "a2", name: "east", prefab: "mossy"),
            (id: "a3", name: "fire", prefab: "campfire", overrides: { "c8": (material: "bark") }),
        ])"#,
    );
    let errors = errors(&check(&project));
    let stale = one_containing(&errors, "part 00000000000000c9");
    assert!(stale.contains("prefabs/mossy.prefab:"), "{stale}");
    let scene = one_containing(&errors, "part 00000000000000c8");
    assert!(scene.contains("scenes/main.ron:"), "{scene}");
    assert!(scene.contains("`fire`"), "{scene}");
    let looped = one_containing(&errors, "a prefab containing itself?");
    assert!(looped.contains("prefabs/loop.prefab:"), "{looped}");
}

#[test]
fn a_component_scenes_name_is_a_file_in_src_components() {
    let project = project("components");
    let root = project.root();
    write(
        &root.join("scenes/doors.ron"),
        r#"(entities: [
            (id: "a1", name: "gate", model: "builtin:cube", components: { "spn": (degrees_per_second: 10.0) }),
            (id: "a2", name: "wheel", model: "builtin:cube", components: { "spin": (degrees_per_second: 10.0) }),
        ])"#,
    );
    let errors = errors(&check(&project));
    let line = one_containing(&errors, "no component `spn`");
    assert!(line.contains("did you mean `spin`?"), "{line}");
    assert!(line.contains("`gate`"), "{line}");

    scrap_cli::add::component(&project, "door").unwrap();
    write(
        &root.join("scenes/doors.ron"),
        r#"(entities: [(id: "a1", name: "gate", model: "builtin:cube", components: { "door": () })])"#,
    );
    assert!(errors_of(&project).is_empty(), "{:#?}", errors_of(&project));
}

fn errors_of(project: &Project) -> Vec<String> {
    errors(&check(project))
}

#[test]
fn a_joint_to_nothing_is_named() {
    let project = project("joints");
    write(
        &project.root().join("scenes/door.ron"),
        r#"(entities: [
            (id: "a1", name: "frame", model: "builtin:cube", body: Static),
            (id: "a2", name: "door", model: "builtin:cube", body: Dynamic, joint: Hinge(to: "a1")),
            (id: "a3", name: "flap", model: "builtin:cube", body: Dynamic, joint: Hinge(to: "a9")),
        ])"#,
    );
    let errors = errors(&check(&project));
    let line = one_containing(&errors, "its joint hangs from");
    assert!(
        line.contains("`flap`") && line.contains("00000000000000a9"),
        "{line}"
    );
}

#[test]
fn a_layer_no_file_names_is_found() {
    let project = project("layers");
    write(
        &project.root().join("scenes/shards.ron"),
        r#"(entities: [
            (id: "a1", name: "shard", model: "builtin:cube", body: Dynamic, layer: "debris"),
            (id: "a2", name: "hero", model: "builtin:cube", body: Dynamic, layer: "playr"),
        ])"#,
    );
    let errors = errors(&check(&project));
    let line = one_containing(&errors, "no layer");
    assert!(
        line.contains("`playr`") && line.contains("did you mean `player`?"),
        "{line}"
    );
    write(
        &project.root().join("layers.ron"),
        r#"(layers: ["x", "x"])"#,
    );
    let errors = errors_of(&project);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("layers.ron") && e.contains("named twice")),
        "{errors:#?}"
    );
}

#[test]
fn a_screen_with_two_elements_of_one_id_is_found() {
    let project = project("screens");
    assert!(
        project.root().join("ui/hud.ron").is_file(),
        "a new project has a HUD"
    );
    write(
        &project.root().join("ui/menu.ron"),
        r#"(elements: [
            (id: "play", size: (200, 40), kind: Button("Play")),
            (id: "play", size: (200, 40), kind: Button("Again")),
        ])"#,
    );
    let errors = errors(&check(&project));
    let line = one_containing(&errors, "is the id of two elements");
    assert!(line.contains("ui/menu.ron"), "{line}");
}

#[test]
fn a_word_a_language_lacks_is_listed() {
    let project = project("strings");
    assert!(errors_of(&project).is_empty(), "{:#?}", errors_of(&project));
    write(&project.root().join("strings/ru.ron"), r#"{}"#);
    let errors = errors_of(&project);
    let line = one_containing(&errors, "no `hud.quit`");
    assert!(line.contains("strings/ru.ron"), "{line}");
}

#[test]
fn the_game_settings_name_a_scene_and_a_language_that_are_there() {
    let project = project("settings");
    let manifest = project.root().join("scrap.ron");
    let text = std::fs::read_to_string(&manifest).unwrap();
    assert!(
        text.contains("start_scene: \"main\""),
        "the knobs are in the file: {text}"
    );
    write(
        &manifest,
        &text
            .replace("start_scene: \"main\"", "start_scene: \"mian\"")
            .replace("language: \"en\"", "language: \"ru\""),
    );
    let project = Project::open(project.root()).unwrap();
    let lines = errors(&check(&project));
    let scene = one_containing(&lines, "start_scene");
    assert!(scene.contains("did you mean `main`?"), "{scene}");
    one_containing(&lines, "language `ru` has no strings/ru.ron");

    let (name, settings) =
        scrap::project::GameSettings::load(&project.root().to_string_lossy()).unwrap();
    assert_eq!(name, "settings");
    assert_eq!(settings.start_scene, "mian");
    assert!((settings.fixed_delta() - 1.0 / 60.0).abs() < 1e-6);
}

#[test]
fn a_file_outside_the_layout_is_named_with_where_it_goes() {
    let project = project("layout");
    write(&project.root().join("rock.obj"), CUBE);
    write(&project.root().join("notes.txt"), "todo");
    let findings = check(&project);
    let warnings: Vec<String> = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .map(ToString::to_string)
        .collect();
    one_containing(&warnings, "`rock.obj` is outside the layout");
    assert!(one_containing(&warnings, "rock.obj").contains("assets/"));
    one_containing(&warnings, "`notes.txt` is not part of the project layout");
    assert!(
        errors(&findings).is_empty(),
        "warnings, not errors: {findings:#?}"
    );
}

#[test]
fn a_component_value_that_does_not_fit_the_game_s_type_is_found() {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Spin {
        degrees_per_second: f32,
    }
    let project = project("shapes");
    let mut components = scrap::Components::new();
    components.register::<Spin>("spin");
    components
        .write_shapes(project.root().join(scrap::project::SHAPES))
        .unwrap();
    assert!(
        errors(&check(&project)).is_empty(),
        "the starter scene fits"
    );
    let main = project.scenes().join("main.ron");
    let text = std::fs::read_to_string(&main).unwrap();
    write(
        &main,
        &text.replace("degrees_per_second: 45.0", "degrees_per_secnd: 45.0"),
    );
    let lines = errors(&check(&project));
    let line = one_containing(&lines, "degrees_per_secnd");
    assert!(
        line.contains("did you mean `degrees_per_second`?") && line.contains("`cube`"),
        "{line}"
    );
}

#[test]
fn a_component_linking_an_asset_that_is_not_there_is_named() {
    let project = project("typed-links");
    write(
        &project.prefabs().join("campfire.prefab"),
        r#"(name: "campfire")"#,
    );
    let scene = project.scenes().join("camp.ron");
    write(
        &scene,
        r#"(entities: [(name: "spawner", components: { "spawner": (what: PrefabLink("campfir")) })])"#,
    );
    let lines = errors(&check(&project));
    let found = one_containing(&lines, "links to prefab `campfir`");
    assert!(found.contains("campfire"), "{found}");
    write(
        &scene,
        r#"(entities: [(name: "spawner", components: { "spawner": (what: PrefabLink("campfire")) })])"#,
    );
    assert!(!errors(&check(&project))
        .iter()
        .any(|l| l.contains("links to")));
}

#[test]
fn a_dialogue_that_does_not_join_up_is_found() {
    let project = project("dialogue");
    write(
        &project.root().join("dialogues/captain.ron"),
        r#"(start: "hello", lines: {
            "hello": (text: "Ahoy.", next: "ask"),
            "lost": (text: "?"),
        })"#,
    );
    let found = errors(&check(&project));
    let line = one_containing(&found, "leads to `ask`");
    assert!(line.contains("dialogues/captain.ron"), "{line}");
    one_containing(&found, "`lost` is never reached");
}

#[test]
fn an_animators_cases_are_played_by_check() {
    let project = project("animator-cases");
    write(
        &project.root().join("animators/hero.ron"),
        r#"(start: "idle", states: {
            "idle": (clip: "idle", transitions: [(to: "walk", when: [Above("speed", 0.1)])]),
            "walk": (clip: "walk"),
        })"#,
    );
    write(
        &project.root().join("animators/hero.cases.ron"),
        r#"(cases: [
            (name: "walks", steps: [(set: {"speed": 1.0}, expect: "walk")]),
            (name: "wrong", steps: [(set: {"speed": 1.0}, expect: "idle")]),
        ])"#,
    );
    let found = errors(&check(&project));
    let line = one_containing(&found, "case `wrong`");
    assert!(
        line.contains("hero.cases.ron") && line.contains("in `walk`"),
        "{line}"
    );
}

#[test]
fn a_field_no_module_reads_is_kept_and_named_and_one_that_does_not_fit_is_an_error() {
    // DNA, postulate 3: a disabled module's component is not lost. A field
    // this build has no module for — or a typo — stays in the file as
    // written, and check says so; a field it does read must fit.
    let project = project("parts");
    let root = project.root();
    let text = r#"(sun: (hour: 9.0), glare: 2.0, entities: [
            (id: "a1", name: "raft", model: "builtin:cube", buoyancy: (floats: true), lihgt: (range: 3.0)),
            (id: "a2", name: "lamp", model: "builtin:cube", light: "bright"),
        ])"#;
    write(&root.join("scenes/main.ron"), text);
    let findings = check(&project);
    let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
    let buoyancy = one_containing(&lines, "`buoyancy`");
    assert!(
        buoyancy.contains("`raft`") && buoyancy.contains("no module"),
        "{buoyancy}"
    );
    assert!(
        one_containing(&lines, "`lihgt`").contains("did you mean `light`"),
        "{lines:#?}"
    );
    assert!(
        one_containing(&lines, "`glare`").contains("the scene"),
        "{lines:#?}"
    );
    let errors = errors(&findings);
    assert!(
        one_containing(&errors, "`lamp`").contains("`light` does not read"),
        "{errors:#?}"
    );
    // And the scene keeps what it did not understand, byte for byte.
    let scene = scrap::Scene::load(root.join("scenes/main.ron")).unwrap();
    assert_eq!(
        scene.entities[0].parts.raw("buoyancy"),
        Some("(floats: true)")
    );
    scene.save(root.join("scenes/main.ron")).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("scenes/main.ron"))
            .unwrap()
            .trim(),
        text.trim()
    );
}

#[test]
fn the_modules_scrap_ron_lists_and_what_cargo_builds_are_held_together() {
    let root = project("modules").root().to_path_buf();
    let manifest = root.join("scrap.ron");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let listed = text.trim_end().trim_end_matches(')').to_string()
        + "    modules: [\"render\", \"physics\", \"shel\"],\n)\n";
    write(&manifest, &listed);
    let said = errors(&check(&Project::open(&root).unwrap()));
    one_containing(&said, "did you mean `shell`");

    write(&manifest, &listed.replace("\"shel\"", "\"shell\""));
    let project = Project::open(&root).unwrap();
    // A new game asks for the shell and sound; the list says physics too.
    let said = errors(&check(&project));
    one_containing(&said, "scrap modules sync");
    let features = scrap_cli::modules::sync(&project).unwrap();
    assert_eq!(features, ["desktop-shell", "physics"]);
    assert!(errors(&check(&project)).is_empty());
}

#[test]
fn a_tuning_record_that_does_not_fit_the_game_s_type_is_found() {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Enemy {
        hp: u32,
        #[serde(default)]
        speed: f32,
    }
    let project = project("tuning-records");
    let mut components = scrap::Components::new();
    components.register_tuning::<std::collections::BTreeMap<String, Enemy>>("enemies");
    components
        .write_shapes(project.root().join(scrap::project::SHAPES))
        .unwrap();
    let file = project.root().join("tuning/enemies.ron");
    write(
        &file,
        r#"{ "goblin": (hp: 10, speed: 2.5), "orc": (hp: 30) }"#,
    );
    assert!(errors(&check(&project)).is_empty(), "the records fit");
    write(
        &file,
        r#"{ "goblin": (hp: 10, sped: 2.5), "orc": (hp: "lots") }"#,
    );
    let found = errors(&check(&project));
    let typo = one_containing(&found, "record `goblin`");
    assert!(typo.contains("tuning/enemies.ron"), "{typo}");
    assert!(typo.contains("did you mean `speed`?"), "{typo}");
    let wrong = one_containing(&found, "record `orc`");
    assert!(wrong.contains("whole number"), "{wrong}");
}
