//! Collision layers: which bodies pass through which.
//!
//! One file per project, `layers.ron`, names the layers and the pairs that
//! do not collide; a scene line puts a body on one with `layer: "debris"`.
//! Unity's layers and collision matrix, as text a diff shows:
//!
//! ```text
//! (
//!     layers: ["default", "player", "debris"],
//!     ignore: [("debris", "player")],
//! )
//! ```
//!
//! A line with no layer is on `default`, which every project has whether
//! the file names it or not. At most 32, which is what the physics world
//! can tell apart. A name the file does not have is `check`'s to report;
//! the physics world puts such a body on `default` rather than dropping it.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file, in a project's root.
pub const FILE: &str = "layers.ron";

/// A project's layers and the pairs of them that do not collide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layers {
    pub layers: Vec<String>,
    #[serde(default)]
    pub ignore: Vec<(String, String)>,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            layers: vec!["default".into()],
            ignore: Vec::new(),
        }
    }
}

impl Layers {
    /// Read a layers file. What cannot mean anything — more than 32, a name
    /// twice, an ignored pair naming a layer that is not there — is an error
    /// in words.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text =
            crate::files::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut layers: Layers =
            ron::from_str(&text).map_err(|e| format!("{}:{e}", path.display()))?;
        if !layers.layers.iter().any(|l| l == "default") {
            layers.layers.insert(0, "default".into());
        }
        let problems = layers.problems();
        if !problems.is_empty() {
            return Err(format!("{}: {}", path.display(), problems.join("; ")));
        }
        Ok(layers)
    }

    /// A project's layers: its `layers.ron`, or just `default` without one.
    pub fn of(project: &crate::Project) -> Result<Self, String> {
        let path = project.root().join(FILE);
        if crate::files::is_file(&path) {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.layers.len() > 32 {
            out.push(format!(
                "{} layers, and physics tells 32 apart",
                self.layers.len()
            ));
        }
        for (i, name) in self.layers.iter().enumerate() {
            if self.layers[..i].contains(name) {
                out.push(format!("`{name}` is named twice"));
            }
        }
        for (a, b) in &self.ignore {
            for name in [a, b] {
                if !self.layers.contains(name) {
                    out.push(format!(
                        "`{name}` in ignore is not one of the layers{}",
                        crate::spelling::closest(name, self.layers.iter().map(String::as_str))
                            .map(|n| format!(" — did you mean `{n}`?"))
                            .unwrap_or_default()
                    ));
                }
            }
        }
        out
    }

    /// Which layer a line's `layer` names: empty is `default`.
    pub fn index(&self, name: &str) -> Option<usize> {
        let name = if name.is_empty() { "default" } else { name };
        self.layers.iter().position(|l| l == name)
    }

    /// The bit a layer is, and the bits of every layer it collides with. An
    /// unknown name is `default`'s.
    pub fn groups(&self, name: &str) -> (u32, u32) {
        let index = self.index(name).or_else(|| self.index("")).unwrap_or(0);
        let me = &self.layers[index];
        let mut filter = 0u32;
        for (i, other) in self.layers.iter().enumerate().take(32) {
            let ignored = self
                .ignore
                .iter()
                .any(|(a, b)| (a == me && b == other) || (a == other && b == me));
            if !ignored {
                filter |= 1 << i;
            }
        }
        (1 << index.min(31), filter)
    }

    /// The bits of the named layers together: a query that looks only at
    /// those.
    pub fn mask(&self, names: &[&str]) -> u32 {
        names
            .iter()
            .filter_map(|n| self.index(n))
            .fold(0, |mask, i| mask | (1 << i.min(31)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layers() -> Layers {
        Layers {
            layers: vec!["default".into(), "player".into(), "debris".into()],
            ignore: vec![("debris".into(), "player".into())],
        }
    }

    #[test]
    fn an_ignored_pair_passes_through_each_other_and_nothing_else_does() {
        let l = layers();
        let (player, player_hits) = l.groups("player");
        let (debris, debris_hits) = l.groups("debris");
        let (default, default_hits) = l.groups("");
        assert_eq!((player, debris, default), (0b010, 0b100, 0b001));
        assert_eq!(player_hits & debris, 0, "the player walks through debris");
        assert_eq!(debris_hits & player, 0, "and debris through the player");
        assert_ne!(debris_hits & debris, 0, "debris piles on debris");
        assert_ne!(default_hits & player, 0);
        assert_eq!(l.groups("nonsense"), l.groups("default"));
        assert_eq!(l.mask(&["player", "debris"]), 0b110);
    }

    #[test]
    fn a_file_that_cannot_mean_anything_says_why() {
        let dir = std::env::temp_dir().join("runity-layers");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        std::fs::write(
            &path,
            r#"(layers: ["player", "debris", "player"], ignore: [("debri", "player")])"#,
        )
        .unwrap();
        let e = Layers::load(&path).unwrap_err();
        assert!(e.contains("`player` is named twice"), "{e}");
        assert!(e.contains("did you mean `debris`?"), "{e}");
        std::fs::write(&path, r#"(layers: ["player"])"#).unwrap();
        assert_eq!(
            Layers::load(&path).unwrap().layers,
            ["default", "player"],
            "default is always there"
        );
    }
}
