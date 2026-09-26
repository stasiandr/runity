//! A particle effect's graph: `shaders/<name>.vfx.ron`, compiled to the
//! three functions the GPU's particles run — where each is born and how it
//! leaves, how it moves each step, and how it looks — so an emitter on the
//! GPU names it (`particles: (gpu: true, graph: "<name>")`) and its
//! particles do what the graph says. Unity's VFX Graph, its Spawn,
//! Initialize, Update and Output contexts: the spawn — how many and when —
//! stays the emitter's (`rate`, `bursts`, `once`).
//!
//! ```ron
//! (
//!     nodes: {
//!         "wind": Turbulence(at: "position", scale: 0.7),
//!         "push": Multiply(a: "wind", b: 3.0),
//!         "drag": Multiply(a: "velocity", b: -0.8),
//!         "pull": Add(a: "push", b: "drag"),
//!         "step": Multiply(a: "pull", b: "dt"),
//!         "moved": Add(a: "velocity", b: "step"),
//!         "hue": Random(low: (1.0, 0.3, 0.05), high: (1.0, 0.8, 0.3)),
//!     },
//!     spawn: (velocity: "cone"),
//!     update: (velocity: "moved"),
//!     output: (color: "hue", size: 0.05),
//! )
//! ```
//!
//! What each part can read, besides the nodes:
//!
//! * everywhere: `time` (seconds the emitter has run), `seed` (a number
//!   from 0 to 1 of the particle's own), `center` (where the emitter is),
//!   the emitter's numbers — `speed`, `gravity` (a vec3), `life`, `size`,
//!   `end_size`, `color`, `end_color`, `alpha`, `end_alpha` — and the
//!   graph's own `params` by name, from the emitter's eight `params` as a
//!   material's are (Unity's exposed properties);
//! * `spawn`: `origin` (where it is born by the emitter's shape — its
//!   `from`, `box_size`, `radius`) and `cone` (a way out within its
//!   `spread_deg` of its direction, length 1);
//! * `update` and `output`: `position`, `velocity`, `age` (seconds),
//!   `t` (its age over its life, 0 to 1), and `custom`, a vec3 of the
//!   particle's own that `spawn` gives it and `update` changes (Unity's
//!   custom attributes: a colour picked at birth, a heat that cools);
//!   `update` also `dt`, the step's seconds.
//!
//! `output: (frame: …)` picks the frame of the emitter's `sheet` — the
//! picture of its `material`, frames across and down — by number: without
//! it, the emitter's `frames` over each one's life.
//!
//! What is left out does what an emitter without a graph does: born at
//! `origin`, leaving along `cone` at `speed`, living `life`; falling by
//! `gravity`; `color` to `end_color`, `alpha` to `end_alpha`, `size` to
//! `end_size` over its life. Positions are the emitter's own when it is
//! `local`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::expr::{self, Context, Input, Node, Ty, Value};

/// The file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectGraph {
    /// Its properties, in order, from the emitter's eight `params`: a
    /// name alone is one number, `("tint", Color)` three.
    #[serde(default)]
    pub params: Vec<crate::surface::Param>,
    #[serde(default)]
    pub nodes: BTreeMap<String, Node>,
    /// A new one: where, how fast, for how long.
    #[serde(default)]
    pub spawn: Spawn,
    /// Each step of a living one.
    #[serde(default)]
    pub update: Update,
    /// How a living one looks this frame.
    #[serde(default)]
    pub output: Output,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    #[serde(default)]
    pub position: Option<Input>,
    #[serde(default)]
    pub velocity: Option<Input>,
    /// Seconds it lives.
    #[serde(default)]
    pub life: Option<Input>,
    /// Its own vec3, from birth: zero when not said.
    #[serde(default)]
    pub custom: Option<Input>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    /// Its velocity after this step.
    #[serde(default)]
    pub velocity: Option<Input>,
    /// Where it is after this step; left out, it moves by its velocity.
    #[serde(default)]
    pub position: Option<Input>,
    /// Its own vec3 after this step; left out, it keeps it.
    #[serde(default)]
    pub custom: Option<Input>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    #[serde(default)]
    pub color: Option<Input>,
    #[serde(default)]
    pub alpha: Option<Input>,
    /// Metres across.
    #[serde(default)]
    pub size: Option<Input>,
    /// Which frame of the emitter's sheet, by number from 0.
    #[serde(default)]
    pub frame: Option<Input>,
}

/// What a file of this name is called as an effect: `sparks.vfx.ron` is
/// `sparks`. `None` for any other file.
pub fn effect_name(file_name: &str) -> Option<&str> {
    file_name.strip_suffix(".vfx.ron").filter(|n| !n.is_empty())
}

/// Read a graph from its text; the error says where.
pub fn parse(text: &str) -> Result<EffectGraph, String> {
    expr::from_ron(text).map_err(|e| e.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Spawn,
    Update,
    Output,
}

impl Part {
    fn said(self) -> &'static str {
        match self {
            Part::Spawn => "spawn",
            Part::Update => "update",
            Part::Output => "output",
        }
    }
}

/// Everywhere: the emitter's numbers, the clock, the particle's own seed.
const EVERYWHERE: [(&str, &str, Ty); 13] = [
    ("time", "e.time", Ty::F1),
    ("center", "e.center", Ty::F3),
    ("seed", "e.seed", Ty::F1),
    ("speed", "params.motion.x", Ty::F1),
    ("gravity", "vec3<f32>(0.0, params.motion.z, 0.0)", Ty::F3),
    ("life", "e.life", Ty::F1),
    ("size", "params.shape.x", Ty::F1),
    ("end_size", "params.shape.y", Ty::F1),
    ("color", "params.color.rgb", Ty::F3),
    ("end_color", "params.end_color.rgb", Ty::F3),
    ("alpha", "params.color.a", Ty::F1),
    ("end_alpha", "params.end_color.a", Ty::F1),
    ("t", "e.t", Ty::F1),
];
const BORN: [(&str, &str, Ty); 2] = [("origin", "e.origin", Ty::F3), ("cone", "e.cone", Ty::F3)];
const LIVING: [(&str, &str, Ty); 4] = [
    ("position", "e.position", Ty::F3),
    ("velocity", "e.velocity", Ty::F3),
    ("age", "e.age", Ty::F1),
    ("custom", "e.custom", Ty::F3),
];

/// The emitter's eight numbers, in the particles' shader.
const NUMBERS: &str = "params.values";

struct Particles<'a> {
    part: Part,
    params: &'a [crate::surface::Param],
}

impl Particles<'_> {
    fn all(&self) -> Vec<(&'static str, &'static str, Ty)> {
        let mut out: Vec<_> = EVERYWHERE.to_vec();
        match self.part {
            Part::Spawn => {
                // Not born yet: no age, and its life is what is being said.
                out.retain(|(n, _, _)| *n != "t");
                out.retain(|(n, _, _)| *n != "life");
                out.push(("life", "params.motion.w", Ty::F1));
                out.extend(BORN);
            }
            Part::Update => {
                out.extend(LIVING);
                out.push(("dt", "e.dt", Ty::F1));
            }
            Part::Output => out.extend(LIVING),
        }
        out
    }
}

impl Context for Particles<'_> {
    fn builtin(&self, name: &str) -> Option<Value> {
        self.all()
            .into_iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, code, ty)| Value::new(code, ty))
            .or_else(|| crate::surface::param_value_of(self.params, name, NUMBERS))
    }

    fn builtins(&self) -> Vec<String> {
        self.all()
            .into_iter()
            .map(|(n, _, _)| n.to_string())
            .chain(self.params.iter().map(|p| p.name().to_string()))
            .collect()
    }

    fn random(&self, salt: u32, ty: Ty) -> Result<Value, String> {
        let one = |k: usize| {
            format!(
                "sg_random(e.seed, {}u)",
                salt.wrapping_add((k as u32).wrapping_mul(0x9e37_79b9))
            )
        };
        let code = match ty {
            Ty::F1 => one(0),
            _ => format!(
                "{}({})",
                ty.wgsl(),
                (0..ty.size()).map(one).collect::<Vec<_>>().join(", ")
            ),
        };
        Ok(Value::new(code, ty))
    }
}

/// Always there: the particle's own random numbers, from its seed.
const RANDOM: &str = "fn sg_random(seed: f32, salt: u32) -> f32 {
    var x = bitcast<u32>(seed) ^ salt;
    x = x * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x) / 4294967295.0;
}
";

/// The graph as the GPU particles' three functions: `effect_spawn`,
/// `effect_update`, `effect_output`, and the helpers they need. `from`
/// names the file, in the first comment.
pub fn to_wgsl(graph: &EffectGraph, from: &str) -> Result<String, String> {
    to_wgsl_with(graph, from, &crate::subgraph::NoSubgraphs)
}

/// The graph with its subgraphs put in ([`crate::subgraph::expand`]).
fn expanded(
    graph: &EffectGraph,
    library: &dyn crate::subgraph::Library,
) -> Result<EffectGraph, String> {
    let mut out = graph.clone();
    let mut outputs: Vec<&mut Input> = [
        &mut out.spawn.position,
        &mut out.spawn.velocity,
        &mut out.spawn.life,
        &mut out.spawn.custom,
        &mut out.update.velocity,
        &mut out.update.position,
        &mut out.update.custom,
        &mut out.output.color,
        &mut out.output.alpha,
        &mut out.output.size,
        &mut out.output.frame,
    ]
    .into_iter()
    .filter_map(Option::as_mut)
    .collect();
    out.nodes = crate::subgraph::expand(&graph.nodes, &mut outputs, library)?;
    Ok(out)
}

/// [`to_wgsl`], its `Subgraph` nodes found in `library`.
pub fn to_wgsl_with(
    graph: &EffectGraph,
    from: &str,
    library: &dyn crate::subgraph::Library,
) -> Result<String, String> {
    shape(graph)?;
    let graph = &expanded(graph, library)?;
    let mut helpers: Vec<String> = Vec::new();
    let mut functions = String::new();
    for part in [Part::Spawn, Part::Update, Part::Output] {
        let fields = fields(graph, part);
        let wanted: Vec<(String, &Input)> = fields
            .iter()
            .map(|(n, i, _)| (format!("{} `{n}`", part.said()), *i))
            .collect();
        let compiled = expr::compile(&graph.nodes, &wanted, &Particles { part, params: &graph.params })?;
        for h in compiled.helpers.split_inclusive("\n}\n") {
            if !h.trim().is_empty() && !helpers.iter().any(|x| x == h) {
                helpers.push(h.to_string());
            }
        }
        let mut set = String::new();
        for ((name, _, ty), value) in fields.iter().zip(&compiled.outputs) {
            let code = spread(value, *ty).map_err(|e| format!("{} `{name}` {e}", part.said()))?;
            set.push_str(&format!("    o.{name} = {code};\n"));
        }
        let (head, start) = match part {
            Part::Spawn => (
                "fn effect_spawn(e: Effect) -> Born {\n",
                "    var o = born_default(e);\n",
            ),
            Part::Update => (
                "fn effect_update(e: Effect) -> Moved {\n",
                "    var o = moved_default(e);\n",
            ),
            Part::Output => (
                "fn effect_output(e: Effect) -> Looks {\n",
                "    var o = looks_default(e);\n",
            ),
        };
        functions.push_str(head);
        functions.push_str(&compiled.body);
        functions.push_str(start);
        functions.push_str(&set);
        // A position left out follows the velocity the graph set.
        if part == Part::Update && graph.update.position.is_none() {
            functions.push_str("    o.position = e.position + o.velocity * e.dt;\n");
        }
        functions.push_str("    return o;\n}\n");
    }
    let mut out = format!("// Made from {from} by the effect graph: edit that, not this.\n");
    out.push_str(RANDOM);
    for h in helpers {
        out.push_str(&h);
    }
    out.push_str(&functions);
    Ok(out)
}

/// What a graph is fine with but is likely a mistake: nodes nothing reads.
pub fn problems(graph: &EffectGraph) -> Vec<String> {
    problems_with(graph, &crate::subgraph::NoSubgraphs)
}

/// [`problems`], its `Subgraph` nodes found in `library`: said of the
/// nodes as written.
pub fn problems_with(graph: &EffectGraph, library: &dyn crate::subgraph::Library) -> Vec<String> {
    let Ok(expanded) = expanded(graph, library) else {
        return Vec::new();
    };
    let original = graph;
    let graph = &expanded;
    let mut unused: Option<Vec<String>> = None;
    for part in [Part::Spawn, Part::Update, Part::Output] {
        let fields = fields(graph, part);
        let wanted: Vec<(String, &Input)> =
            fields.iter().map(|(n, i, _)| (n.to_string(), *i)).collect();
        let Ok(compiled) = expr::compile(&graph.nodes, &wanted, &Particles { part, params: &graph.params }) else {
            return Vec::new();
        };
        unused = Some(match unused {
            None => compiled.unused,
            Some(before) => before
                .into_iter()
                .filter(|n| compiled.unused.contains(n))
                .collect(),
        });
    }
    let unused = unused.unwrap_or_default();
    original
        .nodes
        .iter()
        .filter(|(n, node)| match node {
            Node::Subgraph { .. } => {
                let outs: Vec<&String> = graph
                    .nodes
                    .keys()
                    .filter(|k| k.starts_with(&format!("{n}__out__")))
                    .collect();
                !outs.is_empty() && outs.iter().all(|k| unused.contains(k))
            }
            _ => unused.contains(n),
        })
        .map(|(n, _)| format!("node `{n}` is read by nothing the effect sets"))
        .collect()
}

fn fields(graph: &EffectGraph, part: Part) -> Vec<(&'static str, &Input, Ty)> {
    let all: Vec<(&'static str, &Option<Input>, Ty)> = match part {
        Part::Spawn => vec![
            ("position", &graph.spawn.position, Ty::F3),
            ("velocity", &graph.spawn.velocity, Ty::F3),
            ("life", &graph.spawn.life, Ty::F1),
            ("custom", &graph.spawn.custom, Ty::F3),
        ],
        Part::Update => vec![
            ("velocity", &graph.update.velocity, Ty::F3),
            ("position", &graph.update.position, Ty::F3),
            ("custom", &graph.update.custom, Ty::F3),
        ],
        Part::Output => vec![
            ("color", &graph.output.color, Ty::F3),
            ("alpha", &graph.output.alpha, Ty::F1),
            ("size", &graph.output.size, Ty::F1),
            ("frame", &graph.output.frame, Ty::F1),
        ],
    };
    all.into_iter()
        .filter_map(|(n, i, t)| i.as_ref().map(|i| (n, i, t)))
        .collect()
}

fn shape(graph: &EffectGraph) -> Result<(), String> {
    let numbers: usize = graph.params.iter().map(|p| p.kind().size()).sum();
    if numbers > 8 {
        return Err(format!(
            "an emitter hands its effect 8 numbers, and `params` takes {numbers} ({})",
            crate::surface::slot_names_of(&graph.params).join(" ")
        ));
    }
    let mut reserved: Vec<&str> = EVERYWHERE
        .iter()
        .chain(&BORN)
        .chain(&LIVING)
        .map(|(n, _, _)| *n)
        .collect();
    reserved.push("dt");
    let mut seen: Vec<&str> = Vec::new();
    for p in &graph.params {
        let name = p.name();
        if name.is_empty() || name.contains(['.', ' ']) {
            return Err(format!("`{name}` cannot name a parameter: no dots or spaces"));
        }
        if reserved.contains(&name) || seen.contains(&name) || graph.nodes.contains_key(name) {
            return Err(format!("parameter `{name}` has the name of an input or node it would hide — call it something else"));
        }
        seen.push(name);
    }
    for name in graph.nodes.keys() {
        if name.is_empty() || name.contains(['.', ' ']) {
            return Err(format!("`{name}` cannot name a node: no dots or spaces"));
        }
        if reserved.contains(&name.as_str()) {
            return Err(format!(
                "node `{name}` has the name of an input it would hide — call it something else"
            ));
        }
    }
    Ok(())
}

fn spread(value: &Value, ty: Ty) -> Result<String, String> {
    if value.ty == ty {
        Ok(value.code.clone())
    } else if value.ty == Ty::F1 {
        Ok(format!("{}({})", ty.wgsl(), value.code))
    } else {
        Err(format!(
            "is {} and has to be {}{}",
            value.ty.said(),
            ty.said(),
            if ty == Ty::F3 && value.ty == Ty::F4 {
                " — take `.rgb`"
            } else {
                ""
            }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_graph_is_the_emitter_as_it_is() {
        let wgsl = to_wgsl(&parse("()").unwrap(), "x").unwrap();
        for f in [
            "fn effect_spawn",
            "fn effect_update",
            "fn effect_output",
            "born_default(e)",
            "o.position = e.position + o.velocity * e.dt",
        ] {
            assert!(wgsl.contains(f), "{f}: {wgsl}");
        }
    }

    #[test]
    fn each_part_reads_only_what_it_has_and_says_which_part() {
        let g = parse(r#"(nodes: { "old": Multiply(a: "age", b: 2.0) }, spawn: (life: "old"))"#)
            .unwrap();
        let e = to_wgsl(&g, "x").unwrap_err();
        assert!(
            e.contains("node `old`, input `a`") && e.contains("`age`"),
            "{e}"
        );
        let g = parse(r#"(nodes: { "old": Multiply(a: "age", b: 2.0) }, output: (size: "old"))"#)
            .unwrap();
        to_wgsl(&g, "x").unwrap();
        let g = parse(r#"(spawn: (velocity: "cone"), update: (velocity: "cone"))"#).unwrap();
        let e = to_wgsl(&g, "x").unwrap_err();
        assert!(e.starts_with("update `velocity`"), "{e}");
    }

    #[test]
    fn random_is_a_particles_own_and_apart_for_each_node() {
        let g = parse(r#"(nodes: { "a": Random(), "b": Random(low: (0.0, 0.0, 0.0), high: 2.0) }, output: (size: "a", color: "b"))"#).unwrap();
        let wgsl = to_wgsl(&g, "x").unwrap();
        assert!(wgsl.contains("sg_random(e.seed"), "{wgsl}");
        let salts: std::collections::BTreeSet<&str> = wgsl
            .match_indices("sg_random(e.seed, ")
            .map(|(i, _)| &wgsl[i + 18..i + 30])
            .collect();
        assert!(
            salts.len() >= 4,
            "one salt a component and a node: {salts:?}"
        );
        assert!(problems(&g).is_empty());
        let g = parse(r#"(nodes: { "lost": Random() })"#).unwrap();
        assert_eq!(
            problems(&g),
            vec!["node `lost` is read by nothing the effect sets".to_string()]
        );
    }

    #[test]
    fn a_node_may_not_hide_an_input() {
        let g = parse(r#"(nodes: { "velocity": Random() })"#).unwrap();
        assert!(to_wgsl(&g, "x").unwrap_err().contains("would hide"));
    }

    #[test]
    fn what_needs_pixels_is_refused_in_a_particles_graph_and_the_rest_is_there() {
        let g = parse(r#"(nodes: { "soft": Ddx(of: "age") }, output: (size: "soft"))"#).unwrap();
        let e = to_wgsl(&g, "x").unwrap_err();
        assert!(
            e.contains("node `soft` (Ddx)") && e.contains("no pixels"),
            "{e}"
        );
        let g = parse(
            r#"(nodes: {
                "hue": Hue(of: "color", offset: "seed"),
                "warm": Blend(base: "hue", blend: (1.0, 0.5, 0.0), mode: Multiply),
                "fade": InverseLerp(a: 1.0, b: 0.0, of: "t"),
                "late": Comparison(a: "t", b: 0.8, op: Greater),
                "out": Branch(when: "late", yes: 0.0, no: "fade"),
            }, output: (color: "warm", alpha: "out"))"#,
        )
        .unwrap();
        let wgsl = to_wgsl(&g, "x").unwrap();
        assert!(
            wgsl.contains("fn sg_hsv_to_rgb") && wgsl.contains("select("),
            "{wgsl}"
        );
    }
}
