//! A graph's file changed where the change is, and nowhere else: an input
//! of one node set, a node added or taken out — the comments, the order
//! and every other line stay as they were, so the diff is the change
//! (DNA, postulate 2). What an editor's window writes with.

/// Where the value after `"name":` starts in `text` — the node's kind —
/// and where its closing parenthesis is.
fn node_span(text: &str, name: &str) -> Option<(usize, usize, usize)> {
    let key = format!("\"{name}\"");
    let mut from = 0;
    while let Some(i) = text[from..].find(&key) {
        let at = from + i + key.len();
        let rest = &text[at..];
        let colon = rest.len() - rest.trim_start().len();
        if rest[colon..].starts_with(':') {
            let kind_at = at + colon + 1;
            let open = kind_at + text[kind_at..].find('(')?;
            let close = closing(text, open)?;
            return Some((kind_at, open, close));
        }
        from = at;
    }
    None
}

/// The index of the bracket that closes the one at `open`, strings and
/// comments skipped.
fn closing(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The top-level `field: value` pairs between `open` and `close`, each as
/// (field, where its value starts, where it ends).
fn fields(text: &str, open: usize, close: usize) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut i = open + 1;
    while i < close {
        let rest = &text[i..close];
        let skip = rest.len() - rest.trim_start().len();
        i += skip;
        if i >= close {
            break;
        }
        let Some(colon) = text[i..close].find(':') else {
            break;
        };
        let field = text[i..i + colon].trim().to_string();
        let value_start = i + colon + 1;
        // The value ends at the next top-level comma.
        let mut j = value_start;
        let bytes = text.as_bytes();
        while j < close {
            match bytes[j] {
                b'(' | b'[' | b'{' => j = closing(text, j).unwrap_or(close),
                b'"' => {
                    j += 1;
                    while j < close && bytes[j] != b'"' {
                        j += 1;
                    }
                }
                b',' => break,
                _ => {}
            }
            j += 1;
        }
        let value_end = j.min(close);
        out.push((field, value_start, value_end));
        i = value_end + 1;
    }
    out
}

/// `text` with input `field` of node `name` set to `value` (written as
/// RON: `"noise.x"`, `0.5`, `(1.0, 0.2, 0.0)`), added when the node leaves
/// it to its default. `None` when there is no such node.
pub fn set_input(text: &str, name: &str, field: &str, value: &str) -> Option<String> {
    let (_, open, close) = node_span(text, name)?;
    let found = fields(text, open, close);
    if let Some((_, start, end)) = found.iter().find(|(f, _, _)| f == field) {
        let lead = &text[*start..*end];
        let pad = lead.len() - lead.trim_start().len();
        let tail = lead.len() - lead.trim_end().len();
        return Some(format!(
            "{}{}{}{}",
            &text[..start + pad],
            value,
            &text[end - tail..],
            ""
        ));
    }
    let inside = text[open + 1..close].trim();
    let insert = if inside.is_empty() {
        format!("{field}: {value}")
    } else if inside.ends_with(',') {
        format!(" {field}: {value}")
    } else {
        format!(", {field}: {value}")
    };
    let at = open + 1 + text[open + 1..close].trim_end().len();
    Some(format!("{}{insert}{}", &text[..at], &text[at..]))
}

/// `text` with a node added at the end of `nodes: { … }`, on a line of its
/// own like the one before it: `"name": Kind(…)`. `None` when there is no
/// `nodes` map, or the name is taken.
pub fn add_node(text: &str, name: &str, node: &str) -> Option<String> {
    if node_span(text, name).is_some() {
        return None;
    }
    let at = text.find("nodes:")?;
    let open = at + text[at..].find('{')?;
    let close = closing(text, open)?;
    let inside = &text[open + 1..close];
    let body = inside.trim_end();
    // Indented as the last node is, or four past the map.
    let indent = inside
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .unwrap_or(8);
    let comma = if body.trim().is_empty() || body.ends_with(',') {
        ""
    } else {
        ","
    };
    let end_indent = text[..close]
        .rsplit('\n')
        .next()
        .map(|l| l.len() - l.trim_start().len())
        .unwrap_or(4);
    Some(format!(
        "{}{}{comma}\n{}\"{name}\": {node},\n{}{}",
        &text[..open + 1],
        body,
        " ".repeat(indent.max(4)),
        " ".repeat(end_indent.min(indent)),
        &text[close..]
    ))
}

/// `text` without node `name`, its line and all. `None` when there is no
/// such node.
pub fn remove_node(text: &str, name: &str) -> Option<String> {
    let key = format!("\"{name}\"");
    let (_, _, close) = node_span(text, name)?;
    let start = text[..text.find(&key)?].rfind('\n').map_or(0, |i| i + 1);
    let mut end = close + 1;
    let rest = &text[end..];
    if let Some(after) = rest.strip_prefix(',') {
        end += 1;
        let _ = after;
    }
    // The rest of the line: a trailing comment goes with it.
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i + 1);
    Some(format!("{}{}", &text[..start], &text[line_end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = r#"// A test.
(
    nodes: {
        "a": Multiply(a: "time", b: 2.0),
        // The noise.
        "n": Noise(at: "position"),
        "c": Combine(x: "a", y: (1.0, 2.0), z: 0.0),
    },
    surface: (albedo: "c"),
)
"#;

    #[test]
    fn an_input_is_set_where_it_is_and_added_where_it_was_left_out() {
        let t = set_input(GRAPH, "a", "b", "3.5").unwrap();
        assert!(t.contains(r#""a": Multiply(a: "time", b: 3.5),"#), "{t}");
        assert_eq!(t.lines().count(), GRAPH.lines().count());
        assert!(t.contains("// The noise."), "comments stay");
        let t = set_input(GRAPH, "n", "scale", "4.0").unwrap();
        assert!(
            t.contains(r#""n": Noise(at: "position", scale: 4.0),"#),
            "{t}"
        );
        crate::surface::parse(&t).unwrap();
        assert!(set_input(GRAPH, "nothing", "a", "1.0").is_none());
    }

    #[test]
    fn a_node_is_added_on_a_line_of_its_own_and_taken_out_with_its_line() {
        let t = add_node(GRAPH, "s", "Sine(of: \"a\")").unwrap();
        assert!(t.contains("        \"s\": Sine(of: \"a\"),\n    },"), "{t}");
        crate::surface::parse(&t).unwrap();
        assert!(
            add_node(GRAPH, "a", "Sine(of: 1.0)").is_none(),
            "a name is one node's"
        );
        let t = remove_node(GRAPH, "n").unwrap();
        assert!(!t.contains("Noise"), "{t}");
        assert!(t.contains("// The noise."), "the comment above stays: {t}");
        crate::surface::parse(&t).unwrap();
    }
}
