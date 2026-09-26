//! `scrap check`: what in a project does not resolve, said so it can be
//! fixed.
//!
//! The engine is forgiving at run time on purpose — a model nobody has is
//! an entity with nothing drawn, a material typo is grey — because a scene
//! that refuses to open is worse than one with a visible mistake. The price
//! is that a mistake can sit unnoticed. This is where it gets noticed: on
//! the command line, in CI, and by an agent that has just written a scene
//! and wants to know whether it holds together (DNA, postulate 5: errors an
//! agent can act on).
//!
//! Every finding names the file, the entity (by name and ID), what is wrong,
//! and what would fix it, closest name included.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use scrap::scene::MaterialRef;
use scrap::{EntityDesc, EntityId, Project, Scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something that will not be what the file says: a model not drawn, a
    /// colour gone grey, an instance of nothing.
    Error,
    /// Something that works today and will cost later: a sidecar that does
    /// not match its source, an entity without an ID.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    /// Project-relative, forward slashes.
    pub file: String,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{severity}: {}: {}", self.file, self.message)
    }
}

/// Every name a scene can use, and where each comes from.
struct Names {
    /// Model name → the source files that would build an asset of that name.
    models: HashMap<String, Vec<String>>,
    materials: HashMap<String, Vec<String>>,
    prefabs: HashSet<String>,
    /// Every model's and prefab's ID, from their sidecars: a link with one
    /// of these is found whatever name it still says.
    ids: HashSet<scrap::AssetId>,
    /// Names typed links can name that the lists above do not have.
    sounds: HashSet<String>,
    textures: HashSet<String>,
    scenes: HashSet<String>,
    /// The graphs in `animators/`, by name.
    animators: HashSet<String>,
    /// Each graph's parameters, by its name: what a wire may pull.
    parameters: HashMap<String, std::collections::BTreeSet<String>>,
    /// The game's components, from `src/components/`; `None` when the
    /// project has no such folder and only its code knows.
    components: Option<Vec<String>>,
    /// The collision layers, from `layers.ron`.
    layers: scrap::layers::Layers,
    /// What the game's components look like, as the game last wrote them.
    shapes: std::collections::BTreeMap<String, scrap::shape::Shape>,
}

const MODEL_SOURCES: [&str; 6] = ["gltf", "glb", "obj", "scrterrain", "scrpoly", "scrbrush"];

/// Look the whole project over.
pub fn check(project: &Project) -> Vec<Finding> {
    let mut out = Vec::new();
    let names = names(project, &mut out);

    let prefab_dir = project.prefabs();
    for path in files(&prefab_dir, "prefab") {
        let file = relative(project, &path);
        match scrap::Prefabs::read(&path) {
            Ok(_) => {
                // Read raw as well: `read` fills missing IDs in, which is
                // what is being checked.
                if let Some(desc) = parse::<EntityDesc>(&path, &file, &mut out) {
                    check_entities(std::slice::from_ref(&desc), &file, &names, &mut out);
                }
            }
            Err(e) => out.push(error(&file, e)),
        }
    }

    for path in files(&project.scenes(), "ron") {
        let file = relative(project, &path);
        if let Some(scene) = parse::<Scene>(&path, &file, &mut out) {
            check_entities(&scene.entities, &file, &names, &mut out);
            check_parts(
                &scene.parts,
                "the scene",
                &file,
                &scrap::scene::part_kinds(),
                &mut out,
            );
        }
    }

    expansion(project, &mut out);
    animators(project, &mut out);
    shaders(project, &mut out);

    let input = project.root().join(scrap::project::INPUT);
    if input.is_file() {
        match scrap::Actions::load(&input) {
            Ok(actions) => {
                for problem in actions.map.problems() {
                    out.push(error(scrap::project::INPUT, problem));
                }
            }
            Err(e) => out.push(error(scrap::project::INPUT, e)),
        }
    }

    let mut keys: Vec<String> = Vec::new();
    for path in files(&project.root().join(scrap::project::UI), "ron") {
        let file = relative(project, &path);
        if let Some(layout) = parse::<scrap::screen::Layout>(&path, &file, &mut out) {
            for problem in layout.problems() {
                out.push(error(&file, problem));
            }
            keys.extend(scrap::screen::Screen::keys(&layout));
        }
    }
    // Dialogues and quests: what does not join up, their cases played,
    // the flags between them, and their texts' keys, checked against
    // strings/ with the screens'.
    keys.extend(dialogues(project, &mut out));
    let strings = project.root().join(scrap::strings::DIR);
    let mut tables = Vec::new();
    for (language, path) in scrap::strings::tables(&strings) {
        let file = relative(project, &path);
        if let Some(table) = parse::<scrap::strings::Table>(&path, &file, &mut out) {
            tables.push((language, table));
        }
    }
    if tables.is_empty() && !keys.is_empty() {
        out.push(error(
            scrap::strings::DIR,
            format!(
                "screens ask for `@{}` and there is no language in strings/ to say it",
                keys[0]
            ),
        ));
    }
    unused_strings(project, &tables, &keys, &mut out);
    for missing in scrap::strings::missing(&tables, &keys) {
        let (language, rest) = missing.split_once(": ").unwrap_or(("", &missing));
        out.push(error(
            &format!("{}/{language}.ron", scrap::strings::DIR),
            rest,
        ));
    }

    // The game starts on a scene and speaks a language that are there.
    let game = &project.manifest().game;
    let scenes = project.scene_names();
    if !scenes.contains(&game.start_scene) {
        out.push(error(
            scrap::project::FILE,
            format!(
                "start_scene `{}` is not in scenes/{}",
                game.start_scene,
                suggest(&game.start_scene, scenes.iter().map(String::as_str))
            ),
        ));
    }
    let languages: Vec<String> = tables.iter().map(|(l, _)| l.clone()).collect();
    if !languages.is_empty() && !languages.contains(&game.language) {
        out.push(error(
            scrap::project::FILE,
            format!(
                "language `{}` has no strings/{}.ron{}",
                game.language,
                game.language,
                suggest(&game.language, languages.iter().map(String::as_str))
            ),
        ));
    }
    if game.steps_per_second == 0 || game.width == 0 || game.height == 0 {
        out.push(error(
            scrap::project::FILE,
            "steps_per_second, width and height are more than 0",
        ));
    }

    // The numbers are RON, and fit what the game reads them as when it
    // has said (library/tuning.ron): a misspelt field of one record is
    // named with the record and the nearest field there is.
    let tuning = project.root().join(scrap::project::TUNING);
    let shapes: std::collections::BTreeMap<String, scrap::shape::Shape> =
        std::fs::read_to_string(project.root().join(scrap::project::TUNING_SHAPES))
            .ok()
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default();
    for path in files(&tuning, "ron") {
        let file = relative(project, &path);
        let Some(_) = parse::<ron::Value>(&path, &file, &mut out) else {
            continue;
        };
        let name = path
            .strip_prefix(&tuning)
            .unwrap_or(&path)
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        let (Some(shape), Ok(text)) = (shapes.get(&name), std::fs::read_to_string(&path)) else {
            continue;
        };
        for (record, problem) in scrap::records::problems(Some(shape), &text) {
            out.push(error(
                &file,
                match record {
                    Some(record) => format!("record `{record}`: {problem}"),
                    None => problem,
                },
            ));
        }
    }

    // The modules scrap.ron lists hold together, and Cargo.toml builds
    // the engine with them.
    for problem in crate::modules::problems(project) {
        out.push(error(scrap::project::FILE, problem));
    }

    check_sidecars(project, &mut out);
    check_layout(project, &mut out);
    out.sort_by(|a, b| (a.severity, &a.file).cmp(&(b.severity, &b.file)));
    out
}

/// What is at the top of the project that the layout has no place for
/// (DNA, postulate 7): a model dropped next to `scrap.ron` is a model no
/// tool looks for. A warning with where it goes, when that can be told.
fn check_layout(project: &Project, out: &mut Vec<Finding>) {
    use scrap::project::*;
    let known = [
        FILE,
        SCENES,
        PREFABS,
        MATERIALS,
        ASSETS,
        LIBRARY,
        SRC,
        UI,
        INPUT,
        TUNING,
        ANIMATORS,
        SHADERS,
        scrap::layers::FILE,
        scrap::strings::DIR,
        scrap::dialogue::DIR,
        scrap::quest::DIR,
        scrap::motion::DIR,
        crate::perf::BUDGETS,
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "target",
        "build",
        "CLAUDE.md",
        "README.md",
        "LICENSE",
    ];
    let Ok(entries) = std::fs::read_dir(project.root()) else {
        return;
    };
    let mut strays: Vec<(String, bool)> = entries
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                e.path().is_dir(),
            )
        })
        .filter(|(name, _)| !name.starts_with('.') && !known.contains(&name.as_str()))
        .collect();
    strays.sort();
    for (name, dir) in strays {
        let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let home = match extension.as_deref() {
            _ if dir => None,
            Some("prefab") => Some(PREFABS),
            Some("scrmat") => Some(MATERIALS),
            Some(
                "gltf" | "glb" | "obj" | "fbx" | "png" | "jpg" | "jpeg" | "wav" | "ogg" | "mp3"
                | "flac" | "scrterrain" | "scrpoly" | "scrbrush",
            ) => Some(ASSETS),
            Some("ron") => Some(SCENES),
            _ => None,
        };
        let message = match home {
            Some(home) => format!(
                "`{name}` is outside the layout, where no tool looks for it — it goes in {home}/"
            ),
            None => format!(
                "`{name}` is not part of the project layout; nothing reads it (see the layout in CLAUDE.md)"
            ),
        };
        out.push(Finding {
            severity: Severity::Warning,
            file: name.clone(),
            message,
        });
    }
}

/// What only expanding prefabs finds: an override for a part the prefab no
/// longer has, and a variant that is, some levels down, a variant of
/// itself. A missing prefab is not repeated here; the line naming it was
/// already reported.
///
/// Prefab files first, each placed once on its own, so a variant's stale
/// override is reported against the variant — and then not again for every
/// scene that places it.
fn expansion(project: &Project, out: &mut Vec<Finding>) {
    let (prefabs, _) = scrap::Prefabs::of(project);
    let mut said: HashSet<String> = HashSet::new();
    let mut report = |file: &str, done: scrap::Instanced, out: &mut Vec<Finding>| {
        for problem in done.problems {
            if problem.reason == "no prefab by that name" {
                continue;
            }
            let message = format!(
                "`{}` (an instance of `{}`): {}",
                problem.entity_name, problem.prefab, problem.reason
            );
            if said.insert(message.clone()) {
                out.push(error(file, message));
            }
        }
    };
    for path in files(&project.prefabs(), "prefab") {
        let file = relative(project, &path);
        let Some(name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let alone = Scene {
            entities: vec![EntityDesc {
                name: name.clone(),
                prefab: name.into(),
                ..EntityDesc::default()
            }],
            ..Scene::default()
        };
        report(&file, scrap::instantiate(&alone, &prefabs), out);
    }
    for path in files(&project.scenes(), "ron") {
        let file = relative(project, &path);
        if let Ok(scene) = Scene::load(&path) {
            let done = scrap::instantiate(&scene, &prefabs);
            for link in done.scene.broken_links() {
                out.push(error(
                    &file,
                    format!(
                        "`{}` ({}): `{}` links to {}, which is not in the scene",
                        link.holder_name, link.holder, link.component, link.target
                    ),
                ));
            }
            report(&file, done, out);
        }
    }
}

fn names(project: &Project, out: &mut Vec<Finding>) -> Names {
    let mut models: HashMap<String, Vec<String>> = HashMap::new();
    let mut materials: HashMap<String, Vec<String>> = HashMap::new();
    for root in [project.assets(), project.materials()] {
        scrap_import::walk(&root, &mut |path| {
            let (Some(stem), Some(extension)) = (
                path.file_stem().map(|s| s.to_string_lossy().into_owned()),
                path.extension().map(|e| e.to_string_lossy().to_lowercase()),
            ) else {
                return;
            };
            let file = relative(project, path);
            if MODEL_SOURCES.contains(&extension.as_str()) {
                models.entry(stem).or_default().push(file);
            } else if extension == "scrmat" {
                materials.entry(stem).or_default().push(file);
            }
        });
    }
    // Materials are still named, not linked (docs/refs.md): two with one
    // name are one name with two meanings.
    let mut clashes: Vec<_> = materials
        .iter()
        .filter(|(_, files)| files.len() > 1)
        .collect();
    clashes.sort();
    for (name, files) in clashes {
        let mut files = files.clone();
        files.sort();
        out.push(error(
            &files[0],
            format!(
                "material name `{name}` is used by {} — scenes cannot tell them apart; rename all but one",
                files.join(", ")
            ),
        ));
    }
    let prefab_files = files(&project.prefabs(), "prefab");
    let prefabs = prefab_files
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    let mut ids = HashSet::new();
    let mut sources = prefab_files;
    for root in [project.assets(), project.materials()] {
        scrap_import::walk(&root, &mut |path| sources.push(path.to_path_buf()));
    }
    let scene_files = files(&project.scenes(), "ron");
    sources.extend(scene_files.iter().cloned());
    let (mut sounds, mut textures) = (HashSet::new(), HashSet::new());
    scrap_import::walk(&project.assets(), &mut |path| {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        if let (Some(stem), Some(extension)) = (stem, extension) {
            match extension.as_str() {
                "wav" | "mp3" | "ogg" | "flac" => {
                    sounds.insert(stem);
                }
                "png" | "jpg" | "jpeg" | "tga" | "bmp" => {
                    textures.insert(stem);
                }
                _ => {}
            }
        }
    });
    let scenes: HashSet<String> = scene_files
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    for source in sources {
        if let Some(id) = scrap::asset::sidecar_id(scrap::asset::sidecar_of(&source)) {
            ids.insert(id);
        }
    }
    let graphs: Vec<PathBuf> = files(&project.root().join(scrap::project::ANIMATORS), "ron")
        .into_iter()
        .filter(|p| !p.to_string_lossy().ends_with(".cases.ron"))
        .collect();
    let animators = graphs
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    // Read quietly: `animators` says what does not read.
    let parameters = graphs
        .iter()
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().into_owned();
            let text = std::fs::read_to_string(p).ok()?;
            let graph: scrap::animgraph::Graph = ron::from_str(&text).ok()?;
            Some((name, graph.parameters()))
        })
        .collect();
    Names {
        animators,
        parameters,
        models,
        materials,
        prefabs,
        ids,
        sounds,
        textures,
        scenes,
        components: project.component_names(),
        shapes: std::fs::read_to_string(project.root().join(scrap::project::SHAPES))
            .ok()
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default(),
        layers: match scrap::layers::Layers::of(project) {
            Ok(layers) => layers,
            Err(e) => {
                out.push(error(scrap::layers::FILE, e));
                scrap::layers::Layers::default()
            }
        },
    }
}

fn check_entities(entities: &[EntityDesc], file: &str, names: &Names, out: &mut Vec<Finding>) {
    // Every id in the file, for joints to be checked against: a joint names
    // a body in the same file.
    let mut all: HashSet<EntityId> = HashSet::new();
    // And by id, for wires: a wire names a thing in the same file too.
    let mut by_id: HashMap<EntityId, &EntityDesc> = HashMap::new();
    let mut walk: Vec<&EntityDesc> = entities.iter().collect();
    while let Some(e) = walk.pop() {
        all.insert(e.id);
        by_id.insert(e.id, e);
        walk.extend(e.children.iter());
    }
    let mut seen: HashSet<EntityId> = HashSet::new();
    let mut unnamed = 0;
    let mut stack: Vec<&EntityDesc> = entities.iter().rev().collect();
    while let Some(entity) = stack.pop() {
        stack.extend(entity.children.iter().rev());
        let who = if entity.id.is_unassigned() {
            unnamed += 1;
            format!("`{}`", entity.name)
        } else {
            if !seen.insert(entity.id) {
                out.push(error(
                    file,
                    format!(
                        "`{}`: id {} is used twice — a copied block? delete the id line of the copy and it gets a new one",
                        entity.name, entity.id
                    ),
                ));
            }
            format!("`{}` ({})", entity.name, entity.id)
        };

        if !entity.prefab.is_empty() {
            let by_id = entity.prefab.id.is_some_and(|id| names.ids.contains(&id));
            if !by_id && !names.prefabs.contains(entity.prefab.as_str()) {
                out.push(error(
                    file,
                    format!(
                        "{who}: no prefab named `{}` in prefabs/{}",
                        entity.prefab,
                        suggest(&entity.prefab, names.prefabs.iter().map(String::as_str))
                    ),
                ));
            }
        } else if !entity.model().is_empty() {
            check_model(&entity.model(), &who, file, names, out);
        }
        if let Some(along) = &entity.along() {
            check_model(
                &along.model,
                &format!("{who} (along its spline)"),
                file,
                names,
                out,
            );
        }
        if !entity.animator().is_empty() && !names.animators.contains(&entity.animator()) {
            out.push(error(
                file,
                format!(
                    "{who}: animator `{}` is not a graph in animators/{}",
                    entity.animator(),
                    suggest(
                        &entity.animator(),
                        names.animators.iter().map(String::as_str)
                    )
                ),
            ));
        }
        if let Some(sound) = &entity.sound() {
            let clip = &sound.clip;
            let by_id = clip.id.is_some_and(|id| names.ids.contains(&id));
            if clip.is_empty() {
                out.push(error(file, format!("{who}: a sound with no clip")));
            } else if !by_id && !names.sounds.contains(clip.as_str()) {
                out.push(error(
                    file,
                    format!(
                        "{who}: its sound plays `{clip}`, and there is no such sound in assets/{}",
                        suggest(clip, names.sounds.iter().map(String::as_str))
                    ),
                ));
            }
        }
        // A game component's links to assets.
        for (component, value) in &entity.components {
            for (kind, link) in scrap::refs::links_in(value.get_ron()) {
                if link.is_empty() || link.id.is_some_and(|id| names.ids.contains(&id)) {
                    continue;
                }
                let known: Vec<&str> = match kind {
                    "model" => names
                        .models
                        .keys()
                        .map(String::as_str)
                        .chain(scrap::builtin::NAMES.iter().copied())
                        .collect(),
                    "material" => names.materials.keys().map(String::as_str).collect(),
                    "prefab" => names.prefabs.iter().map(String::as_str).collect(),
                    "sound" => names.sounds.iter().map(String::as_str).collect(),
                    "texture" => names.textures.iter().map(String::as_str).collect(),
                    _ => names.scenes.iter().map(String::as_str).collect(),
                };
                if !known.contains(&link.as_str()) {
                    out.push(error(
                        file,
                        format!(
                            "{who}: `{component}` links to {kind} `{link}`, which is not there{}",
                            suggest(&link, known.iter().copied())
                        ),
                    ));
                }
            }
        }
        if let MaterialRef::Named(link) = &entity.material_ref() {
            // Found by its ID, whatever name the line still says.
            if !link.id.is_some_and(|id| names.ids.contains(&id)) {
                check_material(link, &who, file, names, out);
            }
        }
        let layers: Vec<String> = std::iter::once(entity.layer())
            .chain(entity.overrides.values().filter_map(|o| o.layer()))
            .collect();
        for layer in layers.iter().filter(|l| !l.is_empty()) {
            if names.layers.index(layer).is_none() {
                out.push(error(
                    file,
                    format!(
                        "{who}: no layer `{layer}` in layers.ron{}",
                        suggest(layer, names.layers.layers.iter().map(String::as_str))
                    ),
                ));
            }
        }
        for problem in scrap::wires::problems(
            entity,
            |id| by_id.get(&id).copied(),
            |graph| names.parameters.get(graph).cloned(),
            |link| {
                link.id.is_some_and(|id| names.ids.contains(&id))
                    || names.prefabs.contains(link.as_str())
            },
        ) {
            out.push(error(file, format!("{who}: {problem}")));
        }
        for wire in entity.wires() {
            if !wire.only.is_empty() && names.layers.index(&wire.only).is_none() {
                out.push(error(
                    file,
                    format!(
                        "{who}: a wire only for layer `{}`, which is not in layers.ron{}",
                        wire.only,
                        suggest(&wire.only, names.layers.layers.iter().map(String::as_str))
                    ),
                ));
            }
        }
        if let Some(to) = entity.joint().to().filter(|to| !to.is_unassigned()) {
            if !all.contains(&to) {
                out.push(error(
                    file,
                    format!("{who}: its joint hangs from {to}, which is not in this file — the body it holds on to has to be"),
                ));
            }
        }
        let values = entity
            .components
            .iter()
            .chain(entity.overrides.values().flat_map(|o| o.components.iter()));
        for (component, value) in values {
            if let Some(shape) = names.shapes.get(component) {
                for problem in shape.problems(value.get_ron()) {
                    out.push(error(file, format!("{who}: `{component}`: {problem}")));
                }
            }
        }
        if let Some(known) = &names.components {
            let used = entity
                .components
                .keys()
                .chain(entity.overrides.values().flat_map(|o| o.components.keys()));
            for component in used {
                if !known.contains(component) {
                    out.push(error(
                        file,
                        format!(
                            "{who}: no component `{component}` — a component is a file in src/components/ named what scenes call it{}",
                            suggest(component, known.iter().map(String::as_str))
                        ),
                    ));
                }
            }
        }
    }
    if unnamed > 0 {
        out.push(Finding {
            severity: Severity::Warning,
            file: file.to_string(),
            message: format!(
                "{unnamed} entities have no id — they get one on load, and it is written on the next save from the editor"
            ),
        });
    }
    // Every module field on every line, and in every override: one no
    // module of this build reads is kept as written and named here; one
    // whose text does not fit its field is an error.
    let kinds = scrap::scene::part_kinds();
    let mut stack: Vec<&EntityDesc> = entities.iter().rev().collect();
    while let Some(entity) = stack.pop() {
        stack.extend(entity.children.iter().rev());
        let who = format!("`{}`", entity.name);
        check_parts(&entity.parts, &who, file, &kinds, out);
        for (part, change) in &entity.overrides {
            check_parts(
                &change.parts,
                &format!("{who}, override of {part}"),
                file,
                &kinds,
                out,
            );
        }
    }
}

/// A line's module fields against the fields this build's modules read.
fn check_parts(
    parts: &scrap::parts::Parts,
    who: &str,
    file: &str,
    kinds: &[scrap::parts::PartKind],
    out: &mut Vec<Finding>,
) {
    for (name, text) in parts.iter() {
        match kinds.iter().find(|k| k.name == name) {
            Some(kind) => {
                if let Err(e) = (kind.check)(text) {
                    out.push(error(file, format!("{who}: `{name}` does not read: {e}")));
                }
            }
            None => out.push(Finding {
                severity: Severity::Warning,
                file: file.to_string(),
                message: format!(
                    "{who}: `{name}` is a field no module of this build reads — kept as written; a typo, or a module switched off?{}",
                    suggest(name, kinds.iter().map(|k| k.name))
                ),
            }),
        }
    }
}

fn check_model(
    link: &scrap::AssetLink,
    who: &str,
    file: &str,
    names: &Names,
    out: &mut Vec<Finding>,
) {
    let model = link.as_str();
    // Found by its ID, whatever name the line still says (docs/refs.md).
    if link.id.is_some_and(|id| names.ids.contains(&id)) {
        return;
    }
    if model.starts_with("builtin:") {
        if scrap::builtin::by_name(model).is_none() {
            out.push(error(
                file,
                format!(
                    "{who}: no builtin model `{model}`{}",
                    suggest(model, scrap::builtin::NAMES.iter().copied())
                ),
            ));
        }
        return;
    }
    if link.id.is_none() {
        if let Some(files) = names.models.get(model).filter(|files| files.len() > 1) {
            let mut files = files.clone();
            files.sort();
            out.push(error(
                file,
                format!(
                    "{who}: `{model}` is the name of {} — open and save the scene in the editor to pick one by its ID, or rename all but one",
                    files.join(", ")
                ),
            ));
            return;
        }
    }
    if !names.models.contains_key(model) {
        let known = names
            .models
            .keys()
            .map(String::as_str)
            .chain(scrap::builtin::NAMES.iter().copied());
        out.push(error(
            file,
            format!(
                "{who}: no model named `{model}` — a model is a .gltf, .glb, .obj, .scrterrain, .scrpoly or .scrbrush in assets/, named by its file name without the extension{}",
                suggest(model, known)
            ),
        ));
    }
}

fn check_material(name: &str, who: &str, file: &str, names: &Names, out: &mut Vec<Finding>) {
    let builtins = scrap::material::builtin::NAMES.iter().copied();
    let found = match name.strip_prefix("builtin:") {
        Some(builtin) => scrap::material::builtin::by_name(builtin).is_some(),
        None => {
            names.materials.contains_key(name) || scrap::material::builtin::by_name(name).is_some()
        }
    };
    if !found {
        let known = names.materials.keys().map(String::as_str).chain(builtins);
        out.push(error(
            file,
            format!(
                "{who}: no material named `{name}`, so it draws grey — add materials/{name}.scrmat or use an existing one{}",
                suggest(name, known)
            ),
        ));
    }
}

/// Every source should have a sidecar that describes it: otherwise the
/// next `scrap sync` writes one, and a clone that did not run it builds
/// something else.
fn check_sidecars(project: &Project, out: &mut Vec<Finding>) {
    for root in [project.assets(), project.materials()] {
        scrap_import::walk(&root, &mut |source| {
            if !scrap_import::importable(source) {
                return;
            }
            let file = relative(project, source);
            let sidecar = scrap_import::sidecar_for(source);
            let stale = match scrap_import::ImportSettings::load(&sidecar) {
                Err(_) => Some("has no .scrimport"),
                Ok(settings) if settings.source != file => {
                    Some("has a .scrimport naming another file")
                }
                Ok(settings)
                    if scrap_import::content_hash(source).ok().as_ref()
                        != Some(&settings.hash) =>
                {
                    Some("changed since its .scrimport was written")
                }
                Ok(_) => None,
            };
            if let Some(why) = stale {
                out.push(Finding {
                    severity: Severity::Warning,
                    file,
                    message: format!("{why} — run `scrap sync` and commit the .scrimport"),
                });
            }
        });
    }
}

fn parse<T: serde::de::DeserializeOwned>(
    path: &Path,
    file: &str,
    out: &mut Vec<Finding>,
) -> Option<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            out.push(error(file, e));
            return None;
        }
    };
    match ron::from_str(&text) {
        Ok(value) => Some(value),
        Err(e) => {
            // ron says where: line, column, and what it expected.
            out.push(error(file, e));
            None
        }
    }
}

/// The animator graphs: their shape (a state never reached, never left, a
/// transition never taken), and every parameter they read that the game's
/// code never names — a graph waiting on a number nobody sets.
fn animators(project: &Project, out: &mut Vec<Finding>) {
    // The motion clips: each reads.
    for path in files(&project.root().join(scrap::motion::DIR), "ron") {
        let file = relative(project, &path);
        parse::<scrap::motion::Motion>(&path, &file, out);
    }
    let dir = project.root().join(scrap::project::ANIMATORS);
    let graphs = files(&dir, "ron");
    if graphs.is_empty() {
        return;
    }
    // What the scenes' and prefabs' wires pull: set, as by the code.
    let wired = wired_parameters(project);
    // The game's code, as text: a parameter is set by name.
    let code = game_code(project);
    for path in graphs {
        if path.to_string_lossy().ends_with(".cases.ron") {
            continue;
        }
        let file = relative(project, &path);
        let Some(graph) = parse::<scrap::animgraph::Graph>(&path, &file, out) else {
            continue;
        };
        // Its cases, played without the game.
        let cases_path = path.with_extension("cases.ron");
        if cases_path.is_file() {
            let cases_file = relative(project, &cases_path);
            if let Some(cases) = parse::<scrap::animgraph::Cases>(&cases_path, &cases_file, out) {
                for failed in cases.run(&graph) {
                    out.push(error(&cases_file, failed));
                }
            }
        }
        for problem in graph.shape_problems() {
            out.push(Finding {
                severity: Severity::Warning,
                file: file.clone(),
                message: problem,
            });
        }
        if code.is_empty() {
            continue;
        }
        for parameter in graph.parameters() {
            if !code.contains(&format!("\"{parameter}\"")) && !wired.contains(&parameter) {
                out.push(Finding {
                    severity: Severity::Warning,
                    file: file.clone(),
                    message: format!(
                        "parameter `{parameter}` is never set: no \"{parameter}\" in src/, and no wire pulls it"
                    ),
                });
            }
        }
    }
}

/// The game's code under `src/`, as one text: what names a flag, a
/// parameter or an event in a string is looked for in.
fn game_code(project: &Project) -> String {
    let mut code = String::new();
    let mut stack = vec![project.root().join("src")];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                code.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    code
}

/// Dialogues and quests: each file's own problems, a dialogue's line
/// written twice, its cases played, and then between them all — a flag
/// or number read that nothing sets, one set that nothing reads, an
/// event nothing hears (the game's code counts as setting, reading and
/// hearing when it names it in a string). The keys they ask `strings/`
/// for.
fn dialogues(project: &Project, out: &mut Vec<Finding>) -> Vec<String> {
    use scrap::dialogue::{named_twice, Cases, Dialogue};
    use std::collections::BTreeSet;
    let dir = project.root().join(scrap::dialogue::DIR);
    let mut keys = Vec::new();
    let mut found: Vec<(String, Dialogue)> = Vec::new();
    for path in files(&dir, "ron") {
        if crate::lines::is_cases(&path) {
            continue;
        }
        let file = relative(project, &path);
        for twice in named_twice(&std::fs::read_to_string(&path).unwrap_or_default()) {
            out.push(error(&file, twice));
        }
        let Some(mut dialogue) = parse::<Dialogue>(&path, &file, out) else {
            continue;
        };
        dialogue.name = crate::lines::name(&dir, &path);
        for problem in dialogue.problems() {
            out.push(error(&file, problem));
        }
        let cases_path = path.with_extension("cases.ron");
        if cases_path.is_file() {
            let cases_file = relative(project, &cases_path);
            if let Some(cases) = parse::<Cases>(&cases_path, &cases_file, out) {
                for failed in cases.run(&dialogue) {
                    out.push(error(&cases_file, failed));
                }
            }
        }
        keys.extend(dialogue.keys());
        found.push((file, dialogue));
    }
    let mut quests: Vec<(String, scrap::quest::Quest)> = Vec::new();
    for path in files(&project.root().join(scrap::quest::DIR), "ron") {
        let file = relative(project, &path);
        if let Some(quest) = parse::<scrap::quest::Quest>(&path, &file, out) {
            for problem in quest.problems() {
                out.push(error(&file, problem));
            }
            keys.extend(quest.keys());
            quests.push((file, quest));
        }
    }
    if found.is_empty() && quests.is_empty() {
        return keys;
    }
    let code = game_code(project);
    let named = |n: &str| code.contains(&format!("\"{n}\""));
    let set: BTreeSet<String> = found.iter().flat_map(|(_, d)| d.writes()).collect();
    let read: BTreeSet<String> = found
        .iter()
        .flat_map(|(_, d)| d.reads())
        .chain(quests.iter().flat_map(|(_, q)| q.reads()))
        .collect();
    let warn = |out: &mut Vec<Finding>, file: &str, message: String| {
        out.push(Finding {
            severity: Severity::Warning,
            file: file.to_string(),
            message,
        })
    };
    let readers: Vec<(&String, BTreeSet<String>)> = found
        .iter()
        .map(|(f, d)| (f, d.reads()))
        .chain(
            quests
                .iter()
                .map(|(f, q)| (f, q.reads().into_iter().collect())),
        )
        .collect();
    for (file, reads) in readers {
        for name in reads.iter().filter(|n| !set.contains(*n) && !named(n)) {
            warn(
                out,
                file,
                format!("`{name}` is read and nothing sets it: no dialogue does, and no \"{name}\" in src/"),
            );
        }
    }
    for (file, dialogue) in &found {
        for name in dialogue
            .writes()
            .iter()
            .filter(|n| !read.contains(*n) && !named(n))
        {
            warn(
                out,
                file,
                format!("`{name}` is set and nothing reads it: no dialogue or quest does, and no \"{name}\" in src/"),
            );
        }
        if code.is_empty() {
            continue;
        }
        for event in dialogue.events().iter().filter(|e| !named(e)) {
            warn(
                out,
                file,
                format!("event `{event}` is told and nothing hears it: no \"{event}\" in src/"),
            );
        }
    }
    keys
}

/// Words in `strings/` nothing asks for: no screen, dialogue or quest, and
/// no `"key"` or `"@key"` in the game's code. A warning on the first
/// language that has it — a key's words may still be wanted, or be put
/// together in code.
fn unused_strings(
    project: &Project,
    tables: &[(String, scrap::strings::Table)],
    used: &[String],
    out: &mut Vec<Finding>,
) {
    if tables.is_empty() {
        return;
    }
    let code = game_code(project);
    let used: HashSet<&str> = used.iter().map(String::as_str).collect();
    let mut told: HashSet<&str> = HashSet::new();
    for (language, table) in tables {
        for key in table.keys() {
            if used.contains(key.as_str())
                || code.contains(&format!("\"{key}\""))
                || code.contains(&format!("\"@{key}\""))
                || !told.insert(key)
            {
                continue;
            }
            out.push(Finding {
                severity: Severity::Warning,
                file: format!("{}/{language}.ron", scrap::strings::DIR),
                message: format!(
                    "`{key}` is asked for by no screen, dialogue or quest, and no \"{key}\" in src/"
                ),
            });
        }
    }
}

/// Every animator parameter a wire in a scene or a prefab pulls.
fn wired_parameters(project: &Project) -> HashSet<String> {
    let mut lines: Vec<EntityDesc> = Vec::new();
    for path in files(&project.scenes(), "ron") {
        if let Some(scene) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| ron::from_str::<Scene>(&t).ok())
        {
            lines.extend(scene.entities);
        }
    }
    for path in files(&project.prefabs(), "prefab") {
        if let Some(desc) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| ron::from_str::<EntityDesc>(&t).ok())
        {
            lines.push(desc);
        }
    }
    let mut out = HashSet::new();
    while let Some(line) = lines.pop() {
        for wire in line.wires() {
            out.extend(wire.act.parameter().map(str::to_string));
        }
        lines.extend(line.children);
    }
    out
}

/// Every material shader in `shaders/` — hand-written or a graph — built
/// over the standard shader as the renderer would, without a GPU; and what
/// a graph holds that nothing reads.
fn shaders(project: &Project, out: &mut Vec<Finding>) {
    let dir = project.root().join(scrap::project::SHADERS);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if scrap::render::material_shader_name(&path).is_none() {
            continue;
        }
        let file = relative(project, &path);
        let source = match scrap::render::material_shader_source(&path) {
            Ok(source) => source,
            Err(e) => {
                out.push(error(&file, e));
                continue;
            }
        };
        if let Err(e) = scrap::render::check_material_shader(&source) {
            out.push(error(&file, e));
            continue;
        }
        if file.ends_with(".graph.ron") {
            if let Ok(graph) = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| scrap::shader_graph::surface::parse(&t)) {
                for problem in scrap::shader_graph::surface::problems(&graph) {
                    out.push(Finding {
                        severity: Severity::Warning,
                        file: file.clone(),
                        message: problem,
                    });
                }
            }
        }
    }
}

fn error(file: &str, message: impl fmt::Display) -> Finding {
    Finding {
        severity: Severity::Error,
        file: file.to_string(),
        message: message.to_string(),
    }
}

fn relative(project: &Project, path: &Path) -> String {
    project
        .relative(path)
        .unwrap_or_else(|| path.display().to_string())
}

/// Files with this extension anywhere under `root`, sorted.
fn files(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    scrap_import::walk(root, &mut |path| {
        if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            found.push(path.to_path_buf());
        }
    });
    found.sort();
    found
}

/// ` — did you mean `x`?` for the closest known name, or nothing when none
/// is close enough to be a typo.
fn suggest<'a>(wanted: &str, known: impl Iterator<Item = &'a str>) -> String {
    scrap::spelling::closest(wanted, known)
        .map(|name| format!(" — did you mean `{name}`?"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typo_is_suggested_and_a_stranger_is_not() {
        let known = ["rock", "pine_large", "campfire"];
        assert_eq!(
            suggest("rok", known.iter().copied()),
            " — did you mean `rock`?"
        );
        assert_eq!(
            suggest("pine_lrage", known.iter().copied()),
            " — did you mean `pine_large`?"
        );
        assert_eq!(suggest("helicopter", known.iter().copied()), "");
    }
}
