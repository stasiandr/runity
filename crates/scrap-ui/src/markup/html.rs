//! The HTML a screen is written in, read into a tree: elements with their
//! attributes, and text. Only the reading — which tags mean what is
//! [`super::Screen`]'s. A mistake is a [`Problem`] at its line and column,
//! never a guess: a browser forgives an unclosed `<div>`, this does not,
//! because the agent that wrote it should hear about it.

use super::{Pos, Problem};

/// An element as written: `<button id="play" class="big">Play</button>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Lowercase, as HTML is.
    pub tag: String,
    /// In the order written; names lowercase.
    pub attrs: Vec<Attr>,
    pub children: Vec<Content>,
    pub at: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    pub value: String,
    pub at: Pos,
}

/// What an element holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Element(Element),
    /// Words, each run of whitespace collapsed to one space, as a browser
    /// shows them; never only whitespace. The ends keep their one space:
    /// `Score: <b>5</b>` is two pieces with a space between, and the
    /// screen trims what is at the edges of an element.
    Text(String, Pos),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    /// The classes in `class="a b"`.
    pub fn classes(&self) -> impl Iterator<Item = &str> {
        self.attr("class").unwrap_or("").split_whitespace()
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Content::Element(e) => Some(e),
            Content::Text(..) => None,
        })
    }
}

/// Elements that never have content: `<input>`, not `<input></input>`.
const VOID: [&str; 6] = ["input", "img", "link", "meta", "br", "hr"];
/// Elements whose content is not HTML: a `<style>`'s CSS.
const RAW: [&str; 2] = ["style", "script"];

/// Read a document: the elements and text at its top.
pub fn parse(file: &str, src: &str) -> Result<Vec<Content>, Problem> {
    let mut reader = Reader {
        file,
        chars: src.chars().collect(),
        i: 0,
        line: 1,
        column: 1,
    };
    reader.nodes(None)
}

struct Reader<'a> {
    file: &'a str,
    chars: Vec<char>,
    i: usize,
    line: u32,
    column: u32,
}

impl Reader<'_> {
    fn pos(&self) -> Pos {
        Pos {
            line: self.line,
            column: self.column,
        }
    }

    fn problem(&self, at: Pos, message: impl Into<String>) -> Problem {
        Problem::new(self.file, at, message)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        let mut i = self.i;
        for c in s.chars() {
            match self.chars.get(i) {
                Some(have) if have.eq_ignore_ascii_case(&c) => i += 1,
                _ => return false,
            }
        }
        true
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn skip(&mut self, n: usize) {
        for _ in 0..n {
            self.bump();
        }
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    /// Up to and past `end`; what was before it.
    fn until(&mut self, end: &str, opened: Pos, what: &str) -> Result<String, Problem> {
        let mut out = String::new();
        while !self.starts_with(end) {
            match self.bump() {
                Some(c) => out.push(c),
                None => {
                    return Err(
                        self.problem(opened, format!("{what} opened here never ends: no `{end}`"))
                    )
                }
            }
        }
        self.skip(end.chars().count());
        Ok(out)
    }

    /// Content up to the `</tag>` that closes `open`, or to the end of
    /// the file when nothing is open.
    fn nodes(&mut self, open: Option<(&str, Pos)>) -> Result<Vec<Content>, Problem> {
        let mut out = Vec::new();
        loop {
            let at = self.pos();
            if self.peek().is_none() {
                return match open {
                    None => Ok(out),
                    Some((tag, at)) => Err(self.problem(
                        at,
                        format!("`<{tag}>` is never closed: no `</{tag}>` before the end"),
                    )),
                };
            }
            if self.starts_with("<!--") {
                self.skip(4);
                self.until("-->", at, "a comment")?;
            } else if self.starts_with("<!") {
                // `<!doctype html>`: says nothing to us.
                self.until(">", at, "`<!`")?;
            } else if self.starts_with("</") {
                self.skip(2);
                let name = self.name();
                self.skip_space();
                if self.peek() != Some('>') {
                    return Err(self.problem(self.pos(), format!("`</{name}` wants a `>`")));
                }
                self.bump();
                return match open {
                    Some((tag, _)) if tag == name => Ok(out),
                    Some((tag, opened)) => Err(self.problem(
                        at,
                        format!(
                            "`</{name}>` here, but the open element is `<{tag}>` (line {})",
                            opened.line
                        ),
                    )),
                    None => Err(self.problem(at, format!("`</{name}>` closes nothing"))),
                };
            } else if self.peek() == Some('<')
                && self
                    .chars
                    .get(self.i + 1)
                    .is_some_and(|c| c.is_ascii_alphabetic())
            {
                out.push(Content::Element(self.element()?));
            } else {
                let mut text = String::new();
                while let Some(c) = self.peek() {
                    if c == '<' {
                        break;
                    }
                    text.push(c);
                    self.bump();
                }
                let text = collapse(&decode(&text));
                if !text.trim().is_empty() {
                    out.push(Content::Text(text, at));
                }
            }
        }
    }

    fn name(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':' {
                name.push(c.to_ascii_lowercase());
                self.bump();
            } else {
                break;
            }
        }
        name
    }

    fn element(&mut self) -> Result<Element, Problem> {
        let at = self.pos();
        self.bump(); // <
        let tag = self.name();
        let mut attrs: Vec<Attr> = Vec::new();
        let closed = loop {
            self.skip_space();
            let here = self.pos();
            match self.peek() {
                None => {
                    return Err(self.problem(at, format!("`<{tag}` never ends: no `>`")));
                }
                Some('>') => {
                    self.bump();
                    break false;
                }
                Some('/') if self.starts_with("/>") => {
                    self.skip(2);
                    break true;
                }
                _ => {}
            }
            let name = self.name();
            if name.is_empty() {
                let c = self.peek().unwrap_or(' ');
                return Err(self.problem(here, format!("`{c}` where an attribute's name goes")));
            }
            self.skip_space();
            let value = if self.peek() == Some('=') {
                self.bump();
                self.skip_space();
                match self.peek() {
                    Some(q @ ('"' | '\'')) => {
                        self.bump();
                        decode(&self.until(&q.to_string(), here, "an attribute's value")?)
                    }
                    _ => {
                        let mut v = String::new();
                        while let Some(c) = self.peek() {
                            if c.is_whitespace() || c == '>' {
                                break;
                            }
                            v.push(c);
                            self.bump();
                        }
                        decode(&v)
                    }
                }
            } else {
                String::new()
            };
            if attrs.iter().any(|a| a.name == name) {
                return Err(self.problem(here, format!("`{name}` is given twice on `<{tag}>`")));
            }
            attrs.push(Attr {
                name,
                value,
                at: here,
            });
        };
        let children = if closed || VOID.contains(&tag.as_str()) {
            Vec::new()
        } else if RAW.contains(&tag.as_str()) {
            let inside = self.pos();
            let raw = self.until(&format!("</{tag}>"), at, &format!("`<{tag}>`"))?;
            vec![Content::Text(raw, inside)]
        } else {
            self.nodes(Some((&tag, at)))?
        };
        Ok(Element {
            tag,
            attrs,
            children,
            at,
        })
    }
}

/// `&amp;` and its kind, as text.
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let entity = end.map(|e| &rest[1..e]);
        let c = match entity {
            Some("amp") => Some('&'),
            Some("lt") => Some('<'),
            Some("gt") => Some('>'),
            Some("quot") => Some('"'),
            Some("apos") | Some("#39") => Some('\''),
            Some("nbsp") => Some('\u{a0}'),
            Some(e) if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            Some(e) if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match (c, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Runs of whitespace as one space.
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(src: &str) -> Element {
        match parse("t.html", src).unwrap().remove(0) {
            Content::Element(e) => e,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn elements_attributes_and_text_read_as_written() {
        let e = element(
            "<!doctype html>\n<!-- a menu -->\n<div id=main class='a  b'>\n  <button disabled>Play &amp; win</button>\n  <input placeholder=\"Name\">\n  <br/>\n</div>",
        );
        assert_eq!(e.tag, "div");
        assert_eq!(e.id(), Some("main"));
        assert_eq!(e.classes().collect::<Vec<_>>(), ["a", "b"]);
        let kids: Vec<_> = e.elements().map(|e| e.tag.as_str()).collect();
        assert_eq!(kids, ["button", "input", "br"]);
        let button = e.elements().next().unwrap();
        assert_eq!(button.attr("disabled"), Some(""));
        assert_eq!(
            button.children,
            [Content::Text(
                "Play & win".into(),
                Pos {
                    line: 4,
                    column: 20
                }
            )]
        );
    }

    #[test]
    fn a_style_keeps_its_css_as_it_is() {
        let e = element("<style>a > b { color: red }</style>");
        assert_eq!(
            e.children,
            [Content::Text(
                "a > b { color: red }".into(),
                Pos { line: 1, column: 8 }
            )]
        );
    }

    #[test]
    fn a_mistake_says_where() {
        let p = parse("hud.html", "<div>\n  <span>hi</div>").unwrap_err();
        assert_eq!(
            p.to_string(),
            "hud.html:2:11: `</div>` here, but the open element is `<span>` (line 2)"
        );
        let p = parse("hud.html", "<div>\n<p>x</p>").unwrap_err();
        assert_eq!(
            p.to_string(),
            "hud.html:1:1: `<div>` is never closed: no `</div>` before the end"
        );
        let p = parse("hud.html", "<b id=a id=b></b>").unwrap_err();
        assert!(p.to_string().contains("`id` is given twice"), "{p}");
    }
}
