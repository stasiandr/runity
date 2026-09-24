//! Layouts: where the panels are and how big the dock areas are, as text.
//!
//! One format for three things: the project's `.runity/studio.ron` (the
//! layout as it was left, back next run), the built-in presets (Unity's
//! Default and Tall) and the presets a person saves, which are theirs and
//! not the project's — `layouts/<name>.ron` in the per-user config folder
//! ([`config_dir`]), so every project they open offers them.
//!
//! ```text
//! (left: 320, right: 340, lower: 220, name: "Tall",
//!  docks: (left: down(0.450, [*hierarchy], [*project]), right: [*inspector],
//!          lower: [*console, history, git], closed: []))
//! ```
//!
//! Text, not a binary blob, so it reads in a diff and by hand (DNA,
//! postulate 2). The format before stacks could split (`docks:
//! "hierarchy|inspector|project,console"`) still reads.

use std::path::{Path, PathBuf};

use crate::dock::Arrangement;

/// Presets every editor has, in the order the menus list them.
pub const BUILT_IN: [&str; 2] = ["Default", "Tall"];

/// Stands in for the per-user config folder when set: tests point it at a
/// folder of their own.
pub const CONFIG_DIR_VAR: &str = "RUNITY_CONFIG_DIR";

/// The editor's per-user config folder: `~/Library/Application
/// Support/runity` on macOS, `%APPDATA%\runity` on Windows,
/// `$XDG_CONFIG_HOME/runity` or `~/.config/runity` elsewhere — or
/// `RUNITY_CONFIG_DIR`. Not made here: nothing needs it until a layout is
/// saved.
pub fn config_dir() -> Option<PathBuf> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env(CONFIG_DIR_VAR) {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    }?;
    Some(base.join("runity"))
}

/// A layout read from text: whatever of it the text had.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Layout {
    /// The left area's width, the right's, the height of the one under the
    /// view.
    pub sizes: [Option<f32>; 3],
    /// The preset it came from, shown on the toolbar's Layout button.
    pub name: Option<String>,
    pub arrangement: Option<Arrangement>,
    /// The Console's Clear on Play: kept with the layout in the project's
    /// file, not in a preset.
    pub clear_on_play: Option<bool>,
}

impl Layout {
    /// A built-in preset.
    pub fn built_in(name: &str) -> Option<Layout> {
        let (sizes, arrangement) = match name {
            "Default" => ([270.0, 340.0, 210.0], Arrangement::default_layout()),
            "Tall" => ([320.0, 340.0, 220.0], Arrangement::tall()),
            _ => return None,
        };
        Some(Layout {
            sizes: sizes.map(Some),
            name: Some(name.to_string()),
            arrangement: Some(arrangement),
            clear_on_play: None,
        })
    }

    /// As the files hold it.
    pub fn write(&self) -> String {
        let mut out = String::from("(");
        for (key, size) in ["left", "right", "lower"].into_iter().zip(self.sizes) {
            if let Some(v) = size {
                out.push_str(&format!("{key}: {v:.0}, "));
            }
        }
        if let Some(name) = &self.name {
            out.push_str(&format!("name: {name:?}, "));
        }
        if let Some(a) = &self.arrangement {
            out.push_str(&format!("docks: {}, ", a.write()));
        }
        if let Some(on) = self.clear_on_play {
            out.push_str(&format!("clear_on_play: {on}, "));
        }
        let mut out = out.trim_end_matches(", ").to_string();
        out.push_str(")\n");
        out
    }

    /// Read what [`Layout::write`] wrote, or the format before it.
    pub fn read(text: &str) -> Layout {
        // The sizes come before the tree, whose areas have the same keys.
        let head = &text[..text.find("docks:").unwrap_or(text.len())];
        let number = |key: &str| -> Option<f32> {
            let at = head.find(&format!("{key}:"))? + key.len() + 1;
            head[at..].split([',', ')']).next()?.trim().parse().ok()
        };
        let quoted = |key: &str| -> Option<String> {
            let at = text.find(&format!("{key}:"))? + key.len() + 1;
            let rest = text[at..].trim_start().strip_prefix('"')?;
            Some(rest.split('"').next()?.to_string())
        };
        let docks = text.find("docks:").map(|at| text[at + 6..].trim_start());
        let arrangement = match docks {
            Some(rest) if rest.starts_with('"') => Arrangement::from_old(
                &quoted("docks").unwrap_or_default(),
                &quoted("active").unwrap_or_default(),
            ),
            Some(rest) => balanced(rest).and_then(Arrangement::read),
            None => None,
        };
        let clear_on_play = text.find("clear_on_play:").and_then(|at| {
            let rest = text[at + 14..].trim_start();
            if rest.starts_with("true") {
                Some(true)
            } else if rest.starts_with("false") {
                Some(false)
            } else {
                None
            }
        });
        Layout {
            sizes: [number("left"), number("right"), number("lower")],
            name: quoted("name"),
            arrangement,
            clear_on_play,
        }
    }
}

/// The parenthesis `text` starts with, up to the one that closes it.
fn balanced(text: &str) -> Option<&str> {
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where a person's saved layouts are.
pub fn user_dir(config: &Path) -> PathBuf {
    config.join("layouts")
}

/// The saved layouts' names, sorted.
pub fn saved(config: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(user_dir(config))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path.extension()? == "ron").then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    names.sort();
    names
}

/// A name a layout can be saved under: a file name on every system, and
/// not a built-in's.
pub fn check_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') {
        return Err("a layout needs a name".into());
    }
    if name
        .chars()
        .any(|c| "/\\:*?\"<>|".contains(c) || c.is_control())
    {
        return Err(format!(
            "{name:?}: a layout's name cannot have / \\ : * ? \" < > |"
        ));
    }
    if BUILT_IN.iter().any(|b| b.eq_ignore_ascii_case(name)) {
        return Err(format!("{name} is built in: save under another name"));
    }
    Ok(name)
}

fn file(config: &Path, name: &str) -> PathBuf {
    user_dir(config).join(format!("{name}.ron"))
}

/// Save a layout as `name`, over one of that name.
pub fn save(config: &Path, name: &str, layout: &Layout) -> Result<PathBuf, String> {
    let name = check_name(name)?;
    let path = file(config, name);
    std::fs::create_dir_all(user_dir(config)).map_err(|e| e.to_string())?;
    std::fs::write(&path, layout.write()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// A layout by name: a built-in or a saved one.
pub fn load(config: Option<&Path>, name: &str) -> Result<Layout, String> {
    if let Some(layout) = Layout::built_in(name) {
        return Ok(layout);
    }
    let config = config.ok_or("no config folder to keep layouts in")?;
    let path = file(config, name);
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no layout called {name}"))?;
    let mut layout = Layout::read(&text);
    if layout.arrangement.is_none() {
        return Err(format!(
            "{}: not a layout this editor reads",
            path.display()
        ));
    }
    layout.name = Some(name.to_string());
    layout.clear_on_play = None;
    Ok(layout)
}

/// Delete a saved layout.
pub fn delete(config: &Path, name: &str) -> Result<(), String> {
    let name = name.trim();
    if Layout::built_in(name).is_some() {
        return Err(format!("{name} is built in and cannot be deleted"));
    }
    // A name, not a path: nothing outside the layouts folder goes.
    let name = check_name(name)?;
    let path = file(config, name);
    if !path.is_file() {
        return Err(format!("no layout called {name}"));
    }
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_reads_back_what_it_writes() {
        let mut tall = Layout::built_in("Tall").unwrap();
        tall.clear_on_play = Some(true);
        assert_eq!(Layout::read(&tall.write()), tall, "{}", tall.write());
    }

    #[test]
    fn the_format_before_splits_still_reads() {
        let old = "(left: 300, right: 340, lower: 210, docks: \"hierarchy,console|inspector|project\", active: \"console|inspector|project\")\n";
        let layout = Layout::read(old);
        assert_eq!(layout.sizes, [Some(300.0), Some(340.0), Some(210.0)]);
        let a = layout.arrangement.expect("mapped onto stacks");
        assert_eq!(
            a.regions[0],
            crate::dock::Tree::Stack {
                tabs: vec![crate::dock::Panel::Hierarchy, crate::dock::Panel::Console],
                active: Some(crate::dock::Panel::Console),
            }
        );
    }

    #[test]
    fn a_name_is_a_file_name_and_not_a_built_in() {
        assert!(check_name("My Layout").is_ok());
        assert!(check_name("a/b").is_err());
        assert!(check_name("tall").is_err());
        assert!(check_name("  ").is_err());
    }
}
