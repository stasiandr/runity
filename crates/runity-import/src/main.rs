//! `runity-import <source>... --library <dir>` — the command line behind the
//! editor's drag-and-drop.
//!
//! The editor will call the library directly rather than shell out, but this
//! binary is what makes the pipeline usable before the editor exists, and
//! what rebuilds a whole library when the importer changes.

use std::path::PathBuf;

use anyhow::{bail, Result};
use runity_import::{import_file, ImportSettings};

fn main() -> Result<()> {
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut library = PathBuf::from("library");
    let mut settings = ImportSettings::default();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--library" | "-o" => {
                library = args.next().map(PathBuf::from).unwrap_or(library);
            }
            "--scale" => {
                settings.scale = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(settings.scale);
            }
            "--keep-origin" => settings.origin_to_base = false,
            // A normal map, a roughness map or a mask is not colour, and
            // decoding it from sRGB bends every value in it.
            "--linear" => settings.srgb = false,
            "--recompute-normals" => settings.recompute_normals = true,
            "--help" | "-h" => {
                println!(
                    "runity-import <source>... [--library DIR] [--scale F] \
                     [--keep-origin] [--recompute-normals] [--linear]\n\
                     \n\
                     sources: .gltf .glb .obj .png .jpg .tga .bmp"
                );
                return Ok(());
            }
            other if other.starts_with('-') => bail!("unknown option {other}"),
            other => sources.push(PathBuf::from(other)),
        }
    }

    if sources.is_empty() {
        bail!("nothing to import; pass one or more source files");
    }

    for source in &sources {
        // The source path is what the id is derived from, so an asset keeps
        // its identity across re-imports as long as the file stays put.
        let mut settings = settings.clone();
        settings.source = source.to_string_lossy().into_owned();
        let out = import_file(source, &library, settings)?;
        println!(
            "{} -> {} ({})",
            source.display(),
            out.asset.display(),
            out.id
        );
    }
    Ok(())
}
