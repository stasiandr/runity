//! The Table: many things of one kind as rows, their fields as columns —
//! Odin's table, Scriptable Sheets, Unreal's DataTable, the view a
//! designer balances a game in.
//!
//! Two kinds of rows, named by a source:
//!
//! * **`tuning/enemies.ron`** — the records of a tuning file. A file that
//!   is a map `{ "goblin": (…), "orc": (…) }` is a row a record; a file
//!   that is one struct is one row. The columns are the fields the game
//!   reads the records as (`library/tuning.ron`, written by the game like
//!   `components.ron`), or, before it has said, the fields the records
//!   write.
//! * **anything else is a search of the open scene** (`c:enemy`,
//!   `p:campfire`): a row an entity. With a `c:` term the columns are that
//!   component's fields; without one, the entity's own — position, model,
//!   material.
//!
//! The file stays the truth (DNA, postulate 2): a cell is set in the text
//! where it stands, and nothing else in the file moves. An empty cell is a
//! field the record leaves out, at its default; setting a cell to nothing
//! takes the field out again. A scene's cell is an edit of the document,
//! undone with everything else; a tuning file is written at once — the
//! running game picks it up (postulate 1) — and [`Session::undo_cell`]
//! takes the writes back, newest first.

use std::collections::BTreeMap;
use std::path::PathBuf;

use scrap::records::{self, Record};
use scrap::shape::Shape;
use scrap::EntityId;

use crate::{EditError, EditResult, Session};

/// A table, as [`Session::table`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// What it was read from: `tuning/enemies.ron`, or a search.
    pub source: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    /// What is wrong with the file as a whole: text that is not RON, a
    /// value that is not records.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// What [`Session::set_cell`] calls it: a field's name.
    pub name: String,
    /// What it holds, `number`, `bool`; empty when nothing has said.
    pub shape: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// What [`Session::set_cell`] calls it: a record's key (empty for a
    /// file that is one struct), an entity's id.
    pub key: String,
    /// What a person reads it as: the key, the entity's name.
    pub label: String,
    /// A cell per column, as the file writes it; empty where the record
    /// leaves the field at its default.
    pub cells: Vec<String>,
    /// What is wrong with this record for the game's type.
    pub problems: Vec<String>,
}

/// A tuning file as it was before a cell was set, and after.
#[derive(Debug, Clone)]
pub(crate) struct CellEdit {
    path: PathBuf,
    before: String,
    after: String,
    what: String,
}

/// The fields a scene's row shows when no component is named.
const ENTITY_COLUMNS: [&str; 7] = [
    "name", "model", "material", "position", "rotation", "scale", "layer",
];

/// What a source is.
enum Source {
    /// A tuning file, and its name in `tuning/` (`enemies`).
    Tuning(PathBuf, String),
    /// A search of the scene, and the component its `c:` names.
    Scene(String, Option<String>),
}

impl Session {
    /// Every table there is to open: each file in `tuning/`, and `c:<name>`
    /// for each component the open scene uses.
    pub fn table_sources(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(project) = self.project() {
            let dir = project.root().join(scrap::project::TUNING);
            let mut files = Vec::new();
            scrap_import::walk(&dir, &mut |p| {
                if p.extension().is_some_and(|e| e == "ron") {
                    files.push(p.to_path_buf());
                }
            });
            files.sort();
            out.extend(
                files
                    .iter()
                    .filter_map(|p| project.relative(p).map(|r| r.replace('\\', "/"))),
            );
        }
        let flat = self.expanded().flatten();
        let mut components: Vec<&String> =
            flat.iter().flat_map(|(e, _)| e.components.keys()).collect();
        components.sort();
        components.dedup();
        out.extend(components.into_iter().map(|c| format!("c:{c}")));
        out
    }

    /// What the game reads each tuning file as, as it last wrote it
    /// (`library/tuning.ron`). Empty when it has not.
    pub fn tuning_shapes(&self) -> BTreeMap<String, Shape> {
        self.project()
            .map(|p| p.root().join(scrap::project::TUNING_SHAPES))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| scrap::ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn source(&self, source: &str) -> EditResult<Source> {
        let source = source.trim();
        let dir = format!("{}/", scrap::project::TUNING);
        if let Some(rest) = source.strip_prefix(&dir) {
            let project = self.project().ok_or(EditError::NotInProject)?;
            let name = rest.strip_suffix(".ron").unwrap_or(rest).to_string();
            let path = project
                .root()
                .join(scrap::project::TUNING)
                .join(format!("{name}.ron"));
            if !path.is_file() {
                let known: Vec<String> = self
                    .table_sources()
                    .into_iter()
                    .filter(|s| s.starts_with(&dir))
                    .collect();
                return Err(EditError::Scene(format!(
                    "no file {source}{}",
                    scrap::spelling::closest(source, known.iter().map(String::as_str))
                        .map(|n| format!(" — did you mean {n}?"))
                        .unwrap_or_default()
                )));
            }
            return Ok(Source::Tuning(path, name));
        }
        let component = source.split_whitespace().find_map(|term| {
            let (key, value) = term.split_once(':')?;
            matches!(key, "c" | "component").then(|| value.to_string())
        });
        // The component's name as the scene writes it: `c:` matches any case.
        let component = component.map(|wanted| {
            self.expanded()
                .flatten()
                .iter()
                .flat_map(|(e, _)| e.components.keys())
                .find(|k| k.eq_ignore_ascii_case(&wanted))
                .cloned()
                .unwrap_or(wanted)
        });
        Ok(Source::Scene(source.to_string(), component))
    }

    /// The rows and columns of `source`: a tuning file (`tuning/enemies.ron`)
    /// or a search of the scene (`c:enemy`). See the module's words.
    pub fn table(&self, source: &str) -> EditResult<Table> {
        match self.source(source)? {
            Source::Tuning(path, name) => self.tuning_table(source, &path, &name),
            Source::Scene(query, component) => self.scene_table(&query, component.as_deref()),
        }
    }

    fn tuning_table(&self, source: &str, path: &std::path::Path, name: &str) -> EditResult<Table> {
        let text = std::fs::read_to_string(path).map_err(|e| EditError::Io(e.to_string()))?;
        let shape = self.tuning_shapes().remove(name);
        let mut table = Table {
            source: source.to_string(),
            columns: Vec::new(),
            rows: Vec::new(),
            problems: Vec::new(),
        };
        let found = records::problems(shape.as_ref(), &text);
        table.problems = found
            .iter()
            .filter(|(r, _)| r.is_none())
            .map(|(_, p)| p.clone())
            .collect();
        let all = match records::records(&text) {
            Ok(all) => all,
            Err(e) => {
                if !table.problems.contains(&e) {
                    table.problems.push(e);
                }
                return Ok(table);
            }
        };
        table.columns = match shape.as_ref().and_then(records::record_fields) {
            Some(fields) => fields
                .iter()
                .map(|(n, s)| Column {
                    name: n.clone(),
                    shape: s.to_string(),
                })
                .collect(),
            None => written_fields(all.iter().map(|r| records::field_names(&text, r))),
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        for record in &all {
            let key = record.key.clone().unwrap_or_default();
            table.rows.push(Row {
                label: record.key.clone().unwrap_or_else(|| stem.clone()),
                cells: table
                    .columns
                    .iter()
                    .map(|c| records::field(&text, record, &c.name).unwrap_or_default())
                    .collect(),
                problems: found
                    .iter()
                    .filter(|(r, _)| r.is_some() && r.as_deref() == record.key.as_deref())
                    .map(|(_, p)| p.clone())
                    .collect(),
                key,
            });
        }
        Ok(table)
    }

    /// A component's text on an entity, `None` without it.
    fn component_text(&self, id: EntityId, component: &str) -> Option<String> {
        self.line(id)?
            .components
            .get(component)
            .map(|v| v.get_ron().trim().to_string())
    }

    fn scene_table(&self, query: &str, component: Option<&str>) -> EditResult<Table> {
        let ids = self.search(query)?;
        let shape = component.and_then(|c| self.component_shapes().remove(c));
        let columns: Vec<Column> = match component {
            None => ENTITY_COLUMNS
                .iter()
                .map(|n| Column {
                    name: n.to_string(),
                    shape: String::new(),
                })
                .collect(),
            Some(component) => {
                let fields = match shape.as_ref() {
                    Some(Shape::Struct(fields)) => fields
                        .iter()
                        .map(|(n, s)| Column {
                            name: n.clone(),
                            shape: s.to_string(),
                        })
                        .collect(),
                    _ => written_fields(ids.iter().map(|id| {
                        self.component_text(*id, component)
                            .map(|t| records::field_names(&t, &whole(&t)))
                            .unwrap_or_default()
                    })),
                };
                std::iter::once(Column {
                    name: "name".into(),
                    shape: "text".into(),
                })
                .chain(fields)
                .collect()
            }
        };
        let mut rows = Vec::new();
        for id in ids {
            let Some(line) = self.line(id) else { continue };
            let label = line.name.clone();
            let cells = match component {
                None => {
                    let fields = self.inspect(id).unwrap_or_default();
                    columns
                        .iter()
                        .map(|c| {
                            fields
                                .iter()
                                .find(|f| f.name == c.name)
                                .map(|f| f.value.clone())
                                .unwrap_or_default()
                        })
                        .collect()
                }
                Some(component) => {
                    let text = self.component_text(id, component);
                    columns
                        .iter()
                        .map(|c| match (c.name.as_str(), &text) {
                            ("name", _) => label.clone(),
                            (field, Some(t)) => {
                                records::field(t, &whole(t), field).unwrap_or_default()
                            }
                            (_, None) => String::new(),
                        })
                        .collect()
                }
            };
            let problems = match (component, &shape) {
                (Some(component), Some(shape)) => self
                    .component_text(id, component)
                    .map(|t| shape.problems(&t))
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            rows.push(Row {
                key: id.to_string(),
                label,
                cells,
                problems,
            });
        }
        Ok(Table {
            source: query.to_string(),
            columns,
            rows,
            problems: Vec::new(),
        })
    }

    /// Set one cell from RON text — `35`, `"fast"`, `Some(2.0)` — or take
    /// the field out, back to its default, with empty text. A scene's cell
    /// is one undo step; a tuning file's is written at once, and
    /// [`Session::undo_cell`] takes it back. Text that does not read, or
    /// does not fit what the game reads the field as, changes nothing and
    /// says why.
    pub fn set_cell(
        &mut self,
        source: &str,
        row: &str,
        column: &str,
        value: &str,
    ) -> EditResult<()> {
        self.set_cells(source, &[row.to_string()], column, value)
    }

    /// [`Session::set_cell`] for several rows of one column at once — a
    /// column selected and typed into — as one step.
    pub fn set_cells(
        &mut self,
        source: &str,
        rows: &[String],
        column: &str,
        value: &str,
    ) -> EditResult<()> {
        let value = value.trim();
        match self.source(source)? {
            Source::Tuning(path, name) => {
                is_value(value)?;
                self.set_tuning_cells(&path, &name, rows, column, value)
            }
            Source::Scene(_, component) => {
                let ids = rows
                    .iter()
                    .map(|r| {
                        r.parse::<EntityId>()
                            .map_err(|_| EditError::Scene(format!("`{r}` is not an entity id")))
                    })
                    .collect::<EditResult<Vec<_>>>()?;
                let before = self.history.depth();
                let result = ids.iter().try_for_each(|id| {
                    self.set_scene_cell(*id, component.as_deref(), column, value)
                });
                if let Err(e) = result {
                    while self.history.depth() > before {
                        self.undo()?;
                    }
                    return Err(e);
                }
                self.history
                    .squash(self.history.depth().saturating_sub(before));
                Ok(())
            }
        }
    }

    fn set_scene_cell(
        &mut self,
        id: EntityId,
        component: Option<&str>,
        column: &str,
        value: &str,
    ) -> EditResult<()> {
        let Some(component) = component.filter(|_| column != "name") else {
            return self.set_field(id, column, value);
        };
        is_value(value)?;
        if let Some(Shape::Struct(fields)) = self.component_shapes().get(component) {
            check_field(fields, column, value)?;
        }
        let text = self
            .component_text(id, component)
            .unwrap_or_else(|| "()".to_string());
        let next = scrap::ron_edit::set_field(&text, column, (!value.is_empty()).then_some(value))
            .ok_or_else(|| {
                EditError::Scene(format!(
                    "`{component}` on {id}: could not find where `{column}` goes"
                ))
            })?;
        self.set_field(id, &format!("components.{component}"), &next)
    }

    fn set_tuning_cells(
        &mut self,
        path: &std::path::Path,
        name: &str,
        rows: &[String],
        column: &str,
        value: &str,
    ) -> EditResult<()> {
        let before = std::fs::read_to_string(path).map_err(|e| EditError::Io(e.to_string()))?;
        let shape = self.tuning_shapes().remove(name);
        if let Some(fields) = shape.as_ref().and_then(records::record_fields) {
            check_field(fields, column, value)?;
        }
        let mut after = before.clone();
        for row in rows {
            let key = (!row.is_empty()).then_some(row.as_str());
            after = records::set(&after, key, column, (!value.is_empty()).then_some(value))
                .map_err(|e| EditError::Scene(format!("{}: {e}", path.display())))?;
        }
        if let Err(e) = scrap::ron::from_str::<scrap::ron::Value>(&after) {
            return Err(EditError::Scene(format!(
                "{}: the file would not read after this: {e}",
                path.display()
            )));
        }
        if after == before {
            return Ok(());
        }
        std::fs::write(path, &after).map_err(|e| EditError::Io(e.to_string()))?;
        let what = match rows {
            [one] if one.is_empty() => format!("{name} {column}"),
            [one] => format!("{name} {one}.{column}"),
            many => format!("{name} {column} of {}", many.len()),
        };
        self.cell_edits.push(CellEdit {
            path: path.to_path_buf(),
            before,
            after,
            what,
        });
        Ok(())
    }

    /// Take back the last cell set in a tuning file: the file as it was
    /// before it. `None` when there is nothing to take back; an error, and
    /// the file left alone, when it changed since — by hand, or by the
    /// game.
    pub fn undo_cell(&mut self) -> EditResult<Option<String>> {
        let Some(edit) = self.cell_edits.pop() else {
            return Ok(None);
        };
        let now = std::fs::read_to_string(&edit.path).map_err(|e| EditError::Io(e.to_string()))?;
        if now != edit.after {
            return Err(EditError::Scene(format!(
                "{} changed since `{}` was set; not undone",
                edit.path.display(),
                edit.what
            )));
        }
        std::fs::write(&edit.path, &edit.before).map_err(|e| EditError::Io(e.to_string()))?;
        Ok(Some(edit.what))
    }

    /// What [`Session::undo_cell`] would take back, in words.
    pub fn undo_cell_label(&self) -> Option<String> {
        self.cell_edits.last().map(|e| format!("set {}", e.what))
    }
}

/// Text that reads as a RON value, or nothing — a cell emptied.
fn is_value(value: &str) -> EditResult<()> {
    if value.is_empty() || scrap::ron::from_str::<scrap::ron::Value>(value).is_ok() {
        return Ok(());
    }
    Err(EditError::Scene(format!(
        "`{value}` is not a value: a number, true or false, \"text\" in quotes, (field: value)"
    )))
}

/// A component's text as one record.
fn whole(text: &str) -> Record {
    Record {
        key: None,
        span: 0..text.len(),
    }
}

/// The fields records write, in the order they first come: the columns
/// of a table nothing has described.
fn written_fields(each: impl Iterator<Item = Vec<String>>) -> Vec<Column> {
    let mut names: Vec<String> = Vec::new();
    for fields in each {
        for field in fields {
            if !names.contains(&field) {
                names.push(field);
            }
        }
    }
    names
        .into_iter()
        .map(|name| Column {
            name,
            shape: String::new(),
        })
        .collect()
}

/// A value for a field of a described record: a field it has, of a shape
/// the value fits.
fn check_field(fields: &[(String, Shape)], column: &str, value: &str) -> EditResult<()> {
    let Some((_, shape)) = fields.iter().find(|(n, _)| n == column) else {
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        return Err(EditError::Scene(format!(
            "no field `{column}`{} — there are {}",
            scrap::spelling::closest(column, names.iter().copied())
                .map(|n| format!(" (did you mean `{n}`?)"))
                .unwrap_or_default(),
            names.join(", ")
        )));
    };
    if value.is_empty() {
        return Ok(());
    }
    let problems = shape.problems(value);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(EditError::Scene(format!(
            "`{column}`: {}",
            problems.join("; ")
        )))
    }
}
