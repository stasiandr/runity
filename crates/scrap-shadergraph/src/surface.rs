//! A material's shader graph: `shaders/<name>.graph.ron`, compiled to the
//! same `surface` function a hand-written `shaders/<name>.wgsl` is, so a
//! material names it the same way (`shader: "<name>"`) and everything that
//! loads, reloads and checks a shader serves it.
//!
//! ```ron
//! (
//!     params: ["speed", "glow"],
//!     textures: ["_Noise"],
//!     nodes: {
//!         "drift": Multiply(a: "time", b: "speed"),
//!         "flow": TilingOffset(offset: "drift"),
//!         "heat": Texture(name: "_Noise", uv: "flow"),
//!         "hot": Lerp(a: (0.2, 0.02, 0.0), b: (4.0, 1.2, 0.2), t: "heat.r"),
//!     },
//!     surface: (albedo: "hot", emission: "hot", smoothness: 0.2),
//! )
//! ```
//!
//! What a node can read besides other nodes: `uv`, `time`, `position` (in
//! the world), `normal`, `view` (toward the eye), what the material alone
//! would give — `albedo`, `alpha`, `metallic`, `smoothness`, `emission` —
//! and the graph's own `params` by name (the material's eight numbers, in
//! this order). What it sets is what `surface` names; the rest stays as the
//! material gives it. `clip` cuts away what has less alpha than it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::expr::{self, Context, Input, Node, Ty, Value};

/// How many numbers and textures a material hands its shader.
pub const PARAMS: usize = 8;
pub const TEXTURES: usize = 4;

/// The file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderGraph {
    /// Names for the material's `params`, in order: at most eight numbers.
    #[serde(default)]
    pub params: Vec<String>,
    /// Names of the material's `textures` the graph reads: at most four.
    #[serde(default)]
    pub textures: Vec<String>,
    #[serde(default)]
    pub nodes: BTreeMap<String, Node>,
    #[serde(default)]
    pub surface: SurfaceOut,
    /// What the vertex stage sets: where each vertex is and its normal —
    /// grass in the wind, a wave, a flag. Left out, vertices stay put.
    #[serde(default)]
    pub vertex: VertexOut,
}

/// What the graph sets of each vertex, in the world.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VertexOut {
    /// Where it is: `Add(a: "position", b: …)` moves it.
    #[serde(default)]
    pub position: Option<Input>,
    /// Its normal, turned toward.
    #[serde(default)]
    pub normal: Option<Input>,
}

impl VertexOut {
    fn fields(&self) -> Vec<(&'static str, &Input, Ty)> {
        [("position", &self.position), ("normal", &self.normal)]
            .into_iter()
            .filter_map(|(n, i)| i.as_ref().map(|i| (n, i, Ty::F3)))
            .collect()
    }
}

/// What a node can read in the vertex stage: the vertex as placed, the
/// model, the clock and the material's numbers — nothing of the screen.
const VERTEX_BUILTINS: &[(&str, &str, Ty)] = &[
    ("position", "in.position", Ty::F3),
    ("normal", "in.normal", Ty::F3),
    ("object", "in.object", Ty::F3),
    ("origin", "in.origin", Ty::F3),
    ("uv", "in.uv", Ty::F2),
    ("time", "in.time", Ty::F1),
    ("vertex_color", "in.vertex_color", Ty::F4),
];

struct VertexStage<'a> {
    graph: &'a ShaderGraph,
}

impl Context for VertexStage<'_> {
    fn builtin(&self, name: &str) -> Option<Value> {
        if let Some((_, code, ty)) = VERTEX_BUILTINS.iter().find(|(n, _, _)| *n == name) {
            return Some(Value::new(*code, *ty));
        }
        let i = self.graph.params.iter().position(|p| p == name)?;
        Some(Value::new(format!("in.params[{}].{}", i / 4, ["x", "y", "z", "w"][i % 4]), Ty::F1))
    }

    fn builtins(&self) -> Vec<String> {
        let mut out: Vec<String> = VERTEX_BUILTINS.iter().map(|(n, _, _)| n.to_string()).collect();
        out.extend(self.graph.params.iter().cloned());
        out
    }

    fn texture(&self, _name: &str, _uv: &str) -> Result<Value, String> {
        Err("the vertex stage reads no textures".to_string())
    }
}

/// What the graph sets of the surface; what is left out stays as the
/// material gives it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceOut {
    #[serde(default)]
    pub albedo: Option<Input>,
    #[serde(default)]
    pub alpha: Option<Input>,
    #[serde(default)]
    pub metallic: Option<Input>,
    #[serde(default)]
    pub smoothness: Option<Input>,
    /// A normal in the world, turned toward: normalised after.
    #[serde(default)]
    pub normal: Option<Input>,
    #[serde(default)]
    pub emission: Option<Input>,
    /// Cut away where the alpha is below this.
    #[serde(default)]
    pub clip: Option<Input>,
}

impl SurfaceOut {
    fn fields(&self) -> Vec<(&'static str, &Input, Ty)> {
        let mut out = Vec::new();
        for (name, input, ty) in [
            ("albedo", &self.albedo, Ty::F3),
            ("alpha", &self.alpha, Ty::F1),
            ("metallic", &self.metallic, Ty::F1),
            ("smoothness", &self.smoothness, Ty::F1),
            ("normal", &self.normal, Ty::F3),
            ("emission", &self.emission, Ty::F3),
            ("clip", &self.clip, Ty::F1),
        ] {
            if let Some(input) = input {
                out.push((name, input, ty));
            }
        }
        out
    }
}

/// What a file of this name is called as a shader: `lava.graph.ron` is
/// `lava`. `None` for any other file.
pub fn shader_name(file_name: &str) -> Option<&str> {
    file_name
        .strip_suffix(".graph.ron")
        .filter(|n| !n.is_empty())
}

/// Read a graph from its text; the error says where.
pub fn parse(text: &str) -> Result<ShaderGraph, String> {
    expr::from_ron(text).map_err(|e| e.to_string())
}

const BUILTINS: &[(&str, &str, Ty)] = &[
    ("uv", "in.uv", Ty::F2),
    ("time", "in.time", Ty::F1),
    ("position", "in.world_position", Ty::F3),
    ("normal", "in.normal", Ty::F3),
    (
        "view",
        "normalize(frame.camera_position.xyz - in.world_position)",
        Ty::F3,
    ),
    ("albedo", "out.albedo", Ty::F3),
    ("alpha", "out.alpha", Ty::F1),
    ("metallic", "out.metallic", Ty::F1),
    ("smoothness", "out.smoothness", Ty::F1),
    ("emission", "out.emission", Ty::F3),
    // Where it is drawn.
    ("screen", "surface_screen(in)", Ty::F2),
    ("depth", "surface_depth(in)", Ty::F1),
    ("scene_depth", "surface_scene_depth(in)", Ty::F1),
    ("front", "in.front", Ty::F1),
    ("vertex_color", "in.vertex_color", Ty::F4),
    ("tangent", "surface_tangent(in)", Ty::F3),
    ("bitangent", "cross(in.normal, surface_tangent(in))", Ty::F3),
    ("camera", "frame.camera_position.xyz", Ty::F3),
    // The sun, toward it, and its colour; the sky's light from all round.
    (
        "light_direction",
        "-normalize(frame.sun_direction.xyz)",
        Ty::F3,
    ),
    ("light_color", "frame.sun_color.rgb", Ty::F3),
    ("ambient", "frame.sky_color.rgb", Ty::F3),
];

struct Material<'a> {
    graph: &'a ShaderGraph,
}

impl Context for Material<'_> {
    fn builtin(&self, name: &str) -> Option<Value> {
        if let Some((_, code, ty)) = BUILTINS.iter().find(|(n, _, _)| *n == name) {
            return Some(Value::new(*code, *ty));
        }
        let i = self.graph.params.iter().position(|p| p == name)?;
        Some(Value::new(
            format!("in.params[{}].{}", i / 4, ["x", "y", "z", "w"][i % 4]),
            Ty::F1,
        ))
    }

    fn builtins(&self) -> Vec<String> {
        let mut out: Vec<String> = BUILTINS.iter().map(|(n, _, _)| n.to_string()).collect();
        out.extend(self.graph.params.iter().cloned());
        out
    }

    fn texture(&self, name: &str, uv: &str) -> Result<Value, String> {
        match self.graph.textures.iter().position(|t| t == name) {
            Some(slot) => Ok(Value::new(format!("texture_at(in, {slot}u, {uv})"), Ty::F4)),
            None => {
                let hint = scrap_core::spelling::closest(
                    name,
                    self.graph.textures.iter().map(String::as_str),
                )
                .map(|n| format!(" — did you mean `{n}`?"))
                .unwrap_or_default();
                Err(format!(
                    "the graph declares no texture `{name}` in `textures`{hint}"
                ))
            }
        }
    }

    fn derivatives(&self) -> bool {
        true
    }

    fn normal_from_height(&self, height: &str, strength: &str) -> Result<Value, String> {
        Ok(Value::new(
            format!("surface_normal_from_height(in, {height}, {strength})"),
            Ty::F3,
        ))
    }

    fn normal_from_texture(&self, name: &str, uv: &str, strength: &str) -> Result<Value, String> {
        let texel = self.texture(name, uv)?;
        Ok(Value::new(
            format!(
                "mapped_normal(in.normal, in.world_position, {uv}, ({}).xyz, {strength})",
                texel.code
            ),
            Ty::F3,
        ))
    }

    fn scene_color(&self, at: &str) -> Result<Value, String> {
        Ok(Value::new(format!("surface_scene_color({at})"), Ty::F3))
    }

    fn fresnel(&self, power: &str) -> Result<Value, String> {
        Ok(Value::new(
            format!(
                "pow(1.0 - saturate(dot(normalize(in.normal), normalize(frame.camera_position.xyz - in.world_position))), {power})"
            ),
            Ty::F1,
        ))
    }
}

/// The graph as a material shader's WGSL: its `// scrap:params` and
/// `// scrap:textures` lines, the helpers it needs, and `fn surface`. `from`
/// names the file, in the first comment.
pub fn to_wgsl(graph: &ShaderGraph, from: &str) -> Result<String, String> {
    shape(graph)?;
    let context = Material { graph };
    let outputs = graph.surface.fields();
    let wanted: Vec<(String, &Input)> = outputs
        .iter()
        .map(|(name, input, _)| (format!("surface `{name}`"), *input))
        .collect();
    let compiled = expr::compile(&graph.nodes, &wanted, &context)?;
    let moves = graph.vertex.fields();
    let vertex_wanted: Vec<(String, &Input)> = moves
        .iter()
        .map(|(name, input, _)| (format!("vertex `{name}`"), *input))
        .collect();
    let vertex = if moves.is_empty() {
        None
    } else {
        Some(expr::compile(&graph.nodes, &vertex_wanted, &VertexStage { graph })?)
    };
    let mut out = String::new();
    out.push_str(&format!(
        "// Made from {from} by the shader graph: edit that, not this.\n"
    ));
    if !graph.params.is_empty() {
        out.push_str(&format!("// scrap:params {}\n", graph.params.join(" ")));
    }
    if !graph.textures.is_empty() {
        out.push_str(&format!("// scrap:textures {}\n", graph.textures.join(" ")));
    }
    // Each helper once, whichever stage wanted it.
    let mut helpers: Vec<&str> = Vec::new();
    for h in std::iter::once(&compiled).chain(vertex.as_ref()).flat_map(|c| c.helpers.split_inclusive("\n}\n")) {
        if !h.trim().is_empty() && !helpers.contains(&h) {
            helpers.push(h);
        }
    }
    out.extend(helpers);
    out.push_str("fn surface(in: SurfaceIn, out: Surface) -> Surface {\n");
    out.push_str(&compiled.body);
    out.push_str("    var o = out;\n");
    for ((name, _, ty), value) in outputs.iter().zip(&compiled.outputs) {
        let code = spread(value, *ty).map_err(|e| format!("surface `{name}` {e}"))?;
        if *name == "clip" {
            out.push_str(&format!(
                "    if (o.alpha < {code}) {{\n        discard;\n    }}\n"
            ));
        } else {
            out.push_str(&format!("    o.{name} = {code};\n"));
        }
    }
    out.push_str("    return o;\n}\n");
    if let Some(vertex) = vertex {
        out.push_str("fn vertex(in: VertexIn, out: Vertex) -> Vertex {\n");
        out.push_str(&vertex.body);
        out.push_str("    var o = out;\n");
        for ((name, _, ty), value) in moves.iter().zip(&vertex.outputs) {
            let code = spread(value, *ty).map_err(|e| format!("vertex `{name}` {e}"))?;
            out.push_str(&format!("    o.{name} = {code};\n"));
        }
        out.push_str("    return o;\n}\n");
    }
    Ok(out)
}

/// What a graph is fine with but is likely a mistake: nodes nothing reads,
/// parameters and textures declared and not read.
pub fn problems(graph: &ShaderGraph) -> Vec<String> {
    let context = Material { graph };
    let outputs = graph.surface.fields();
    let wanted: Vec<(String, &Input)> = outputs
        .iter()
        .map(|(name, input, _)| (name.to_string(), *input))
        .collect();
    let Ok(mut compiled) = expr::compile(&graph.nodes, &wanted, &context) else {
        return Vec::new();
    };
    let moves = graph.vertex.fields();
    let vertex_wanted: Vec<(String, &Input)> = moves.iter().map(|(n, i, _)| (n.to_string(), *i)).collect();
    if !moves.is_empty() {
        let Ok(vertex) = expr::compile(&graph.nodes, &vertex_wanted, &VertexStage { graph }) else {
            return Vec::new();
        };
        compiled.unused.retain(|n| vertex.unused.contains(n));
    }
    let mut out: Vec<String> = compiled
        .unused
        .iter()
        .map(|n| format!("node `{n}` is read by nothing the surface or the vertex stage sets"))
        .collect();
    let reads = |name: &str| {
        graph.nodes.iter().any(|(n, node)| {
            !compiled.unused.contains(n)
                && (node
                    .inputs()
                    .iter()
                    .any(|(_, i)| matches!(i, Input::Name(s) if expr::split(s).0 == name))
                    || matches!(node, Node::Texture { name: t, .. } if t == name))
        }) || outputs
            .iter()
            .chain(&moves)
            .any(|(_, i, _)| matches!(i, Input::Name(s) if expr::split(s).0 == name))
    };
    for p in &graph.params {
        if !reads(p) {
            out.push(format!("parameter `{p}` is read by no node"));
        }
    }
    for t in &graph.textures {
        if !reads(t) {
            out.push(format!("texture `{t}` is read by no node"));
        }
    }
    out
}

/// What the file says that no graph can be, before its nodes are looked at.
fn shape(graph: &ShaderGraph) -> Result<(), String> {
    if graph.params.len() > PARAMS {
        return Err(format!(
            "a material hands its shader {PARAMS} numbers, and `params` names {}",
            graph.params.len()
        ));
    }
    if graph.textures.len() > TEXTURES {
        return Err(format!(
            "a material hands its shader {TEXTURES} textures, and `textures` names {}",
            graph.textures.len()
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    for name in graph
        .params
        .iter()
        .chain(&graph.textures)
        .map(String::as_str)
    {
        if name.is_empty() || name.contains(['.', ' ']) {
            return Err(format!(
                "`{name}` cannot name a parameter or texture: no dots or spaces"
            ));
        }
        if seen.contains(&name) {
            return Err(format!("`{name}` is declared twice"));
        }
        seen.push(name);
    }
    for name in graph.nodes.keys() {
        if name.is_empty() || name.contains(['.', ' ']) {
            return Err(format!("`{name}` cannot name a node: no dots or spaces"));
        }
        if BUILTINS.iter().chain(VERTEX_BUILTINS).any(|(b, _, _)| b == name) || graph.params.contains(name) {
            return Err(format!(
                "node `{name}` has the name of an input it would hide — call it something else"
            ));
        }
    }
    if graph.surface.fields().is_empty() && graph.vertex.fields().is_empty() {
        return Err("`surface` sets nothing: give it at least one of albedo, alpha, metallic, smoothness, normal, emission, clip — or `vertex` a position or a normal".into());
    }
    Ok(())
}

/// `value` as `ty`: a number is spread; a vector has to be the size.
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

    fn wgsl(text: &str) -> Result<String, String> {
        to_wgsl(&parse(text).unwrap(), "x")
    }

    #[test]
    fn the_newer_nodes_say_what_is_wrong() {
        let e = wgsl(r#"(nodes: { "b": Branch(when: (1.0, 0.0), yes: (1.0, 0.0, 0.0), no: 0.0) }, surface: (albedo: "b"))"#).unwrap_err();
        assert!(
            e.contains("`when` is a vec2 where `yes` and `no` are a vec3"),
            "{e}"
        );
        let e = wgsl(r#"(nodes: { "g": Gradient(t: "uv.x", keys: []) }, surface: (albedo: "g"))"#)
            .unwrap_err();
        assert!(e.contains("`keys` is empty"), "{e}");
        let e = wgsl(
            r#"(nodes: { "h": Hue(of: "vertex_color", offset: 0.1) }, surface: (albedo: "h"))"#,
        )
        .unwrap_err();
        assert!(e.contains("take `.rgb`"), "{e}");
        let e = wgsl(r#"(nodes: { "t": Triplanar(name: "_Rock") }, surface: (albedo: "t.rgb"))"#)
            .unwrap_err();
        assert!(e.contains("no texture `_Rock`"), "{e}");
    }

    #[test]
    fn a_gradient_is_straight_between_its_keys_in_their_order() {
        let w = wgsl(r#"(nodes: { "g": Gradient(t: "uv.x", keys: [(1.0, (1.0, 1.0, 1.0)), (0.0, (0.0, 0.0, 0.0))]) }, surface: (albedo: "g"))"#).unwrap();
        // Sorted: black first, mixed toward white.
        assert!(
            w.contains("mix(vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(1.0, 1.0, 1.0)"),
            "{w}"
        );
    }
}
