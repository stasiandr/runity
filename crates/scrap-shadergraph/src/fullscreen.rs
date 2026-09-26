//! A fullscreen graph: `shaders/<name>.post.ron`, run over the whole
//! picture once a frame — Unity's Fullscreen Shader Graph at its Before
//! Post Process point: the scene lit and drawn, in linear light, before
//! the tonemapper, bloom and the grade. A scene names it
//! (`fullscreen: (graph: "<name>", params: [...])`) and its picture is what
//! the graph says, pixel by pixel.
//!
//! ```ron
//! (
//!     params: ["strength"],
//!     nodes: {
//!         "grey": Luminance(of: "color"),
//!         "far": Smoothstep(low: 20.0, high: 60.0, of: "depth"),
//!         "faded": Lerp(a: "color", b: "grey", t: "far"),
//!     },
//!     output: (color: "faded"),
//! )
//! ```
//!
//! What it can read, besides its nodes: `screen` (where on the screen, 0
//! to 1 from the top left), `color` (the picture there, linear), `depth`
//! (metres along the view to what is drawn there; the far plane where
//! there is nothing), `time` (seconds), `resolution` (the picture's size in
//! pixels), and its own `params` by name, from the scene's eight numbers as
//! a material's are. `SceneColor(at: …)` reads the picture anywhere else.
//! Left out, `color` is the picture as it was.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::expr::{self, Context, Input, Node, Ty, Value};
use crate::surface::Param;

/// The file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullscreenGraph {
    /// Its properties, in order, from the scene's eight `params`.
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub nodes: BTreeMap<String, Node>,
    #[serde(default)]
    pub output: FullscreenOut,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullscreenOut {
    /// The picture's colour at this pixel, linear.
    #[serde(default)]
    pub color: Option<Input>,
}

/// What a file of this name is called as a fullscreen graph:
/// `grey.post.ron` is `grey`. `None` for any other file.
pub fn fullscreen_name(file_name: &str) -> Option<&str> {
    file_name.strip_suffix(".post.ron").filter(|n| !n.is_empty())
}

/// Read a graph from its text; the error says where.
pub fn parse(text: &str) -> Result<FullscreenGraph, String> {
    expr::from_ron(text).map_err(|e| e.to_string())
}

const BUILTINS: [(&str, &str, Ty); 5] = [
    ("screen", "in.uv", Ty::F2),
    ("color", "fs_color(in.uv)", Ty::F3),
    ("depth", "fs_depth(in.uv)", Ty::F1),
    ("time", "fs.time_size.x", Ty::F1),
    ("resolution", "fs.time_size.yz", Ty::F2),
];

/// The scene's eight numbers, in the fullscreen shader.
const NUMBERS: &str = "fs.values";

struct Screen<'a> {
    params: &'a [Param],
}

impl Context for Screen<'_> {
    fn builtin(&self, name: &str) -> Option<Value> {
        BUILTINS
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, code, ty)| Value::new(*code, *ty))
            .or_else(|| crate::surface::param_value_of(self.params, name, NUMBERS))
    }

    fn builtins(&self) -> Vec<String> {
        BUILTINS
            .iter()
            .map(|(n, _, _)| n.to_string())
            .chain(self.params.iter().map(|p| p.name().to_string()))
            .collect()
    }

    fn derivatives(&self) -> bool {
        true
    }

    fn scene_color(&self, at: &str) -> Result<Value, String> {
        Ok(Value::new(format!("fs_color({at})"), Ty::F3))
    }
}

/// The graph with its subgraphs put in ([`crate::subgraph::expand`]).
fn expanded(graph: &FullscreenGraph, library: &dyn crate::subgraph::Library) -> Result<FullscreenGraph, String> {
    let mut out = graph.clone();
    let mut outputs: Vec<&mut Input> = out.output.color.as_mut().into_iter().collect();
    out.nodes = crate::subgraph::expand(&graph.nodes, &mut outputs, library)?;
    Ok(out)
}

/// What the file says that no graph can be, before its nodes are looked at.
fn shape(graph: &FullscreenGraph) -> Result<(), String> {
    let numbers: usize = graph.params.iter().map(|p| p.kind().size()).sum();
    if numbers > 8 {
        return Err(format!(
            "a scene hands its fullscreen graph 8 numbers, and `params` takes {numbers} ({})",
            crate::surface::slot_names_of(&graph.params).join(" ")
        ));
    }
    let reserved: Vec<&str> = BUILTINS.iter().map(|(n, _, _)| *n).collect();
    let mut seen: Vec<&str> = Vec::new();
    for name in graph.params.iter().map(Param::name).chain(graph.nodes.keys().map(String::as_str)) {
        if name.is_empty() || name.contains(['.', ' ']) {
            return Err(format!("`{name}` cannot name a node or parameter: no dots or spaces"));
        }
        if reserved.contains(&name) || seen.contains(&name) {
            return Err(format!("`{name}` has the name of an input or node it would hide — call it something else"));
        }
        seen.push(name);
    }
    Ok(())
}

/// The graph as the fullscreen pass's `fn fullscreen(in: FsIn) ->
/// vec3<f32>`, and the helpers it needs. `from` names the file, in the
/// first comment.
pub fn to_wgsl(graph: &FullscreenGraph, from: &str) -> Result<String, String> {
    to_wgsl_with(graph, from, &crate::subgraph::NoSubgraphs)
}

/// [`to_wgsl`], its `Subgraph` nodes found in `library`.
pub fn to_wgsl_with(graph: &FullscreenGraph, from: &str, library: &dyn crate::subgraph::Library) -> Result<String, String> {
    shape(graph)?;
    let graph = &expanded(graph, library)?;
    let color = graph.output.color.clone().unwrap_or_else(|| Input::from("color"));
    let wanted = vec![("output `color`".to_string(), &color)];
    let compiled = expr::compile(&graph.nodes, &wanted, &Screen { params: &graph.params })?;
    let value = &compiled.outputs[0];
    let code = match value.ty {
        Ty::F3 => value.code.clone(),
        Ty::F1 => format!("vec3<f32>({})", value.code),
        Ty::F4 => format!("({}).rgb", value.code),
        Ty::F2 => return Err(format!("output `color` is a colour, not {}", value.ty.said())),
    };
    let mut out = format!("// Made from {from} by the fullscreen graph: edit that, not this.\n");
    out.push_str(&compiled.helpers);
    out.push_str("fn fullscreen(in: FsIn) -> vec3<f32> {\n");
    out.push_str(&compiled.body);
    out.push_str(&format!("    return {code};\n}}\n"));
    Ok(out)
}

/// What a graph is fine with but is likely a mistake: nodes nothing reads,
/// properties no node reads.
pub fn problems_with(graph: &FullscreenGraph, library: &dyn crate::subgraph::Library) -> Vec<String> {
    let Ok(expanded) = expanded(graph, library) else {
        return Vec::new();
    };
    let color = expanded.output.color.clone().unwrap_or_else(|| Input::from("color"));
    let wanted = vec![("color".to_string(), &color)];
    let Ok(compiled) = expr::compile(&expanded.nodes, &wanted, &Screen { params: &expanded.params }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, node) in &graph.nodes {
        let unused = match node {
            Node::Subgraph { .. } => {
                let outs: Vec<&String> =
                    expanded.nodes.keys().filter(|k| k.starts_with(&format!("{name}__out__"))).collect();
                !outs.is_empty() && outs.iter().all(|k| compiled.unused.contains(k))
            }
            _ => compiled.unused.contains(name),
        };
        if unused {
            out.push(format!("node `{name}` is read by nothing"));
        }
    }
    for p in &graph.params {
        if !graph.nodes.values().any(|n| n.inputs().iter().any(|(_, i)| matches!(i, Input::Name(s) if s.split('.').next() == Some(p.name())))) {
            out.push(format!("parameter `{}` is read by no node", p.name()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_graph_is_the_picture_as_it_was() {
        let wgsl = to_wgsl(&FullscreenGraph::default(), "x").unwrap();
        assert!(wgsl.contains("return fs_color(in.uv);"), "{wgsl}");
    }

    #[test]
    fn a_graph_reads_the_picture_its_depth_and_its_numbers() {
        let g = parse(
            r#"(
                params: [("fog", Color), "reach"],
                nodes: {
                    "far": Smoothstep(low: 0.0, high: "reach", of: "depth"),
                    "wobble": SceneColor(at: "screen"),
                    "mixed": Lerp(a: "wobble", b: "fog", t: "far"),
                    "idle": Sine(of: "time"),
                },
                output: (color: "mixed"),
            )"#,
        )
        .unwrap();
        let wgsl = to_wgsl(&g, "shaders/haze.post.ron").unwrap();
        assert!(wgsl.contains("fs_depth(in.uv)") && wgsl.contains("fs.values[0].x"), "{wgsl}");
        assert_eq!(problems_with(&g, &crate::subgraph::NoSubgraphs), vec!["node `idle` is read by nothing".to_string()]);
        let bad = parse(r#"(nodes: { "c": Multiply(a: "colr", b: 2.0) }, output: (color: "c"))"#).unwrap();
        assert!(to_wgsl(&bad, "x").unwrap_err().contains("did you mean `color`"));
        let hides = parse(r#"(params: ["depth"])"#).unwrap();
        assert!(to_wgsl(&hides, "x").is_err());
    }
}
