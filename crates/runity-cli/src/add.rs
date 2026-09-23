//! `runity add component NAME` and `runity add system NAME`: the game's
//! code, laid out the one way (DNA, postulate 7).
//!
//! A component is a file in `src/components/`, and that is all: the game's
//! `build.rs` registers it under the file's name. A system is a file in
//! `src/systems/` plus one call in `step`, where the order systems run in
//! is written down — added at the end of that list, which is where a new
//! system usually goes and where anyone reading the order will see it.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use runity::project::{self, Project};

/// The line `step` runs a system with.
pub fn system_call(name: &str) -> String {
    format!("systems::{name}::run(world, seconds);")
}

/// Where `step`'s list of systems is marked in `src/main.rs`.
const MARKER: &str = "// systems, in order";

/// Write `src/components/NAME.rs`.
pub fn component(project: &Project, name: &str) -> Result<PathBuf> {
    project::valid_name(name).map_err(anyhow::Error::msg)?;
    let path = project
        .root()
        .join(project::COMPONENTS)
        .join(format!("{name}.rs"));
    if path.exists() {
        bail!(
            "{} is already there",
            project.relative(&path).unwrap_or_default()
        );
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, project::component_file(name))?;
    Ok(path)
}

/// What adding a system did.
#[derive(Debug)]
pub struct AddedSystem {
    pub file: PathBuf,
    /// Whether the call went into `step`. `false` when `src/main.rs` has no
    /// `// systems, in order` line to find the list by; the caller says
    /// where the call goes instead of guessing.
    pub called: bool,
}

/// Write `src/systems/NAME.rs` and run it last in the game's step (`tick`).
pub fn system(project: &Project, name: &str) -> Result<AddedSystem> {
    project::valid_name(name).map_err(anyhow::Error::msg)?;
    let file = project
        .root()
        .join(project::SYSTEMS)
        .join(format!("{name}.rs"));
    if file.exists() {
        bail!(
            "{} is already there",
            project.relative(&file).unwrap_or_default()
        );
    }
    let main = project.root().join(project::SRC).join("main.rs");
    let text = std::fs::read_to_string(&main).with_context(|| main.display().to_string())?;
    let called = match insert_call(&text, name) {
        Some(changed) => {
            std::fs::write(&main, changed)?;
            true
        }
        None => false,
    };
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, project::system_file(name))?;
    Ok(AddedSystem { file, called })
}

/// `main.rs` with the call added after the last system call under the
/// marker, indented like it.
fn insert_call(text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let marker = lines.iter().position(|l| l.trim() == MARKER)?;
    let indent: String = lines[marker]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let mut last = marker;
    while lines
        .get(last + 1)
        .is_some_and(|l| l.trim_start().starts_with("systems::"))
    {
        last += 1;
    }
    // Called the way the one before it is: `run(world, seconds)` in a
    // project made now, `run(&mut self.world, seconds)` in an older one.
    let call = match lines[last].trim_start().split_once("::run(") {
        Some((_, args)) if last > marker => format!("systems::{name}::run({args}"),
        _ => system_call(name),
    };
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.insert(last + 1, format!("{indent}{call}"));
    let mut joined = out.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_goes_last_in_the_list_with_its_indent() {
        let text = "fn step() {\n    // systems, in order\n    systems::spin::run(&mut self.world, seconds);\n    physics();\n}\n";
        assert_eq!(
            insert_call(text, "patrol").unwrap(),
            "fn step() {\n    // systems, in order\n    systems::spin::run(&mut self.world, seconds);\n    systems::patrol::run(&mut self.world, seconds);\n    physics();\n}\n"
        );
        assert!(insert_call("fn step() {}\n", "patrol").is_none());
        // A list with nothing in it yet takes the current form.
        assert_eq!(
            insert_call("fn tick() {\n    // systems, in order\n}\n", "patrol").unwrap(),
            "fn tick() {\n    // systems, in order\n    systems::patrol::run(world, seconds);\n}\n"
        );
    }
}
