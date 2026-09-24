//! `scrap new --template NAME`: a project of the engine's own — a folder
//! of `examples/` with a `scrap.ron` — copied and handed over (DNA,
//! postulate 8), renamed, and pointed at the engine the new project gets.
//! CI builds the examples, so a template is a game that works.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use scrap::project::Engine;
use scrap::Project;

/// What is made from a template's sources, never copied.
const DERIVED: [&str; 5] = ["target", "library", ".scrap", "build", "Cargo.lock"];

/// Where the engine's examples are, in the checkout this command was built
/// from.
pub fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// The templates there are: every example with a `scrap.ron`.
pub fn names() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(examples())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(scrap::project::FILE).is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// Copy the template `template` to `folder` as the project `name`, its game
/// getting the engine from `engine`.
pub fn create(template: &str, folder: &Path, name: &str, engine: &Engine) -> Result<Project> {
    let from = examples().join(template);
    if !from.join(scrap::project::FILE).is_file() {
        bail!("no template `{template}` — there are: {}", names().join(", "));
    }
    if folder.join(scrap::project::FILE).exists() {
        bail!("{} is already a project", folder.display());
    }
    copy(&from, folder)?;

    let manifest = folder.join(scrap::project::FILE);
    let text = std::fs::read_to_string(&manifest)?;
    std::fs::write(&manifest, renamed_manifest(&text, name))?;
    let cargo = folder.join("Cargo.toml");
    if cargo.is_file() {
        let text = std::fs::read_to_string(&cargo)?;
        let text = pointed(&text, &scrap::project::crate_name(name), &engine.cargo_source(folder))
            .context("the template's Cargo.toml has no scrap line with a path")?;
        std::fs::write(&cargo, text)?;
    }
    Project::open(folder).map_err(|e| anyhow::anyhow!("{e}"))
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if DERIVED.iter().any(|d| name == *d) {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if entry.file_type()?.is_dir() {
            copy(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// `scrap.ron` with the project's `name` in place of the template's.
fn renamed_manifest(text: &str, name: &str) -> String {
    let mut done = false;
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if !done && trimmed.starts_with("name:") {
                done = true;
                let indent = &line[..line.len() - trimmed.len()];
                format!("{indent}name: {name:?},")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// `Cargo.toml` with the package named `crate_name` and the `scrap` line
/// getting the engine from `source` rather than the template's path.
fn pointed(text: &str, crate_name: &str, source: &str) -> Option<String> {
    let mut named = false;
    let mut pointed = false;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if !named && line.trim_start().starts_with("name =") {
            named = true;
            out.push_str(&format!("name = \"{crate_name}\"\n"));
            continue;
        }
        if line.trim_start().starts_with("scrap =") {
            let start = line.find("path = \"")?;
            let after = start + "path = \"".len();
            let end = after + line[after..].find('"')? + 1;
            pointed = true;
            out.push_str(&format!("{}{source}{}\n", &line[..start], &line[end..]));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    pointed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_renamed_and_pointed_at_the_engine() {
        let cargo = "[package]\nname = \"kitchen\"\n\n[dependencies]\nruntity = { path = \"../../crates/scrap\", features = [\"audio\"] }\n"
            .replace("runtity", "scrap");
        let out = pointed(&cargo, "soup", "git = \"https://example.com/scrap\"").unwrap();
        assert!(out.contains("name = \"soup\""), "{out}");
        assert!(
            out.contains("scrap = { git = \"https://example.com/scrap\", features = [\"audio\"] }"),
            "{out}"
        );
        let ron = renamed_manifest("(\n    name: \"kitchen\",\n    engine: \"0.1.0\",\n)", "Soup");
        assert!(ron.contains("name: \"Soup\","), "{ron}");
        assert!(names().contains(&"kitchen".to_string()));
    }
}
