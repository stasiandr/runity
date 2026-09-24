//! `scrap` — a project from the command line.
//!
//! ```text
//! scrap new <folder> [--name NAME] [--engine-path PATH] [--set bare|basic|full | --template NAME]
//! scrap test  [PROJECT]  the game's tests, headless
//! scrap run   [PROJECT] [--hot] [--release] [--scene NAME] [--players N [--link BAD]]  the game
//! scrap sync  [PROJECT]     build library/ from the sources
//! scrap check [PROJECT]     what does not resolve, with file and entity
//! scrap modules [sync] [PROJECT]  the engine's modules; Cargo.toml from scrap.ron
//! scrap rebuild-time [PROJECT] [--runs N] [--budget SECONDS]
//! scrap perf [PROJECT] [--frames N] [--scene NAME] [--counts] [--write]  frames against budgets.ron
//! scrap build [PROJECT] [--out DIR] [--debug | --size]  a folder to ship
//! scrap merge BASE OURS THEIRS [PATH]   the git merge driver for scenes
//! scrap git-setup [PROJECT]             turn the driver on in this clone
//! scrap rename FROM TO                   move an asset, and what names it
//! scrap uses FILE                        every line that names an asset
//! scrap assets [PROJECT]                 every asset, and how much it is used
//! scrap delete FILE                      remove an asset nothing uses
//! scrap duplicate FROM TO                copy an asset as a new one
//! scrap add component|system|scene NAME [PROJECT]  a new file where it goes
//! scrap import-unity UNITY_PROJECT [PROJECT] [--models] [--models-matching NAME] [--blender PATH] [--shaders DIR]
//! scrap bench [--seeds N] [--scenarios a,b] [--rungs a,b] [--out FILE]
//! ```
//!
//! PROJECT is any path inside a project; the current folder by default.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use scrap::project::Engine;
use scrap::Project;
use scrap_cli::Severity;
use scrap_import::Change;

const HELP: &str = "\
scrap new <folder> [--name NAME] [--engine-path PATH] [--set SET | --template NAME]
    Make a project: the standard layout, a scene, and a game crate.
    The game depends on the engine from git, or from a local checkout
    of scrap's crates/scrap with --engine-path. --set picks its modules:
    basic (a window, the picture, input, a score on screen, sound,
    collisions, played together; the default), full (every module), or
    bare (the core alone: no window, a server or a simulation). They are
    listed in scrap.ron; add or drop one there and `scrap modules sync`.
    --template NAME copies one of the engine's example projects instead.
scrap run [PROJECT] [--hot] [--release] [--scene NAME] [--players N [--link BAD]]
    Run the game, on scenes/main.ron or scenes/NAME.ron. Scenes, prefabs,
    assets, shaders and tuning reload while it runs; with --hot, so does its
    own Rust (under `dx serve --hotpatch`, from `cargo install dioxus-cli`).
    With --players 2 to 4, that many windows play together on this machine:
    player 1 hosts, the others join, each line marked with whose it is.
    --link poor|awful|latency=80,jitter=10,loss=3 plays the others over a
    bad link on purpose.
scrap relay [--port N]
    Run a relay (UDP, 47778 by default): players behind NAT in different
    homes play through it, in a room whose code the host reads out.
scrap test [PROJECT]
    The game's tests: a new project's plays its start scene for two seconds
    without a window — systems, physics — and fails if anything breaks.
scrap import-unity UNITY_PROJECT [PROJECT] [--models] [--blender PATH]
    Bring a Unity project's content over: scenes and prefabs as RON with
    stable ids, URP Lit materials as .scrmat, the textures they use, animator
    controllers as animators/. MonoBehaviours become components written as
    text. --models converts FBX through Blender (slow). Says what it left
    behind, then syncs the library. A material's own shader becomes
    shaders/<name>.wgsl: a stub to write again, or the one written again in
    --shaders DIR (examples/dacha/shaders for Dacha Simulator).
scrap bench [--seeds N] [--scenarios a,b] [--rungs a,b] [--out FILE]
    The physics bench: thirteen scenarios from Dacha Simulator played
    alone, then by three players passing the bodies around over links from
    perfect to awful, measured against the solo run. Prints the frontier —
    how far down the ladder each holds — and every run; writes it all as
    RON to FILE (bench.ron). Real time: the full ladder takes about an
    hour a seed.
scrap sync [PROJECT]
    Build library/ from assets/ and materials/: changed sources by content
    hash, moved ones found by it, new ones imported, sidecars written.
scrap check [PROJECT]
    Every model, material and prefab a scene names, every id, every sidecar.
    Exits 1 when something does not resolve.
scrap modules [PROJECT]
    Every module of the engine, what it is and stands on, and whether the
    project lists it in scrap.ron (`modules: [...]`).
scrap modules sync [PROJECT]
    Write the engine's features in the game's Cargo.toml from the modules
    scrap.ron lists and what they stand on. `check` says when they part.
scrap build [PROJECT] [--out DIR] [--debug | --size]
    Sync the library, compile the game, and lay out DIR (build/ in the
    project by default): the executable and data/ with scenes, prefabs and
    the built library. Sources stay home. Compiled for speed by default;
    --size for the smallest download (opt-level z, LTO, stripped); --debug
    quick to make.
scrap git-setup [PROJECT]
    Turn on the scene merge driver in this clone: scenes and prefabs merge
    by entity and field, and conflicts are said in words.
scrap merge BASE OURS THEIRS [PATH]
    The driver git calls; writes the merge over OURS. Exits 1 on conflict.
scrap rename FROM TO
    Rename or move an asset source — a model, texture or sound in assets/,
    a .scrmat, a .prefab — with its .scrimport, and rewrite every scene and
    prefab line that named it. Refused when the new name already means
    something.
scrap uses FILE
    Every scene and prefab line that names FILE: what a rename changes,
    and whether deleting it breaks anything.
scrap assets [PROJECT]
    Every asset source — model, texture, sound, material, prefab — with how
    many lines use it; an unused one is listed as used 0.
scrap delete FILE
    Remove an asset source, its .scrimport and its built asset. Refused, with
    the lines, while anything names it.
scrap duplicate FROM TO
    Copy an asset source as a new asset with the same import settings.
scrap add component NAME [PROJECT]
    Write src/components/NAME.rs. Scenes then give it by NAME:
    `components: { \"NAME\": (...) }`; build.rs registers it.
scrap add system NAME [PROJECT]
    Write src/systems/NAME.rs and run it last in `tick` in src/main.rs.
scrap add scene NAME [PROJECT]
    Write scenes/NAME.ron: a ground with the metre grid, to build on.
scrap rebuild-time [PROJECT] [--runs N] [--budget SECONDS]
    Build the game, then time rebuilding it after a one-line edit to
    src/main.rs, N times (3), and report the best. Exits 1 over budget.
scrap perf [PROJECT] [--frames N] [--scene NAME] [--counts] [--write]
    Draw every scene (or one) offscreen at budgets.ron's size and measure
    it: the GPU's milliseconds a frame (over N frames, 10), the draws, the
    triangles. Exits 1 when one is over its budget in budgets.ron; time is
    held only on a real GPU, and not at all with --counts (a shared
    runner's GPU is not the one the budgets were set on). --write sets the
    budgets from this machine, with room.

PROJECT is any path inside a project; the current folder by default.";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "new" => new(&rest),
        "sync" => sync(&find(&rest)?),
        "bench" => bench(&rest),
        "import-unity" => {
            let mut options = scrap_import::unity::Options::default();
            let mut paths: Vec<String> = Vec::new();
            let mut args = rest.iter();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--models" => options.models = true,
                    "--models-matching" => {
                        options.models = true;
                        options.models_matching = Some(args.next().context("--models-matching wants a name")?.clone());
                    }
                    "--shaders" => {
                        options.shaders = Some(PathBuf::from(
                            args.next().context("--shaders wants a folder")?,
                        ))
                    }
                    "--blender" => {
                        options.blender = Some(PathBuf::from(
                            args.next().context("--blender wants a path")?,
                        ))
                    }
                    other if other.starts_with('-') => bail!("unknown option {other}"),
                    other => paths.push(other.to_string()),
                }
            }
            let unity = PathBuf::from(paths.first().context("which Unity project?")?);
            let project = find(&paths[1..])?;
            let report = scrap_import::unity::import_unity(&unity, &project, &options)?;
            print!("{report}");
            sync(&project)
        }
        "run" => {
            let mut at: Vec<String> = Vec::new();
            let (mut hot, mut release) = (false, false);
            let mut scene: Option<String> = None;
            let mut count = 1u32;
            let mut link = String::new();
            let mut args = rest.iter();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--hot" => hot = true,
                    "--release" => release = true,
                    "--scene" => scene = Some(args.next().context("--scene wants a name")?.clone()),
                    "--link" => {
                        link = args
                            .next()
                            .context("--link wants poor, awful or latency=80,loss=3")?
                            .clone()
                    }
                    "--players" => {
                        count = args
                            .next()
                            .and_then(|n| n.parse().ok())
                            .context("--players wants a number, 2 to 4")?;
                    }
                    other if other.starts_with('-') => bail!("unknown option {other}"),
                    other => at.push(other.to_string()),
                }
            }
            let project = find(&at)?;
            let scene = scene
                .map(|name| scrap_cli::run::scene(&project, &name))
                .transpose()?;
            if count > 1 {
                if hot {
                    bail!("--hot patches one process; play together without it");
                }
                let ok =
                    scrap_cli::run::players(&project, count, release, scene.as_deref(), &link)?;
                return Ok(if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                });
            }
            let dx = scrap_cli::run::on_path("dx");
            let mut command = scrap_cli::run::command(&project, hot, release, dx.as_deref())?;
            if let Some(name) = scene {
                command.env(scrap_cli::run::SCENE_VAR, name);
            }
            let status = command.status()?;
            Ok(if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "check" => check(&find(&rest)?),
        "modules" => {
            let (sync, rest) = match rest.split_first() {
                Some((first, rest)) if first == "sync" => (true, rest.to_vec()),
                _ => (false, rest.to_vec()),
            };
            let project = find(&rest)?;
            if sync {
                let features = scrap_cli::modules::sync(&project)?;
                println!("Cargo.toml: scrap with [{}]", features.join(", "));
            } else {
                for (module, on) in scrap_cli::modules::table(&project) {
                    let depends = if module.depends.is_empty() {
                        String::new()
                    } else {
                        format!(" (on {})", module.depends.join(", "))
                    };
                    println!(
                        "{} {:<11} {}{depends}",
                        if on { "+" } else { " " },
                        module.name,
                        module.what
                    );
                }
                if project.manifest().modules.is_empty() {
                    println!("scrap.ron lists no modules: the engine's default set");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        "relay" => {
            // A server between homes: peers in a room (a code the host reads
            // out) hear each other through it, NAT or not.
            let mut port = 47_778u16;
            let mut args = rest.iter();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--port" => port = args.next().context("--port wants a number")?.parse()?,
                    other => bail!("unknown option {other}"),
                }
            }
            let mut server = scrap::relay::RelayServer::bind(&format!("0.0.0.0:{port}"))?;
            eprintln!("scrap relay on {}", server.local_address()?);
            server.run()
        }
        "test" => {
            // Play mode without a window: the game's own tests, which a new
            // project starts with one of — its start scene played headless.
            let project = find(&rest)?;
            if !project.root().join("Cargo.toml").is_file() {
                bail!("{} has no game crate to test", project.root().display());
            }
            let status = std::process::Command::new("cargo")
                .arg("test")
                .current_dir(project.root())
                .status()?;
            Ok(if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "rebuild-time" => rebuild_time(&rest),
        "perf" => perf(&rest),
        "merge" => merge(&rest),
        "build" => build(&rest),
        "rename" => rename(&rest),
        "uses" => uses(&rest),
        "add" => add(&rest),
        "assets" => {
            let project = find(&rest)?;
            let entries = scrap_import::assets::list(&project)?;
            for e in &entries {
                println!(
                    "{:<9} {:>3} used  {}{}",
                    e.kind,
                    e.uses,
                    e.file,
                    if e.built {
                        ""
                    } else {
                        "  (not built: scrap sync)"
                    }
                );
            }
            println!("{}: {} assets", project.name(), entries.len());
            Ok(ExitCode::SUCCESS)
        }
        "delete" => {
            let [file] = rest.as_slice() else {
                bail!("scrap delete FILE");
            };
            let file = std::path::absolute(file)?;
            let project = Project::find(&file).map_err(|e| anyhow::anyhow!("{e}"))?;
            scrap_import::assets::delete(&project, &file)?;
            println!("deleted {}", project.relative(&file).unwrap_or_default());
            Ok(ExitCode::SUCCESS)
        }
        "duplicate" => {
            let [from, to] = rest.as_slice() else {
                bail!("scrap duplicate FROM TO");
            };
            let (from, to) = (std::path::absolute(from)?, std::path::absolute(to)?);
            let project = Project::find(&from).map_err(|e| anyhow::anyhow!("{e}"))?;
            let synced = scrap_import::assets::duplicate(&project, &from, &to)?;
            for failed in synced.iter().filter_map(|r| r.result.as_ref().err()) {
                eprintln!("warning: {failed}");
            }
            println!(
                "{} -> {}",
                project.relative(&from).unwrap_or_default(),
                project.relative(&to).unwrap_or_default()
            );
            Ok(ExitCode::SUCCESS)
        }
        "git-setup" => {
            let project = find(&rest)?;
            for line in scrap_cli::merge::git_setup(project.root())? {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        "" | "-h" | "--help" | "help" => {
            println!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        other => bail!("no command `{other}`\n\n{HELP}"),
    }
}

fn find(rest: &[String]) -> Result<Project> {
    let at = rest
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    Project::find(&at).map_err(|e| anyhow::anyhow!("{e}"))
}

fn new(rest: &[String]) -> Result<ExitCode> {
    let mut folder: Option<PathBuf> = None;
    let mut name: Option<String> = None;
    let mut engine = Engine::default();
    let mut set = String::from("basic");
    let mut template: Option<String> = None;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = args.next().cloned(),
            "--set" => set = args.next().context("--set wants bare, basic or full")?.clone(),
            "--template" => template = Some(args.next().context("--template wants a name")?.clone()),
            "--engine-path" => {
                let path = args.next().context("--engine-path wants a path")?;
                engine = Engine::Path(std::path::absolute(path)?);
            }
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => folder = Some(PathBuf::from(other)),
        }
    }
    let folder = folder.context("scrap new <folder>")?;
    let name = match name {
        Some(name) => name,
        None => std::path::absolute(&folder)?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .context("the folder has no name; pass --name")?,
    };
    let project = match template {
        Some(template) => scrap_cli::template::create(&template, &folder, &name, &engine)?,
        None => {
            let known = scrap::modules::official();
            let listed = scrap::modules::set(&set).with_context(|| {
                format!("no set `{set}` — there are: {}", scrap::modules::SETS.join(", "))
            })?;
            let features = scrap::modules::features(&listed, &known);
            Project::create_with_modules(&folder, &name, &engine, Some((&listed, &features)))
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
    };
    println!(
        "made {} in {}\n\n  cd {}\n  scrap run       # the game, reloading scenes as you save them
  scrap run --hot # and the game's own code too (needs dioxus-cli)\n  scrap check     # what does not resolve",
        project.name(),
        project.root().display(),
        folder.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn sync(project: &Project) -> Result<ExitCode> {
    let done = scrap_import::sync(project);
    let mut failed = 0;
    for item in &done {
        let what = match &item.change {
            Change::New => "new".to_string(),
            Change::Changed => "changed".to_string(),
            Change::Built => "built".to_string(),
            Change::Outdated => "rebuilt in the new format".to_string(),
            Change::Moved { from } => format!("moved from {from}"),
            Change::Gone => "gone".to_string(),
        };
        let source = project
            .relative(&item.source)
            .unwrap_or_else(|| item.source.display().to_string());
        match &item.result {
            Ok(id) => println!("{source} ({what}) -> {id}"),
            Err(e) => {
                failed += 1;
                eprintln!("{source} ({what}): {e}");
            }
        }
    }
    println!(
        "{}: {} brought up to date, {failed} could not be",
        project.name(),
        done.len() - failed
    );
    Ok(if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn check(project: &Project) -> Result<ExitCode> {
    let findings = scrap_cli::check(project);
    for finding in &findings {
        println!("{finding}");
    }
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings.len() - errors;
    println!("{}: {errors} errors, {warnings} warnings", project.name());
    Ok(if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn rebuild_time(rest: &[String]) -> Result<ExitCode> {
    let mut at: Vec<String> = Vec::new();
    let mut runs = 3usize;
    let mut budget: Option<f64> = None;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runs" => runs = args.next().context("--runs wants a number")?.parse()?,
            "--budget" => budget = Some(args.next().context("--budget wants seconds")?.parse()?),
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => at.push(other.to_string()),
        }
    }
    let project = find(&at)?;
    let timed = scrap_cli::rebuild_time(&project, runs)?;
    for (i, took) in timed.runs.iter().enumerate() {
        println!("rebuild {}: {:.2} s", i + 1, took.as_secs_f64());
    }
    let best = timed.best().as_secs_f64();
    match budget {
        Some(budget) if best > budget => {
            println!(
                "{}: best {best:.2} s, over the budget of {budget:.2} s",
                project.name()
            );
            Ok(ExitCode::FAILURE)
        }
        Some(budget) => {
            println!("{}: best {best:.2} s, within {budget:.2} s", project.name());
            Ok(ExitCode::SUCCESS)
        }
        None => {
            println!("{}: best {best:.2} s", project.name());
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn perf(rest: &[String]) -> Result<ExitCode> {
    let mut at: Vec<String> = Vec::new();
    let mut frames = 10u32;
    let mut only: Option<String> = None;
    let mut write = false;
    let mut counts = false;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => frames = args.next().context("--frames wants a number")?.parse()?,
            "--scene" => only = Some(args.next().context("--scene wants a name")?.clone()),
            "--write" => write = true,
            "--counts" => counts = true,
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => at.push(other.to_string()),
        }
    }
    let project = find(&at)?;
    let mut budgets = scrap_cli::perf::read(&project)?;
    let mut over = 0usize;
    println!(
        "{:<18} {:>9} {:>7} {:>11} {:>8} {:>8} {:>10} {:>9} {:>9}",
        "scene", "gpu ms", "draws", "triangles", "step ms", "build ms", "render cpu", "frame", "threaded"
    );
    for name in project.scene_names() {
        if only.as_ref().is_some_and(|o| *o != name) {
            continue;
        }
        let path = project.scenes().join(format!("{name}.ron"));
        let (measured, real_gpu) = scrap_cli::perf::measure(&path, budgets.size, frames)
            .with_context(|| format!("drawing {name}"))?;
        let ms = measured.gpu_ms.map_or("-".to_string(), |ms| format!("{ms:.2}"));
        let budget = budgets.scenes.get(&name).copied().unwrap_or_default();
        let problems = scrap_cli::perf::over(&budget, &measured, real_gpu && !counts);
        println!(
            "{name:<18} {ms:>9} {:>7} {:>11} {:>8.2} {:>8.2} {:>10.2} {:>9.2} {:>9.2}{}",
            measured.draws,
            measured.triangles,
            measured.step_ms,
            measured.build_ms,
            measured.render_cpu_ms,
            measured.frame_ms,
            measured.pipelined_ms,
            if problems.is_empty() { String::new() } else { format!("  over: {}", problems.join("; ")) }
        );
        over += usize::from(!problems.is_empty());
        if write {
            budgets.scenes.insert(name, scrap_cli::perf::with_room(&measured));
        }
    }
    if write {
        let text = ron::ser::to_string_pretty(&budgets, ron::ser::PrettyConfig::default())?;
        std::fs::write(project.root().join(scrap_cli::perf::BUDGETS), text + "\n")?;
        println!("wrote {}", scrap_cli::perf::BUDGETS);
        return Ok(ExitCode::SUCCESS);
    }
    println!("{}: {over} over budget", project.name());
    Ok(if over > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn merge(rest: &[String]) -> Result<ExitCode> {
    let [base, ours, theirs, path @ ..] = rest else {
        bail!("scrap merge BASE OURS THEIRS [PATH]");
    };
    let path = path.first().map(PathBuf::from);
    let outcome = scrap_cli::merge::merge_files(
        base.as_ref(),
        ours.as_ref(),
        theirs.as_ref(),
        path.as_deref(),
    )?;
    let name = path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| ours.clone());
    for conflict in &outcome.conflicts {
        eprintln!("scrap merge: {name}: {conflict}");
    }
    if outcome.conflicts.is_empty() && !outcome.by_lines {
        eprintln!("scrap merge: {name}: merged by entity");
    }
    Ok(if outcome.conflicts.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn add(rest: &[String]) -> Result<ExitCode> {
    let [what, name, at @ ..] = rest else {
        bail!("scrap add component|system|scene NAME [PROJECT]");
    };
    let project = find(at)?;
    match what.as_str() {
        "component" => {
            let file = scrap_cli::add::component(&project, name)?;
            println!(
                "wrote {}\n  a scene gives it as components: {{ \"{name}\": () }}",
                project.relative(&file).unwrap_or_default()
            );
        }
        "system" => {
            let added = scrap_cli::add::system(&project, name)?;
            println!(
                "wrote {}",
                project.relative(&added.file).unwrap_or_default()
            );
            if added.called {
                println!("  and tick in src/main.rs runs it last");
            } else {
                println!(
                    "  src/main.rs has no `// systems, in order` list; call it from the step:\n    {}",
                    scrap_cli::add::system_call(name)
                );
            }
        }
        "scene" => {
            let file = project.new_scene(name).map_err(anyhow::Error::msg)?;
            println!("wrote {}", project.relative(&file).unwrap_or_default());
        }
        other => bail!("scrap add component|system|scene NAME — not `{other}`"),
    }
    Ok(ExitCode::SUCCESS)
}

fn rename(rest: &[String]) -> Result<ExitCode> {
    let [from, to] = rest else {
        bail!("scrap rename FROM TO");
    };
    // Paths as typed, from wherever the command was run; the project is
    // the one the file is in.
    let from = std::path::absolute(from)?;
    let to = std::path::absolute(to)?;
    let project = Project::find(&from).map_err(|e| anyhow::anyhow!("{e}"))?;
    let done = scrap_import::assets::rename(&project, &from, &to)?;
    println!("{} -> {}", done.from, done.to);
    if let Some((old, new)) = &done.reference {
        println!("scenes said {old}, now {new}");
    }
    for (file, count) in &done.rewritten {
        println!("  {file}: {count} rewritten");
    }
    let mut failed = 0;
    for item in &done.synced {
        if let Err(e) = &item.result {
            failed += 1;
            eprintln!("warning: {e}");
        }
    }
    Ok(if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn uses(rest: &[String]) -> Result<ExitCode> {
    let [file] = rest else {
        bail!("scrap uses FILE");
    };
    let file = std::path::absolute(file)?;
    let project = Project::find(&file).map_err(|e| anyhow::anyhow!("{e}"))?;
    let found = scrap_import::assets::usages(&project, &file)?;
    for usage in &found {
        println!("{usage}");
    }
    let shown = project
        .relative(&file)
        .unwrap_or_else(|| file.display().to_string());
    println!("{shown}: named {} times", found.len());
    Ok(ExitCode::SUCCESS)
}

fn build(rest: &[String]) -> Result<ExitCode> {
    let mut at: Vec<String> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut profile = scrap_cli::build::Profile::Speed;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = Some(args.next().context("--out wants a folder")?.into()),
            "--debug" => profile = scrap_cli::build::Profile::Debug,
            "--size" => profile = scrap_cli::build::Profile::Size,
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => at.push(other.to_string()),
        }
    }
    let project = find(&at)?;
    let out = out.unwrap_or_else(|| project.root().join("build"));
    let built = scrap_cli::build::build_with(&project, &out, profile)?;
    for line in &built.stale {
        eprintln!("warning: {line}");
    }
    println!(
        "built {} into {}\n  run {}",
        project.name(),
        built.folder.display(),
        built.executable.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// `scrap bench`: the physics bench's ladder.
fn bench(rest: &[String]) -> Result<ExitCode> {
    let mut seeds = 1u64;
    let (mut scenarios, mut rungs): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut out = PathBuf::from("bench.ron");
    let mut args = rest.iter();
    let list = |v: Option<&String>| -> Result<Vec<String>> {
        Ok(v.context("wants a comma-separated list")?
            .split(',')
            .map(|s| s.trim().to_string())
            .collect())
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seeds" => seeds = args.next().context("--seeds wants a number")?.parse()?,
            "--scenarios" => scenarios = list(args.next())?,
            "--rungs" => rungs = list(args.next())?,
            "--out" => out = PathBuf::from(args.next().context("--out wants a file")?),
            other => bail!("unknown option {other}"),
        }
    }
    let wanted = |list: &[String], name: &str| list.is_empty() || list.iter().any(|w| w == name);
    let report = scrap::bench::measure(
        seeds,
        |s| wanted(&scenarios, s),
        |r| wanted(&rungs, r),
        |line| println!("{line}"),
    );
    println!("\n{}", report.text());
    let text = scrap::ron::ser::to_string_pretty(&report, Default::default())?;
    std::fs::write(&out, text).with_context(|| out.display().to_string())?;
    println!("written to {}", out.display());
    Ok(ExitCode::SUCCESS)
}
