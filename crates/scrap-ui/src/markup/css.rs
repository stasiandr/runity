//! The CSS a screen is styled with: the part of CSS that `scrap-ui` can
//! draw, and nothing that it cannot. A property it does not know, a value
//! it cannot read, a selector it does not match is a [`Problem`] at its
//! line and column — not a rule quietly dropped, as a browser would.
//!
//! What it reads: rules of simple selectors (`button`, `.card`, `#play`,
//! `*`, compounds of them, descendants and `>` children, `:hover` and
//! `:active` at the end), custom properties in `:root` used through
//! `var()`, and the properties in [`apply`]. Layout is taffy's, which is
//! CSS's flexbox; every element is a flex container already, a column
//! unless it says otherwise.

use std::collections::HashMap;

use taffy::prelude::{auto, length, percent};

use super::{Pos, Problem};
use crate::style::{Color, Style, TextAlign};

/// A stylesheet: its rules in the order written, and `:root`'s variables.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    pub rules: Vec<Rule>,
    pub vars: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub decls: Vec<Decl>,
}

/// `property: value`, where it was written.
#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub property: String,
    pub value: String,
    pub file: String,
    pub at: Pos,
}

/// Which pointer state a rule is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Always,
    Hover,
    Active,
}

/// `nav > .item:hover`: compounds from the outermost, each with how it
/// stands to the one before it.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub parts: Vec<(Combinator, Compound)>,
    pub state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// The first compound, or anywhere under the one before.
    Descendant,
    /// Right under the one before.
    Child,
}

/// `button.primary#play`: a tag (or any), an id, classes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

/// What a selector sees of an element.
#[derive(Debug, Clone, Default)]
pub struct Subject<'a> {
    pub tag: &'a str,
    pub id: Option<&'a str>,
    pub classes: Vec<&'a str>,
}

impl Compound {
    fn matches(&self, s: &Subject) -> bool {
        self.tag.as_ref().is_none_or(|t| t == s.tag)
            && self.id.as_ref().is_none_or(|id| Some(id.as_str()) == s.id)
            && self.classes.iter().all(|c| s.classes.contains(&c.as_str()))
    }
}

impl Selector {
    /// CSS's specificity: ids, then classes and states, then tags.
    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut out = (0, 0, 0);
        for (_, c) in &self.parts {
            out.0 += c.id.is_some() as u32;
            out.1 += c.classes.len() as u32;
            out.2 += c.tag.is_some() as u32;
        }
        out.1 += (self.state != State::Always) as u32;
        out
    }

    /// Whether it picks the last of `chain`, the elements from the top
    /// down to the one being styled.
    pub fn matches(&self, chain: &[Subject]) -> bool {
        // The combinator in a part says how it stands to the part before,
        // so the walk looks at the part's own combinator one step up.
        fn walk(parts: &[(Combinator, Compound)], chain: &[Subject]) -> bool {
            let Some(((how, last), rest)) = parts.split_last() else {
                return true;
            };
            let Some((subject, above)) = chain.split_last() else {
                return false;
            };
            if !last.matches(subject) {
                return false;
            }
            if rest.is_empty() {
                return true;
            }
            match how {
                Combinator::Child => walk(rest, above),
                Combinator::Descendant => (0..above.len()).rev().any(|i| walk(rest, &above[..=i])),
            }
        }
        walk(&self.parts, chain)
    }
}

/// Byte offset to line and column, for problems.
struct Lines<'a> {
    src: &'a str,
    base: Pos,
}

impl Lines<'_> {
    fn at(&self, offset: usize) -> Pos {
        let before = &self.src[..offset.min(self.src.len())];
        let line = before.matches('\n').count() as u32;
        let column = match before.rfind('\n') {
            Some(i) => before[i + 1..].chars().count() as u32 + 1,
            None => before.chars().count() as u32 + self.base.column,
        };
        Pos {
            line: self.base.line + line,
            column,
        }
    }
}

/// Read a stylesheet. `base` is where `src` starts in `file`: line 1 of a
/// `.css`, or the line of a `<style>` in the HTML. Every problem in it is
/// listed, not only the first. The values are tried by [`check`].
pub fn parse(file: &str, src: &str, base: Pos) -> Result<Sheet, Vec<Problem>> {
    let lines = Lines { src, base };
    let problem = |offset: usize, message: String| Problem::new(file, lines.at(offset), message);
    // Comments become spaces, so offsets — and lines — stay where they were.
    let mut text: Vec<u8> = src.as_bytes().to_vec();
    let mut i = 0;
    let mut problems = Vec::new();
    while i + 1 < text.len() {
        if &text[i..i + 2] == b"/*" {
            let start = i;
            let end = src[i + 2..].find("*/").map(|e| i + 2 + e + 2);
            let end = match end {
                Some(e) => e,
                None => {
                    problems.push(problem(
                        start,
                        "a comment opened here never ends: no `*/`".into(),
                    ));
                    text.len()
                }
            };
            for b in &mut text[start..end] {
                if *b != b'\n' {
                    *b = b' ';
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    let text = String::from_utf8(text).expect("only ASCII was replaced");

    let mut sheet = Sheet::default();
    let mut rest = 0;
    while let Some(open) = text[rest..].find('{').map(|o| rest + o) {
        let head = &text[rest..open];
        let head_at = rest + (head.len() - head.trim_start().len());
        let head = head.trim();
        // The `}` that closes this `{`, counting any inside it.
        let mut depth = 0;
        let mut close = None;
        let mut nested = None;
        for (i, b) in text[open..].bytes().enumerate() {
            match b {
                b'{' => {
                    depth += 1;
                    if depth == 2 && nested.is_none() {
                        nested = Some(open + i);
                    }
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            problems.push(problem(head_at, format!("`{head} {{` never ends: no `}}`")));
            break;
        };
        rest = close + 1;
        if head.starts_with('@') {
            problems.push(problem(
                head_at,
                format!("`{head}`: @-rules are not read here (no @media, @keyframes or @import)"),
            ));
            continue;
        }
        if let Some(nested) = nested {
            problems.push(problem(
                nested,
                "a block inside a block: nesting is not read here".into(),
            ));
            continue;
        }
        let body = &text[open + 1..close];
        // The declarations.
        let mut decls = Vec::new();
        let mut offset = open + 1;
        for part in body.split(';') {
            let at = offset + (part.len() - part.trim_start().len());
            offset += part.len() + 1;
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((property, value)) = part.split_once(':') else {
                problems.push(problem(at, format!("`{part}` wants `property: value`")));
                continue;
            };
            let property = property.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if value.ends_with("!important") {
                problems.push(problem(
                    at,
                    format!("`{property}`: `!important` is not read here — make the selector more specific"),
                ));
                continue;
            }
            decls.push(Decl {
                property,
                value,
                file: file.to_string(),
                at: lines.at(at),
            });
        }
        if head == ":root" {
            for d in decls {
                match d.property.strip_prefix("--") {
                    Some(name) => {
                        sheet.vars.insert(name.to_string(), d.value);
                    }
                    None => problems.push(Problem::new(
                        file,
                        d.at,
                        format!("`{}` in `:root`: only custom properties (`--name`) live there; style `body`", d.property),
                    )),
                }
            }
            continue;
        }
        let mut selectors = Vec::new();
        let mut sel_offset = head_at;
        for one in head.split(',') {
            let at = sel_offset + (one.len() - one.trim_start().len());
            sel_offset += one.len() + 1;
            match selector(one.trim()) {
                Ok(s) => selectors.push(s),
                Err(e) => problems.push(problem(at, e)),
            }
        }
        for d in &decls {
            if d.property.starts_with("--") {
                problems.push(Problem::new(
                    file,
                    d.at,
                    format!(
                        "`{}`: custom properties are read from `:root` only",
                        d.property
                    ),
                ));
            }
        }
        if !selectors.is_empty() {
            sheet.rules.push(Rule { selectors, decls });
        }
    }
    let tail = text[rest..].trim();
    if !tail.is_empty() {
        let at = rest + (text[rest..].len() - text[rest..].trim_start().len());
        problems.push(problem(at, format!("`{tail}` is not a rule: no `{{`")));
    }
    if problems.is_empty() {
        Ok(sheet)
    } else {
        Err(problems)
    }
}

/// Every value of a sheet tried on a style: a wrong one is heard when the
/// screen is read, not when the element it styles first shows. Run on the
/// sheets of a screen together, since a variable may come from another.
pub fn check(sheet: &Sheet) -> Vec<Problem> {
    let mut problems = Vec::new();
    let probe = Context {
        viewport: (1280.0, 720.0),
        vars: &sheet.vars,
    };
    for rule in &sheet.rules {
        let states: std::collections::BTreeSet<State> =
            rule.selectors.iter().map(|s| s.state).collect();
        for d in &rule.decls {
            if d.property.starts_with("--") {
                continue;
            }
            for state in states.iter().copied() {
                let mut style = Style::column();
                let mut late = Late::default();
                if let Err(e) = apply(&mut style, &mut late, state, &d.property, &d.value, &probe) {
                    problems.push(Problem::new(&d.file, d.at, e));
                }
            }
        }
    }
    problems
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for State {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

fn selector(s: &str) -> Result<Selector, String> {
    if s.is_empty() {
        return Err("an empty selector".into());
    }
    let mut parts = Vec::new();
    let mut state = State::Always;
    let mut how = Combinator::Descendant;
    let spaced = s.replace('>', " > ");
    let words: Vec<&str> = spaced.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        if *word == ">" {
            if parts.is_empty() {
                return Err(format!("`{s}` starts with `>`"));
            }
            how = Combinator::Child;
            continue;
        }
        if state != State::Always {
            return Err(format!(
                "`{s}`: `:hover` and `:active` go on the last element only"
            ));
        }
        let (compound, st) = compound(word).map_err(|e| format!("`{s}`: {e}"))?;
        if st != State::Always && i + 1 != words.len() {
            return Err(format!(
                "`{s}`: `:hover` and `:active` go on the last element only"
            ));
        }
        state = st;
        parts.push((how, compound));
        how = Combinator::Descendant;
    }
    if how == Combinator::Child {
        return Err(format!("`{s}` ends with `>`"));
    }
    Ok(Selector { parts, state })
}

fn compound(word: &str) -> Result<(Compound, State), String> {
    let mut c = Compound::default();
    let mut state = State::Always;
    let mut chars = word.char_indices().peekable();
    let ident_end = |from: usize| {
        word[from..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
            .map_or(word.len(), |e| from + e)
    };
    while let Some((i, ch)) = chars.next() {
        let (kind, from) = match ch {
            '*' if i == 0 => {
                continue;
            }
            '.' | '#' => (ch, i + 1),
            ':' => {
                if word[i..].starts_with("::") {
                    return Err(format!(
                        "`{}`: pseudo-elements are not read here",
                        &word[i..]
                    ));
                }
                (ch, i + 1)
            }
            c if i == 0 && c.is_ascii_alphabetic() => ('t', 0),
            '[' => return Err("attribute selectors are not read here; give it a class".into()),
            '+' | '~' => {
                return Err(format!(
                    "the `{ch}` combinator is not read here; give it a class"
                ))
            }
            other => return Err(format!("`{other}` cannot be read in a selector")),
        };
        let end = ident_end(from);
        let name = &word[from..end];
        if name.is_empty() {
            return Err(format!("`{kind}` wants a name after it"));
        }
        match kind {
            't' => c.tag = Some(name.to_ascii_lowercase()),
            '.' => c.classes.push(name.to_string()),
            '#' => c.id = Some(name.to_string()),
            _ => {
                state = match name {
                    "hover" => State::Hover,
                    "active" => State::Active,
                    "root" => return Err("`:root` stands alone: `:root { --name: value }`".into()),
                    other => {
                        return Err(format!(
                            "`:{other}` is not read here: only `:hover` and `:active`"
                        ))
                    }
                }
            }
        }
        while chars.peek().is_some_and(|(j, _)| *j < end) {
            chars.next();
        }
    }
    Ok((c, state))
}

/// What values are read against: the window, for `vw` and `vh`, and the
/// sheet's variables.
pub struct Context<'a> {
    pub viewport: (f32, f32),
    pub vars: &'a HashMap<String, String>,
}

/// What can only be settled after every declaration: a line height in
/// pixels is a multiple of the font size, which may come after it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Late {
    pub line_height_px: Option<f32>,
}

impl Late {
    pub fn settle(self, style: &mut Style) {
        if let Some(px) = self.line_height_px {
            style.text.line_height = px / style.text.size.max(1.0);
        }
    }
}

/// `var(--name)` and `var(--name, fallback)` replaced by what they say.
fn substitute(value: &str, vars: &HashMap<String, String>) -> Result<String, String> {
    let mut out = value.to_string();
    for _ in 0..16 {
        let Some(start) = out.find("var(") else {
            return Ok(out);
        };
        let mut depth = 0;
        let mut end = None;
        for (i, c) in out[start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.ok_or_else(|| format!("`{value}`: `var(` is never closed"))?;
        let inner = &out[start + 4..end];
        let (name, fallback) = match inner.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner.trim(), None),
        };
        let name = name
            .strip_prefix("--")
            .ok_or_else(|| format!("`var({inner})` wants a `--name`"))?;
        let with = match (vars.get(name), fallback) {
            (Some(v), _) => v.clone(),
            (None, Some(f)) => f.to_string(),
            (None, None) => {
                return Err(format!(
                    "`--{name}` is not in `:root`, and `var()` gives no fallback"
                ))
            }
        };
        out.replace_range(start..=end, &with);
    }
    Err(format!(
        "`{value}`: variables refer to each other in a loop"
    ))
}

/// A length as CSS writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Len {
    Px(f32),
    Percent(f32),
    Auto,
}

fn len(v: &str, cx: &Context) -> Result<Len, String> {
    let v = v.trim();
    if v == "auto" {
        return Ok(Len::Auto);
    }
    let num = |s: &str| {
        s.trim()
            .parse::<f32>()
            .map_err(|_| format!("`{v}` is not a length (px, %, vw, vh, rem)"))
    };
    if let Some(n) = v.strip_suffix("px") {
        Ok(Len::Px(num(n)?))
    } else if let Some(n) = v.strip_suffix('%') {
        Ok(Len::Percent(num(n)? / 100.0))
    } else if let Some(n) = v.strip_suffix("vw") {
        Ok(Len::Px(num(n)? / 100.0 * cx.viewport.0))
    } else if let Some(n) = v.strip_suffix("vh") {
        Ok(Len::Px(num(n)? / 100.0 * cx.viewport.1))
    } else if let Some(n) = v.strip_suffix("rem") {
        Ok(Len::Px(num(n)? * REM))
    } else if v == "0" {
        Ok(Len::Px(0.0))
    } else if v.ends_with("em") {
        Err(format!("`{v}`: `em` is not read here; `rem` is 16px"))
    } else {
        Err(format!("`{v}` is not a length (px, %, vw, vh, rem)"))
    }
}

/// The size of a `rem`.
pub const REM: f32 = 16.0;

fn dimension(v: &str, cx: &Context) -> Result<taffy::Dimension, String> {
    Ok(match len(v, cx)? {
        Len::Px(p) => length(p),
        Len::Percent(p) => percent(p),
        Len::Auto => auto(),
    })
}

fn length_percentage(v: &str, cx: &Context) -> Result<taffy::LengthPercentage, String> {
    Ok(match len(v, cx)? {
        Len::Px(p) => length(p),
        Len::Percent(p) => percent(p),
        Len::Auto => return Err(format!("`{v}`: `auto` does not go here")),
    })
}

fn length_percentage_auto(v: &str, cx: &Context) -> Result<taffy::LengthPercentageAuto, String> {
    Ok(match len(v, cx)? {
        Len::Px(p) => length(p),
        Len::Percent(p) => percent(p),
        Len::Auto => auto(),
    })
}

fn px(v: &str, cx: &Context) -> Result<f32, String> {
    match len(v, cx)? {
        Len::Px(p) => Ok(p),
        _ => Err(format!("`{v}` wants pixels here")),
    }
}

/// One to four values, CSS's top, right, bottom, left.
fn four<T: Copy>(
    v: &str,
    one: impl Fn(&str) -> Result<T, String>,
) -> Result<taffy::Rect<T>, String> {
    let parts: Vec<T> = v.split_whitespace().map(one).collect::<Result<_, _>>()?;
    let (top, right, bottom, left) = match parts.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => return Err(format!("`{v}` wants one to four values")),
    };
    Ok(taffy::Rect {
        left,
        right,
        top,
        bottom,
    })
}

/// `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()` or a few names.
pub fn color(v: &str) -> Result<Color, String> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok();
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let c = match hex.len() {
            3 | 4 if hex.is_ascii() => (|| {
                let a = if hex.len() == 4 { digit(3)? * 17 } else { 255 };
                Some(Color::rgba(
                    digit(0)? * 17,
                    digit(1)? * 17,
                    digit(2)? * 17,
                    a,
                ))
            })(),
            6 | 8 if hex.is_ascii() => (|| {
                let a = if hex.len() == 8 { pair(6)? } else { 255 };
                Some(Color::rgba(pair(0)?, pair(2)?, pair(4)?, a))
            })(),
            _ => None,
        };
        return c.ok_or_else(|| format!("`{v}` is not a colour"));
    }
    if let Some(inner) = v
        .strip_prefix("rgba(")
        .or_else(|| v.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner
            .split([',', '/', ' '])
            .filter(|p| !p.trim().is_empty())
            .collect();
        let channel = |s: &str| -> Option<u8> {
            let s = s.trim();
            match s.strip_suffix('%') {
                Some(p) => p
                    .parse::<f32>()
                    .ok()
                    .map(|p| (p * 2.55).round().clamp(0.0, 255.0) as u8),
                None => s
                    .parse::<f32>()
                    .ok()
                    .map(|c| c.round().clamp(0.0, 255.0) as u8),
            }
        };
        let alpha = |s: &str| -> Option<u8> {
            let s = s.trim();
            let a = match s.strip_suffix('%') {
                Some(p) => p.parse::<f32>().ok()? / 100.0,
                None => s.parse::<f32>().ok()?,
            };
            Some((a.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        let c = match parts.as_slice() {
            [r, g, b] => (|| Some(Color::rgba(channel(r)?, channel(g)?, channel(b)?, 255)))(),
            [r, g, b, a] => (|| {
                Some(Color::rgba(
                    channel(r)?,
                    channel(g)?,
                    channel(b)?,
                    alpha(a)?,
                ))
            })(),
            _ => None,
        };
        return c.ok_or_else(|| format!("`{v}` is not a colour"));
    }
    let named = match v.to_ascii_lowercase().as_str() {
        "transparent" => Color::TRANSPARENT,
        "white" => Color::hex(0xffffff),
        "black" => Color::hex(0x000000),
        "red" => Color::hex(0xff0000),
        "green" => Color::hex(0x008000),
        "lime" => Color::hex(0x00ff00),
        "blue" => Color::hex(0x0000ff),
        "yellow" => Color::hex(0xffff00),
        "orange" => Color::hex(0xffa500),
        "purple" => Color::hex(0x800080),
        "gray" | "grey" => Color::hex(0x808080),
        "silver" => Color::hex(0xc0c0c0),
        _ => {
            return Err(format!(
                "`{v}` is not a colour: write #rrggbb, rgb(), rgba(), or a basic name"
            ))
        }
    };
    Ok(named)
}

fn number(v: &str) -> Result<f32, String> {
    v.trim()
        .parse::<f32>()
        .map_err(|_| format!("`{v}` is not a number"))
}

fn align_items(v: &str) -> Result<Option<taffy::AlignItems>, String> {
    use taffy::AlignItems as A;
    Ok(Some(match v {
        "flex-start" | "start" => A::FLEX_START,
        "flex-end" | "end" => A::FLEX_END,
        "center" => A::CENTER,
        "stretch" | "normal" => A::STRETCH,
        "baseline" => A::BASELINE,
        _ => return Err(format!("`{v}` is not a value of align-items")),
    }))
}

fn align_content(v: &str) -> Result<Option<taffy::AlignContent>, String> {
    use taffy::AlignContent as A;
    Ok(Some(match v {
        "flex-start" | "start" | "normal" => A::FLEX_START,
        "flex-end" | "end" => A::FLEX_END,
        "center" => A::CENTER,
        "stretch" => A::STRETCH,
        "space-between" => A::SPACE_BETWEEN,
        "space-around" => A::SPACE_AROUND,
        "space-evenly" => A::SPACE_EVENLY,
        _ => {
            return Err(format!(
                "`{v}` is not a value of justify-content or align-content"
            ))
        }
    }))
}

/// The properties a `:hover` or `:active` rule may set: `scrap-ui` changes
/// a node's look under the pointer, never its layout.
const STATEFUL: &str = "on `:hover` only `background`, `background-color` and `border-color` change, on `:active` only the background";

/// Apply one declaration to a style. The text properties (`color`,
/// `font-*`, `line-height`, `text-align`, `white-space`) are inherited:
/// the caller starts a child's style from its parent's text.
pub fn apply(
    style: &mut Style,
    late: &mut Late,
    state: State,
    property: &str,
    value: &str,
    cx: &Context,
) -> Result<(), String> {
    let value = substitute(value, cx.vars)?;
    let v = value.trim();
    let l = &mut style.layout;
    match state {
        State::Always => {}
        State::Hover => {
            match property {
                "background" | "background-color" => {
                    style.look.hover_background = Some(color(v)?);
                }
                "border-color" => style.look.hover_border = Some(color(v)?),
                _ => return Err(format!("`{property}` on `:hover`: {STATEFUL}")),
            }
            style.sense.click = true;
            return Ok(());
        }
        State::Active => {
            match property {
                "background" | "background-color" => {
                    style.look.press_background = Some(color(v)?);
                }
                _ => return Err(format!("`{property}` on `:active`: {STATEFUL}")),
            }
            style.sense.click = true;
            return Ok(());
        }
    }
    match property {
        // --- layout ---------------------------------------------------
        "display" => {
            l.display = match v {
                "flex" => taffy::Display::Flex,
                "none" => taffy::Display::None,
                _ => {
                    return Err(format!(
                        "`display: {v}`: every element is already a flex column here; write `flex` or `none` (no grid yet)"
                    ))
                }
            }
        }
        "flex-direction" => {
            l.flex_direction = match v {
                "row" => taffy::FlexDirection::Row,
                "column" => taffy::FlexDirection::Column,
                "row-reverse" => taffy::FlexDirection::RowReverse,
                "column-reverse" => taffy::FlexDirection::ColumnReverse,
                _ => return Err(format!("`{v}` is not a value of flex-direction")),
            }
        }
        "flex-wrap" => {
            l.flex_wrap = match v {
                "wrap" => taffy::FlexWrap::Wrap,
                "nowrap" => taffy::FlexWrap::NoWrap,
                "wrap-reverse" => taffy::FlexWrap::WrapReverse,
                _ => return Err(format!("`{v}` is not a value of flex-wrap")),
            }
        }
        "flex" => {
            let parts: Vec<&str> = v.split_whitespace().collect();
            let (grow, shrink, basis) = match parts.as_slice() {
                ["none"] => (0.0, 0.0, auto()),
                ["auto"] => (1.0, 1.0, auto()),
                [g] => (number(g)?, 1.0, length(0.0)),
                [g, s] => (number(g)?, number(s)?, length(0.0)),
                [g, s, b] => (number(g)?, number(s)?, dimension(b, cx)?),
                _ => return Err(format!("`flex: {v}` wants grow, shrink and basis")),
            };
            l.flex_grow = grow;
            l.flex_shrink = shrink;
            l.flex_basis = basis;
        }
        "flex-grow" => l.flex_grow = number(v)?,
        "flex-shrink" => l.flex_shrink = number(v)?,
        "flex-basis" => l.flex_basis = dimension(v, cx)?,
        "justify-content" => l.justify_content = align_content(v)?,
        "align-content" => l.align_content = align_content(v)?,
        "align-items" => l.align_items = align_items(v)?,
        "align-self" => {
            l.align_self = if v == "auto" { None } else { align_items(v)? }
        }
        "gap" => {
            let parts: Vec<&str> = v.split_whitespace().collect();
            let (row, column) = match parts.as_slice() {
                [a] => (*a, *a),
                [a, b] => (*a, *b),
                _ => return Err(format!("`gap: {v}` wants one or two lengths")),
            };
            l.gap = taffy::Size {
                width: length_percentage(column, cx)?,
                height: length_percentage(row, cx)?,
            };
        }
        "row-gap" => l.gap.height = length_percentage(v, cx)?,
        "column-gap" => l.gap.width = length_percentage(v, cx)?,
        "width" => l.size.width = dimension(v, cx)?,
        "height" => l.size.height = dimension(v, cx)?,
        "min-width" => l.min_size.width = length_percentage_auto(v, cx)?,
        "min-height" => l.min_size.height = length_percentage_auto(v, cx)?,
        "max-width" => l.max_size.width = length_percentage_auto(v, cx)?,
        "max-height" => l.max_size.height = length_percentage_auto(v, cx)?,
        "aspect-ratio" => {
            l.aspect_ratio = Some(match v.split_once('/') {
                Some((a, b)) => number(a)? / number(b)?.max(f32::EPSILON),
                None => number(v)?,
            })
        }
        "padding" => l.padding = four(v, |p| length_percentage(p, cx))?,
        "padding-top" => l.padding.top = length_percentage(v, cx)?,
        "padding-right" => l.padding.right = length_percentage(v, cx)?,
        "padding-bottom" => l.padding.bottom = length_percentage(v, cx)?,
        "padding-left" => l.padding.left = length_percentage(v, cx)?,
        "margin" => l.margin = four(v, |m| length_percentage_auto(m, cx))?,
        "margin-top" => l.margin.top = length_percentage_auto(v, cx)?,
        "margin-right" => l.margin.right = length_percentage_auto(v, cx)?,
        "margin-bottom" => l.margin.bottom = length_percentage_auto(v, cx)?,
        "margin-left" => l.margin.left = length_percentage_auto(v, cx)?,
        "position" => {
            l.position = match v {
                "relative" | "static" => taffy::Position::Relative,
                "absolute" => taffy::Position::Absolute,
                _ => {
                    return Err(format!(
                        "`position: {v}`: `relative` or `absolute` (the screen is the page: no `fixed`)"
                    ))
                }
            }
        }
        "top" => l.inset.top = length_percentage_auto(v, cx)?,
        "right" => l.inset.right = length_percentage_auto(v, cx)?,
        "bottom" => l.inset.bottom = length_percentage_auto(v, cx)?,
        "left" => l.inset.left = length_percentage_auto(v, cx)?,
        "inset" => l.inset = four(v, |m| length_percentage_auto(m, cx))?,
        "overflow" => match v {
            "visible" => {
                style.look.clip = false;
                l.overflow = taffy::Point {
                    x: taffy::Overflow::Visible,
                    y: taffy::Overflow::Visible,
                };
            }
            "hidden" => {
                style.look.clip = true;
                l.overflow = taffy::Point {
                    x: taffy::Overflow::Hidden,
                    y: taffy::Overflow::Hidden,
                };
            }
            "scroll" | "auto" => {
                style.look.clip = true;
                l.overflow = taffy::Point {
                    x: taffy::Overflow::Hidden,
                    y: taffy::Overflow::Scroll,
                };
            }
            _ => return Err(format!("`{v}` is not a value of overflow")),
        },
        // --- look -----------------------------------------------------
        "background" | "background-color" => {
            if v.contains("gradient(") || v.contains("url(") {
                return Err(format!(
                    "`{property}: {v}`: only a colour — no gradients or pictures yet"
                ));
            }
            style.look.background = color(v)?;
        }
        "border" => {
            if v == "none" || v == "0" {
                style.look.border_width = 0.0;
                l.border = four("0", |p| length_percentage(p, cx))?;
                return Ok(());
            }
            let mut width = 1.0;
            let mut paint = style.look.border;
            for word in split_outside_parens(v) {
                if word == "solid" {
                } else if let Ok(w) = px(&word, cx) {
                    width = w;
                } else if matches!(word.as_str(), "dashed" | "dotted" | "double") {
                    return Err(format!("`{word}` borders are not drawn here: `solid` only"));
                } else {
                    paint = color(&word)?;
                }
            }
            style.look.border = paint;
            style.look.border_width = width;
            l.border = four(&format!("{width}px"), |p| length_percentage(p, cx))?;
        }
        "border-width" => {
            let w = px(v, cx)
                .map_err(|_| format!("`border-width: {v}`: one width for all sides"))?;
            style.look.border_width = w;
            l.border = four(&format!("{w}px"), |p| length_percentage(p, cx))?;
        }
        "border-color" => style.look.border = color(v)?,
        "border-style" => {
            if v != "solid" && v != "none" {
                return Err(format!("`{v}` borders are not drawn here: `solid` only"));
            }
            if v == "none" {
                style.look.border_width = 0.0;
                l.border = four("0", |p| length_percentage(p, cx))?;
            }
        }
        "border-radius" => {
            style.look.radius = px(v, cx)
                .map_err(|_| format!("`border-radius: {v}`: one radius in px for every corner"))?
        }
        "opacity" => style.look.opacity = number(v)?.clamp(0.0, 1.0),
        "cursor" => match v {
            "pointer" => style.sense.click = true,
            "default" | "auto" => {}
            _ => return Err(format!("`cursor: {v}`: `pointer` makes it clickable; nothing else is read")),
        },
        "pointer-events" => match v {
            "none" => style.sense = crate::style::Sense::NONE,
            "auto" => {}
            _ => return Err(format!("`{v}` is not a value of pointer-events")),
        },
        // --- text -----------------------------------------------------
        "color" => style.text.color = color(v)?,
        "font-size" => {
            style.text.size = match len(v, cx)? {
                Len::Px(p) => p,
                Len::Percent(p) => style.text.size * p,
                Len::Auto => return Err("`font-size: auto` means nothing".into()),
            }
        }
        "font-weight" => {
            style.text.weight = match v {
                "normal" => 400,
                "bold" => 700,
                n => n
                    .parse::<u16>()
                    .ok()
                    .filter(|w| (100..=900).contains(w))
                    .ok_or_else(|| format!("`{v}` is not a font weight"))?,
            }
        }
        "font-family" => {
            let first = v.split(',').next().unwrap_or("").trim().trim_matches(['"', '\'']);
            match first {
                "monospace" => style.text.mono = true,
                "sans-serif" | "Inter" | "system-ui" => style.text.mono = false,
                _ => {
                    return Err(format!(
                        "`font-family: {v}`: `sans-serif` (Inter) or `monospace`; a game's own font is not read yet"
                    ))
                }
            }
        }
        "line-height" => {
            if let Ok(n) = number(v) {
                style.text.line_height = n;
                late.line_height_px = None;
            } else {
                late.line_height_px = Some(px(v, cx)?);
            }
        }
        "text-align" => {
            style.text.align = match v {
                "left" | "start" => TextAlign::Left,
                "center" => TextAlign::Center,
                "right" | "end" => TextAlign::Right,
                _ => return Err(format!("`{v}` is not a value of text-align")),
            }
        }
        "white-space" => match v {
            "nowrap" | "pre" => style.text.nowrap = true,
            "normal" => style.text.nowrap = false,
            _ => return Err(format!("`{v}` is not a value of white-space")),
        },
        "transition" | "animation" | "transform" => {
            return Err(format!(
                "`{property}` is not read yet: motion on screens is a card of its own"
            ))
        }
        "z-index" => {
            return Err(
                "`z-index` is not read: later elements draw over earlier ones".to_string(),
            )
        }
        _ => return Err(format!("`{property}` is not a property this UI reads")),
    }
    Ok(())
}

fn split_outside_parens(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut word = String::new();
    for c in v.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if c.is_whitespace() && depth == 0 {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// The properties that pass from an element to what is inside it.
pub fn inherited(property: &str) -> bool {
    matches!(
        property,
        "color"
            | "font-size"
            | "font-weight"
            | "font-family"
            | "line-height"
            | "text-align"
            | "white-space"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(src: &str) -> Sheet {
        let sheet = parse("t.css", src, Pos { line: 1, column: 1 }).unwrap();
        assert_eq!(check(&sheet), []);
        sheet
    }

    fn subject<'a>(tag: &'a str, id: Option<&'a str>, classes: &[&'a str]) -> Subject<'a> {
        Subject {
            tag,
            id,
            classes: classes.to_vec(),
        }
    }

    #[test]
    fn selectors_match_as_css_does() {
        let s = sheet(
            "nav .item { color: red }\n nav > button.big:hover { background: #fff }\n * { gap: 2px } #x, .y { gap: 1px }",
        );
        let nav = subject("nav", None, &[]);
        let div = subject("div", None, &[]);
        let item = subject("span", None, &["item"]);
        let big = subject("button", None, &["big", "red"]);
        let first = &s.rules[0].selectors[0];
        assert!(first.matches(&[nav.clone(), div.clone(), item.clone()]));
        assert!(!first.matches(&[div.clone(), item.clone()]));
        let second = &s.rules[1].selectors[0];
        assert_eq!(second.state, State::Hover);
        assert!(second.matches(&[nav.clone(), big.clone()]));
        assert!(
            !second.matches(&[nav.clone(), div.clone(), big.clone()]),
            "a child, not a grandchild"
        );
        assert_eq!(second.specificity(), (0, 2, 2));
        assert!(s.rules[2].selectors[0].matches(std::slice::from_ref(&div)));
        assert_eq!(s.rules[3].selectors.len(), 2);
        assert!(s.rules[3].selectors[0].matches(&[subject("p", Some("x"), &[])]));
    }

    #[test]
    fn values_read_into_a_style() {
        let vars = HashMap::from([("accent".to_string(), "#e07a5f".to_string())]);
        let cx = Context {
            viewport: (1000.0, 500.0),
            vars: &vars,
        };
        let mut style = Style::column();
        let mut late = Late::default();
        for (p, v) in [
            ("flex-direction", "row"),
            ("width", "50vw"),
            ("height", "10%"),
            ("padding", "4px 8px"),
            ("background", "var(--accent)"),
            ("border", "2px solid rgba(255, 255, 255, 0.5)"),
            ("font-size", "2rem"),
            ("line-height", "48px"),
            ("text-align", "center"),
        ] {
            apply(&mut style, &mut late, State::Always, p, v, &cx).unwrap();
        }
        late.settle(&mut style);
        assert_eq!(style.layout.flex_direction, taffy::FlexDirection::Row);
        assert_eq!(style.layout.size.width, length(500.0));
        assert_eq!(style.layout.size.height, percent(0.1));
        assert_eq!(style.layout.padding.left, length(8.0));
        assert_eq!(style.layout.padding.top, length(4.0));
        assert_eq!(style.look.background, Color::hex(0xe07a5f));
        assert_eq!(style.look.border, Color::rgba(255, 255, 255, 128));
        assert_eq!(style.look.border_width, 2.0);
        assert_eq!(style.text.size, 32.0);
        assert_eq!(style.text.line_height, 1.5);
        assert_eq!(style.text.align, TextAlign::Center);
    }

    #[test]
    fn what_cannot_be_drawn_is_said_where_it_is_written() {
        // What does not read stops the sheet; what reads but cannot be
        // drawn is `check`'s.
        let problems = parse(
            "hud.css",
            "@media (x) { .c { color: red } }\n.e:first-child { gap: 1px }\n.f { .g { gap: 1px } }",
            Pos { line: 1, column: 1 },
        )
        .unwrap_err();
        let text: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
        assert_eq!(
            text,
            [
                "hud.css:1:1: `@media (x)`: @-rules are not read here (no @media, @keyframes or @import)",
                "hud.css:2:1: `.e:first-child`: `:first-child` is not read here: only `:hover` and `:active`",
                "hud.css:3:9: a block inside a block: nesting is not read here",
            ]
        );
        let sheet = parse(
            "hud.css",
            ".a {\n  color: red;\n  float: left;\n}\n.b:hover { width: 3px }\n.d { background: linear-gradient(red, blue) }",
            Pos { line: 1, column: 1 },
        )
        .unwrap();
        let text: Vec<String> = check(&sheet).iter().map(|p| p.to_string()).collect();
        assert_eq!(
            text[0],
            "hud.css:3:3: `float` is not a property this UI reads"
        );
        assert!(
            text[1].starts_with("hud.css:5:12: `width` on `:hover`"),
            "{text:#?}"
        );
        assert!(
            text[2].starts_with("hud.css:6:6:") && text[2].contains("no gradients"),
            "{text:#?}"
        );
        assert_eq!(text.len(), 3);
    }

    #[test]
    fn colours_read_every_way_css_writes_them() {
        assert_eq!(color("#fff").unwrap(), Color::hex(0xffffff));
        assert_eq!(
            color("#11223380").unwrap(),
            Color::rgba(0x11, 0x22, 0x33, 0x80)
        );
        assert_eq!(
            color("rgb(10 20 30)").unwrap(),
            Color::rgba(10, 20, 30, 255)
        );
        assert_eq!(
            color("rgba(10,20,30,0)").unwrap(),
            Color::rgba(10, 20, 30, 0)
        );
        assert!(color("rebeccapurple").is_err());
    }
}
