//! What the Configs window shows: the project's `configs/`, read as tables.
//!
//! A config is whatever RON the game keeps there — one struct of numbers
//! (`world.ron`), or a struct with a list of shots in it (`flyby.ron`), or
//! a map of records by name (docs/data.md). None of it is typed here: the
//! editor does not link the game, so a file is read as it is written, and
//! whatever is a list or a map of like things becomes a grid — one row a
//! record, one column a field — and everything else a line of its own.
//! Text, not values: a cell shows what the file says, `Burn(seconds: 3.0)`
//! and all, so an enum keeps its variant's name and a number its digits.
//! Whatever draws the window — the studio, a test, an agent — draws this.

use std::path::{Path, PathBuf};

use scrap::ron_text::{outline, Kind, Part};

/// A config file, as the window shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// Fields of the top struct that are not tables: name and text.
    pub fields: Vec<(String, String)>,
    pub tables: Vec<Table>,
}

/// Like things side by side: a list's items or a map's records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    /// The field it is under, `None` when it is the whole file.
    pub name: Option<String>,
    /// The first is the rows' own name: a map's key or a list's `#`.
    pub columns: Vec<String>,
    /// As many cells as there are columns; a field a record does not have
    /// is an empty cell, as a record that takes it from its `base`.
    pub rows: Vec<Vec<String>>,
}

/// The RON files under `configs/`, folders and all, in order.
pub fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&root.join(scrap::project::CONFIGS), &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "ron") {
            out.push(path);
        }
    }
}

/// `text` as tables. An error in words, with the line and column, when it
/// is not RON.
pub fn read(text: &str) -> Result<Config, String> {
    // The scan is linear; serde's reading of a whole file as a value is
    // not, so it only says what is wrong.
    let Some(top) = outline(text) else {
        return Err(match scrap::ron::from_str::<scrap::ron::Value>(text) {
            Err(e) => e.to_string(),
            Ok(_) => "the file does not scan as RON".into(),
        });
    };
    let mut config = Config::default();
    match &top.group {
        Some((Kind::Struct, fields)) => {
            for field in fields {
                let name = field.key.clone().unwrap_or_default();
                match table(Some(name.clone()), field) {
                    Some(t) => config.tables.push(t),
                    None => config.fields.push((name, flat(field.text))),
                }
            }
        }
        _ => match table(None, &top) {
            Some(t) => config.tables.push(t),
            None => config.fields.push((String::new(), flat(top.text))),
        },
    }
    Ok(config)
}

/// A list or a map as a grid: its items' fields are the columns, in the
/// order they are first met. Items that are not structs are one column,
/// `value`. `None` for anything that is not a list or a map, and for an
/// empty one — there is nothing to line up.
fn table(name: Option<String>, part: &Part) -> Option<Table> {
    let (kind, items) = part.group.as_ref()?;
    if *kind == Kind::Struct || items.is_empty() {
        return None;
    }
    let first = match kind {
        Kind::Map => "name",
        _ => "#",
    };
    let mut columns = vec![first.to_string()];
    for item in items {
        for field in record(item) {
            let key = field.key.clone().unwrap_or_default();
            if !columns.contains(&key) {
                columns.push(key);
            }
        }
    }
    let plain = columns.len() == 1;
    if plain {
        columns.push("value".into());
    }
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mut row = vec![String::new(); columns.len()];
            row[0] = match &item.key {
                Some(key) => unquote(key),
                None => i.to_string(),
            };
            if plain {
                row[1] = flat(item.text);
            }
            for field in record(item) {
                let key = field.key.as_deref().unwrap_or_default();
                if let Some(at) = columns.iter().position(|c| c == key) {
                    row[at] = flat(field.text);
                }
            }
            row
        })
        .collect();
    Some(Table {
        name,
        columns,
        rows,
    })
}

/// An item's fields, when it is a struct: `(at: …, look: …)`.
fn record<'p, 'a>(item: &'p Part<'a>) -> &'p [Part<'a>] {
    match &item.group {
        Some((Kind::Struct, fields)) => fields,
        _ => &[],
    }
}

/// A map's key without its quotes: `"Палка"` is Палка.
fn unquote(key: &str) -> String {
    key.strip_prefix('"')
        .and_then(|k| k.strip_suffix('"'))
        .unwrap_or(key)
        .to_string()
}

/// A value on one line: its comments out, its runs of space one space.
fn flat(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                while chars.peek().is_some_and(|&c| c != '\n') {
                    chars.next();
                }
            }
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
    }
    // No space just inside brackets or before a comma: `[ (a: 1) , ]`
    // reads as `[(a: 1),]` would.
    let out = out
        .replace("( ", "(")
        .replace("[ ", "[")
        .replace("{ ", "{")
        .replace(" )", ")")
        .replace(" ]", "]")
        .replace(" }", "}")
        .replace(" ,", ",")
        .replace(",)", ")")
        .replace(",]", "]")
        .replace(",}", "}");
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_struct_of_numbers_is_lines() {
        let config =
            read("// the world\n(\n    gravity: -9.81, // down\n    wind: (1.0, 0.0),\n)").unwrap();
        assert_eq!(
            config.fields,
            [
                ("gravity".to_string(), "-9.81".to_string()),
                ("wind".to_string(), "(1.0, 0.0)".to_string()),
            ]
        );
        assert!(config.tables.is_empty());
    }

    #[test]
    fn a_list_of_structs_in_a_struct_is_a_grid_under_its_field() {
        let config = read(
            "(\n    travel: 3.5,\n    shots: [\n        // front\n        (at: (0.5, 5.4, 6.7), look: (1.9, 0.4, 0.2)),\n        (at: (1.0, 2.0, 3.0), look: (0.0, 0.0, 0.0), hold: 1.0),\n    ],\n)",
        )
        .unwrap();
        assert_eq!(config.fields, [("travel".to_string(), "3.5".to_string())]);
        let shots = &config.tables[0];
        assert_eq!(shots.name.as_deref(), Some("shots"));
        assert_eq!(shots.columns, ["#", "at", "look", "hold"]);
        assert_eq!(
            shots.rows[0],
            ["0", "(0.5, 5.4, 6.7)", "(1.9, 0.4, 0.2)", ""]
        );
        assert_eq!(shots.rows[1][3], "1.0");
    }

    #[test]
    fn a_map_of_records_is_a_grid_by_name_with_variants_as_written() {
        let config = read(
            "{\n    \"Палка\": (id: \"4c1e\", effects: [Burn(per_second: 1.0, seconds: 3.0)]),\n    \"Доска\": (id: \"9a02\", base: \"Палка\"),\n}",
        )
        .unwrap();
        let table = &config.tables[0];
        assert_eq!(table.name, None);
        assert_eq!(table.columns, ["name", "id", "effects", "base"]);
        assert_eq!(table.rows[0][0], "Палка");
        assert_eq!(table.rows[0][2], "[Burn(per_second: 1.0, seconds: 3.0)]");
        assert_eq!(table.rows[1], ["Доска", "\"9a02\"", "", "\"Палка\""]);
    }

    #[test]
    fn a_list_of_plain_values_is_one_column() {
        let table = &read("[1, 2, 3]").unwrap().tables[0];
        assert_eq!(table.columns, ["#", "value"]);
        assert_eq!(table.rows[2], ["2", "3"]);
    }

    #[test]
    fn what_is_not_ron_says_where() {
        let e = read("(\n    gravity: -9.81\n    wind: 1,\n)").unwrap_err();
        assert!(e.contains("3:"), "{e}");
    }

    #[test]
    fn a_comment_or_a_string_with_spaces_stays_right_on_one_line() {
        assert_eq!(flat("[\n  \"a  b\", // why\n  2,\n]"), "[\"a  b\", 2]");
    }

    #[test]
    fn the_files_are_every_ron_under_configs_folders_too() {
        let root = std::env::temp_dir().join(format!("scrap-configs-{}", std::process::id()));
        let configs = root.join(scrap::project::CONFIGS);
        std::fs::create_dir_all(configs.join("items")).unwrap();
        std::fs::write(configs.join("world.ron"), "()").unwrap();
        std::fs::write(configs.join("items/tools.ron"), "{}").unwrap();
        std::fs::write(configs.join("notes.txt"), "").unwrap();
        let names: Vec<String> = files(&root)
            .iter()
            .map(|p| {
                p.strip_prefix(&configs)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["items/tools.ron", "world.ron"]);
        std::fs::remove_dir_all(root).ok();
    }
}
