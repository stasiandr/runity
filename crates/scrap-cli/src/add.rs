//! `scrap add component NAME` and `scrap add system NAME`: the game's
//! code, laid out the one way (DNA, postulate 7).
//!
//! Code lies by feature (docs/layout.md): `scrap add component cooking/pot`
//! writes `src/cooking/pot.rs`, and the game's `build.rs` registers it
//! under the file's name, `"pot"`. Without a feature, the file goes in a
//! folder of its own name (`src/pot/pot.rs`) — or in `src/components/` of
//! a project laid out before. A system is such a file plus one call in
//! `step`, where the order systems run in is written down — added at the
//! end of that list, which is where a new system usually goes and where
//! anyone reading the order will see it.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use scrap::project::{self, Project};

/// The line `step` runs a system with.
pub fn system_call(name: &str) -> String {
    format!("profile.time({name:?}, || systems::{name}::run(world, seconds));")
}

/// Whether a line of `step` is a system's call: `systems::x::run(…)`, or
/// that inside `profile.time("x", || …)` as a project made now writes it.
fn is_call(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("systems::")
        || (line.starts_with("profile.time(") && line.contains("systems::"))
}

/// Where `step`'s list of systems is marked in `src/main.rs`.
const MARKER: &str = "// systems, in order";

/// Where `spec` — `name` or `feature/name` — puts a new file, and its
/// name. `legacy` is the folder a project laid out before keeps the kind in.
fn place(project: &Project, spec: &str, legacy: &str) -> Result<(PathBuf, String)> {
    let spec = spec.trim_matches('/').replace('\\', "/");
    let (feature, name) = match spec.rsplit_once('/') {
        Some((feature, name)) => (Some(feature.to_string()), name.to_string()),
        None => (None, spec.clone()),
    };
    project::valid_name(&name).map_err(anyhow::Error::msg)?;
    if let Some(feature) = &feature {
        for part in feature.split('/') {
            project::valid_name(part).map_err(anyhow::Error::msg)?;
        }
    }
    let src = project.root().join(project::SRC);
    let dir = match feature {
        Some(feature) => src.join(feature),
        None if project.root().join(legacy).is_dir() => project.root().join(legacy),
        None => src.join(&name),
    };
    let path = dir.join(format!("{name}.rs"));
    if path.exists() {
        bail!(
            "{} is already there",
            project.relative(&path).unwrap_or_default()
        );
    }
    if let Some((_, there)) = project::component_files(&src)
        .into_iter()
        .find(|(n, _)| *n == name)
    {
        bail!(
            "a component is already called `{name}`: {}",
            project.relative(&there).unwrap_or_default()
        );
    }
    Ok((path, name))
}

/// Write the component `spec` names: `src/<feature>/NAME.rs`.
pub fn component(project: &Project, spec: &str) -> Result<PathBuf> {
    let (path, name) = place(project, spec, project::legacy::COMPONENTS)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, project::component_file(&name))?;
    Ok(path)
}

/// What adding a system did.
#[derive(Debug)]
pub struct AddedSystem {
    pub file: PathBuf,
    /// The system's name: its file's, what `step` calls it by.
    pub name: String,
    /// Whether the call went into `step`. `false` when `src/main.rs` has no
    /// `// systems, in order` line to find the list by; the caller says
    /// where the call goes instead of guessing.
    pub called: bool,
}

/// Write the system `spec` names and run it last in the game's step
/// (`tick`).
pub fn system(project: &Project, spec: &str) -> Result<AddedSystem> {
    let (file, name) = place(project, spec, project::legacy::SYSTEMS)?;
    let main = project.root().join(project::SRC).join("main.rs");
    let text = std::fs::read_to_string(&main).with_context(|| main.display().to_string())?;
    let called = match insert_call(&text, &name) {
        Some(changed) => {
            std::fs::write(&main, changed)?;
            true
        }
        None => false,
    };
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, project::system_file(&name))?;
    Ok(AddedSystem { file, name, called })
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
    while lines.get(last + 1).is_some_and(|l| is_call(l)) {
        last += 1;
    }
    // Called the way the one before it is: timed as a project made now
    // does it, or `run(&mut self.world, seconds)` as an older one.
    let before = lines[last].trim_start();
    let call = if last == marker || before.starts_with("profile.time(") {
        system_call(name)
    } else {
        match before.split_once("::run(") {
            Some((_, args)) => format!("systems::{name}::run({args}"),
            None => system_call(name),
        }
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
            "fn tick() {\n    // systems, in order\n    profile.time(\"patrol\", || systems::patrol::run(world, seconds));\n}\n"
        );
        // After a timed call, timed.
        let timed = "    // systems, in order\n    profile.time(\"spin\", || systems::spin::run(world, seconds));\n";
        assert!(insert_call(timed, "patrol")
            .unwrap()
            .contains("    profile.time(\"patrol\", || systems::patrol::run(world, seconds));"));
    }
}
