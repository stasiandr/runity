//! A field's RON as a tree of what a form shows, each part with where its
//! text is.
//!
//! The Inspector lays a value out as a form and writes one box back at a
//! time. It does so by changing that box's text in the value and nothing
//! else: every other number stays the text it was, so a value edited in one
//! place is that value with one place changed, and a value not edited is
//! the same bytes (DNA, postulate 2). What the file then holds is the
//! engine's business — it reads the value as its type and writes it.

use std::ops::Range;

/// A part of a value, and where its text is in the value's.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub span: Range<usize>,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `(a: 1.0, b: true)`, or a variant with fields, `Box(half: …)`.
    Struct {
        name: Option<String>,
        fields: Vec<(String, Node)>,
    },
    /// `(1.0, 2.0, 3.0)`, or a variant or wrapper around values:
    /// `Some(2.0)`, `Named("grass")`.
    Tuple {
        name: Option<String>,
        items: Vec<Node>,
    },
    List(Vec<Node>),
    Map(Vec<(Node, Node)>),
    Number,
    Bool(bool),
    /// A string, as it reads: quotes and escapes gone.
    Text(String),
    Char,
    /// A bare name: a variant with nothing in it, `None`.
    Name(String),
    /// `()`.
    Unit,
}

/// One step into a value: a field by name, or an item by place.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    Key(String),
    At(usize),
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Key(k) => f.write_str(k),
            Step::At(i) => write!(f, "{i}"),
        }
    }
}

impl Node {
    /// The part at `path`, if the value has one there.
    pub fn get(&self, path: &[Step]) -> Option<&Node> {
        let Some((step, rest)) = path.split_first() else {
            return Some(self);
        };
        let next = match (&self.kind, step) {
            (Kind::Struct { fields, .. }, Step::Key(k)) => {
                fields.iter().find(|(n, _)| n == k).map(|(_, v)| v)
            }
            (Kind::Map(entries), Step::Key(k)) => entries
                .iter()
                .find(|(key, _)| key_text(key).as_deref() == Some(k.as_str()))
                .map(|(_, v)| v),
            (Kind::Tuple { items, .. } | Kind::List(items), Step::At(i)) => items.get(*i),
            (Kind::Map(entries), Step::At(i)) => entries.get(*i).map(|(_, v)| v),
            _ => None,
        }?;
        next.get(rest)
    }

    /// Whether every item is a number: a vector, a colour.
    pub fn numbers(&self) -> Option<&[Node]> {
        match &self.kind {
            Kind::Tuple { name: None, items }
                if !items.is_empty() && items.iter().all(|i| i.kind == Kind::Number) =>
            {
                Some(items)
            }
            _ => None,
        }
    }
}

/// A map's key as a label: its text, or the string it is.
fn key_text(key: &Node) -> Option<String> {
    match &key.kind {
        Kind::Text(t) | Kind::Name(t) => Some(t.clone()),
        _ => None,
    }
}

/// The value `text` holds, as a tree; `None` when it is not RON this
/// understands.
pub fn parse(text: &str) -> Option<Node> {
    let mut p = Parser { text, at: 0 };
    let node = p.value()?;
    p.space();
    (p.at == text.len()).then_some(node)
}

/// `text` with the part at `path` replaced by `value`. A field the value
/// leaves out is added to the struct that would hold it, at its end; a
/// struct written `()` becomes one with that field. `None` when there is
/// nowhere to put it.
pub fn set(text: &str, path: &[Step], value: &str) -> Option<String> {
    let root = parse(text)?;
    if let Some(node) = root.get(path) {
        return Some(splice(text, node.span.clone(), value));
    }
    let (Step::Key(key), parent) = path.split_last()? else {
        return None;
    };
    let parent = root.get(parent)?;
    match &parent.kind {
        Kind::Struct { fields, .. } => Some(match fields.last() {
            Some((_, last)) => splice(
                text,
                last.span.end..last.span.end,
                &format!(", {key}: {value}"),
            ),
            None => {
                let close = parent.span.end - 1;
                splice(text, close..close, &format!("{key}: {value}"))
            }
        }),
        Kind::Unit => Some(splice(
            text,
            parent.span.clone(),
            &format!("({key}: {value})"),
        )),
        _ => None,
    }
}

/// `text` with the item at `path` taken out of its list.
pub fn remove(text: &str, path: &[Step]) -> Option<String> {
    let root = parse(text)?;
    let (Step::At(i), parent) = path.split_last()? else {
        return None;
    };
    let Kind::List(items) = &root.get(parent)?.kind else {
        return None;
    };
    let span = match (
        items.get(*i)?,
        items.get(i + 1),
        i.checked_sub(1).map(|p| &items[p]),
    ) {
        // Up to the next one, so its comma goes with it; the last one takes
        // the comma before it.
        (item, Some(next), _) => item.span.start..next.span.start,
        (item, None, Some(before)) => before.span.end..item.span.end,
        (item, None, None) => item.span.clone(),
    };
    Some(splice(text, span, ""))
}

/// `text` with `value` added at the end of the list at `path`.
pub fn push(text: &str, path: &[Step], value: &str) -> Option<String> {
    let root = parse(text)?;
    let list = root.get(path)?;
    let Kind::List(items) = &list.kind else {
        return None;
    };
    Some(match items.last() {
        Some(last) => splice(text, last.span.end..last.span.end, &format!(", {value}")),
        None => {
            let close = list.span.end - 1;
            splice(text, close..close, value)
        }
    })
}

fn splice(text: &str, span: Range<usize>, with: &str) -> String {
    format!("{}{with}{}", &text[..span.start], &text[span.end..])
}

/// A string as RON writes one: in quotes, `"` and `\` escaped.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    /// Past whitespace and comments.
    fn space(&mut self) {
        loop {
            let rest = &self.text[self.at..];
            let trimmed = rest.trim_start();
            self.at += rest.len() - trimmed.len();
            if trimmed.starts_with("//") {
                self.at += trimmed.find('\n').unwrap_or(trimmed.len());
            } else if trimmed.starts_with("/*") {
                self.at += trimmed.find("*/").map_or(trimmed.len(), |n| n + 2);
            } else {
                return;
            }
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Option<String> {
        let rest = &self.text[self.at..];
        let raw = rest.starts_with("r#");
        let body = if raw { &rest[2..] } else { rest };
        let len = body
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
            .map_or(body.len(), |(i, _)| i);
        if len == 0 || body.as_bytes()[0].is_ascii_digit() {
            return None;
        }
        self.at += len + if raw { 2 } else { 0 };
        Some(body[..len].to_string())
    }

    fn value(&mut self) -> Option<Node> {
        self.space();
        let start = self.at;
        let kind = match self.peek()? {
            b'(' => return self.group(start, None),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    if self.eat(b']') {
                        break;
                    }
                    items.push(self.value()?);
                    if !self.eat(b',') {
                        if !self.eat(b']') {
                            return None;
                        }
                        break;
                    }
                }
                Kind::List(items)
            }
            b'{' => {
                self.at += 1;
                let mut entries = Vec::new();
                loop {
                    if self.eat(b'}') {
                        break;
                    }
                    let key = self.value()?;
                    if !self.eat(b':') {
                        return None;
                    }
                    let value = self.value()?;
                    entries.push((key, value));
                    if !self.eat(b',') {
                        if !self.eat(b'}') {
                            return None;
                        }
                        break;
                    }
                }
                Kind::Map(entries)
            }
            b'"' => Kind::Text(self.string()?),
            // `r"…"` and `r#"…"#` are strings; `r#None` is a name.
            b'r' if {
                let rest = &self.text[self.at + 1..];
                rest.starts_with('"')
                    || (rest.starts_with('#') && rest.trim_start_matches('#').starts_with('"'))
            } =>
            {
                Kind::Text(self.raw_string()?)
            }
            b'\'' => {
                self.at += 1;
                let rest = &self.text[self.at..];
                // `'\''`: an escaped quote is not the closing one.
                let end = match rest.strip_prefix('\\') {
                    Some(escaped) => 2 + escaped.get(1..)?.find('\'')?,
                    None => rest.find('\'')?,
                };
                self.at += end + 1;
                Kind::Char
            }
            c if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' => {
                let rest = &self.text[self.at..];
                let len = rest
                    .char_indices()
                    .find(|(i, c)| {
                        !(c.is_ascii_alphanumeric()
                            || *c == '.'
                            || *c == '_'
                            || ((*c == '-' || *c == '+')
                                && (*i == 0 || rest[..*i].ends_with(['e', 'E']))))
                    })
                    .map_or(rest.len(), |(i, _)| i);
                self.at += len;
                Kind::Number
            }
            _ => return self.named(start),
        };
        Some(Node {
            span: start..self.at,
            kind,
        })
    }

    /// Something that starts with a name: `true`, `None`, `Static`,
    /// `Box(half: …)`, `inf`.
    fn named(&mut self, start: usize) -> Option<Node> {
        let name = self.ident()?;
        let kind = match name.as_str() {
            "true" => Kind::Bool(true),
            "false" => Kind::Bool(false),
            "inf" | "NaN" => Kind::Number,
            _ => {
                let save = self.at;
                self.space();
                if self.peek() == Some(b'(') {
                    return self.group(start, Some(name));
                }
                self.at = save;
                Kind::Name(name)
            }
        };
        Some(Node {
            span: start..self.at,
            kind,
        })
    }

    /// `( … )` from its opening bracket: a struct when it starts with
    /// `key:`, a tuple otherwise, `()` when empty.
    fn group(&mut self, start: usize, name: Option<String>) -> Option<Node> {
        self.space();
        self.at += 1; // the `(`
        if self.eat(b')') {
            let kind = match name {
                Some(name) => Kind::Tuple {
                    name: Some(name),
                    items: Vec::new(),
                },
                None => Kind::Unit,
            };
            return Some(Node {
                span: start..self.at,
                kind,
            });
        }
        // A struct: a name, then a colon that is not the start of `::`.
        self.space();
        let save = self.at;
        let is_struct = self.ident().is_some() && {
            self.space();
            self.text[self.at..].starts_with(':') && !self.text[self.at..].starts_with("::")
        };
        self.at = save;
        let kind = if is_struct {
            let mut fields = Vec::new();
            loop {
                if self.eat(b')') {
                    break;
                }
                self.space();
                let key = self.ident()?;
                if !self.eat(b':') {
                    return None;
                }
                let value = self.value()?;
                fields.push((key, value));
                if !self.eat(b',') {
                    if !self.eat(b')') {
                        return None;
                    }
                    break;
                }
            }
            Kind::Struct { name, fields }
        } else {
            let mut items = Vec::new();
            loop {
                if self.eat(b')') {
                    break;
                }
                items.push(self.value()?);
                if !self.eat(b',') {
                    if !self.eat(b')') {
                        return None;
                    }
                    break;
                }
            }
            Kind::Tuple { name, items }
        };
        Some(Node {
            span: start..self.at,
            kind,
        })
    }

    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        let mut chars = self.text[self.at..].char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    self.at += i + 1;
                    return Some(out);
                }
                '\\' => {
                    let (_, e) = chars.next()?;
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        other => other,
                    });
                }
                c => out.push(c),
            }
        }
        None
    }

    fn raw_string(&mut self) -> Option<String> {
        self.at += 1; // the `r`
        let hashes = self.text[self.at..]
            .bytes()
            .take_while(|b| *b == b'#')
            .count();
        self.at += hashes;
        if self.peek() != Some(b'"') {
            return None;
        }
        self.at += 1;
        let close = format!("\"{}", "#".repeat(hashes));
        let end = self.text[self.at..].find(&close)?;
        let out = self.text[self.at..self.at + end].to_string();
        self.at += end + close.len();
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> Step {
        Step::Key(k.into())
    }

    #[test]
    fn a_value_reads_as_its_parts() {
        let text = "(color:(0.62,0.68,0.74),start:18.0,end:90.0,mode:Exponential)";
        let root = parse(text).unwrap();
        let color = root.get(&[key("color")]).unwrap();
        assert_eq!(color.numbers().map(<[Node]>::len), Some(3));
        assert_eq!(&text[color.span.clone()], "(0.62,0.68,0.74)");
        assert_eq!(
            root.get(&[key("mode")]).unwrap().kind,
            Kind::Name("Exponential".into())
        );
        let collider = parse("Box(half: (0.5, 0.5, 0.5))").unwrap();
        let Kind::Struct { name, fields } = &collider.kind else {
            panic!("{collider:?}")
        };
        assert_eq!(name.as_deref(), Some("Box"));
        assert_eq!(fields[0].0, "half");
        assert_eq!(parse("Static").unwrap().kind, Kind::Name("Static".into()));
        assert_eq!(parse("None").unwrap().kind, Kind::Name("None".into()));
        assert_eq!(parse("()").unwrap().kind, Kind::Unit);
        assert_eq!(
            parse(r#""a \"b\"""#).unwrap().kind,
            Kind::Text("a \"b\"".into())
        );
        assert_eq!(parse("-1.5e-3").unwrap().kind, Kind::Number);
        let some = parse("(mass:Some(2.0),tags:[\"x\",\"y\"])").unwrap();
        assert!(matches!(
            some.get(&[key("mass")]).unwrap().kind,
            Kind::Tuple { ref name, .. } if name.as_deref() == Some("Some")
        ));
        assert_eq!(
            some.get(&[key("tags"), Step::At(1)]).unwrap().kind,
            Kind::Text("y".into())
        );
        assert!(parse("(a: 1.0").is_none(), "not closed");
        assert!(parse("(a: 1.0) x").is_none(), "something after it");
    }

    #[test]
    fn one_place_changes_and_the_rest_is_the_same_bytes() {
        let text = "(hour:9.0,intensity:1.15,ground:(0.3,0.28,0.25))";
        // Nothing edited: every part's text is where the tree says.
        let root = parse(text).unwrap();
        assert_eq!(root.span, 0..text.len());
        assert_eq!(set(text, &[], text).as_deref(), Some(text));
        assert_eq!(
            set(text, &[key("intensity")], "1.15").as_deref(),
            Some(text),
            "the same number back is the same bytes"
        );
        assert_eq!(
            set(text, &[key("ground"), Step::At(1)], "0.5").as_deref(),
            Some("(hour:9.0,intensity:1.15,ground:(0.3,0.5,0.25))")
        );
        // Left out: added at the end of its struct.
        assert_eq!(
            set(text, &[key("toward")], "(0.0, 1.0, 0.0)").as_deref(),
            Some("(hour:9.0,intensity:1.15,ground:(0.3,0.28,0.25), toward: (0.0, 1.0, 0.0))")
        );
        assert_eq!(set("()", &[key("a")], "1.0").as_deref(), Some("(a: 1.0)"));
        assert_eq!(
            set(
                "Box(half: (0.5, 0.5, 0.5))",
                &[key("center")],
                "(0.0, 1.0, 0.0)"
            )
            .as_deref(),
            Some("Box(half: (0.5, 0.5, 0.5), center: (0.0, 1.0, 0.0))")
        );
        assert_eq!(set(text, &[key("ground"), key("x")], "1.0"), None);
        // Comments and spacing a person wrote stay.
        let hand = "(\n    // warm\n    hour: 9.0,\n    intensity:   1.15,\n)";
        assert_eq!(
            set(hand, &[key("hour")], "10.0").as_deref(),
            Some("(\n    // warm\n    hour: 10.0,\n    intensity:   1.15,\n)")
        );
    }

    #[test]
    fn a_list_takes_an_item_and_gives_one_up() {
        let text = "(points:[(0.0,0.0,0.0),(0.0,2.0,0.0)],speed:1.0)";
        let points = [key("points")];
        let longer = push(text, &points, "(1.0, 2.0, 3.0)").unwrap();
        assert_eq!(
            longer,
            "(points:[(0.0,0.0,0.0),(0.0,2.0,0.0), (1.0, 2.0, 3.0)],speed:1.0)"
        );
        let mut at = points.to_vec();
        at.push(Step::At(2));
        assert_eq!(remove(&longer, &at).as_deref(), Some(text));
        at[1] = Step::At(0);
        assert_eq!(
            remove(text, &at).as_deref(),
            Some("(points:[(0.0,2.0,0.0)],speed:1.0)")
        );
        assert_eq!(push("[]", &[], "1").as_deref(), Some("[1]"));
    }

    #[test]
    fn a_string_goes_back_in_quotes() {
        assert_eq!(quote("a \"b\" \\"), r#""a \"b\" \\""#);
        let text = quote("line\nnext");
        assert_eq!(parse(&text).unwrap().kind, Kind::Text("line\nnext".into()));
    }
}
