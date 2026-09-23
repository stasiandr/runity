//! `runity` — a project from the command line.
//!
//! ```text
//! runity new <folder> [--name NAME] [--engine-path PATH]
//! runity run   [PROJECT] [--hot] [--release]  the game
//! runity sync  [PROJECT]     build library/ from the sources
//! runity check [PROJECT]     what does not resolve, with file and entity
//! runity rebuild-time [PROJECT] [--runs N] [--budget SECONDS]
//! runity build [PROJECT] [--out DIR] [--debug]  a folder to ship
//! runity merge BASE OURS THEIRS [PATH]   the git merge driver for scenes
//! runity git-setup [PROJECT]             turn the driver on in this clone
//! runity rename FROM TO                   move an asset, and what names it
//! runity uses FILE                        every line that names an asset
//! runity assets [PROJECT]                 every asset, and how much it is used
//! runity delete FILE                      remove an asset nothing uses
//! runity duplicate FROM TO                copy an asset as a new one
//! runity add component|system NAME [PROJECT]  a new file where it goes
//! ```
//!
//! PROJECT is any path inside a project; the current folder by default.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use runity::project::Engine;
use runity::Project;
use runity_cli::Severity;
use runity_import::Change;

const HELP: &str = "\
runity new <folder> [--name NAME] [--engine-path PATH]
    Make a project: the standard layout, a scene, and a game crate.
    The game depends on the engine from git, or from a local checkout
    of runity's crates/runity with --engine-path.
runity run [PROJECT] [--hot] [--release]
    Run the game. Scenes, prefabs, assets, shaders and tuning reload while
    it runs; with --hot, so does its own Rust (under `dx serve --hotpatch`,
    from `cargo install dioxus-cli`).
runity sync [PROJECT]
    Build library/ from assets/ and materials/: changed sources by content
    hash, moved ones found by it, new ones imported, sidecars written.
runity check [PROJECT]
    Every model, material and prefab a scene names, every id, every sidecar.
    Exits 1 when something does not resolve.
runity build [PROJECT] [--out DIR] [--debug]
    Sync the library, compile the game (release unless --debug), and lay out
    DIR (build/ in the project by default): the executable and data/ with
    scenes, prefabs and the built library. Sources stay home.
runity git-setup [PROJECT]
    Turn on the scene merge driver in this clone: scenes and prefabs merge
    by entity and field, and conflicts are said in words.
runity merge BASE OURS THEIRS [PATH]
    The driver git calls; writes the merge over OURS. Exits 1 on conflict.
runity rename FROM TO
    Rename or move an asset source — a model, texture or sound in assets/,
    a .rmat, a .prefab — with its .rimport, and rewrite every scene and
    prefab line that named it. Refused when the new name already means
    something.
runity uses FILE
    Every scene and prefab line that names FILE: what a rename changes,
    and whether deleting it breaks anything.
runity assets [PROJECT]
    Every asset source — model, texture, sound, material, prefab — with how
    many lines use it; an unused one is listed as used 0.
runity delete FILE
    Remove an asset source, its .rimport and its built asset. Refused, with
    the lines, while anything names it.
runity duplicate FROM TO
    Copy an asset source as a new asset with the same import settings.
runity add component NAME [PROJECT]
    Write src/components/NAME.rs. Scenes then give it by NAME:
    `components: { \"NAME\": (...) }`; build.rs registers it.
runity add system NAME [PROJECT]
    Write src/systems/NAME.rs and run it last in `step` in src/main.rs.
runity rebuild-time [PROJECT] [--runs N] [--budget SECONDS]
    Build the game, then time rebuilding it after a one-line edit to
    src/main.rs, N times (3), and report the best. Exits 1 over budget.

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
        "run" => {
            let mut at: Vec<String> = Vec::new();
            let (mut hot, mut release) = (false, false);
            for arg in &rest {
                match arg.as_str() {
                    "--hot" => hot = true,
                    "--release" => release = true,
                    other if other.starts_with('-') => bail!("unknown option {other}"),
                    other => at.push(other.to_string()),
                }
            }
            let project = find(&at)?;
            let dx = runity_cli::run::on_path("dx");
            let status =
                runity_cli::run::command(&project, hot, release, dx.as_deref())?.status()?;
            Ok(if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "check" => check(&find(&rest)?),
        "rebuild-time" => rebuild_time(&rest),
        "merge" => merge(&rest),
        "build" => build(&rest),
        "rename" => rename(&rest),
        "uses" => uses(&rest),
        "add" => add(&rest),
        "assets" => {
            let project = find(&rest)?;
            let entries = runity_import::assets::list(&project)?;
            for e in &entries {
                println!(
                    "{:<9} {:>3} used  {}{}",
                    e.kind,
                    e.uses,
                    e.file,
                    if e.built {
                        ""
                    } else {
                        "  (not built: runity sync)"
                    }
                );
            }
            println!("{}: {} assets", project.name(), entries.len());
            Ok(ExitCode::SUCCESS)
        }
        "delete" => {
            let [file] = rest.as_slice() else {
                bail!("runity delete FILE");
            };
            let file = std::path::absolute(file)?;
            let project = Project::find(&file).map_err(|e| anyhow::anyhow!("{e}"))?;
            runity_import::assets::delete(&project, &file)?;
            println!("deleted {}", project.relative(&file).unwrap_or_default());
            Ok(ExitCode::SUCCESS)
        }
        "duplicate" => {
            let [from, to] = rest.as_slice() else {
                bail!("runity duplicate FROM TO");
            };
            let (from, to) = (std::path::absolute(from)?, std::path::absolute(to)?);
            let project = Project::find(&from).map_err(|e| anyhow::anyhow!("{e}"))?;
            let synced = runity_import::assets::duplicate(&project, &from, &to)?;
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
            for line in runity_cli::merge::git_setup(project.root())? {
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
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = args.next().cloned(),
            "--engine-path" => {
                let path = args.next().context("--engine-path wants a path")?;
                engine = Engine::Path(std::path::absolute(path)?);
            }
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => folder = Some(PathBuf::from(other)),
        }
    }
    let folder = folder.context("runity new <folder>")?;
    let name = match name {
        Some(name) => name,
        None => std::path::absolute(&folder)?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .context("the folder has no name; pass --name")?,
    };
    let project =
        Project::create_with(&folder, &name, &engine).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "made {} in {}\n\n  cd {}\n  runity run       # the game, reloading scenes as you save them
  runity run --hot # and the game's own code too (needs dioxus-cli)\n  runity check     # what does not resolve",
        project.name(),
        project.root().display(),
        folder.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn sync(project: &Project) -> Result<ExitCode> {
    let done = runity_import::sync(project);
    let mut failed = 0;
    for item in &done {
        let what = match &item.change {
            Change::New => "new".to_string(),
            Change::Changed => "changed".to_string(),
            Change::Built => "built".to_string(),
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
    let findings = runity_cli::check(project);
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
    let timed = runity_cli::rebuild_time(&project, runs)?;
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

fn merge(rest: &[String]) -> Result<ExitCode> {
    let [base, ours, theirs, path @ ..] = rest else {
        bail!("runity merge BASE OURS THEIRS [PATH]");
    };
    let path = path.first().map(PathBuf::from);
    let outcome = runity_cli::merge::merge_files(
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
        eprintln!("runity merge: {name}: {conflict}");
    }
    if outcome.conflicts.is_empty() && !outcome.by_lines {
        eprintln!("runity merge: {name}: merged by entity");
    }
    Ok(if outcome.conflicts.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn add(rest: &[String]) -> Result<ExitCode> {
    let [what, name, at @ ..] = rest else {
        bail!("runity add component|system NAME [PROJECT]");
    };
    let project = find(at)?;
    match what.as_str() {
        "component" => {
            let file = runity_cli::add::component(&project, name)?;
            println!(
                "wrote {}\n  a scene gives it as components: {{ \"{name}\": () }}",
                project.relative(&file).unwrap_or_default()
            );
        }
        "system" => {
            let added = runity_cli::add::system(&project, name)?;
            println!(
                "wrote {}",
                project.relative(&added.file).unwrap_or_default()
            );
            if added.called {
                println!("  and step in src/main.rs runs it last");
            } else {
                println!(
                    "  src/main.rs has no `// systems, in order` list; call it from step:\n    {}",
                    runity_cli::add::system_call(name)
                );
            }
        }
        other => bail!("runity add component|system NAME — not `{other}`"),
    }
    Ok(ExitCode::SUCCESS)
}

fn rename(rest: &[String]) -> Result<ExitCode> {
    let [from, to] = rest else {
        bail!("runity rename FROM TO");
    };
    // Paths as typed, from wherever the command was run; the project is
    // the one the file is in.
    let from = std::path::absolute(from)?;
    let to = std::path::absolute(to)?;
    let project = Project::find(&from).map_err(|e| anyhow::anyhow!("{e}"))?;
    let done = runity_import::assets::rename(&project, &from, &to)?;
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
        bail!("runity uses FILE");
    };
    let file = std::path::absolute(file)?;
    let project = Project::find(&file).map_err(|e| anyhow::anyhow!("{e}"))?;
    let found = runity_import::assets::usages(&project, &file)?;
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
    let mut release = true;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = Some(args.next().context("--out wants a folder")?.into()),
            "--debug" => release = false,
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => at.push(other.to_string()),
        }
    }
    let project = find(&at)?;
    let out = out.unwrap_or_else(|| project.root().join("build"));
    let built = runity_cli::build::build(&project, &out, release)?;
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
