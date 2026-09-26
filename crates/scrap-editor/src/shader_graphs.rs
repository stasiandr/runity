//! The project's shader graphs — `shaders/<name>.graph.ron` (a material's),
//! `<name>.vfx.ron` (a particle effect's) and `<name>.subgraph.ron` — as
//! the editor and an agent both handle them (DNA, postulate 5: what the
//! editor can do, the agent can): listed, read, checked in the compiler's
//! words, told in words, and changed in the file where the change is and
//! nowhere else ([`scrap::shader_graph::text`]). The Studio's Shader Graph
//! window and the MCP server's `shader_graph` tools are both this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use scrap::shader_graph::{effect::EffectGraph, subgraph::SubGraph, Input, Node, ShaderGraph};

/// What a graph file is, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Material,
    Effect,
    Subgraph,
}

impl Kind {
    pub fn of(path: &Path) -> Option<Kind> {
        let f = path.file_name()?.to_str()?;
        if f.ends_with(".subgraph.ron") {
            Some(Kind::Subgraph)
        } else if f.ends_with(".vfx.ron") {
            Some(Kind::Effect)
        } else if f.ends_with(".graph.ron") {
            Some(Kind::Material)
        } else {
            None
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Kind::Material => "material",
            Kind::Effect => "effect",
            Kind::Subgraph => "subgraph",
        }
    }
}

/// A graph file, read.
#[derive(Debug, Clone)]
pub enum Doc {
    Material(ShaderGraph),
    Effect(EffectGraph),
    Subgraph(SubGraph),
}

impl Doc {
    pub fn parse(kind: Kind, text: &str) -> Result<Doc, String> {
        match kind {
            Kind::Subgraph => scrap::shader_graph::subgraph::parse(text).map(Doc::Subgraph),
            Kind::Effect => scrap::shader_graph::effect::parse(text).map(Doc::Effect),
            Kind::Material => scrap::shader_graph::surface::parse(text).map(Doc::Material),
        }
    }

    pub fn load(path: &Path) -> Result<Doc, String> {
        let kind =
            Kind::of(path).ok_or_else(|| format!("{} is not a shader graph", path.display()))?;
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Doc::parse(kind, &text)
    }

    pub fn nodes(&self) -> &BTreeMap<String, Node> {
        match self {
            Doc::Material(g) => &g.nodes,
            Doc::Effect(g) => &g.nodes,
            Doc::Subgraph(g) => &g.nodes,
        }
    }

    /// What the graph sets, each part with its fields and what it reads.
    pub fn outputs(&self) -> Vec<(&'static str, Vec<(String, &Input)>)> {
        fn some<'a>(v: Vec<(&str, &'a Option<Input>)>) -> Vec<(String, &'a Input)> {
            v.into_iter()
                .filter_map(|(k, i)| i.as_ref().map(|i| (k.to_string(), i)))
                .collect()
        }
        match self {
            Doc::Material(g) => {
                let s = &g.surface;
                let mut out = vec![(
                    "surface",
                    some(vec![
                        ("albedo", &s.albedo),
                        ("alpha", &s.alpha),
                        ("metallic", &s.metallic),
                        ("smoothness", &s.smoothness),
                        ("normal", &s.normal),
                        ("emission", &s.emission),
                        ("clip", &s.clip),
                    ]),
                )];
                let v = some(vec![
                    ("position", &g.vertex.position),
                    ("normal", &g.vertex.normal),
                ]);
                if !v.is_empty() {
                    out.push(("vertex", v));
                }
                out
            }
            Doc::Effect(g) => vec![
                (
                    "spawn",
                    some(vec![
                        ("position", &g.spawn.position),
                        ("velocity", &g.spawn.velocity),
                        ("life", &g.spawn.life),
                    ]),
                ),
                (
                    "update",
                    some(vec![
                        ("velocity", &g.update.velocity),
                        ("position", &g.update.position),
                    ]),
                ),
                (
                    "output",
                    some(vec![
                        ("color", &g.output.color),
                        ("alpha", &g.output.alpha),
                        ("size", &g.output.size),
                    ]),
                ),
            ],
            Doc::Subgraph(g) => vec![(
                "outputs",
                g.outputs.iter().map(|(k, v)| (k.clone(), v)).collect(),
            )],
        }
    }

    /// Every read of one node by another, or by what the graph sets: (the
    /// node read, the reader).
    pub fn edges(&self) -> Vec<(String, String)> {
        let nodes = self.nodes();
        let mut edges = Vec::new();
        for (name, node) in nodes {
            for (_, input) in node.inputs() {
                if let Some(b) = base(input).filter(|b| nodes.contains_key(*b)) {
                    edges.push((b.to_string(), name.clone()));
                }
            }
        }
        for (out, reads) in self.outputs() {
            for (_, input) in reads {
                if let Some(b) = base(input).filter(|b| nodes.contains_key(*b)) {
                    edges.push((b.to_string(), out.to_string()));
                }
            }
        }
        edges.sort();
        edges.dedup();
        edges
    }
}

/// An input as it is written in the file.
pub fn written(input: &Input) -> String {
    match input {
        Input::Name(s) => format!("\"{s}\""),
        Input::Number(n) => format!("{n:?}"),
        Input::Vector(v) => format!(
            "({})",
            v.iter()
                .map(|n| format!("{n:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The node a read names, without its swizzle.
pub fn base(input: &Input) -> Option<&str> {
    match input {
        Input::Name(s) => Some(scrap::shader_graph::expr::split(s).0),
        _ => None,
    }
}

/// The graph files in a project, wherever they lie (docs/layout.md), by
/// path.
pub fn list(root: &Path) -> Vec<PathBuf> {
    scrap::layout::files(root, scrap::layout::Kind::Shader)
        .into_iter()
        .filter(|p| Kind::of(p).is_some())
        .collect()
}

/// A graph named as a person would — `lava`, `embers.vfx`, `cracks.subgraph`,
/// `lava.graph.ron`, or `shaders/lava.graph.ron` — as its file, when there
/// is one; else the names there are.
pub fn find(root: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim().trim_start_matches("shaders/");
    let tries = [
        name.to_string(),
        format!("{name}.ron"),
        format!("{name}.graph.ron"),
    ];
    // A path from the root, or a file's own name wherever it lies.
    let all = list(root);
    for t in &tries {
        let p = root.join(t);
        if Kind::of(&p).is_some() && p.is_file() {
            return Ok(p);
        }
        if let Some(p) = all.iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy() == *t)) {
            return Ok(p.clone());
        }
    }
    let names: Vec<String> = all
        .iter()
        .filter_map(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
        .collect();
    // Named as a person names them: `lava`, `embers.vfx`.
    let short: Vec<&str> = names
        .iter()
        .map(|n| {
            n.strip_suffix(".graph.ron")
                .or_else(|| n.strip_suffix(".ron"))
                .unwrap_or(n)
        })
        .collect();
    let near = scrap::spelling::closest(name, short.iter().copied())
        .map(|n| format!(" — did you mean `{n}`?"))
        .unwrap_or_default();
    Err(format!(
        "no graph `{name}` in the project{near} (there are: {})",
        short.join(", ")
    ))
}

/// Whether a graph builds, and what in it is likely a mistake.
#[derive(Debug, Clone)]
pub struct Checked {
    /// `Err` in the compiler's words.
    pub built: Result<(), String>,
    pub problems: Vec<String>,
}

impl Checked {
    /// The node the compiler's words name, if any: a subgraph's node, put
    /// in under its call, is the call.
    pub fn blamed(&self) -> Option<String> {
        let words = self.built.as_ref().err()?;
        let at = words.find("node `")? + 6;
        let end = words[at..].find('`')?;
        let name = &words[at..at + end];
        Some(name.split("__").next().unwrap_or(name).to_string())
    }
}

/// Build the graph at `path` as the renderer would, without a GPU.
pub fn check(path: &Path, doc: &Doc) -> Checked {
    let library = scrap::render::subgraphs_beside(path);
    match doc {
        Doc::Material(g) => Checked {
            problems: scrap::shader_graph::surface::problems_with(g, &library),
            built: scrap::render::material_shader_source(path)
                .and_then(|s| scrap::render::check_material_shader(&s)),
        },
        Doc::Effect(g) => Checked {
            problems: scrap::shader_graph::effect::problems_with(g, &library),
            built: scrap::render::effect_source(path)
                .and_then(|s| scrap::particles_gpu::check_effect(&s)),
        },
        // A subgraph builds as the graphs that call it do: those, checked.
        Doc::Subgraph(_) => {
            let stem = path
                .file_name()
                .and_then(|f| f.to_str())
                .and_then(|f| f.strip_suffix(".subgraph.ron"))
                .unwrap_or_default()
                .to_string();
            let root = path.parent().and_then(Path::parent);
            let mut built = Ok(());
            for caller in root.map(list).unwrap_or_default() {
                let Ok(d) = Doc::load(&caller) else { continue };
                let calls = d
                    .nodes()
                    .values()
                    .any(|n| matches!(n, Node::Subgraph { name, .. } if *name == stem));
                if calls && !matches!(d, Doc::Subgraph(_)) {
                    if let Err(e) = check(&caller, &d).built {
                        let f = caller
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        built = Err(format!("{f}: {e}"));
                        break;
                    }
                }
            }
            Checked {
                built,
                problems: Vec::new(),
            }
        }
    }
}

/// A change to one graph file.
#[derive(Debug, Clone)]
pub enum Edit {
    /// Input `field` of node `node` set to `value`, as RON: `"noise.x"`,
    /// `0.5`, `(1.0, 0.2, 0.0)`.
    Set {
        node: String,
        field: String,
        value: String,
    },
    /// A node added: `kind` as it is written, `Sine(of: "time")`.
    Add {
        node: String,
        kind: String,
    },
    Remove {
        node: String,
    },
    /// `field` of what the graph sets — `surface`, `vertex`, `spawn`,
    /// `update`, `output` — set to `value`.
    SetPart {
        part: String,
        field: String,
        value: String,
    },
}

impl Edit {
    /// Node `from` read by `to` as its `field`: a node's input, or a field
    /// of what the graph sets when `to` is one of its parts.
    pub fn connect(doc: &Doc, from: &str, to: &str, field: &str) -> Edit {
        let value = format!("\"{from}\"");
        if doc.nodes().contains_key(to) {
            Edit::Set {
                node: to.into(),
                field: field.into(),
                value,
            }
        } else {
            Edit::SetPart {
                part: to.into(),
                field: field.into(),
                value,
            }
        }
    }
}

/// What `to` can read another node as: a node's inputs, or the fields of
/// one of the parts the graph sets.
pub fn fields_of(doc: &Doc, to: &str) -> Vec<&'static str> {
    if let Some(node) = doc.nodes().get(to) {
        return node
            .inputs()
            .into_iter()
            .map(|(f, _)| f)
            .filter(|f| *f != "in")
            .collect();
    }
    match (doc, to) {
        (Doc::Material(_), "surface") => vec![
            "albedo",
            "alpha",
            "metallic",
            "smoothness",
            "normal",
            "emission",
            "clip",
        ],
        (Doc::Material(_), "vertex") => vec!["position", "normal"],
        (Doc::Effect(_), "spawn") => vec!["position", "velocity", "life"],
        (Doc::Effect(_), "update") => vec!["velocity", "position"],
        (Doc::Effect(_), "output") => vec!["color", "alpha", "size"],
        _ => Vec::new(),
    }
}

/// Write `edit` into the graph at `path` where it goes, the rest of the file
/// as it was — when the file still reads as a graph of its kind after it.
/// The graph, read again.
pub fn apply(path: &Path, edit: &Edit) -> Result<Doc, String> {
    use scrap::shader_graph::text;
    let kind = Kind::of(path).ok_or_else(|| format!("{} is not a shader graph", path.display()))?;
    let old = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc = Doc::parse(kind, &old)?;
    let known = |node: &str| -> Result<(), String> {
        if doc.nodes().contains_key(node) {
            return Ok(());
        }
        let near = scrap::spelling::closest(node, doc.nodes().keys().map(String::as_str))
            .map(|n| format!(" — did you mean `{n}`?"))
            .unwrap_or_default();
        Err(format!("no node `{node}`{near}"))
    };
    let new = match edit {
        Edit::Set { node, field, value } => {
            known(node)?;
            text::set_input(&old, node, field, value.trim())
        }
        Edit::Add { node, kind } => {
            if doc.nodes().contains_key(node) {
                return Err(format!("`{node}` is already a node: a name is one node's"));
            }
            text::add_node(&old, node, kind.trim())
        }
        Edit::Remove { node } => {
            known(node)?;
            text::remove_node(&old, node)
        }
        Edit::SetPart { part, field, value } => {
            let parts: &[&str] = match kind {
                Kind::Material => &["surface", "vertex"],
                Kind::Effect => &["spawn", "update", "output"],
                Kind::Subgraph => &[],
            };
            if !parts.contains(&part.as_str()) {
                return Err(format!(
                    "a {} graph sets {}, not `{part}`",
                    kind.word(),
                    parts.join(", ")
                ));
            }
            text::set_part(&old, part, field, value.trim())
        }
    }
    .ok_or_else(|| format!("{}: could not find where that goes", path.display()))?;
    let read =
        Doc::parse(kind, &new).map_err(|e| format!("not written, it would not read: {e}"))?;
    std::fs::write(path, new).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(read)
}

/// The graph told in words, for an agent: what it is, its properties, every
/// node with its inputs, what it sets, whether it builds, and its warnings.
pub fn describe(path: &Path, doc: &Doc) -> String {
    let kind = Kind::of(path).map_or("graph", Kind::word);
    let mut out = format!(
        "{} ({kind})\n",
        path.file_name()
            .map(|f| f.to_string_lossy())
            .unwrap_or_default()
    );
    let params = |p: &[scrap::shader_graph::Param]| -> String {
        p.iter()
            .map(|p| format!("{} {:?}", p.name(), p.kind()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match doc {
        Doc::Material(g) => {
            if !g.params.is_empty() {
                out.push_str(&format!("params: {}\n", params(&g.params)));
            }
            if !g.textures.is_empty() {
                out.push_str(&format!("textures: {}\n", g.textures.join(", ")));
            }
        }
        Doc::Effect(g) => {
            if !g.params.is_empty() {
                out.push_str(&format!("params (the emitter's `params`): {}\n", params(&g.params)));
            }
        }
        Doc::Subgraph(g) => {
            for (name, input) in &g.inputs {
                out.push_str(&format!("input {name} = {}\n", written(input)));
            }
        }
    }
    for (name, node) in doc.nodes() {
        let inputs: Vec<String> = node
            .inputs()
            .iter()
            .map(|(k, v)| format!("{k}: {}", written(v)))
            .collect();
        let kind = match node {
            Node::Subgraph { name: sub, inputs } => {
                let given: Vec<String> = inputs
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", written(v)))
                    .collect();
                format!("Subgraph {sub} {{{}}}", given.join(", "))
            }
            n => format!("{}({})", n.kind(), inputs.join(", ")),
        };
        out.push_str(&format!("node {name}: {kind}\n"));
    }
    for (part, fields) in doc.outputs() {
        let f: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("{k}: {}", written(v)))
            .collect();
        out.push_str(&format!("{part}: {}\n", f.join(", ")));
    }
    let checked = check(path, doc);
    match &checked.built {
        Ok(()) => out.push_str("builds\n"),
        Err(e) => out.push_str(&format!("does not build: {e}\n")),
    }
    for p in &checked.problems {
        out.push_str(&format!("warning: {p}\n"));
    }
    out
}
