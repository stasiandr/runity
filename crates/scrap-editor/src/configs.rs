//! What the Configs window shows and does: the project's `configs/`, read
//! as tables and edited a cell at a time (docs/data.md).
//!
//! A config is whatever RON the game keeps there — one struct of numbers
//! (`world.ron`), or a struct with a list of shots in it (`flyby.ron`), or
//! a table of records by name (`materials.ron`). A file is read as it is
//! written, and whatever is a list or a map of like things becomes a grid —
//! one row a record, one column a field — and everything else a line of
//! its own. Text, not values: a cell shows what the file says,
//! `Burn(seconds: 3.0)` and all, so an enum keeps its variant's name and a
//! number its digits. A table's record shows what it takes from its `base`
//! too, marked as taken; its `id` is not shown, being nobody's to type.
//!
//! An edit is a cell's new text: it changes that span of the file and
//! nothing else ([`set`]), must still read — and, where the game has said
//! what its table holds (`library/tables.ron`), still fit — before it is
//! written, writes the `id` of every record that had none, and is one step
//! of the window's own undo ([`Edits`]). Whatever draws the window — the
//! studio, a test, an agent — does this.

use std::path::{Path, PathBuf};

use scrap::ron_text::{outline, Kind, Part};
use scrap::shape::Shape;
use scrap::table::{self, TableShape};

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
    /// is an empty cell.
    pub rows: Vec<Vec<String>>,
    /// Which cells the record takes from its `base` rather than saying
    /// itself; as many as the cells, all `false` outside a table.
    pub inherited: Vec<Vec<bool>>,
    /// Whether its rows are a table's records (docs/data.md).
    pub records: bool,
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

/// The top of `text`, scanned: an error in words, with the line and
/// column, when it is not RON. The scan is linear; serde's reading of a
/// whole file as a value is not (a table of thousands takes seconds), so it
/// is only asked what is wrong.
fn scan(text: &str) -> Result<Part<'_>, String> {
    outline(text).ok_or_else(|| match scrap::ron::from_str::<scrap::ron::Value>(text) {
        Err(e) => e.to_string(),
        Ok(_) => "the file does not scan as RON".into(),
    })
}

/// `text` as tables.
pub fn read(text: &str) -> Result<Config, String> {
    read_with(text, None)
}

/// `text` as tables, a table's records with a column for every field the
/// game's type has (`shape`), said or not: what is left out is there to
/// fill in.
pub fn read_with(text: &str, shape: Option<&TableShape>) -> Result<Config, String> {
    let top = scan(text)?;
    let mut config = Config::default();
    match &top.group {
        Some((Kind::Struct, fields)) => {
            for field in fields {
                let name = field.key.clone().unwrap_or_default();
                match grid(Some(name.clone()), field) {
                    Some(t) => config.tables.push(t),
                    None => config.fields.push((name, flat(field.text))),
                }
            }
        }
        Some((Kind::Map, _)) => match records(text, shape) {
            Some(t) => config.tables.push(t),
            None => config.tables.extend(grid(None, &top)),
        },
        _ => match grid(None, &top) {
            Some(t) => config.tables.push(t),
            None => config.fields.push((String::new(), flat(top.text))),
        },
    }
    Ok(config)
}

/// A table of records as a grid: each record with its base's fields under
/// its own, the ones it takes marked; `id` left out. `None` when the map
/// is not written as records.
fn records(text: &str, shape: Option<&TableShape>) -> Option<Table> {
    let written = table::written(text).ok()?;
    if written.is_empty() {
        return None;
    }
    let (resolved, _) = table::resolved(&written);
    let mut columns = vec!["name".to_string()];
    if written.iter().any(|r| r.base.is_some()) {
        columns.push("base".into());
    }
    for fields in &resolved {
        for (key, _) in fields {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    if let Some(Shape::Struct(fields)) = shape.map(|s| &s.shape) {
        for (key, _) in fields {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    let mut rows = Vec::with_capacity(written.len());
    let mut inherited = Vec::with_capacity(written.len());
    for (record, fields) in written.iter().zip(&resolved) {
        let mut row = vec![String::new(); columns.len()];
        let mut taken = vec![false; columns.len()];
        row[0] = record.name.clone();
        if let Some(base) = &record.base {
            row[1] = scrap::ron::to_string(base).unwrap_or_default();
        }
        for (key, text) in fields {
            if let Some(at) = columns.iter().position(|c| c == key) {
                row[at] = flat(text);
                taken[at] = !record.fields.iter().any(|(k, _)| k == key);
            }
        }
        rows.push(row);
        inherited.push(taken);
    }
    Some(Table {
        name: None,
        columns,
        rows,
        inherited,
        records: true,
    })
}

/// A list or a map as a grid: its items' fields are the columns, in the
/// order they are first met. Items that are not structs are one column,
/// `value`. `None` for anything that is not a list or a map, and for an
/// empty one — there is nothing to line up.
fn grid(name: Option<String>, part: &Part) -> Option<Table> {
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
        for field in fields_of(item) {
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
    let rows: Vec<Vec<String>> = items
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
            for field in fields_of(item) {
                let key = field.key.as_deref().unwrap_or_default();
                if let Some(at) = columns.iter().position(|c| c == key) {
                    row[at] = flat(field.text);
                }
            }
            row
        })
        .collect();
    let inherited = rows.iter().map(|r| vec![false; r.len()]).collect();
    Some(Table {
        name,
        columns,
        rows,
        inherited,
        records: false,
    })
}

/// An item's fields, when it is a struct: `(at: …, look: …)`.
fn fields_of<'p, 'a>(item: &'p Part<'a>) -> &'p [Part<'a>] {
    match &item.group {
        Some((Kind::Struct, fields)) => fields,
        _ => &[],
    }
}

/// A map's key without its quotes: `"Палка"` is Палка.
fn unquote(key: &str) -> String {
    scrap::ron::from_str::<String>(key).unwrap_or_else(|_| key.to_string())
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

// --- editing ------------------------------------------------------------

/// Where an edit goes: a line of the top struct, or a grid's cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Place {
    /// A field of the file's top struct, by name.
    Field(String),
    /// A grid's cell: the grid (the field it is under, `None` for the whole
    /// file), its row by place, its column by name.
    Cell {
        grid: Option<String>,
        row: usize,
        column: String,
    },
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Place::Field(key) => write!(f, "{key}"),
            Place::Cell { grid, row, column } => match grid {
                Some(grid) => write!(f, "{grid}[{row}].{column}"),
                None => write!(f, "[{row}].{column}"),
            },
        }
    }
}

/// `text` with the value at `place` made `value` (RON text; empty takes the
/// field away — a record then takes it from its base), the rest of the file
/// as it was. For a table's `name` column, `value` is the record's new name
/// as it reads, without quotes; for its `base`, a record's name, quoted or
/// not.
pub fn set(text: &str, place: &Place, value: &str) -> Result<String, String> {
    let value = value.trim();
    let new = (!value.is_empty()).then_some(value);
    let refused = || format!("{place}: the file is shaped past what an edit here understands");
    match place {
        Place::Field(key) => scrap::ron_edit::set_field(text, key, new).ok_or_else(refused),
        Place::Cell { grid, row, column } => {
            let top = scan(text)?;
            // A table's records: by name, the name itself renamed.
            if grid.is_none() {
                if let Some(Table {
                    rows,
                    records: true,
                    ..
                }) = records(text, None)
                {
                    let name = rows.get(*row).map(|r| r[0].clone()).ok_or_else(refused)?;
                    return match column.as_str() {
                        "name" if value.is_empty() => Err("a record needs a name".into()),
                        "name" => table::rename_record(text, &name, value).ok_or_else(refused),
                        "base" => {
                            let base = new.map(quoted);
                            table::set_field(text, &name, "base", base.as_deref())
                                .ok_or_else(refused)
                        }
                        _ => table::set_field(text, &name, column, new).ok_or_else(refused),
                    };
                }
            }
            let holder = match grid {
                None => &top,
                Some(field) => match &top.group {
                    Some((Kind::Struct, fields)) => fields
                        .iter()
                        .find(|f| f.key.as_deref() == Some(field))
                        .ok_or_else(|| format!("no field `{field}` in the file"))?,
                    _ => return Err(refused()),
                },
            };
            let Some((_, items)) = &holder.group else {
                return Err(refused());
            };
            let item = items
                .get(*row)
                .ok_or_else(|| format!("{place}: no such row"))?;
            let span = item.span.clone();
            let changed = if column == "value" && item.group.is_none() {
                new.ok_or("a list's item is taken away in the text")?
                    .to_string()
            } else {
                scrap::ron_edit::set_field(item.text, column, new).ok_or_else(refused)?
            };
            Ok(format!(
                "{}{changed}{}",
                &text[..span.start],
                &text[span.end..]
            ))
        }
    }
}

/// A name as RON text: quoted, unless it already is.
fn quoted(value: &str) -> String {
    if value.starts_with('"') {
        value.to_string()
    } else {
        scrap::ron::to_string(value).unwrap_or_default()
    }
}

/// What the game said its table at `file` holds, when it has.
pub fn shape_of(root: &Path, file: &Path) -> Option<TableShape> {
    let rel = file
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    table::read_shapes(root.join(scrap::project::TABLE_SHAPES))?
        .into_iter()
        .find(|s| s.holds(&rel))
}

/// The records of every table the game has said the shape of, by type:
/// what a link's picker offers.
pub fn index(root: &Path) -> table::Index {
    let shapes = table::read_shapes(root.join(scrap::project::TABLE_SHAPES)).unwrap_or_default();
    table::Index::build(&shapes, &|path| table::read_table_files(root, path))
}

/// What is wrong with a config's text as it would be written: not RON, a
/// table that no longer holds together, or — with what the game said of it
/// — a record that does not fit its type. Empty when it may be written.
pub fn problems(text: &str, shape: Option<&TableShape>) -> Vec<String> {
    if let Err(e) = scan(text) {
        return vec![e];
    }
    if !table::is_table(text) {
        return match scrap::ron::from_str::<scrap::ron::Value>(text) {
            Ok(_) => Vec::new(),
            Err(e) => vec![e.to_string()],
        };
    }
    let written = match table::written(text) {
        Ok(w) => w,
        Err(e) => return vec![e],
    };
    let mut out: Vec<String> = table::problems_of(&written)
        .into_iter()
        .filter(|p| !p.warning)
        .map(|p| p.to_string())
        .collect();
    if let Some(shape) = shape {
        let (fields, _) = table::resolved(&written);
        for (record, fields) in written.iter().zip(fields) {
            for problem in shape.shape.problems(&table::compose(&fields)) {
                out.push(format!(
                    "line {}: `{}`: {problem}",
                    record.line, record.name
                ));
            }
        }
    }
    out
}

/// What a cell may be picked from, as (what it reads, the text it writes):
/// an enum's variants, true or false, a table's records for a link to one.
/// `None` when it is typed. The shape is the column's, from what the game
/// said of its table.
pub fn choices(shape: &Shape, index: &table::Index) -> Option<Vec<(String, String)>> {
    match shape {
        Shape::Bool => Some(vec![
            ("true".into(), "true".into()),
            ("false".into(), "false".into()),
        ]),
        Shape::Enum(names) => Some(names.iter().map(|n| (n.clone(), n.clone())).collect()),
        Shape::Tagged(variants) if variants.iter().all(|(_, s)| *s == Shape::Unit) => Some(
            variants
                .iter()
                .map(|(n, _)| (n.clone(), n.clone()))
                .collect(),
        ),
        Shape::Record(record) => Some(
            index
                .names(record)
                .into_iter()
                .map(|name| {
                    let text = scrap::ron::to_string(&name).unwrap_or_default();
                    (name, text)
                })
                .collect(),
        ),
        Shape::Option(inner) => {
            let mut out = vec![("None".to_string(), "None".to_string())];
            out.extend(
                choices(inner, index)?
                    .into_iter()
                    .map(|(label, text)| (label, format!("Some({text})"))),
            );
            Some(out)
        }
        _ => None,
    }
}

/// One edit of a config: the file before and after, and what it was.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub file: PathBuf,
    pub before: String,
    pub after: String,
    /// `tools.ron: Палка.weight`.
    pub label: String,
}

/// The Configs window's own undo: every edit it made, a step each, undone
/// by writing the file back as it was — refused when the file has changed
/// since, so a save from elsewhere is never lost to an undo.
#[derive(Debug, Default)]
pub struct Edits {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl Edits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Make the value at `place` of the config at `file` read `value`, and
    /// write it: checked first (see [`problems`]), with every record's `id`
    /// written. `root` is the project's, where the game's shapes are.
    /// Returns the step's label; empty when nothing changed.
    pub fn set(
        &mut self,
        root: &Path,
        file: &Path,
        place: &Place,
        value: &str,
    ) -> Result<String, String> {
        let before =
            std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut after = set(&before, place, value)?;
        let shape = shape_of(root, file);
        let said = problems(&after, shape.as_ref());
        if !said.is_empty() {
            return Err(format!("not written: {}", said.join("; ")));
        }
        if let Some(settled) = table::settle_ids(&after) {
            after = settled;
        }
        if after == before {
            return Ok(String::new());
        }
        std::fs::write(file, &after).map_err(|e| format!("{}: {e}", file.display()))?;
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = format!("{name}: {}", describe(&before, place));
        self.undo.push(Edit {
            file: file.to_path_buf(),
            before,
            after,
            label: label.clone(),
        });
        self.redo.clear();
        Ok(label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    /// Put the last edit's file back as it was. `Ok(None)` with nothing to
    /// undo; an error, and the step dropped, when the file changed since.
    pub fn undo(&mut self) -> Result<Option<String>, String> {
        let Some(edit) = self.undo.pop() else {
            return Ok(None);
        };
        put(&edit.file, &edit.after, &edit.before)?;
        let label = edit.label.clone();
        self.redo.push(edit);
        Ok(Some(label))
    }

    pub fn redo(&mut self) -> Result<Option<String>, String> {
        let Some(edit) = self.redo.pop() else {
            return Ok(None);
        };
        put(&edit.file, &edit.before, &edit.after)?;
        let label = edit.label.clone();
        self.undo.push(edit);
        Ok(Some(label))
    }
}

/// A place in words: a record's by its name, `Палка.weight`.
fn describe(before: &str, place: &Place) -> String {
    if let Place::Cell {
        grid: None,
        row,
        column,
    } = place
    {
        if let Some(Table {
            rows,
            records: true,
            ..
        }) = records(before, None)
        {
            if let Some(r) = rows.get(*row) {
                return format!("{}.{column}", r[0]);
            }
        }
    }
    place.to_string()
}

/// Write `to` over `file`, if it still says `from`.
fn put(file: &Path, from: &str, to: &str) -> Result<(), String> {
    let now = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if now != from {
        return Err(format!(
            "{} changed since; not undone, so that change is kept",
            file.display()
        ));
    }
    std::fs::write(file, to).map_err(|e| format!("{}: {e}", file.display()))
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
        assert!(!shots.records);
    }

    const TABLE: &str = "// Tools.\n{\n    \"Палка\": (id: \"4c1e\", effects: [Burn(per_second: 1.0, seconds: 3.0)], weight: 1.0),\n    \"Доска\": (\n        id: \"9a02\",\n        base: \"Палка\",\n        weight: 2.0,\n    ),\n}\n";

    #[test]
    fn a_table_s_records_show_what_they_take_from_their_base_and_not_their_id() {
        let config = read(TABLE).unwrap();
        let table = &config.tables[0];
        assert!(table.records);
        assert_eq!(table.name, None);
        assert_eq!(table.columns, ["name", "base", "effects", "weight"]);
        assert_eq!(table.rows[0][0], "Палка");
        assert_eq!(table.rows[0][2], "[Burn(per_second: 1.0, seconds: 3.0)]");
        assert_eq!(
            table.rows[1],
            [
                "Доска",
                "\"Палка\"",
                "[Burn(per_second: 1.0, seconds: 3.0)]",
                "2.0"
            ]
        );
        assert_eq!(table.inherited[1], [false, false, true, false]);
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
    fn a_cell_changes_its_span_of_the_file_and_nothing_else() {
        let cell = |row: usize, column: &str| Place::Cell {
            grid: None,
            row,
            column: column.into(),
        };
        let out = set(TABLE, &cell(1, "weight"), "2.5").unwrap();
        assert_eq!(out, TABLE.replace("weight: 2.0", "weight: 2.5"));
        // Taking a field away: the record takes it from its base again.
        let out = set(TABLE, &cell(0, "weight"), "").unwrap();
        assert!(
            out.contains("(id: \"4c1e\", effects: [Burn(per_second: 1.0, seconds: 3.0)])"),
            "{out}"
        );
        // Given a field of its own where it took its base's.
        let out = set(TABLE, &cell(1, "effects"), "[]").unwrap();
        assert!(
            out.contains("        weight: 2.0,\n        effects: [],\n"),
            "{out}"
        );
        // The name is the key; the base is a name, quoted for it.
        let out = set(TABLE, &cell(0, "name"), "Сук").unwrap();
        assert!(out.contains("\"Сук\": (id: \"4c1e\""), "{out}");
        let out = set(&out, &cell(1, "base"), "Сук").unwrap();
        assert!(out.contains("base: \"Сук\""), "{out}");
        assert!(out.starts_with("// Tools.\n"));
        // A field of the top struct, and a cell of a grid under one.
        let flyby = "(\n    travel: 3.5,\n    shots: [\n        (at: (0.5, 5.4, 6.7), look: (1.9, 0.4, 0.2)),\n        (at: (1.0, 2.0, 3.0)),\n    ],\n)";
        let out = set(flyby, &Place::Field("travel".into()), "4.0").unwrap();
        assert_eq!(out, flyby.replace("3.5", "4.0"));
        let shot = Place::Cell {
            grid: Some("shots".into()),
            row: 1,
            column: "look".into(),
        };
        let out = set(flyby, &shot, "(9.0, 0.0, 0.0)").unwrap();
        assert!(
            out.contains("(at: (1.0, 2.0, 3.0), look: (9.0, 0.0, 0.0))"),
            "{out}"
        );
        let plain = Place::Cell {
            grid: None,
            row: 1,
            column: "value".into(),
        };
        assert_eq!(set("[1, 2, 3]", &plain, "5").unwrap(), "[1, 5, 3]");
    }

    #[test]
    fn a_cell_is_picked_from_what_its_shape_allows() {
        let index = table::Index::default();
        assert_eq!(
            choices(&Shape::Enum(vec!["Hard".into(), "Warm".into()]), &index).unwrap()[1],
            ("Warm".to_string(), "Warm".to_string())
        );
        let optional = choices(&Shape::Option(Box::new(Shape::Bool)), &index).unwrap();
        assert_eq!(optional[0].1, "None");
        assert_eq!(optional[1].1, "Some(true)");
        assert!(choices(&Shape::Float, &index).is_none());
    }

    /// A project with a table the game has said the shape of.
    fn project(name: &str) -> PathBuf {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct Tool {
            weight: f32,
            #[serde(default)]
            made_of: Option<scrap::Link<Tool>>,
        }
        impl scrap::Record for Tool {}
        let root =
            std::env::temp_dir().join(format!("scrap-configs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("configs")).unwrap();
        std::fs::write(
            root.join("configs/tools.ron"),
            "{\n    \"Палка\": (id: \"4c1e\", weight: 1.0),\n    \"Кость\": (weight: 3.0),\n}\n",
        )
        .unwrap();
        let mut tables = scrap::Tables::new();
        tables.register::<Tool>("configs/tools.ron");
        tables
            .write_shapes(root.join(scrap::project::TABLE_SHAPES))
            .unwrap();
        root
    }

    #[test]
    fn an_edit_is_checked_written_with_its_ids_and_undone_as_one_step() {
        let root = project("edit");
        let file = root.join("configs/tools.ron");
        let original = std::fs::read_to_string(&file).unwrap();
        let mut edits = Edits::new();
        let cell = Place::Cell {
            grid: None,
            row: 0,
            column: "weight".into(),
        };
        // What does not fit the game's type is not written.
        let e = edits.set(&root, &file, &cell, "\"heavy\"").unwrap_err();
        assert!(e.contains("`weight` is a number"), "{e}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        assert!(!edits.can_undo());

        let label = edits.set(&root, &file, &cell, "1.5").unwrap();
        assert_eq!(label, "tools.ron: Палка.weight");
        let now = std::fs::read_to_string(&file).unwrap();
        assert!(
            now.contains("\"Палка\": (id: \"4c1e\", weight: 1.5)"),
            "{now}"
        );
        let bone = scrap::RecordId::from_name("Кость");
        assert!(
            now.contains(&format!("\"Кость\": (id: \"{bone}\", weight: 3.0)")),
            "the first save writes ids: {now}"
        );

        assert_eq!(
            edits.undo().unwrap().as_deref(),
            Some("tools.ron: Палка.weight")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        assert_eq!(
            edits.redo().unwrap().as_deref(),
            Some("tools.ron: Палка.weight")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), now);

        // A save from elsewhere since: the undo does not throw it away.
        std::fs::write(&file, now.replace("3.0", "4.0")).unwrap();
        assert!(edits.undo().unwrap_err().contains("changed since"));
        assert!(std::fs::read_to_string(&file).unwrap().contains("4.0"));

        // A link is picked from the table's records.
        let shape = shape_of(&root, &file).unwrap();
        let made_of = shape.field("made_of").unwrap();
        let picks = choices(made_of, &index(&root)).unwrap();
        let labels: Vec<&str> = picks.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["None", "Кость", "Палка"]);
        assert_eq!(picks[2].1, "Some(\"Палка\")");
        std::fs::remove_dir_all(root).ok();
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
