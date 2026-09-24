//! A tuning file as a table: its records as rows, their fields as columns.
//!
//! Designers balance a game in tables — every enemy a row, hit points and
//! speed its columns (Odin's table, Scriptable Sheets, Unreal's
//! DataTable). The truth stays the RON file in `tuning/`, the table is a
//! way of looking at it: a file whose value is a map of records,
//! `{ "goblin": (hp: 10), "orc": (hp: 30) }`, is a table of them — the
//! game reads it as `Tuned<BTreeMap<String, Enemy>>` — and a file whose
//! value is one struct is a table of one row. A cell is changed in the
//! text where it stands and nowhere else, so the diff of a new number is
//! that number (DNA, postulate 2).

use std::ops::Range;

use crate::ron_edit;
use crate::shape::Shape;

/// One record of a tuning file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Its key in the map; `None` for a file that is one struct.
    pub key: Option<String>,
    /// Where its value is in the file's text.
    pub span: Range<usize>,
}

/// Where the file's value starts, past comments and space.
fn first(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if let Some(next) = ron_edit::skip_comment(text, i) {
            i = next;
        } else if b[i].is_ascii_whitespace() {
            i += 1;
        } else {
            return Some(i);
        }
    }
    None
}

/// A map entry's key and where its value is.
fn entry(text: &str, item: Range<usize>) -> Option<(String, Range<usize>)> {
    let b = text.as_bytes();
    let mut i = item.start;
    let key = if b.get(i) == Some(&b'"') {
        let end = ron_edit::skip_string(b, i);
        let key = ron::from_str::<String>(text.get(i..end)?).ok()?;
        i = end;
        key
    } else {
        let colon = text[i..item.end].find(':')? + i;
        let key = text[i..colon].trim().to_string();
        i = colon;
        key
    };
    while i < item.end && b[i] != b':' {
        i += 1;
    }
    i += 1;
    while i < item.end && b[i].is_ascii_whitespace() {
        i += 1;
    }
    (i < item.end).then_some((key, i..item.end))
}

/// The records of a tuning file, in the order it writes them. An error in
/// words for a file that is neither a struct nor a map of them.
pub fn records(text: &str) -> Result<Vec<Record>, String> {
    let start = first(text).ok_or("the file is empty")?;
    if text.as_bytes()[start] == b'{' {
        let found = ron_edit::items(text, start).ok_or("the map does not close")?;
        return found
            .items
            .into_iter()
            .map(|item| {
                entry(text, item.clone())
                    .map(|(key, span)| Record {
                        key: Some(key),
                        span,
                    })
                    .ok_or_else(|| format!("`{}` is not `\"name\": (…)`", &text[item]))
            })
            .collect();
    }
    if ron_edit::outer_open(text).is_some() {
        return Ok(vec![Record {
            key: None,
            span: start..text.trim_end().len(),
        }]);
    }
    Err("the file is not a struct `(…)` or a map of them `{ \"name\": (…) }`".into())
}

/// The fields a record writes, in its order.
pub fn field_names(text: &str, record: &Record) -> Vec<String> {
    let value = &text[record.span.clone()];
    let Some(found) = ron_edit::outer_open(value).and_then(|open| ron_edit::items(value, open))
    else {
        return Vec::new();
    };
    found
        .items
        .into_iter()
        .filter_map(|r| {
            let item = &value[r];
            let (name, _) = item.split_once(':')?;
            let name = name.trim();
            (!name.is_empty() && !name.starts_with('"')).then(|| name.to_string())
        })
        .collect()
}

/// A field of a record as its text writes it; `None` when the record
/// leaves it out (it stands at its default).
pub fn field(text: &str, record: &Record, name: &str) -> Option<String> {
    let value = &text[record.span.clone()];
    let at = ron_edit::value_start(value, name)?;
    let span = ron_edit::value_span(value, at)?;
    Some(value[span].trim().to_string())
}

/// `text` with a field of one record set to `value` (RON), or taken away
/// with `None` — everything else as it was. `key` is `None` for a file
/// that is one struct.
pub fn set(
    text: &str,
    key: Option<&str>,
    name: &str,
    value: Option<&str>,
) -> Result<String, String> {
    let all = records(text)?;
    let record = all
        .iter()
        .find(|r| r.key.as_deref() == key)
        .ok_or_else(|| match key {
            Some(key) => {
                let keys: Vec<&str> = all.iter().filter_map(|r| r.key.as_deref()).collect();
                format!(
                    "no record `{key}`{}",
                    crate::spelling::closest(key, keys.iter().copied())
                        .map(|k| format!(" — did you mean `{k}`?"))
                        .unwrap_or_default()
                )
            }
            None => "the file is a map of records: name one".to_string(),
        })?;
    let old = &text[record.span.clone()];
    let new = ron_edit::set_field(old, name, value)
        .ok_or_else(|| format!("could not find where `{name}` goes"))?;
    let mut out = text.to_string();
    out.replace_range(record.span.clone(), &new);
    Ok(out)
}

/// The fields of one record, by the shape of the whole file: a map of
/// structs, or a struct.
pub fn record_fields(shape: &Shape) -> Option<&[(String, Shape)]> {
    match shape {
        Shape::Map(_, value) => match value.as_ref() {
            Shape::Struct(fields) => Some(fields),
            _ => None,
        },
        Shape::Struct(fields) => Some(fields),
        _ => None,
    }
}

/// What is wrong with a tuning file for the shape the game reads it as:
/// each problem with the record it is in (`None` for the file as a
/// whole). Text that is not RON is a problem without a shape.
pub fn problems(shape: Option<&Shape>, text: &str) -> Vec<(Option<String>, String)> {
    if let Err(e) = ron::from_str::<ron::Value>(text) {
        return vec![(None, e.to_string())];
    }
    match shape {
        Some(Shape::Map(_, record)) => match records(text) {
            Ok(all) => all
                .into_iter()
                .flat_map(|r| {
                    record
                        .problems(&text[r.span.clone()])
                        .into_iter()
                        .map(move |p| (r.key.clone(), p))
                })
                .collect(),
            Err(e) => vec![(None, e)],
        },
        Some(shape) => shape
            .problems(text)
            .into_iter()
            .map(|p| (None, p))
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENEMIES: &str = r#"// Who the player meets.
{
    "goblin": (hp: 10, speed: 2.5), // small
    "orc": (
        hp: 30,
        // slow, but it hits
        speed: 1.0,
    ),
    "ghost": (),
}
"#;

    #[test]
    fn a_map_of_records_is_rows_and_a_struct_is_one() {
        let rows = records(ENEMIES).unwrap();
        let keys: Vec<_> = rows.iter().map(|r| r.key.clone().unwrap()).collect();
        assert_eq!(keys, ["goblin", "orc", "ghost"]);
        assert_eq!(field(ENEMIES, &rows[1], "speed").as_deref(), Some("1.0"));
        assert_eq!(field(ENEMIES, &rows[2], "hp"), None, "left at its default");
        assert_eq!(field_names(ENEMIES, &rows[0]), ["hp", "speed"]);

        let world = "(gravity: -9.81)\n";
        let rows = records(world).unwrap();
        assert_eq!(rows[0].key, None);
        assert_eq!(field(world, &rows[0], "gravity").as_deref(), Some("-9.81"));
        assert!(records("[1, 2]").is_err());
    }

    #[test]
    fn a_cell_changes_where_it_stands_and_nothing_else_does() {
        let out = set(ENEMIES, Some("orc"), "hp", Some("35")).unwrap();
        assert_eq!(out, ENEMIES.replace("hp: 30", "hp: 35"));
        let out = set(ENEMIES, Some("ghost"), "speed", Some("4.0")).unwrap();
        assert!(out.contains("\"ghost\": (speed: 4.0)"), "{out}");
        assert!(out.contains("// slow, but it hits"), "comments stay: {out}");
        let e = set(ENEMIES, Some("orcc"), "hp", Some("1")).unwrap_err();
        assert!(e.contains("did you mean `orc`?"), "{e}");
        assert_eq!(
            set("(gravity: -9.81)", None, "gravity", Some("-3.7")).unwrap(),
            "(gravity: -3.7)"
        );
    }

    #[test]
    fn problems_name_the_record() {
        let shape = Shape::Map(
            Box::new(Shape::Text),
            Box::new(Shape::Struct(vec![
                ("hp".into(), Shape::Int),
                ("speed".into(), Shape::Float),
            ])),
        );
        let text = r#"{ "goblin": (hp: 10, sped: 2.0), "orc": (hp: "lots") }"#;
        let found = problems(Some(&shape), text);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].0.as_deref(), Some("goblin"));
        assert!(found[0].1.contains("did you mean `speed`?"), "{found:?}");
        assert_eq!(found[1].0.as_deref(), Some("orc"));
        assert!(found[1].1.contains("whole number"), "{found:?}");
        assert!(problems(None, text).is_empty(), "no shape, nothing to say");
        assert_eq!(problems(None, "{ oops").len(), 1, "not RON");
    }
}
