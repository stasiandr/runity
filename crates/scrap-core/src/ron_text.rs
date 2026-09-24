//! Saving a RON file without rewriting what did not change.
//!
//! DNA, postulate 2: saving without a change is a zero diff, and a change
//! is a diff of that change. A serializer cannot give that for a file a
//! person wrote: it drops comments, reflows lines, and reorders nothing but
//! reformats everything. So a save here is a merge of three texts:
//!
//! * **old** — the file as it is on disk, comments and all;
//! * **canon** — the old file's value, re-serialized: what the serializer
//!   would have written for it;
//! * **new** — the value being saved, serialized the same way.
//!
//! Where canon and new are the same text, the value did not change, and
//! old's text is kept exactly. Where they differ, the three are walked
//! together — struct fields by name, map entries by key, lists of entities
//! by `id` — and only the parts that differ take new's text, keeping old's
//! comments and spacing around them. What comes out is parsed again and
//! must equal the value being saved; if it does not, the plain
//! serialization is written instead, so a bug here costs formatting, never
//! data.

use std::ops::Range;

/// A value's shape, with the byte ranges a merge needs.
#[derive(Debug)]
enum Node {
    /// `(…)` or `Name(…)` with `key: value` entries, or `{…}` with
    /// `key: value` entries, or `[…]` with plain items.
    Group {
        /// Where the entries start: just after the opening bracket.
        open: usize,
        /// The closing bracket.
        close: usize,
        entries: Vec<Entry>,
        kind: Kind,
    },
    /// Anything else: a number, a string, a tuple, an enum value.
    Atom,
}

/// Which brackets a group has: `(…)` with fields, `{…}` with keys, `[…]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Struct,
    Map,
    List,
}

#[derive(Debug)]
struct Entry {
    /// From just after the previous comma (or the opening bracket): the
    /// entry's leading whitespace and comments.
    start: usize,
    /// Where the entry itself begins, after its leading trivia.
    body: usize,
    /// The field name or map key, as text; `None` for a list item.
    key: Option<String>,
    value: Range<usize>,
    node: Node,
    /// Just past the entry's comma, or its value if it has none.
    end: usize,
    comma: bool,
}

/// Write `value` to `path`, keeping the text of whatever in the file it
/// did not change. A file that is not there, does not parse, or would not
/// merge cleanly is written whole. A file that already says exactly this
/// is not written at all, so its clock does not move and nothing watching
/// it reloads for nothing.
pub(crate) fn write_preserving<T>(
    path: &std::path::Path,
    value: &T,
    pretty: ron::ser::PrettyConfig,
) -> anyhow::Result<()>
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq,
{
    let new = ron::ser::to_string_pretty(value, pretty.clone())? + "\n";
    let old = crate::files::read_to_string(path).ok();
    let merged = old.as_deref().and_then(|old| {
        let parsed: T = ron::from_str(old).ok()?;
        let canon = ron::ser::to_string_pretty(&parsed, pretty).ok()? + "\n";
        let merged = merge(old, &canon, &new)?;
        // The merge is formatting; the value is the contract. Whatever it
        // produced has to read back as exactly what is being saved.
        (ron::from_str::<T>(&merged).ok()? == *value).then_some(merged)
    });
    let text = merged.unwrap_or(new);
    if old.as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    crate::files::write(path, text)?;
    Ok(())
}

/// Merge `new` into `old`, where `canon` is `old`'s value re-serialized.
/// `None` when the texts do not scan — the caller writes `new` whole.
pub(crate) fn merge(old: &str, canon: &str, new: &str) -> Option<String> {
    if canon == new {
        return Some(old.to_string());
    }
    let (old_node, old_range) = scan_document(old)?;
    let (canon_node, canon_range) = scan_document(canon)?;
    let (new_node, new_range) = scan_document(new)?;
    let merged = merge_value(
        Side {
            text: old,
            node: &old_node,
            range: old_range.clone(),
        },
        Side {
            text: canon,
            node: &canon_node,
            range: canon_range,
        },
        Side {
            text: new,
            node: &new_node,
            range: new_range.clone(),
        },
    );
    Some(format!(
        "{}{}{}",
        &old[..old_range.start],
        merged,
        &old[old_range.end..]
    ))
}

/// A piece of a RON document as it is written, for reading it as a
/// person wrote it rather than as a type would: a field's name or a map's
/// key, the value's own text, and, for a group, what is in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Part<'a> {
    /// A field's name or a map's key as written (a map's string key keeps
    /// its quotes); `None` for a list's item.
    pub key: Option<String>,
    /// The value's text, from its first character to its last.
    pub text: &'a str,
    /// A struct, a map or a list, with its entries; `None` for anything
    /// else: a number, a string, a tuple, an enum value without fields.
    pub group: Option<(Kind, Vec<Part<'a>>)>,
}

/// `text`'s value, as written. `None` when it does not scan as RON.
pub fn outline(text: &str) -> Option<Part<'_>> {
    fn part<'a>(text: &'a str, key: Option<String>, range: Range<usize>, node: &Node) -> Part<'a> {
        let group = match node {
            Node::Group { entries, kind, .. } => Some((
                *kind,
                entries
                    .iter()
                    .map(|e| part(text, e.key.clone(), e.value.clone(), &e.node))
                    .collect(),
            )),
            Node::Atom => None,
        };
        Part {
            key,
            text: &text[range],
            group,
        }
    }
    let (node, range) = scan_document(text)?;
    Some(part(text, None, range, &node))
}

#[derive(Clone)]
struct Side<'a> {
    text: &'a str,
    node: &'a Node,
    range: Range<usize>,
}

impl<'a> Side<'a> {
    fn slice(&self) -> &'a str {
        &self.text[self.range.clone()]
    }

    fn entry(&self, entry: &'a Entry) -> Side<'a> {
        Side {
            text: self.text,
            node: &entry.node,
            range: entry.value.clone(),
        }
    }

    fn group(&self) -> Option<(usize, usize, &'a [Entry], Kind)> {
        match self.node {
            Node::Group {
                open,
                close,
                entries,
                kind,
            } => Some((*open, *close, entries, *kind)),
            Node::Atom => None,
        }
    }

    /// `Name(`, `(`, `[` or `{`: what opens the group.
    fn head(&self, open: usize) -> &'a str {
        &self.text[self.range.start..open]
    }
}

fn merge_value(old: Side, canon: Side, new: Side) -> String {
    if canon.slice() == new.slice() {
        return old.slice().to_string();
    }
    let (Some(o), Some(c), Some(n)) = (old.group(), canon.group(), new.group()) else {
        return new.slice().to_string();
    };
    // The same kind of group opened the same way, or this is a different
    // thing that happens to use the same brackets — `Box(…)` becoming
    // `Sphere(…)`.
    if o.3 != c.3 || c.3 != n.3 || canon.head(c.0) != new.head(n.0) {
        return new.slice().to_string();
    }
    match n.3 {
        Kind::List => merge_list(&old, &canon, &new, o, c, n),
        Kind::Struct | Kind::Map => merge_keyed(&old, &canon, &new, o, c, n),
    }
    .unwrap_or_else(|| new.slice().to_string())
}

type Group<'a> = (usize, usize, &'a [Entry], Kind);

/// Fields or map entries, paired by key. Old's order is kept — the order of
/// fields means nothing to the reader, and moving them would be a diff —
/// and what is new goes at the end.
fn merge_keyed(
    old: &Side,
    canon: &Side,
    new: &Side,
    o: Group,
    c: Group,
    n: Group,
) -> Option<String> {
    let find =
        |entries: &[Entry], key: &str| entries.iter().position(|e| e.key.as_deref() == Some(key));
    let new_index = |key: &str| find(n.2, key);
    // Old's entries in old's order, each with where it sits in new's order.
    let mut pieces: Vec<(Option<usize>, String)> = Vec::new();
    for old_entry in o.2 {
        let key = old_entry.key.as_deref()?;
        let piece = match (find(c.2, key), new_index(key)) {
            (Some(ci), Some(ni)) => format!(
                "{}{}",
                &old.text[old_entry.start..old_entry.value.start],
                merge_value(
                    old.entry(old_entry),
                    canon.entry(&c.2[ci]),
                    new.entry(&n.2[ni])
                )
            ),
            // The serializer leaves it out, in both: it was the default
            // and still is. The file's spelling of it stays.
            (None, None) => old.text[old_entry.start..old_entry.value.end].to_string(),
            // Gone from the value.
            (Some(_), None) => continue,
            // Written out in the file though the serializer would skip it,
            // and now set: it takes the new value.
            (None, Some(ni)) => format!(
                "{}{}",
                &old.text[old_entry.start..old_entry.value.start],
                new.entry(&n.2[ni]).slice()
            ),
        };
        pieces.push((new_index(key), piece));
    }
    // What the file did not have goes where the new value has it — before
    // the first old entry that comes after it — so `id` added on a first
    // save is first in its block, as everywhere else. It is laid out like
    // its neighbours: on the same line in a one-line group, on its own
    // line at their indent otherwise.
    let one_line = !old.text[o.0..o.1].contains('\n');
    let indent =
        o.2.iter()
            .map(|e| &old.text[e.start..e.body])
            .find_map(|trivia| trivia.rfind('\n').map(|at| trivia[at..].to_string()));
    for (k, new_entry) in n.2.iter().enumerate() {
        let key = new_entry.key.as_deref()?;
        if find(o.2, key).is_some() {
            continue;
        }
        // The file left out what the serializer writes: if it is what the
        // old value had, it is a default the file omits, and stays omitted.
        if let Some(ci) = find(c.2, key) {
            if canon.entry(&c.2[ci]).slice() == new.entry(new_entry).slice() {
                continue;
            }
        }
        let body = &new.text[new_entry.body..new_entry.value.end];
        let at = pieces
            .iter()
            .position(|(index, _)| index.is_some_and(|i| i > k))
            .unwrap_or(pieces.len());
        let trivia = match (&indent, one_line) {
            (Some(indent), false) => indent.clone(),
            _ if at == 0 => String::new(),
            _ => " ".to_string(),
        };
        if one_line && at == 0 {
            if let Some((_, next)) = pieces.first_mut() {
                if !next.starts_with(char::is_whitespace) {
                    next.insert(0, ' ');
                }
            }
        }
        pieces.insert(at, (Some(k), format!("{trivia}{body}")));
    }
    let pieces = pieces.into_iter().map(|(_, piece)| piece).collect();
    Some(join(old, new, o, n, pieces))
}

/// List items in the new order, each paired with its old self by the `id`
/// inside it, or by position when the items have no ids and the count did
/// not change.
fn merge_list(
    old: &Side,
    canon: &Side,
    new: &Side,
    o: Group,
    c: Group,
    n: Group,
) -> Option<String> {
    if o.2.len() != c.2.len() {
        return None;
    }
    let old_ids: Vec<Option<String>> = o.2.iter().map(|e| id_of(old.text, e)).collect();
    let new_ids: Vec<Option<String>> = n.2.iter().map(|e| id_of(new.text, e)).collect();
    let by_position = old_ids.iter().chain(&new_ids).any(Option::is_none);
    if by_position && o.2.len() != n.2.len() {
        return None;
    }
    let mut pieces = Vec::with_capacity(n.2.len());
    for (j, new_entry) in n.2.iter().enumerate() {
        let matched = if by_position {
            Some(j)
        } else {
            old_ids.iter().position(|id| id == &new_ids[j])
        };
        pieces.push(match matched {
            Some(i) => format!(
                "{}{}",
                &old.text[o.2[i].start..o.2[i].value.start],
                merge_value(
                    old.entry(&o.2[i]),
                    canon.entry(&c.2[i]),
                    new.entry(new_entry)
                )
            ),
            None => new.text[new_entry.start..new_entry.value.end].to_string(),
        });
    }
    Some(join(old, new, o, n, pieces))
}

/// The group's opening from old, the pieces with commas between, and old's
/// ending — the comment after the last entry, the indent, the bracket.
fn join(old: &Side, new: &Side, o: Group, n: Group, pieces: Vec<String>) -> String {
    let mut out = old.text[old.range.start..o.0].to_string();
    // A comma after the last entry only if the old file put one there, or
    // for a group that was empty, if the serializer does.
    let trailing = match o.2.last() {
        Some(last) => last.comma,
        None => n.2.last().is_some_and(|e| e.comma),
    };
    let count = pieces.len();
    for (i, piece) in pieces.into_iter().enumerate() {
        out.push_str(&piece);
        if i + 1 < count || trailing {
            out.push(',');
        }
    }
    match o.2.last() {
        Some(last) => {
            // What followed the last entry's comma (or value): trivia, then
            // the bracket.
            let after = if last.comma { last.end } else { last.value.end };
            out.push_str(&old.text[after..old.range.end]);
        }
        None if count > 0 => {
            // Nothing to take the closing layout from: the new text's.
            let from =
                n.2.last()
                    .map(|e| if e.comma { e.end } else { e.value.end });
            out.push_str(&new.text[from.unwrap_or(n.0)..=n.1]);
        }
        None => out.push_str(&old.text[o.0..old.range.end]),
    }
    out
}

/// The `id: "…"` inside a list item, if it is a struct that has one.
fn id_of(text: &str, entry: &Entry) -> Option<String> {
    let Node::Group {
        kind: Kind::Struct,
        entries,
        ..
    } = &entry.node
    else {
        return None;
    };
    entries
        .iter()
        .find(|e| e.key.as_deref() == Some("id"))
        .map(|e| text[e.value.clone()].to_string())
}

// --- scanning -----------------------------------------------------------

struct Scanner<'a> {
    text: &'a [u8],
    at: usize,
}

fn scan_document(text: &str) -> Option<(Node, Range<usize>)> {
    let mut scanner = Scanner {
        text: text.as_bytes(),
        at: 0,
    };
    scanner.trivia();
    let start = scanner.at;
    let node = scanner.value()?;
    let end = scanner.at;
    Some((node, start..end))
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    /// Whitespace and comments, including RON's nested block comments.
    fn trivia(&mut self) {
        loop {
            match (self.peek(), self.text.get(self.at + 1).copied()) {
                (Some(c), _) if c.is_ascii_whitespace() => self.at += 1,
                (Some(b'/'), Some(b'/')) => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.at += 1;
                    }
                }
                (Some(b'/'), Some(b'*')) => {
                    self.at += 2;
                    let mut depth = 1;
                    while depth > 0 {
                        match (self.peek(), self.text.get(self.at + 1).copied()) {
                            (None, _) => return,
                            (Some(b'/'), Some(b'*')) => {
                                depth += 1;
                                self.at += 2;
                            }
                            (Some(b'*'), Some(b'/')) => {
                                depth -= 1;
                                self.at += 2;
                            }
                            _ => self.at += 1,
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn value(&mut self) -> Option<Node> {
        match self.peek()? {
            b'(' => self.group(b')'),
            b'[' => self.group(b']'),
            b'{' => self.group(b'}'),
            b'"' => {
                self.string()?;
                Some(Node::Atom)
            }
            b'r' if self.raw_string_ahead() => {
                self.raw_string()?;
                Some(Node::Atom)
            }
            // A raw identifier, `r#None`: what the serializer writes for a
            // name that is also a keyword somewhere.
            b'r' if self.text.get(self.at + 1) == Some(&b'#') => {
                self.at += 2;
                self.word();
                Some(Node::Atom)
            }
            b'\'' => {
                self.at += 1;
                while self.peek()? != b'\'' {
                    if self.peek()? == b'\\' {
                        self.at += 1;
                    }
                    self.at += 1;
                }
                self.at += 1;
                Some(Node::Atom)
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                self.word();
                // `Name(…)` is a named struct or a variant with data.
                let after_name = self.at;
                if self.peek() == Some(b'(') {
                    let node = self.group(b')')?;
                    return Some(node);
                }
                self.at = after_name;
                Some(Node::Atom)
            }
            _ => {
                // A number, or a sign: everything up to a delimiter.
                while self.peek().is_some_and(|c| {
                    !matches!(c, b',' | b')' | b']' | b'}') && !c.is_ascii_whitespace()
                }) {
                    self.at += 1;
                }
                Some(Node::Atom)
            }
        }
    }

    /// `r"…"` or `r#"…"#`, as opposed to the raw identifier `r#name`.
    fn raw_string_ahead(&self) -> bool {
        let mut at = self.at + 1;
        while self.text.get(at) == Some(&b'#') {
            at += 1;
        }
        self.text.get(at) == Some(&b'"')
    }

    fn word(&mut self) {
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            self.at += 1;
        }
    }

    fn string(&mut self) -> Option<()> {
        self.at += 1;
        loop {
            match self.peek()? {
                b'\\' => self.at += 2,
                b'"' => {
                    self.at += 1;
                    return Some(());
                }
                _ => self.at += 1,
            }
        }
    }

    fn raw_string(&mut self) -> Option<()> {
        self.at += 1;
        let mut hashes = 0;
        while self.peek()? == b'#' {
            hashes += 1;
            self.at += 1;
        }
        if self.peek()? != b'"' {
            return None;
        }
        self.at += 1;
        loop {
            if self.peek()? == b'"'
                && (0..hashes).all(|i| self.text.get(self.at + 1 + i) == Some(&b'#'))
            {
                self.at += 1 + hashes;
                return Some(());
            }
            self.at += 1;
        }
    }

    /// A bracketed group: its entries, each with a key if it has one.
    fn group(&mut self, closing: u8) -> Option<Node> {
        self.at += 1;
        let open = self.at;
        let mut entries = Vec::new();
        let mut keyed = None;
        loop {
            let start = self.at;
            self.trivia();
            if self.peek()? == closing {
                let close = self.at;
                self.at += 1;
                let kind = match closing {
                    b']' => Kind::List,
                    b'}' => Kind::Map,
                    _ if keyed == Some(true) || entries.is_empty() => Kind::Struct,
                    // A tuple: merged as a whole, like an atom.
                    _ => return Some(Node::Atom),
                };
                return Some(Node::Group {
                    open,
                    close,
                    entries,
                    kind,
                });
            }
            // A key: a field name followed by `:`, or for a map any value
            // followed by `:`.
            let key_start = self.at;
            let first = self.value()?;
            let key_end = self.at;
            self.trivia();
            let (key, value, node) = if self.peek() == Some(b':') && closing != b']' {
                self.at += 1;
                self.trivia();
                let value_start = self.at;
                let node = self.value()?;
                keyed = Some(true);
                (
                    Some(String::from_utf8_lossy(&self.text[key_start..key_end]).into_owned()),
                    value_start..self.at,
                    node,
                )
            } else {
                if closing == b')' {
                    keyed = Some(false);
                }
                (None, key_start..key_end, first)
            };
            let value_end = value.end;
            self.at = value_end;
            self.trivia();
            let comma = self.peek() == Some(b',');
            if comma {
                self.at += 1;
            } else {
                self.at = value_end;
            }
            let end = self.at;
            entries.push(Entry {
                start,
                body: key_start,
                key,
                value,
                node,
                end,
                comma,
            });
            if !comma {
                self.trivia();
                if self.peek()? != closing {
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outline_keeps_names_keys_and_text_as_written() {
        let doc = "// a wolf\n(speed: 4.0, pack: [Alpha(howl: 2), Beta], home: {\"den\": (1, 2)})";
        let top = outline(doc).unwrap();
        let (kind, fields) = top.group.unwrap();
        assert_eq!(kind, Kind::Struct);
        let keys: Vec<_> = fields.iter().map(|f| f.key.clone().unwrap()).collect();
        assert_eq!(keys, ["speed", "pack", "home"]);
        assert_eq!(fields[0].text, "4.0");
        let (kind, pack) = fields[1].group.clone().unwrap();
        assert_eq!(kind, Kind::List);
        assert_eq!(pack[0].text, "Alpha(howl: 2)");
        assert_eq!(pack[1].text, "Beta");
        let (kind, home) = fields[2].group.clone().unwrap();
        assert_eq!(kind, Kind::Map);
        assert_eq!(home[0].key.as_deref(), Some("\"den\""));
        assert_eq!(home[0].text, "(1, 2)");
        assert!(outline("(a: ").is_none());
    }

    #[test]
    fn the_same_value_keeps_the_file_exactly() {
        let old = "(a: 1, // why\n b: 2)";
        assert_eq!(merge(old, "(a: 1, b: 2)", "(a: 1, b: 2)").unwrap(), old);
    }

    #[test]
    fn a_changed_field_changes_alone_and_comments_stay() {
        let old = "(\n    // the answer\n    a: 42,\n    /* keep */ b: \"x\",\n)";
        let canon = "(\n    a: 42,\n    b: \"x\",\n)";
        let new = "(\n    a: 42,\n    b: \"y\",\n)";
        assert_eq!(
            merge(old, canon, new).unwrap(),
            "(\n    // the answer\n    a: 42,\n    /* keep */ b: \"y\",\n)"
        );
    }

    #[test]
    fn a_field_the_file_lacked_goes_where_the_value_puts_it() {
        // A hand-written entity's first save writes its id: first, and on
        // the same line when the block was one line.
        let old = "[(name: \"tree\", model: \"m\")]";
        let canon = "[\n    (\n        name: \"tree\",\n        model: \"m\",\n    ),\n]";
        let new = "[\n    (\n        id: \"7\",\n        name: \"tree\",\n        model: \"m\",\n    ),\n]";
        assert_eq!(
            merge(old, canon, new).unwrap(),
            "[(id: \"7\", name: \"tree\", model: \"m\")]"
        );
        let old = "(\n  // a tree\n  name: \"tree\",\n)";
        let canon = "(\n    name: \"tree\",\n)";
        let new = "(\n    id: \"7\",\n    name: \"tree\",\n)";
        assert_eq!(
            merge(old, canon, new).unwrap(),
            "(\n  id: \"7\",\n  // a tree\n  name: \"tree\",\n)"
        );
    }

    #[test]
    fn list_items_are_matched_by_id_through_inserts_and_removals() {
        let old = "[\n  // first\n  (id: \"1\", v: 1),\n  // second\n  (id: \"2\", v: 2),\n]";
        let canon = "[\n    (id: \"1\", v: 1),\n    (id: \"2\", v: 2),\n]";
        let new = "[\n    (id: \"2\", v: 2),\n    (id: \"3\", v: 3),\n]";
        assert_eq!(
            merge(old, canon, new).unwrap(),
            "[\n  // second\n  (id: \"2\", v: 2),\n    (id: \"3\", v: 3),\n]"
        );
    }
}
