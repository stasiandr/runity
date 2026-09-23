//! `runity-import` — the command line behind the editor's drag-and-drop.
//!
//! ```text
//! runity-import --sync [PROJECT]          build the project's library from its sources
//! runity-import <source>... [options]     import into the source's project
//! runity-import <source>... --library DIR import into a library of your choosing
//! ```
//!
//! `--sync` is what a fresh clone runs: the library is derived and not
//! committed, so it is built from `assets/`, `materials/` and the `.rimport`
//! beside each source — changed ones by content hash, moved ones found by
//! it, new ones imported with the defaults.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use runity_import::{import_file, import_into, sync, Change, ImportSettings};

fn main() -> Result<()> {
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut library: Option<PathBuf> = None;
    let mut settings = ImportSettings::default();
    let mut sync_project: Option<PathBuf> = None;
    let mut sync_requested = false;
    // Only a flag the user gave overrides a sidecar: re-importing without
    // options rebuilds the asset the way it was built before.
    let mut options_given = false;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sync" => {
                sync_requested = true;
                if args.peek().is_some_and(|next| !next.starts_with('-')) {
                    sync_project = args.next().map(PathBuf::from);
                }
            }
            "--library" | "-o" => library = args.next().map(PathBuf::from),
            "--scale" => {
                options_given = true;
                settings.scale = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(settings.scale);
            }
            "--keep-origin" => {
                options_given = true;
                settings.origin_to_base = false;
            }
            // A normal map, a roughness map or a mask is not colour, and
            // decoding it from sRGB bends every value in it.
            "--linear" => {
                options_given = true;
                settings.srgb = false;
            }
            "--recompute-normals" => {
                options_given = true;
                settings.recompute_normals = true;
            }
            "--help" | "-h" => {
                println!(
                    "runity-import --sync [PROJECT]\n\
                     runity-import <source>... [--library DIR] [--scale F] \
                     [--keep-origin] [--recompute-normals] [--linear]\n\
                     \n\
                     --sync builds a project's library/ from its sources.\n\
                     A source is imported into the project it is in, unless --library says where.\n\
                     sources: .gltf .glb .obj .png .jpg .tga .bmp .wav .rmat"
                );
                return Ok(());
            }
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => sources.push(PathBuf::from(other)),
        }
    }

    if sync_requested {
        let at = sync_project.unwrap_or_else(|| PathBuf::from("."));
        let project = runity::Project::find(&at).map_err(|e| anyhow::anyhow!("{e}"))?;
        let done = sync(&project);
        let mut failed = 0;
        for item in &done {
            let what = match &item.change {
                Change::New => "new".to_string(),
                Change::Changed => "changed".to_string(),
                Change::Built => "built".to_string(),
                Change::Moved { from } => format!("moved from {from}"),
                Change::Gone => "gone".to_string(),
            };
            match &item.result {
                Ok(id) => println!("{} ({what}) -> {id}", item.source.display()),
                Err(e) => {
                    failed += 1;
                    eprintln!("{} ({what}): {e}", item.source.display());
                }
            }
        }
        println!(
            "{}: {} brought up to date, {failed} could not be",
            project.name(),
            done.len() - failed
        );
        return Ok(());
    }

    if sources.is_empty() {
        bail!("nothing to import; pass source files, or --sync to build a project's library");
    }

    for source in &sources {
        let out = match &library {
            Some(library) => {
                let mut settings = settings.clone();
                settings.source = source.to_string_lossy().into_owned();
                import_file(source, library, settings)?
            }
            None => {
                let project = runity::Project::find(source)
                    .map_err(|e| anyhow::anyhow!("{e}"))
                    .with_context(|| {
                        format!(
                            "{} is in no project; pass --library DIR to say where it goes",
                            source.display()
                        )
                    })?;
                let report = import_into(&project, source, options_given.then_some(&settings))?;
                if let Some(warning) = &report.warning {
                    eprintln!("warning: {warning}");
                }
                report.imported
            }
        };
        println!(
            "{} -> {} ({})",
            source.display(),
            out.asset.display(),
            out.id
        );
    }
    Ok(())
}
