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

/// Past whitespace and `//` comments from `i`, up to `close`.
fn skip_blank(text: &str, mut i: usize, close: usize) -> usize {
    loop {
        let rest = &text[i..close];
        i += rest.len() - rest.trim_start().len();
        if text[i..close].starts_with("//") {
            i += text[i..close].find('\n').unwrap_or(close - i);
        } else {
            return i;
        }
    }
}

/// The top-level `field: value` pairs between `open` and `close`, each as
/// (field, where its value starts, where it ends) — the end before any
/// comment after it, so a value set keeps the comment.
fn fields(text: &str, open: usize, close: usize) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut i = open + 1;
    let bytes = text.as_bytes();
    while i < close {
        i = skip_blank(text, i, close);
        if i >= close {
            break;
        }
        let Some(colon) = text[i..close].find(':') else {
            break;
        };
        let field = text[i..i + colon].trim().to_string();
        let value_start = i + colon + 1;
        // The value ends at the next top-level comma; a comment in the way
        // is not the value's, and its commas are not.
        let mut j = value_start;
        let mut code_end = None;
        while j < close {
            match bytes[j] {
                b'/' if bytes.get(j + 1) == Some(&b'/') => {
                    code_end.get_or_insert(j);
                    j += text[j..close].find('\n').unwrap_or(close - j);
                    continue;
                }
                b'(' | b'[' | b'{' => j = closing(text, j).unwrap_or(close),
                b'"' => {
                    j += 1;
                    while j < close && bytes[j] != b'"' {
                        j += 1;
                    }
                }
                b',' => break,
                _ => {
                    if !bytes[j].is_ascii_whitespace() {
                        code_end = None;
                    }
                }
            }
            j += 1;
        }
        let end = j.min(close);
        out.push((field, value_start, code_end.unwrap_or(end).min(end)));
        i = end + 1;
    }
    out
}

/// `text` with `field` of the parenthesised value from `open` to `close`
/// set to `value`, or added at its end.
fn set_field(text: &str, open: usize, close: usize, field: &str, value: &str) -> String {
    if let Some((_, start, end)) = fields(text, open, close)
        .into_iter()
        .find(|(f, _, _)| f == field)
    {
        let lead = &text[start..end];
        let pad = lead.len() - lead.trim_start().len();
        let tail = lead.len() - lead.trim_end().len();
        return format!("{}{value}{}", &text[..start + pad], &text[end - tail..]);
    }
    let body = text[open + 1..close].trim_end();
    let at = open + 1 + body.len();
    let insert = if body.trim().is_empty() {
        format!("{field}: {value}")
    } else if body.ends_with(',') {
        format!(" {field}: {value}")
    } else {
        format!(", {field}: {value}")
    };
    format!("{}{insert}{}", &text[..at], &text[at..])
}

/// The graph's own parentheses: the first `(` outside a comment.
fn outer(text: &str) -> Option<(usize, usize)> {
    let open = skip_blank(text, 0, text.len());
    (text[open..].starts_with('(')).then_some(())?;
    Some((open, closing(text, open)?))
}

/// `text` with `field` of the graph's part `part` — `surface`, `vertex`,
/// `spawn`, `update`, `output` — set to `value`: the part added when the
/// graph has none. `None` when the text is not a graph.
pub fn set_part(text: &str, part: &str, field: &str, value: &str) -> Option<String> {
    let (open, close) = outer(text)?;
    if let Some((_, start, _)) = fields(text, open, close)
        .into_iter()
        .find(|(f, _, _)| f == part)
    {
        let paren = start + text[start..].find('(')?;
        let end = closing(text, paren)?;
        return Some(set_field(text, paren, end, field, value));
    }
    Some(set_field(
        text,
        open,
        close,
        part,
        &format!("({field}: {value})"),
    ))
}

/// `text` with input `field` of node `name` set to `value` (written as
/// RON: `"noise.x"`, `0.5`, `(1.0, 0.2, 0.0)`), added when the node leaves
/// it to its default. `None` when there is no such node.
pub fn set_input(text: &str, name: &str, field: &str, value: &str) -> Option<String> {
    let (_, open, close) = node_span(text, name)?;
    Some(set_field(text, open, close, field, value))
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

    #[test]
    fn a_part_of_the_graph_is_set_and_comments_on_the_way_stay() {
        let t = set_part(GRAPH, "surface", "emission", "\"a\"").unwrap();
        assert!(
            t.contains(r#"surface: (albedo: "c", emission: "a"),"#),
            "{t}"
        );
        crate::surface::parse(&t).unwrap();
        let t = set_part(GRAPH, "vertex", "position", "\"position\"").unwrap();
        assert!(t.contains(r#"vertex: (position: "position")"#), "{t}");
        crate::surface::parse(&t).unwrap();
        let commented =
            "(\n    // Lit: from below, the lava.\n    surface: (albedo: \"x\"), // after: this\n)";
        let t = set_part(commented, "surface", "albedo", "\"y\"").unwrap();
        assert_eq!(t, commented.replace("\"x\"", "\"y\""));
    }

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
