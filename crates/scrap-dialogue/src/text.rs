//! A dialogue as text, the way the files are written by hand: a line's
//! entry on one line with its answers one to a line under it, and an edit
//! rewriting only the entries it changed. What the Dialogues window and
//! the agent's dialogue tools both write with — as `graph_text` is for an
//! animator.

use std::collections::BTreeMap;

use crate::dialogue::{Choice, Condition, Dialogue, Line};
use scrap_core::ron_edit::{self as patch, Change};

pub fn quote(s: &str) -> String {
    format!("{s:?}")
}

pub fn condition(c: &Condition) -> String {
    match c {
        Condition::Is(n) => format!("Is({})", quote(n)),
        Condition::Not(n) => format!("Not({})", quote(n)),
        Condition::Var(n, op, v) => format!("Var({}, {op:?}, {v})", quote(n)),
    }
}

pub fn conditions(when: &[Condition]) -> String {
    let inner: Vec<String> = when.iter().map(condition).collect();
    format!("[{}]", inner.join(", "))
}

pub fn names(list: &[String]) -> String {
    let inner: Vec<String> = list.iter().map(|n| quote(n)).collect();
    format!("[{}]", inner.join(", "))
}

pub fn numbers(map: &BTreeMap<String, i64>) -> String {
    let inner: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{}: {v}", quote(k)))
        .collect();
    format!("{{{}}}", inner.join(", "))
}

/// An answer as written in its line: its text, where to, and what differs
/// from the defaults.
pub fn choice_entry(c: &Choice) -> String {
    let mut out = format!("(text: {}, to: {}", quote(&c.text), quote(&c.to));
    if !c.when.is_empty() {
        out += &format!(", when: {}", conditions(&c.when));
    }
    if c.once {
        out += ", once: true";
    }
    if !c.set.is_empty() {
        out += &format!(", set: {}", names(&c.set));
    }
    if !c.add.is_empty() {
        out += &format!(", add: {}", numbers(&c.add));
    }
    if !c.put.is_empty() {
        out += &format!(", put: {}", numbers(&c.put));
    }
    if !c.event.is_empty() {
        out += &format!(", event: {}", quote(&c.event));
    }
    out + ")"
}

/// A line's entry as the files are written by hand: what differs from the
/// defaults on one line, and its answers one to a line under it.
pub fn line_entry(name: &str, l: &Line) -> String {
    let mut fields = Vec::new();
    if !l.speaker.is_empty() {
        fields.push(format!("speaker: {}", quote(&l.speaker)));
    }
    fields.push(format!("text: {}", quote(&l.text)));
    if !l.when.is_empty() {
        fields.push(format!("when: {}", conditions(&l.when)));
    }
    if !l.otherwise.is_empty() {
        fields.push(format!("else: {}", quote(&l.otherwise)));
    }
    if !l.next.is_empty() {
        fields.push(format!("next: {}", quote(&l.next)));
    }
    if !l.set.is_empty() {
        fields.push(format!("set: {}", names(&l.set)));
    }
    if !l.add.is_empty() {
        fields.push(format!("add: {}", numbers(&l.add)));
    }
    if !l.put.is_empty() {
        fields.push(format!("put: {}", numbers(&l.put)));
    }
    if !l.event.is_empty() {
        fields.push(format!("event: {}", quote(&l.event)));
    }
    if !l.choices.is_empty() {
        let mut list = String::from("choices: [\n");
        for c in &l.choices {
            list += &format!("            {},\n", choice_entry(c));
        }
        fields.push(list + "        ]");
    }
    format!("{}: ({})", quote(name), fields.join(", "))
}

/// A whole dialogue written anew, start first, lines by name.
pub fn fresh(d: &Dialogue) -> String {
    let mut out = format!("(\n    start: {},\n    lines: {{\n", quote(&d.start));
    for (name, line) in &d.lines {
        out += &format!("        {},\n", line_entry(name, line));
    }
    out + "    },\n)\n"
}

/// The file `old` with `dialogue` written into it where it differs, or
/// the dialogue written anew when the text is past patching.
pub fn write(old: &str, dialogue: &Dialogue) -> String {
    patched(old, dialogue).unwrap_or_else(|| fresh(dialogue))
}

pub fn patched(old: &str, d: &Dialogue) -> Option<String> {
    let was: Dialogue = ron::from_str(old).ok()?;
    let mut text = old.to_string();
    let open = patch::value_start(&text, "lines")?;
    let found = patch::items(&text, open)?;
    let keys: Vec<String> = found
        .items
        .iter()
        .map(|r| {
            let item = &text[r.clone()];
            ron::from_str::<String>(item.split(':').next().unwrap_or("").trim()).unwrap_or_default()
        })
        .collect();
    let mut changes = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        match d.lines.get(key) {
            None => changes.push(Change::Remove(i)),
            // A key written twice: the last stays, the others go.
            Some(_) if keys[i + 1..].contains(key) => changes.push(Change::Remove(i)),
            Some(l) if was.lines.get(key) != Some(l) => {
                changes.push(Change::Replace(i, line_entry(key, l)))
            }
            Some(_) => {}
        }
    }
    for (key, l) in &d.lines {
        if !keys.contains(key) {
            changes.push(Change::Append(line_entry(key, l)));
        }
    }
    if !changes.is_empty() {
        text = patch::apply(&text, open, &changes)?;
    }
    if was.start != d.start {
        let at = patch::value_start(&text, "start")?;
        let span = patch::value_span(&text, at)?;
        text.replace_range(span, &quote(&d.start));
    }
    let mut check: Dialogue = ron::from_str(&text).ok()?;
    check.name = d.name.clone();
    (check == *d).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogue::Op;

    const FILE: &str = r#"// The captain, at the pier.
(
    start: "hello",
    lines: {
        "hello": (speaker: "@captain", text: "@captain.hello", next: "ask"), // first thing
        // The question.
        "ask": (speaker: "@captain", text: "@captain.ask", choices: [
            (text: "@yes", to: "thanks"),
            (text: "@no", to: "bye"),
        ]),
        "thanks": (text: "@captain.thanks"),
        "bye": (text: "@captain.bye"),
    },
)
"#;

    #[test]
    fn an_edit_touches_only_the_lines_it_changed() {
        let mut d: Dialogue = ron::from_str(FILE).unwrap();
        d.lines.get_mut("ask").unwrap().choices.push(Choice {
            text: "@pay".into(),
            to: "thanks".into(),
            when: vec![Condition::Var("coins".into(), Op::Ge, 3)],
            once: true,
            add: [("coins".to_string(), -3)].into(),
            ..Choice::default()
        });
        let out = write(FILE, &d);
        let added: Vec<&str> = out
            .lines()
            .filter(|l| !FILE.lines().any(|b| b == *l))
            .collect();
        assert_eq!(
            added,
            [
                r#"            (text: "@pay", to: "thanks", when: [Var("coins", Ge, 3)], once: true, add: {"coins": -3}),"#
            ],
            "{out}"
        );
        assert!(out.contains("// first thing") && out.contains("// The question."));

        // A new line, a line gone, the start moved.
        let before = out.clone();
        d.lines.insert(
            "later".into(),
            Line {
                speaker: "@captain".into(),
                text: "@captain.later".into(),
                when: vec![Condition::Is("met".into())],
                otherwise: "hello".into(),
                ..Line::default()
            },
        );
        d.lines.remove("bye");
        d.lines.get_mut("ask").unwrap().choices.pop();
        d.lines.get_mut("ask").unwrap().choices[1].to = "later".into();
        d.start = "later".into();
        let out = write(&before, &d);
        assert!(out.starts_with("// The captain, at the pier."), "{out}");
        assert!(out.contains(r#"start: "later","#));
        assert!(
            out.contains(r#"        "later": (speaker: "@captain", text: "@captain.later", when: [Is("met")], else: "hello"),"#),
            "{out}"
        );
        assert!(!out.contains("\"bye\":"));
        assert_eq!(ron::from_str::<Dialogue>(&out).unwrap(), d);
    }

    #[test]
    fn a_line_written_twice_is_written_once_and_past_patching_is_written_anew() {
        let twice = "(start: \"a\", lines: {\"a\": (text: \"1\"), \"a\": (text: \"2\")})";
        let d: Dialogue = ron::from_str(twice).unwrap();
        let out = write(twice, &d);
        assert!(crate::dialogue::named_twice(&out).is_empty(), "{out}");
        assert_eq!(ron::from_str::<Dialogue>(&out).unwrap(), d);
        let fresh_text = write("not ron", &d);
        assert_eq!(ron::from_str::<Dialogue>(&fresh_text).unwrap(), d);
    }
}
