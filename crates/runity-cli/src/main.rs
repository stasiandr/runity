//! `runity` — a project from the command line.
//!
//! ```text
//! runity new <folder> [--name NAME] [--engine-path PATH]
//! runity sync  [PROJECT]     build library/ from the sources
//! runity check [PROJECT]     what does not resolve, with file and entity
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
runity sync [PROJECT]
    Build library/ from assets/ and materials/: changed sources by content
    hash, moved ones found by it, new ones imported, sidecars written.
runity check [PROJECT]
    Every model, material and prefab a scene names, every id, every sidecar.
    Exits 1 when something does not resolve.

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
        "check" => check(&find(&rest)?),
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
        "made {} in {}\n\n  cd {}\n  cargo run        # the game, reloading scenes as you save them\n  runity check     # what does not resolve",
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
