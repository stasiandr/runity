//! Editing a RON file where it changed, and nowhere else.
//!
//! A panel that edits a file a person also writes by hand — a screen, an
//! animator — must not rewrite the whole of it: the comments would go, the
//! hand-made alignment would go, and the diff of a one-number change would
//! be the file (DNA, postulate 2). This finds where a field's value and a
//! list's or map's items are in the text, strings and comments skipped, and
//! changes those spans only. The caller parses the result back and compares
//! it with what it meant to write; when the text is shaped past what this
//! understands, it writes the file anew instead.

use std::ops::Range;

/// A list's or a map's items, and where it closes.
pub struct Items {
    pub items: Vec<Range<usize>>,
    /// The closing `]` or `}`.
    pub close: usize,
}

/// Skip a string starting at `i` (on its `"`): the index after it.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() && b[i] != b'"' {
        i += if b[i] == b'\\' { 2 } else { 1 };
    }
    i + 1
}

/// Skip a comment starting at `i`, if one does: the index after it.
fn skip_comment(text: &str, i: usize) -> Option<usize> {
    let b = text.as_bytes();
    match (b.get(i), b.get(i + 1)) {
        (Some(b'/'), Some(b'/')) => Some(text[i..].find('\n').map_or(b.len(), |n| i + n)),
        (Some(b'/'), Some(b'*')) => Some(text[i + 2..].find("*/").map_or(b.len(), |n| i + n + 4)),
        _ => None,
    }
}

/// Where the value of the top-level field `key` starts: the first thing
/// after `key:` in the file's outer `( … )`.
pub fn value_start(text: &str, key: &str) -> Option<usize> {
    let b = text.as_bytes();
    let (mut i, mut depth) = (0, 0usize);
    while i < b.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ if depth == 1
                && text[i..].starts_with(key)
                && !b[i - 1].is_ascii_alphanumeric()
                && b[i - 1] != b'_' =>
            {
                let mut j = i + key.len();
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) == Some(&b':') {
                    j += 1;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    return Some(j);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The span of one value starting at `start`: a string, or a bracketed
/// group with whatever name is before it (`Some(…)`, `Above(…)`).
pub fn value_span(text: &str, start: usize) -> Option<Range<usize>> {
    let b = text.as_bytes();
    if b.get(start) == Some(&b'"') {
        return Some(start..skip_string(b, start));
    }
    let (mut i, mut depth) = (start, 0usize);
    while i < b.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return Some(start..i),
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start..i + 1);
                }
            }
            b',' if depth == 0 => return Some(start..i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The items of the list, map or struct that opens at `open` (on its `[`,
/// `{` or `(`).
pub fn items(text: &str, open: usize) -> Option<Items> {
    let b = text.as_bytes();
    if !matches!(b.get(open), Some(b'[' | b'{' | b'(')) {
        return None;
    }
    let mut items = Vec::new();
    let (mut i, mut depth) = (open + 1, 0usize);
    let mut current: Option<(usize, usize)> = None;
    while i < b.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if depth == 0 && current.is_none() && !matches!(c, b',' | b']' | b'}') {
            current = Some((i, i));
        }
        match c {
            b'"' => {
                i = skip_string(b, i);
                if depth == 0 {
                    if let Some((_, end)) = current.as_mut() {
                        *end = i;
                    }
                }
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b']' | b'}' | b')' if depth == 0 => {
                if let Some((s, e)) = current.take() {
                    items.push(s..e);
                }
                return Some(Items { items, close: i });
            }
            b']' | b'}' | b')' => {
                depth -= 1;
                if depth == 0 {
                    if let Some((_, end)) = current.as_mut() {
                        *end = i + 1;
                    }
                }
            }
            b',' if depth == 0 => {
                if let Some((s, e)) = current.take() {
                    items.push(s..e);
                }
            }
            _ if depth == 0 => {
                if let Some((_, end)) = current.as_mut() {
                    *end = i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// One change to a list or a map, by the item's place in it.
pub enum Change {
    Replace(usize, String),
    Remove(usize),
    Append(String),
}

/// Where the file's outermost `( … )` opens: a `.rmat`, a screen, a
/// graph — the value the fields of a file are in.
pub fn outer_open(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if let Some(next) = skip_comment(text, i) {
            i = next;
            continue;
        }
        match b[i] {
            b'(' => return Some(i),
            c if c.is_ascii_whitespace() => i += 1,
            // A named struct, `Name(`: its bracket.
            c if c.is_ascii_alphanumeric() || c == b'_' => i += 1,
            _ => return None,
        }
    }
    None
}

/// Set a top-level field of a file's outer struct to `value` (RON text),
/// or take it away with `None`, leaving the rest of the text as it is.
pub fn set_field(text: &str, key: &str, value: Option<&str>) -> Option<String> {
    let open = outer_open(text)?;
    let found = items(text, open)?;
    let at = found.items.iter().position(|r| {
        let item = &text[r.clone()];
        item.split(':').next().is_some_and(|k| k.trim() == key)
    });
    let change = match (at, value) {
        (Some(i), Some(v)) => Change::Replace(i, format!("{key}: {v}")),
        (Some(i), None) => Change::Remove(i),
        (None, Some(v)) => Change::Append(format!("{key}: {v}")),
        (None, None) => return Some(text.to_string()),
    };
    apply(text, open, &[change])
}

/// `text` with `changes` made to the list or map opening at `open`.
pub fn apply(text: &str, open: usize, changes: &[Change]) -> Option<String> {
    let found = items(text, open)?;
    let b = text.as_bytes();
    // Edits as (span, new text), made from the end back so that the
    // spans before stay where they were found.
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let line_start = |at: usize| text[..at].rfind('\n').map_or(0, |n| n + 1);
    let indent_of = |at: usize| {
        let s = line_start(at);
        let only_space = text[s..at].chars().all(|c| c == ' ' || c == '\t');
        only_space.then(|| text[s..at].to_string())
    };
    for change in changes {
        match change {
            Change::Replace(i, new) => edits.push((found.items.get(*i)?.clone(), new.clone())),
            Change::Remove(i) => {
                let item = found.items.get(*i)?.clone();
                let mut end = item.end;
                while end < b.len() && matches!(b[end], b' ' | b'\t') {
                    end += 1;
                }
                if b.get(end) == Some(&b',') {
                    end += 1;
                }
                while end < b.len() && matches!(b[end], b' ' | b'\t') {
                    end += 1;
                }
                let mut start = item.start;
                if b.get(end) == Some(&b'\n') && indent_of(item.start).is_some() {
                    end += 1;
                    start = line_start(item.start);
                }
                edits.push((start..end, String::new()));
            }
            Change::Append(new) => {
                let close = found.close;
                match found.items.last() {
                    Some(last) => {
                        // A comma after the last item, if it has none.
                        let mut j = last.end;
                        while j < close && b[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        let has_comma = b.get(j) == Some(&b',');
                        let indent = indent_of(last.start);
                        match (indent, indent_of(close)) {
                            (Some(indent), Some(_)) => {
                                let at = line_start(close);
                                if !has_comma {
                                    edits.push((last.end..last.end, ",".into()));
                                }
                                edits.push((at..at, format!("{indent}{new},\n")));
                            }
                            _ => {
                                let sep = if has_comma { " " } else { ", " };
                                let at = if has_comma { j + 1 } else { last.end };
                                edits.push((at..at, format!("{sep}{new}")));
                            }
                        }
                    }
                    None => edits.push((close..close, new.clone())),
                }
            }
        }
    }
    edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    let mut out = text.to_string();
    for (span, new) in edits {
        out.replace_range(span, &new);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"// Who plays what.
(
    start: "idle",
    states: {
        // Standing about.
        "idle": (clip: "idle"),
        "walk": (clip: "walk", speed_from: "speed"), // faster when faster
    },
    transitions: [
        (from: "idle", to: "walk", when: [Above("speed", 0.1)]),
    ],
)
"#;

    #[test]
    fn values_and_items_are_found_past_comments_and_strings() {
        let at = value_start(FILE, "start").unwrap();
        assert_eq!(&FILE[value_span(FILE, at).unwrap()], "\"idle\"");
        let states = value_start(FILE, "states").unwrap();
        let found = items(FILE, states).unwrap();
        let texts: Vec<&str> = found.items.iter().map(|r| &FILE[r.clone()]).collect();
        assert_eq!(
            texts,
            [
                r#""idle": (clip: "idle")"#,
                r#""walk": (clip: "walk", speed_from: "speed")"#
            ]
        );
    }

    #[test]
    fn changes_leave_the_rest_alone() {
        let states = value_start(FILE, "states").unwrap();
        let out = apply(
            FILE,
            states,
            &[
                Change::Remove(0),
                Change::Append(r#""run": (clip: "run")"#.into()),
            ],
        )
        .unwrap();
        assert!(!out.contains(r#""idle": (clip"#));
        assert!(out.contains("// Standing about."), "a comment above stays");
        assert!(out.contains("// faster when faster"));
        assert!(out.contains("        \"run\": (clip: \"run\"),\n    },"));
        let t = value_start(&out, "transitions").unwrap();
        let out = apply(
            &out,
            t,
            &[Change::Replace(0, "(from: \"a\", to: \"b\")".into())],
        )
        .unwrap();
        assert!(out.contains("        (from: \"a\", to: \"b\"),\n    ],"));
        assert!(out.starts_with("// Who plays what.\n"));
    }

    #[test]
    fn a_top_level_field_is_set_added_and_taken_away_in_place() {
        let text = "// Stone after rain.\n(parent: \"stone\", smoothness: 0.8)\n";
        let set = set_field(text, "smoothness", Some("0.9")).unwrap();
        assert_eq!(
            set,
            "// Stone after rain.\n(parent: \"stone\", smoothness: 0.9)\n"
        );
        let added = set_field(&set, "metallic", Some("0.5")).unwrap();
        assert_eq!(
            added,
            "// Stone after rain.\n(parent: \"stone\", smoothness: 0.9, metallic: 0.5)\n"
        );
        let gone = set_field(&added, "smoothness", None).unwrap();
        assert_eq!(
            gone,
            "// Stone after rain.\n(parent: \"stone\", metallic: 0.5)\n"
        );
    }

    #[test]
    fn one_line_lists_grow_on_their_line() {
        let text = "(list: [1, 2])";
        let open = value_start(text, "list").unwrap();
        assert_eq!(
            apply(text, open, &[Change::Append("3".into())]).unwrap(),
            "(list: [1, 2, 3])"
        );
        assert_eq!(
            apply(text, open, &[Change::Remove(0)]).unwrap(),
            "(list: [2])"
        );
        let empty = "(list: [])";
        let open = value_start(empty, "list").unwrap();
        assert_eq!(
            apply(empty, open, &[Change::Append("1".into())]).unwrap(),
            "(list: [1])"
        );
    }
}
