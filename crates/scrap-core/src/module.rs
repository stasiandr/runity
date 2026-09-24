//! A module's manifest (DNA, postulate 3; docs/modules.md): `module.ron`
//! beside a module's `Cargo.toml` — its name and version, which engine it
//! fits, which modules it stands on, whether it brings code to the game or
//! to the editor, the fields of a scene's lines it reads, and its assets.
//! A module with no code is a manifest and `assets/`.
//!
//! A project lists its modules in `scrap.ron` (`modules`); the engine
//! knows what each official one is by its manifest, and `scrap check`
//! says where the list, the build and `Cargo.toml` disagree.

use serde::{Deserialize, Serialize};

use crate::parts::PartKind;

/// What `module.ron` says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// What a project's `scrap.ron` names it by: `physics`.
    pub name: String,
    pub version: String,
    /// The engine versions it fits: `0.1` for any 0.1.x.
    pub engine: String,
    /// One line on what it is, for `scrap new` and the editor to show.
    #[serde(default)]
    pub what: String,
    /// The modules it stands on, by name. The core is always there and is
    /// not listed.
    #[serde(default)]
    pub depends: Vec<String>,
    /// It brings code to the game.
    #[serde(default = "yes")]
    pub runtime: bool,
    /// It brings extensions to the editor.
    #[serde(default)]
    pub editor: bool,
    /// The fields of a scene's lines, overrides and looks it reads.
    #[serde(default)]
    pub parts: Vec<String>,
    /// Folders of assets it brings, relative to it.
    #[serde(default)]
    pub assets: Vec<String>,
    /// The engine's cargo feature that brings it, for one that can be left
    /// out of a build; `None` for one the engine always has.
    #[serde(default)]
    pub feature: Option<String>,
}

fn yes() -> bool {
    true
}

impl Manifest {
    /// Read `module.ron`'s text.
    pub fn parse(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    /// Whether this module fits the engine at `version` (`0.1.3` fits
    /// `engine: "0.1"`): the same numbers as far as the manifest gives them.
    pub fn fits(&self, version: &str) -> bool {
        let want: Vec<&str> = self.engine.split('.').collect();
        let have: Vec<&str> = version.split('.').collect();
        want.len() <= have.len() && want.iter().zip(&have).all(|(w, h)| w == h)
    }

    /// Where the manifest's `parts` and the fields the module's code reads
    /// (`read`, its `part_kinds()`) disagree, in words.
    pub fn part_problems(&self, read: &[PartKind]) -> Vec<String> {
        let mut out = Vec::new();
        for kind in read {
            if !self.parts.iter().any(|p| p == kind.name) {
                out.push(format!(
                    "module `{}` reads `{}` but its manifest does not list it",
                    self.name, kind.name
                ));
            }
        }
        for part in &self.parts {
            if !read.iter().any(|k| k.name == part) {
                out.push(format!(
                    "module `{}` lists `{part}` but reads no such field",
                    self.name
                ));
            }
        }
        out
    }
}

/// Where a project's list of modules does not hold together: a module no
/// manifest answers to, or one that fits another engine. What a listed
/// module stands on comes with it ([`with_depends`]) and need not be
/// listed. `known` is every module the engine knows.
pub fn list_problems(listed: &[String], known: &[Manifest], engine: &str) -> Vec<String> {
    let mut out = Vec::new();
    for name in listed {
        let Some(module) = known.iter().find(|m| &m.name == name) else {
            let names: Vec<&str> = known.iter().map(|m| m.name.as_str()).collect();
            let hint = crate::spelling::closest(name, names.iter().copied())
                .map(|c| format!(" — did you mean `{c}`?"))
                .unwrap_or_default();
            out.push(format!("scrap.ron lists module `{name}`, which the engine does not have{hint}"));
            continue;
        };
        if !module.fits(engine) {
            out.push(format!(
                "module `{name}` is for engine {}, this is {engine}",
                module.engine
            ));
        }
    }
    out
}

/// `listed` with every module they stand on, in the order a build takes
/// them: what a module needs before it.
pub fn with_depends(listed: &[String], known: &[Manifest]) -> Vec<String> {
    fn visit(name: &str, known: &[Manifest], out: &mut Vec<String>) {
        if out.iter().any(|n| n == name) {
            return;
        }
        if let Some(m) = known.iter().find(|m| m.name == name) {
            for dep in &m.depends {
                visit(dep, known, out);
            }
        }
        out.push(name.to_string());
    }
    let mut out = Vec::new();
    for name in listed {
        visit(name, known, &mut out);
    }
    out
}

/// The engine's cargo features a game listing `listed` builds with: those
/// of the listed modules and every module they stand on, sorted. The
/// engine's defaults are not assumed — a game says every one it wants.
pub fn features(listed: &[String], known: &[Manifest]) -> Vec<String> {
    let mut out: Vec<String> = with_depends(listed, known)
        .iter()
        .filter_map(|name| known.iter().find(|m| &m.name == name)?.feature.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(name: &str, depends: &[&str]) -> Manifest {
        Manifest::parse(&format!(
            "(name: {name:?}, version: \"0.1.0\", engine: \"0.1\", depends: {depends:?})"
        ))
        .unwrap()
    }

    #[test]
    fn a_manifest_reads_with_only_what_it_must_say() {
        let m = module("physics", &["geometry"]);
        assert!(m.runtime && !m.editor && m.parts.is_empty() && m.feature.is_none());
        assert!(m.fits("0.1.4") && !m.fits("0.2.0") && !m.fits("0"));
    }

    #[test]
    fn a_list_is_told_what_it_misses_and_misspells() {
        let known = [module("geometry", &[]), module("physics", &["geometry"])];
        let said = list_problems(&["physcs".into()], &known, "0.1.0");
        assert!(said[0].contains("did you mean `physics`"), "{said:?}");
        assert!(list_problems(&["physics".into()], &known, "0.1.0").is_empty());
        assert_eq!(with_depends(&["physics".into()], &known), ["geometry", "physics"]);
        let mut nav = module("navigation", &["physics"]);
        nav.feature = Some("navigation".into());
        let mut phys = module("physics", &["geometry"]);
        phys.feature = Some("physics".into());
        let known = [module("geometry", &[]), phys, nav];
        assert_eq!(features(&["navigation".into()], &known), ["navigation", "physics"]);
    }

    #[test]
    fn parts_a_module_reads_and_its_manifest_lists_must_agree() {
        let mut m = module("wind", &[]);
        m.parts = vec!["wind".into(), "gust".into()];
        let said = m.part_problems(&crate::wind::part_kinds());
        assert_eq!(said, ["module `wind` lists `gust` but reads no such field"]);
    }
}
