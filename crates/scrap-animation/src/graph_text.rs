//! An animator graph as text, the way the files are written by hand: a
//! state's entry on one line with the transitions leaving it one to a line
//! under it, and an edit rewriting only the entries it changed. What the
//! Animator window and the agent's graph tools both write with.

use crate::animgraph::{Condition, Graph, Layer, LayerBlend, State, Transition, ANY};
use crate::ron_edit::{self as patch, Change};

pub fn number(n: f32) -> String {
    let s = format!("{n:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.into()
    }
}

pub fn quote(s: &str) -> String {
    format!("{s:?}")
}

/// `[(0, "idle"), (2, "walk")]`, as a person writes it.
pub fn pairs(p: &[(f32, String)]) -> String {
    let inner: Vec<String> = p
        .iter()
        .map(|(at, name)| format!("({}, {})", number(*at), quote(name)))
        .collect();
    format!("[{}]", inner.join(", "))
}

pub fn read_pairs(text: &str) -> Result<Vec<(f32, String)>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    ron::from_str(text).map_err(|e| e.to_string())
}

pub fn condition(c: &Condition) -> String {
    match c {
        Condition::Above(p, v) => format!("Above({}, {})", quote(p), number(*v)),
        Condition::Below(p, v) => format!("Below({}, {})", quote(p), number(*v)),
        Condition::Is(p) => format!("Is({})", quote(p)),
        Condition::Not(p) => format!("Not({})", quote(p)),
        Condition::Trigger(p) => format!("Trigger({})", quote(p)),
        Condition::Finished => "Finished".into(),
    }
}

pub fn conditions(when: &[Condition]) -> String {
    let inner: Vec<String> = when.iter().map(condition).collect();
    format!("[{}]", inner.join(", "))
}

/// A state's entry as the files are written by hand: what differs from
/// the defaults, on one line, and the transitions leaving it one to a line
/// under it.
pub fn state_entry(name: &str, s: &State, exits: &[&Transition]) -> String {
    state_entry_at(name, s, exits, "        ")
}

/// [`state_entry`] for a state whose entry starts at `indent`.
pub fn state_entry_at(name: &str, s: &State, exits: &[&Transition], indent: &str) -> String {
    let mut fields = Vec::new();
    if (s.blend.is_empty() && s.directional.is_empty()) || !s.clip.is_empty() {
        fields.push(format!("clip: {}", quote(&s.clip)));
    }
    if !s.blend_by.is_empty() {
        fields.push(format!("blend_by: {}", quote(&s.blend_by)));
    }
    if !s.blend.is_empty() {
        fields.push(format!("blend: {}", pairs(&s.blend)));
    }
    if !s.blend_by_y.is_empty() {
        fields.push(format!("blend_by_y: {}", quote(&s.blend_by_y)));
    }
    if !s.directional.is_empty() {
        let points: Vec<String> = s
            .directional
            .iter()
            .map(|(x, y, clip)| format!("({}, {}, {})", number(*x), number(*y), quote(clip)))
            .collect();
        fields.push(format!("directional: [{}]", points.join(", ")));
    }
    if !s.events.is_empty() {
        fields.push(format!("events: {}", pairs(&s.events)));
    }
    if !s.looping {
        fields.push("looping: false".into());
    }
    if s.speed != 1.0 {
        fields.push(format!("speed: {}", number(s.speed)));
    }
    if let Some(p) = &s.speed_from {
        fields.push(format!("speed_from: {}", quote(p)));
    }
    if let Some(p) = &s.time_from {
        fields.push(format!("time_from: {}", quote(p)));
    }
    if !exits.is_empty() {
        fields.push(format!("transitions: {}", exit_list(exits, indent)));
    }
    format!("{}: ({})", quote(name), fields.join(", "))
}

/// A transition as written in the state it leaves: where to, when, and a
/// fade only when it is not the usual one.
pub fn exit_entry(t: &Transition) -> String {
    let mut out = format!("(to: {}", quote(&t.to));
    if !t.when.is_empty() {
        out += &format!(", when: {}", conditions(&t.when));
    }
    if t.fade != 0.2 {
        out += &format!(", fade: {}", number(t.fade));
    }
    out + ")"
}

/// Transitions one to a line, the list closing at `indent`.
pub fn exit_list(exits: &[&Transition], indent: &str) -> String {
    let mut out = String::from("[\n");
    for t in exits {
        out += &format!("{indent}    {},\n", exit_entry(t));
    }
    out + indent + "]"
}

/// A layer's entry in the `layers` list, which stands at four spaces:
/// its name, mask and weight on the first line, then its states one to an
/// entry, as the base graph's are written.
pub fn layer_entry(layer: &Layer) -> String {
    let mut head = vec![format!("name: {}", quote(&layer.name))];
    if !layer.mask.is_empty() {
        let names: Vec<String> = layer.mask.iter().map(|n| quote(n)).collect();
        head.push(format!("mask: [{}]", names.join(", ")));
    }
    if layer.blend == LayerBlend::Additive {
        head.push("blend: Additive".into());
    }
    if layer.weight != 1.0 {
        head.push(format!("weight: {}", number(layer.weight)));
    }
    if let Some(p) = &layer.weight_from {
        head.push(format!("weight_from: {}", quote(p)));
    }
    head.push(format!("start: {}", quote(&layer.graph.start)));
    let mut out = format!("({},\n            states: {{\n", head.join(", "));
    for (name, state) in &layer.graph.states {
        out += &format!(
            "                {},\n",
            state_entry_at(
                name,
                state,
                &leaving(&layer.graph, name),
                "                "
            )
        );
    }
    out += "            },\n";
    let any = leaving(&layer.graph, ANY);
    if !any.is_empty() {
        out += &format!("            any: {},\n", exit_list(&any, "            "));
    }
    out + "        )"
}

/// The whole `layers` list, for a file that has none yet.
pub fn layer_list(layers: &[Layer]) -> String {
    let mut out = String::from("[\n");
    for layer in layers {
        out += &format!("        {},\n", layer_entry(layer));
    }
    out + "    ]"
}

pub fn leaving<'a>(graph: &'a Graph, from: &str) -> Vec<&'a Transition> {
    graph
        .transitions
        .iter()
        .filter(|t| t.from == from)
        .collect()
}

/// The file `old` with `graph` written into it where it differs, or the
/// graph written anew when the text is past patching.
pub fn write(old: &str, graph: &Graph) -> String {
    let mut graph = graph.clone();
    graph.normalize();
    patched(old, &graph).unwrap_or_else(|| {
        let pretty = ron::ser::PrettyConfig::new();
        ron::ser::to_string_pretty(&graph, pretty).unwrap_or_default() + "\n"
    })
}

pub fn patched(old: &str, graph: &Graph) -> Option<String> {
    let was: Graph = ron::from_str(old).ok()?;
    let mut text = old.to_string();
    // A file of the older shape: its one list of every transition goes,
    // and each goes into the state it leaves.
    let older = patch::value_start(&text, "transitions").is_some();
    if older {
        text = patch::set_field(&text, "transitions", None)?;
    }
    let changed = |key: &str| {
        older
            || was.states.get(key) != graph.states.get(key)
            || leaving(&was, key) != leaving(graph, key)
    };
    // States, by name, in the order the file has them; a state's entry
    // holds the transitions that leave it.
    let open = patch::value_start(&text, "states")?;
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
        match graph.states.get(key) {
            None => changes.push(Change::Remove(i)),
            Some(s) if changed(key) => changes.push(Change::Replace(
                i,
                state_entry(key, s, &leaving(graph, key)),
            )),
            Some(_) => {}
        }
    }
    for (key, s) in &graph.states {
        if !keys.contains(key) {
            changes.push(Change::Append(state_entry(key, s, &leaving(graph, key))));
        }
    }
    if !changes.is_empty() {
        text = patch::apply(&text, open, &changes)?;
    }
    // Any State's transitions: the list whole, or gone when there are none.
    if older || leaving(&was, ANY) != leaving(graph, ANY) {
        let any = leaving(graph, ANY);
        let list = (!any.is_empty()).then(|| exit_list(&any, "    "));
        text = patch::set_field(&text, "any", list.as_deref())?;
    }
    if was.start != graph.start {
        let at = patch::value_start(&text, "start")?;
        let span = patch::value_span(&text, at)?;
        text.replace_range(span, &quote(&graph.start));
    }
    if was.layers != graph.layers {
        text = patched_layers(text, &was, graph)?;
    }
    let check: Graph = ron::from_str(&text).ok()?;
    (check == *graph).then_some(text)
}

/// The `layers` list with each changed layer's entry written again, a
/// gone one taken out and a new one added at the end: two people changing
/// different layers change different entries.
fn patched_layers(text: String, was: &Graph, graph: &Graph) -> Option<String> {
    if graph.layers.is_empty() {
        return patch::set_field(&text, "layers", None);
    }
    let Some(open) = patch::value_start(&text, "layers") else {
        return patch::set_field(&text, "layers", Some(&layer_list(&graph.layers)));
    };
    #[derive(serde::Deserialize)]
    struct Named {
        name: String,
    }
    let found = patch::items(&text, open)?;
    let names: Vec<String> = found
        .items
        .iter()
        .map(|r| {
            ron::from_str::<Named>(&text[r.clone()])
                .map(|n| n.name)
                .unwrap_or_default()
        })
        .collect();
    let find = |g: &Graph, name: &str| g.layers.iter().find(|l| l.name == name).cloned();
    let mut changes = Vec::new();
    for (i, name) in names.iter().enumerate() {
        match find(graph, name) {
            None => changes.push(Change::Remove(i)),
            Some(l) if find(was, name).as_ref() != Some(&l) => {
                changes.push(Change::Replace(i, layer_entry(&l)))
            }
            Some(_) => {}
        }
    }
    for layer in &graph.layers {
        if !names.contains(&layer.name) {
            changes.push(Change::Append(layer_entry(layer)));
        }
    }
    patch::apply(&text, open, &changes)
}
