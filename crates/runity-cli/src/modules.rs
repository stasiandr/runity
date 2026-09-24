//! A project's modules (docs/modules.md): `runity.ron` lists them, and the
//! engine's cargo features in the game's `Cargo.toml` follow from the list
//! — written by `runity modules sync`, held to it by `runity check`.

use anyhow::{bail, Context, Result};
use runity::modules::{self, Manifest};
use runity::Project;

/// The modules a project lists, with what they stand on; `None` when its
/// `runity.ron` lists none.
pub fn listed(project: &Project) -> Option<Vec<String>> {
    let known = modules::official();
    let listed = &project.manifest().modules;
    if listed.is_empty() {
        return None;
    }
    Some(modules::with_depends(listed, &known))
}

/// The engine's features the game's `Cargo.toml` asks for, the defaults
/// counted when it does not turn them off; `None` without a `runity`
/// dependency on one line.
pub fn cargo_features(cargo: &str) -> Option<Vec<String>> {
    let line = runity_line(cargo)?;
    let mut out: Vec<String> = match line.find("features = [") {
        Some(at) if !line[..at].ends_with("default-") => {
            let rest = &line[at + "features = [".len()..];
            rest[..rest.find(']')?]
                .split(',')
                .map(|f| f.trim().trim_matches('"').to_string())
                .filter(|f| !f.is_empty())
                .collect()
        }
        _ => Vec::new(),
    };
    if !line.contains("default-features = false") {
        out.extend(modules::DEFAULT_FEATURES.iter().map(|f| f.to_string()));
    }
    // A module's features only: one of a pass (`ray-tracing`) is the game's
    // to keep, and no module's list says it.
    let of_modules: Vec<String> = modules::official().into_iter().filter_map(|m| m.feature).collect();
    out.retain(|f| of_modules.contains(f));
    out.sort();
    out.dedup();
    Some(out)
}

/// The features a `runity` line names, as written.
fn listed_features(line: &str) -> Vec<String> {
    let Some(at) = line.find("features = [").filter(|at| !line[..*at].ends_with("default-")) else {
        return Vec::new();
    };
    let rest = &line[at + "features = [".len()..];
    let Some(end) = rest.find(']') else {
        return Vec::new();
    };
    rest[..end]
        .split(',')
        .map(|f| f.trim().trim_matches('"').to_string())
        .filter(|f| !f.is_empty())
        .collect()
}

/// `cargo` with its `runity` line building exactly `features` of the
/// modules, and whatever else it asked for.
pub fn with_features(cargo: &str, features: &[String]) -> Result<String> {
    let line = runity_line(cargo).context("Cargo.toml has no `runity = { ... }` line")?;
    let open = line.find('{').context("the runity line is not an inline table")?;
    let close = line.rfind('}').context("the runity line is not an inline table")?;
    let kept: Vec<&str> = split_top(&line[open + 1..close])
        .into_iter()
        .map(str::trim)
        .filter(|f| !f.is_empty() && !f.starts_with("features") && !f.starts_with("default-features"))
        .collect();
    // Features that are no module's — a render pass's — stay as they were.
    let of_modules: Vec<String> = modules::official().into_iter().filter_map(|m| m.feature).collect();
    let others: Vec<String> = listed_features(line)
        .into_iter()
        .filter(|f| !of_modules.contains(f))
        .collect();
    let mut all: Vec<String> = features.iter().cloned().chain(others).collect();
    all.sort();
    all.dedup();
    let list: Vec<String> = all.iter().map(|f| format!("\"{f}\"")).collect();
    let new = format!(
        "runity = {{ {}, default-features = false, features = [{}] }}",
        kept.join(", "),
        list.join(", ")
    );
    Ok(cargo.replacen(line, &new, 1))
}

/// Where the list, the engine's modules and `Cargo.toml` part, in words.
pub fn problems(project: &Project) -> Vec<String> {
    let listed = &project.manifest().modules;
    if listed.is_empty() {
        return Vec::new();
    }
    let known = modules::official();
    let mut out = modules::list_problems(listed, &known, env!("CARGO_PKG_VERSION"));
    let Ok(cargo) = std::fs::read_to_string(project.root().join("Cargo.toml")) else {
        return out;
    };
    let Some(have) = cargo_features(&cargo) else {
        return out;
    };
    let want = modules::features(listed, &known);
    if have != want {
        out.push(format!(
            "Cargo.toml builds the engine with [{}], runity.ron's modules want [{}] — `runity modules sync` writes it",
            have.join(", "),
            want.join(", ")
        ));
    }
    out
}

/// Write the game's `Cargo.toml` from `runity.ron`'s modules; what it
/// builds with now.
pub fn sync(project: &Project) -> Result<Vec<String>> {
    let listed = &project.manifest().modules;
    if listed.is_empty() {
        bail!("runity.ron lists no modules: add `modules: [...]` first (`runity modules` shows them)");
    }
    let known = modules::official();
    let problems = modules::list_problems(listed, &known, env!("CARGO_PKG_VERSION"));
    if !problems.is_empty() {
        bail!("{}", problems.join("\n"));
    }
    let path = project.root().join("Cargo.toml");
    let cargo = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let features = modules::features(listed, &known);
    std::fs::write(&path, with_features(&cargo, &features)?)?;
    Ok(features)
}

/// Every official module, and whether the project lists it.
pub fn table(project: &Project) -> Vec<(Manifest, bool)> {
    let listed = listed(project);
    modules::official()
        .into_iter()
        .map(|m| {
            let on = listed.as_ref().is_none_or(|l| l.contains(&m.name));
            (m, on)
        })
        .collect()
}

fn runity_line(cargo: &str) -> Option<&str> {
    cargo
        .lines()
        .find(|l| l.trim_start().starts_with("runity =") || l.trim_start().starts_with("runity="))
}

/// Split at commas not inside brackets or quotes.
fn split_top(text: &str) -> Vec<&str> {
    let (mut out, mut depth, mut quoted, mut start) = (Vec::new(), 0i32, false, 0);
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '{' if !quoted => depth += 1,
            ']' | '}' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KITCHEN: &str = "[dependencies]\nanyhow = \"1\"\nruntity = { path = \"../../crates/runity\", features = [\"desktop-shell\", \"audio\"] }\n";

    #[test]
    fn the_features_a_cargo_line_builds_count_the_defaults_it_keeps() {
        let cargo = KITCHEN.replace("runtity", "runity");
        assert_eq!(
            cargo_features(&cargo).unwrap(),
            ["audio", "desktop-shell", "navigation", "physics"]
        );
        let cargo = cargo.replace("\"audio\"]", "\"audio\", \"ray-tracing\"]");
        let written = with_features(&cargo, &["audio".into()]).unwrap();
        assert!(
            written.contains("runity = { path = \"../../crates/runity\", default-features = false, features = [\"audio\", \"ray-tracing\"] }"),
            "{written}"
        );
        assert_eq!(cargo_features(&written).unwrap(), ["audio"]);
        assert!(written.starts_with("[dependencies]\nanyhow"));
    }
}
