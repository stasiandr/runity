//! The nodes both graphs are made of, and turning a set of them into WGSL.
//!
//! A node is a named value computed from its inputs; an input is another
//! node's name (or a built-in's, or a parameter's), optionally with a
//! swizzle (`noise.x`, `tint.rgb`), or a number, or a vector written as a
//! tuple. What a graph *means* — which built-ins exist, what its outputs
//! are — is its own ([`Context`]); this only knows numbers.
//!
//! Types are `f32` and `vec2`..`vec4<f32>`. A number meets a vector by being
//! spread over it; two vectors of different sizes do not meet — the error
//! says which is which and how to take the part wanted (`.xyz`).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Where a node's input comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Input {
    /// A node, a built-in or a parameter, by name, with an optional
    /// swizzle: `"noise"`, `"uv.x"`, `"tint.rgb"`.
    Name(String),
    Number(f32),
    /// A vector of two to four numbers: `(1.0, 0.5, 0.2)`.
    Vector(Vec<f32>),
}

impl From<&str> for Input {
    fn from(name: &str) -> Self {
        Input::Name(name.to_string())
    }
}

impl From<f32> for Input {
    fn from(n: f32) -> Self {
        Input::Number(n)
    }
}

/// A value's type: how many numbers it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ty {
    F1 = 1,
    F2 = 2,
    F3 = 3,
    F4 = 4,
}

impl Ty {
    pub fn of(n: usize) -> Option<Ty> {
        match n {
            1 => Some(Ty::F1),
            2 => Some(Ty::F2),
            3 => Some(Ty::F3),
            4 => Some(Ty::F4),
            _ => None,
        }
    }

    pub fn size(self) -> usize {
        self as usize
    }

    pub fn wgsl(self) -> &'static str {
        match self {
            Ty::F1 => "f32",
            Ty::F2 => "vec2<f32>",
            Ty::F3 => "vec3<f32>",
            Ty::F4 => "vec4<f32>",
        }
    }

    /// As a person says it, for errors.
    pub fn said(self) -> &'static str {
        match self {
            Ty::F1 => "a number",
            Ty::F2 => "a vec2",
            Ty::F3 => "a vec3",
            Ty::F4 => "a vec4",
        }
    }
}

/// A piece of WGSL and its type.
#[derive(Debug, Clone, PartialEq)]
pub struct Value {
    pub code: String,
    pub ty: Ty,
}

impl Value {
    pub fn new(code: impl Into<String>, ty: Ty) -> Self {
        Value {
            code: code.into(),
            ty,
        }
    }

    /// Spread over `ty` if a number; the same if already `ty`.
    fn to(&self, ty: Ty) -> Option<String> {
        if self.ty == ty {
            Some(self.code.clone())
        } else if self.ty == Ty::F1 {
            Some(format!("{}({})", ty.wgsl(), self.code))
        } else {
            None
        }
    }
}

/// The nodes. Each is written as its kind with its inputs by name:
/// `Lerp(a: "cold", b: "hot", t: "noise.x")`. An input left out takes the
/// default the kind says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    // Two in, the same out.
    Add {
        a: Input,
        b: Input,
    },
    Subtract {
        a: Input,
        b: Input,
    },
    Multiply {
        a: Input,
        b: Input,
    },
    Divide {
        a: Input,
        b: Input,
    },
    Power {
        a: Input,
        b: Input,
    },
    Min {
        a: Input,
        b: Input,
    },
    Max {
        a: Input,
        b: Input,
    },
    /// `a` wrapped into `0..b`, never negative.
    Modulo {
        a: Input,
        b: Input,
    },
    /// 0 below `edge`, 1 from it.
    Step {
        edge: Input,
        of: Input,
    },
    // Two vectors in, a number out.
    Dot {
        a: Input,
        b: Input,
    },
    Distance {
        a: Input,
        b: Input,
    },
    Cross {
        a: Input,
        b: Input,
    },
    // One in, the same out.
    Negate {
        of: Input,
    },
    OneMinus {
        of: Input,
    },
    Abs {
        of: Input,
    },
    Floor {
        of: Input,
    },
    Ceil {
        of: Input,
    },
    Round {
        of: Input,
    },
    Fract {
        of: Input,
    },
    Sign {
        of: Input,
    },
    Sine {
        of: Input,
    },
    Cosine {
        of: Input,
    },
    Sqrt {
        of: Input,
    },
    Exp {
        of: Input,
    },
    /// Clamped to 0..1.
    Saturate {
        of: Input,
    },
    Length {
        of: Input,
    },
    Normalize {
        of: Input,
    },
    /// Between `a` and `b` by `t` (a number, or one per component).
    Lerp {
        a: Input,
        b: Input,
        t: Input,
    },
    Clamp {
        of: Input,
        #[serde(default = "zero")]
        low: Input,
        #[serde(default = "one")]
        high: Input,
    },
    Smoothstep {
        low: Input,
        high: Input,
        of: Input,
    },
    /// From the range `from` (a vec2: low, high) to the range `to`.
    Remap {
        of: Input,
        from: Input,
        to: Input,
    },
    /// So many even steps between 0 and 1.
    Posterize {
        of: Input,
        steps: Input,
    },
    /// A vector of these, in order: two to four numbers.
    Combine {
        x: Input,
        y: Input,
        #[serde(default)]
        z: Option<Input>,
        #[serde(default)]
        w: Option<Input>,
    },
    /// UV repeated `tiling` times and moved by `offset`.
    TilingOffset {
        #[serde(default = "uv")]
        uv: Input,
        #[serde(default = "one")]
        tiling: Input,
        #[serde(default = "zero")]
        offset: Input,
    },
    /// UV turned by `angle` radians about `center`.
    Rotate {
        #[serde(default = "uv")]
        uv: Input,
        #[serde(default = "half")]
        center: Input,
        angle: Input,
    },
    /// Smooth gradient noise, 0..1, at a vec2 or vec3, `scale` cells a
    /// unit.
    Noise {
        #[serde(default = "uv")]
        at: Input,
        #[serde(default = "one")]
        scale: Input,
    },
    /// Cells round scattered points, at a vec2: a vec2 of the distance to
    /// the nearest point (`.x`) and to the next nearest (`.y`) — `.y - .x`
    /// is small along the cells' borders (cracks, scales).
    Voronoi {
        #[serde(default = "uv")]
        at: Input,
        #[serde(default = "one")]
        scale: Input,
    },
    /// Three noises at once, a vec3 from -1 to 1 that changes smoothly
    /// with `at` (a vec3): a wind that differs from place to place.
    Turbulence {
        #[serde(default = "position")]
        at: Input,
        #[serde(default = "one")]
        scale: Input,
    },
    /// A random number from `low` to `high` (numbers or vectors, a random
    /// for each component) — in a particle's graph, drawn once for each
    /// particle and kept all its life. Only where the graph has something
    /// to be random for.
    Random {
        #[serde(default = "zero")]
        low: Input,
        #[serde(default = "one")]
        high: Input,
    },
    /// 0 and 1 in squares, `scale` a unit.
    Checker {
        #[serde(default = "uv")]
        uv: Input,
        #[serde(default = "one")]
        scale: Input,
    },
    /// How much a surface faces away from the eye, 0 facing it, 1 edge-on.
    /// Only where the graph knows an eye and a normal.
    Fresnel {
        #[serde(default = "five")]
        power: Input,
    },
    /// A texture the graph declares, at `uv`: a vec4. Only in a material's
    /// graph.
    Texture {
        name: String,
        #[serde(default = "uv")]
        uv: Input,
    },
}

fn zero() -> Input {
    Input::Number(0.0)
}
fn one() -> Input {
    Input::Number(1.0)
}
fn five() -> Input {
    Input::Number(5.0)
}
fn half() -> Input {
    Input::Vector(vec![0.5, 0.5])
}
fn uv() -> Input {
    Input::Name("uv".to_string())
}
fn position() -> Input {
    Input::Name("position".to_string())
}

impl Node {
    /// Every kind, as written.
    pub const KINDS: &'static [&'static str] = &[
        "Add",
        "Subtract",
        "Multiply",
        "Divide",
        "Power",
        "Min",
        "Max",
        "Modulo",
        "Step",
        "Dot",
        "Distance",
        "Cross",
        "Negate",
        "OneMinus",
        "Abs",
        "Floor",
        "Ceil",
        "Round",
        "Fract",
        "Sign",
        "Sine",
        "Cosine",
        "Sqrt",
        "Exp",
        "Saturate",
        "Length",
        "Normalize",
        "Lerp",
        "Clamp",
        "Smoothstep",
        "Remap",
        "Posterize",
        "Combine",
        "TilingOffset",
        "Rotate",
        "Noise",
        "Voronoi",
        "Turbulence",
        "Random",
        "Checker",
        "Fresnel",
        "Texture",
    ];

    /// The kind, as written.
    pub fn kind(&self) -> &'static str {
        macro_rules! kinds {
            ($($k:ident),*) => {
                match self { $(Node::$k { .. } => stringify!($k),)* }
            };
        }
        kinds!(
            Add,
            Subtract,
            Multiply,
            Divide,
            Power,
            Min,
            Max,
            Modulo,
            Step,
            Dot,
            Distance,
            Cross,
            Negate,
            OneMinus,
            Abs,
            Floor,
            Ceil,
            Round,
            Fract,
            Sign,
            Sine,
            Cosine,
            Sqrt,
            Exp,
            Saturate,
            Length,
            Normalize,
            Lerp,
            Clamp,
            Smoothstep,
            Remap,
            Posterize,
            Combine,
            TilingOffset,
            Rotate,
            Noise,
            Voronoi,
            Turbulence,
            Random,
            Checker,
            Fresnel,
            Texture
        )
    }

    /// Its inputs, by the names they are written with, in order.
    pub fn inputs(&self) -> Vec<(&'static str, &Input)> {
        use Node::*;
        match self {
            Add { a, b }
            | Subtract { a, b }
            | Multiply { a, b }
            | Divide { a, b }
            | Power { a, b }
            | Min { a, b }
            | Max { a, b }
            | Modulo { a, b }
            | Dot { a, b }
            | Distance { a, b }
            | Cross { a, b } => vec![("a", a), ("b", b)],
            Step { edge, of } => vec![("edge", edge), ("of", of)],
            Negate { of }
            | OneMinus { of }
            | Abs { of }
            | Floor { of }
            | Ceil { of }
            | Round { of }
            | Fract { of }
            | Sign { of }
            | Sine { of }
            | Cosine { of }
            | Sqrt { of }
            | Exp { of }
            | Saturate { of }
            | Length { of }
            | Normalize { of } => vec![("of", of)],
            Lerp { a, b, t } => vec![("a", a), ("b", b), ("t", t)],
            Clamp { of, low, high } => vec![("of", of), ("low", low), ("high", high)],
            Smoothstep { low, high, of } => vec![("low", low), ("high", high), ("of", of)],
            Remap { of, from, to } => vec![("of", of), ("from", from), ("to", to)],
            Posterize { of, steps } => vec![("of", of), ("steps", steps)],
            Combine { x, y, z, w } => {
                let mut out = vec![("x", x), ("y", y)];
                if let Some(z) = z {
                    out.push(("z", z));
                }
                if let Some(w) = w {
                    out.push(("w", w));
                }
                out
            }
            TilingOffset { uv, tiling, offset } => {
                vec![("uv", uv), ("tiling", tiling), ("offset", offset)]
            }
            Rotate { uv, center, angle } => vec![("uv", uv), ("center", center), ("angle", angle)],
            Noise { at, scale } | Voronoi { at, scale } | Turbulence { at, scale } => {
                vec![("at", at), ("scale", scale)]
            }
            Random { low, high } => vec![("low", low), ("high", high)],
            Checker { uv, scale } => vec![("uv", uv), ("scale", scale)],
            Fresnel { power } => vec![("power", power)],
            Texture { uv, .. } => vec![("uv", uv)],
        }
    }
}

/// What a graph brings to its nodes: the built-ins it can read, and the
/// nodes only it can make sense of (a texture, an eye to be facing).
pub trait Context {
    /// A built-in by name: `uv`, `time`, a parameter…
    fn builtin(&self, name: &str) -> Option<Value>;
    /// Every built-in's name, for a misspelling's nearest.
    fn builtins(&self) -> Vec<String>;
    /// A texture the graph declares, sampled at `uv` (a vec2's code).
    fn texture(&self, name: &str, uv: &str) -> Result<Value, String> {
        let _ = (name, uv);
        Err("this graph has no textures".to_string())
    }
    /// How much the surface faces away from the eye, to `power`.
    fn fresnel(&self, power: &str) -> Result<Value, String> {
        let _ = power;
        Err("this graph knows no eye to face".to_string())
    }
    /// `ty`'s worth of random numbers from 0 to 1, told apart from other
    /// `Random` nodes by `salt`.
    fn random(&self, salt: u32, ty: Ty) -> Result<Value, String> {
        let _ = (salt, ty);
        Err("a surface has nothing to be random for — `Noise` at `position` is a random that stays put".to_string())
    }
}

/// Helper functions a node needs, emitted once above the code that uses
/// them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Helper {
    #[default]
    Hash,
    Noise2,
    Noise3,
    Voronoi,
}

/// Nodes turned into WGSL: `let` lines in order, and the helpers they need.
#[derive(Debug, Default)]
pub struct Compiled {
    /// Each wanted input's value, in the order asked.
    pub outputs: Vec<Value>,
    /// Module-level helper functions (prefixed `sg_`).
    pub helpers: String,
    /// The body: one `let` a node reached from the outputs, in order.
    pub body: String,
    /// Each node's value (its `let` name and type), by the node's name.
    pub values: BTreeMap<String, Value>,
    /// Nodes no output reaches: not compiled, said by `check`.
    pub unused: Vec<String>,
}

/// Compile what the `wanted` inputs read in `nodes`, in `context`: each is
/// labelled for its errors (`surface `albedo``). Errors name the node and
/// the input.
pub fn compile(
    nodes: &BTreeMap<String, Node>,
    wanted: &[(String, &Input)],
    context: &dyn Context,
) -> Result<Compiled, String> {
    let mut state = State {
        nodes,
        context,
        out: Compiled::default(),
        helpers: Vec::new(),
        walking: Vec::new(),
    };
    for (label, input) in wanted {
        let value = state.read(label, input)?;
        state.out.outputs.push(value);
    }
    let used: Vec<String> = state.out.values.keys().cloned().collect();
    state.out.unused = nodes
        .keys()
        .filter(|n| !used.contains(n))
        .cloned()
        .collect();
    let mut helpers = std::mem::take(&mut state.helpers);
    helpers.sort();
    helpers.dedup();
    for helper in helpers {
        state.out.helpers.push_str(helper_code(helper));
    }
    Ok(state.out)
}

/// A name's node and its swizzle: `"tint.rgb"` → `("tint", Some("rgb"))`.
pub fn split(name: &str) -> (&str, Option<&str>) {
    match name.split_once('.') {
        Some((node, swizzle)) => (node, Some(swizzle)),
        None => (name, None),
    }
}

/// Read RON the way the graphs are written: an optional input is just
/// written (`z: 0.5`), not wrapped in `Some`.
pub fn from_ron<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ron::error::SpannedError> {
    ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(text)
}

/// A node's WGSL name: `n_` and its name with anything else made `_`.
fn ident(name: &str) -> String {
    let mut out = String::from("n_");
    for c in name.chars() {
        out.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    out
}

/// A WGSL float literal.
pub fn number(n: f32) -> Result<String, String> {
    if !n.is_finite() {
        return Err(format!("{n} is not a number a shader can hold"));
    }
    let mut s = format!("{n:?}");
    if !s.contains('.') && !s.contains('e') {
        s.push_str(".0");
    }
    Ok(s)
}

struct State<'a> {
    nodes: &'a BTreeMap<String, Node>,
    context: &'a dyn Context,
    out: Compiled,
    helpers: Vec<Helper>,
    walking: Vec<String>,
}

impl State<'_> {
    /// The value of the node called `name`, compiled if it has not been.
    fn node(&mut self, name: &str) -> Result<Value, String> {
        if let Some(v) = self.out.values.get(name) {
            return Ok(v.clone());
        }
        if let Some(at) = self.walking.iter().position(|n| n == name) {
            let mut round: Vec<&str> = self.walking[at..].iter().map(String::as_str).collect();
            round.push(name);
            return Err(format!(
                "the nodes go round in a circle: {}",
                round.join(" → ")
            ));
        }
        let node = self
            .nodes
            .get(name)
            .expect("asked for by name only when it exists");
        self.walking.push(name.to_string());
        let value = self.emit(name, node);
        self.walking.pop();
        let value = value?;
        let id = ident(name);
        let _ = writeln!(
            self.out.body,
            "    let {id}: {} = {};",
            value.ty.wgsl(),
            value.code
        );
        let named = Value::new(id, value.ty);
        self.out.values.insert(name.to_string(), named.clone());
        Ok(named)
    }

    /// An input's value, for node `of` (named in errors).
    fn input(&mut self, of: &str, field: &str, input: &Input) -> Result<Value, String> {
        self.read(&format!("node `{of}`, input `{field}`"), input)
    }

    /// An input's value; `label` says where it is read, in errors.
    fn read(&mut self, label: &str, input: &Input) -> Result<Value, String> {
        let at = || label.to_string();
        match input {
            Input::Number(n) => Ok(Value::new(
                number(*n).map_err(|e| format!("{}: {e}", at()))?,
                Ty::F1,
            )),
            Input::Vector(v) => {
                let ty = Ty::of(v.len()).filter(|t| *t != Ty::F1).ok_or_else(|| {
                    format!("{}: a vector is two to four numbers, not {}", at(), v.len())
                })?;
                let parts = v
                    .iter()
                    .map(|n| number(*n))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("{}: {e}", at()))?;
                Ok(Value::new(
                    format!("{}({})", ty.wgsl(), parts.join(", ")),
                    ty,
                ))
            }
            Input::Name(name) => {
                let (base, swizzle) = split(name);
                let value = if self.nodes.contains_key(base) {
                    self.node(base)?
                } else if let Some(v) = self.context.builtin(base) {
                    v
                } else {
                    let mut known: Vec<String> = self.nodes.keys().cloned().collect();
                    known.extend(self.context.builtins());
                    let hint =
                        scrap_core::spelling::closest(base, known.iter().map(String::as_str))
                            .map(|n| format!(" — did you mean `{n}`?"))
                            .unwrap_or_default();
                    return Err(format!(
                        "{}: no node, input or parameter called `{base}`{hint}",
                        at()
                    ));
                };
                match swizzle {
                    None => Ok(value),
                    Some(s) => swizzled(&value, s).map_err(|e| format!("{}: `{name}`: {e}", at())),
                }
            }
        }
    }

    fn emit(&mut self, name: &str, node: &Node) -> Result<Value, String> {
        let mut args: Vec<Value> = Vec::new();
        for (field, input) in node.inputs() {
            args.push(self.input(name, field, input)?);
        }
        let at = |what: String| format!("node `{name}` ({}): {what}", node.kind());
        use Node::*;
        let same = |args: &[Value]| -> Result<(Ty, Vec<String>), String> {
            let ty = args.iter().map(|a| a.ty).max().unwrap_or(Ty::F1);
            let fields: Vec<&str> = node.inputs().iter().map(|(f, _)| *f).collect();
            let mut out = Vec::new();
            for (a, f) in args.iter().zip(fields) {
                out.push(a.to(ty).ok_or_else(|| {
                    format!(
                        "`{f}` is {} where the others make {} — take part of it (`.xy`, `.xyz`) or give a number",
                        a.ty.said(),
                        ty.said()
                    )
                })?);
            }
            Ok((ty, out))
        };
        let vector = |a: &Value, what: &str| -> Result<(), String> {
            if a.ty == Ty::F1 {
                Err(format!("`{what}` has to be a vector, not a number"))
            } else {
                Ok(())
            }
        };
        let two = |f: &str, args: &[Value]| -> Result<Value, String> {
            let (ty, a) = same(args).map_err(at)?;
            Ok(Value::new(format!("{f}({}, {})", a[0], a[1]), ty))
        };
        let op = |o: &str, args: &[Value]| -> Result<Value, String> {
            let (ty, a) = same(args).map_err(at)?;
            Ok(Value::new(format!("({} {o} {})", a[0], a[1]), ty))
        };
        let one =
            |f: &str, args: &[Value]| Value::new(format!("{f}({})", args[0].code), args[0].ty);
        let v = match node {
            Add { .. } => op("+", &args)?,
            Subtract { .. } => op("-", &args)?,
            Multiply { .. } => op("*", &args)?,
            Divide { .. } => op("/", &args)?,
            Power { .. } => two("pow", &args)?,
            Min { .. } => two("min", &args)?,
            Max { .. } => two("max", &args)?,
            Modulo { .. } => {
                let (ty, a) = same(&args).map_err(at)?;
                Value::new(format!("({0} - {1} * floor({0} / {1}))", a[0], a[1]), ty)
            }
            Step { .. } => two("step", &args)?,
            Dot { .. } | Distance { .. } => {
                let (_, a) = same(&args).map_err(at)?;
                vector(&args[0], "a").map_err(at)?;
                let f = if matches!(node, Dot { .. }) {
                    "dot"
                } else {
                    "distance"
                };
                Value::new(format!("{f}({}, {})", a[0], a[1]), Ty::F1)
            }
            Cross { .. } => {
                if args.iter().any(|a| a.ty != Ty::F3) {
                    return Err(at("`a` and `b` have to be vec3s".to_string()));
                }
                Value::new(format!("cross({}, {})", args[0].code, args[1].code), Ty::F3)
            }
            Negate { .. } => Value::new(format!("(-{})", args[0].code), args[0].ty),
            OneMinus { .. } => {
                let ty = args[0].ty;
                Value::new(
                    format!(
                        "({} - {})",
                        Value::new("1.0", Ty::F1).to(ty).unwrap(),
                        args[0].code
                    ),
                    ty,
                )
            }
            Abs { .. } => one("abs", &args),
            Floor { .. } => one("floor", &args),
            Ceil { .. } => one("ceil", &args),
            Round { .. } => one("round", &args),
            Fract { .. } => one("fract", &args),
            Sign { .. } => one("sign", &args),
            Sine { .. } => one("sin", &args),
            Cosine { .. } => one("cos", &args),
            Sqrt { .. } => one("sqrt", &args),
            Exp { .. } => one("exp", &args),
            Saturate { .. } => one("saturate", &args),
            Length { .. } => Value::new(format!("length({})", args[0].code), Ty::F1),
            Normalize { .. } => {
                vector(&args[0], "of").map_err(at)?;
                one("normalize", &args)
            }
            Lerp { .. } => {
                let (ty, ab) = same(&args[..2]).map_err(at)?;
                let t = args[2].to(ty).ok_or_else(|| {
                    at(format!(
                        "`t` is {} where `a` and `b` are {}",
                        args[2].ty.said(),
                        ty.said()
                    ))
                })?;
                Value::new(format!("mix({}, {}, {t})", ab[0], ab[1]), ty)
            }
            Clamp { .. } => {
                let (ty, a) = same(&args).map_err(at)?;
                Value::new(format!("clamp({}, {}, {})", a[0], a[1], a[2]), ty)
            }
            Smoothstep { .. } => {
                let (ty, a) = same(&args).map_err(at)?;
                Value::new(format!("smoothstep({}, {}, {})", a[0], a[1], a[2]), ty)
            }
            Remap { .. } => {
                for (i, f) in [(1, "from"), (2, "to")] {
                    if args[i].ty != Ty::F2 {
                        return Err(at(format!(
                            "`{f}` is a range, a vec2 (low, high), not {}",
                            args[i].ty.said()
                        )));
                    }
                }
                let (of, from, to) = (&args[0].code, &args[1].code, &args[2].code);
                Value::new(
                    format!(
                        "({to}.x + ({of} - {from}.x) / ({from}.y - {from}.x) * ({to}.y - {to}.x))"
                    ),
                    args[0].ty,
                )
            }
            Posterize { .. } => {
                let (ty, a) = same(&args).map_err(at)?;
                Value::new(format!("(floor({0} * {1}) / {1})", a[0], a[1]), ty)
            }
            Combine { .. } => {
                if let Some(a) = args.iter().position(|a| a.ty != Ty::F1) {
                    let f = node.inputs()[a].0;
                    return Err(at(format!(
                        "`{f}` has to be a number, not {}",
                        args[a].ty.said()
                    )));
                }
                let ty = Ty::of(args.len()).expect("two to four");
                let parts: Vec<&str> = args.iter().map(|a| a.code.as_str()).collect();
                Value::new(format!("{}({})", ty.wgsl(), parts.join(", ")), ty)
            }
            TilingOffset { .. } => {
                if args[0].ty != Ty::F2 {
                    return Err(at(format!(
                        "`uv` has to be a vec2, not {}",
                        args[0].ty.said()
                    )));
                }
                let tiling = args[1]
                    .to(Ty::F2)
                    .ok_or_else(|| at("`tiling` is a number or a vec2".into()))?;
                let offset = args[2]
                    .to(Ty::F2)
                    .ok_or_else(|| at("`offset` is a number or a vec2".into()))?;
                Value::new(format!("({} * {tiling} + {offset})", args[0].code), Ty::F2)
            }
            Rotate { .. } => {
                if args[0].ty != Ty::F2 || args[1].ty != Ty::F2 || args[2].ty != Ty::F1 {
                    return Err(at(
                        "`uv` and `center` are vec2s, `angle` a number of radians".into(),
                    ));
                }
                let (uv, c, a) = (&args[0].code, &args[1].code, &args[2].code);
                Value::new(
                    format!("({c} + mat2x2<f32>(cos({a}), sin({a}), -sin({a}), cos({a})) * ({uv} - {c}))"),
                    Ty::F2,
                )
            }
            Noise { .. } => {
                let scale = args[1]
                    .to(Ty::F1)
                    .ok_or_else(|| at("`scale` is a number".into()))?;
                match args[0].ty {
                    Ty::F2 => {
                        self.helpers.extend([Helper::Hash, Helper::Noise2]);
                        Value::new(format!("sg_noise2({} * {scale})", args[0].code), Ty::F1)
                    }
                    Ty::F3 => {
                        self.helpers.extend([Helper::Hash, Helper::Noise3]);
                        Value::new(format!("sg_noise3({} * {scale})", args[0].code), Ty::F1)
                    }
                    t => return Err(at(format!("`at` is a vec2 or a vec3, not {}", t.said()))),
                }
            }
            Voronoi { .. } => {
                let scale = args[1]
                    .to(Ty::F1)
                    .ok_or_else(|| at("`scale` is a number".into()))?;
                if args[0].ty != Ty::F2 {
                    return Err(at(format!("`at` is a vec2, not {}", args[0].ty.said())));
                }
                self.helpers.extend([Helper::Hash, Helper::Voronoi]);
                Value::new(format!("sg_voronoi({} * {scale})", args[0].code), Ty::F2)
            }
            Turbulence { .. } => {
                let scale = args[1]
                    .to(Ty::F1)
                    .ok_or_else(|| at("`scale` is a number".into()))?;
                if args[0].ty != Ty::F3 {
                    return Err(at(format!("`at` is a vec3, not {}", args[0].ty.said())));
                }
                self.helpers.extend([Helper::Hash, Helper::Noise3]);
                let p = format!("({} * {scale})", args[0].code);
                Value::new(
                    format!("(vec3<f32>(sg_noise3({p}), sg_noise3({p} + vec3<f32>(31.7, 11.3, 5.9)), sg_noise3({p} + vec3<f32>(-7.1, 23.9, 41.3))) * 2.0 - 1.0)"),
                    Ty::F3,
                )
            }
            Random { .. } => {
                let (ty, a) = same(&args).map_err(at)?;
                let salt = name.bytes().fold(2_166_136_261u32, |h, b| {
                    (h ^ b as u32).wrapping_mul(16_777_619)
                });
                let r = self.context.random(salt, ty).map_err(at)?;
                Value::new(format!("mix({}, {}, {})", a[0], a[1], r.code), ty)
            }
            Checker { .. } => {
                if args[0].ty != Ty::F2 {
                    return Err(at(format!(
                        "`uv` has to be a vec2, not {}",
                        args[0].ty.said()
                    )));
                }
                let scale = args[1]
                    .to(Ty::F2)
                    .ok_or_else(|| at("`scale` is a number or a vec2".into()))?;
                Value::new(
                    format!("(floor(({0} * {scale}).x) + floor(({0} * {scale}).y) - 2.0 * floor((floor(({0} * {scale}).x) + floor(({0} * {scale}).y)) * 0.5))", args[0].code),
                    Ty::F1,
                )
            }
            Fresnel { .. } => {
                let p = args[0]
                    .to(Ty::F1)
                    .ok_or_else(|| at("`power` is a number".into()))?;
                self.context.fresnel(&p).map_err(at)?
            }
            Texture { name: texture, .. } => {
                if args[0].ty != Ty::F2 {
                    return Err(at(format!(
                        "`uv` has to be a vec2, not {}",
                        args[0].ty.said()
                    )));
                }
                self.context.texture(texture, &args[0].code).map_err(at)?
            }
        };
        Ok(v)
    }
}

/// A value's components picked by `s` (`xyzw` or `rgba`, one to four).
fn swizzled(value: &Value, s: &str) -> Result<Value, String> {
    let ty = Ty::of(s.len())
        .ok_or_else(|| "a swizzle is one to four of x, y, z, w (or r, g, b, a)".to_string())?;
    for c in s.chars() {
        let at = match c {
            'x' | 'r' => 0,
            'y' | 'g' => 1,
            'z' | 'b' => 2,
            'w' | 'a' => 3,
            _ => return Err(format!("`{c}` is not x, y, z, w or r, g, b, a")),
        };
        if at >= value.ty.size() {
            return Err(format!("it is {}, which has no `{c}`", value.ty.said()));
        }
    }
    if value.ty == Ty::F1 {
        // A number's "x" is itself; more of it is spread.
        return Ok(if ty == Ty::F1 {
            value.clone()
        } else {
            Value::new(format!("{}({})", ty.wgsl(), value.code), ty)
        });
    }
    let s: String = s
        .chars()
        .map(|c| match c {
            'r' => 'x',
            'g' => 'y',
            'b' => 'z',
            'a' => 'w',
            c => c,
        })
        .collect();
    Ok(Value::new(format!("{}.{s}", value.code), ty))
}

fn helper_code(helper: Helper) -> &'static str {
    match helper {
        Helper::Hash => {
            "fn sg_hash3(p: vec3<f32>) -> vec3<f32> {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yxz + 33.33);
    return fract((q.xxy + q.yxx) * q.zyx) * 2.0 - 1.0;
}
"
        }
        Helper::Noise2 => {
            "fn sg_noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = dot(sg_hash3(vec3<f32>(i, 0.0)).xy, f);
    let b = dot(sg_hash3(vec3<f32>(i + vec2<f32>(1.0, 0.0), 0.0)).xy, f - vec2<f32>(1.0, 0.0));
    let c = dot(sg_hash3(vec3<f32>(i + vec2<f32>(0.0, 1.0), 0.0)).xy, f - vec2<f32>(0.0, 1.0));
    let d = dot(sg_hash3(vec3<f32>(i + vec2<f32>(1.0, 1.0), 0.0)).xy, f - vec2<f32>(1.0, 1.0));
    return clamp(mix(mix(a, b, u.x), mix(c, d, u.x), u.y) * 0.7 + 0.5, 0.0, 1.0);
}
"
        }
        Helper::Noise3 => {
            "fn sg_noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let c000 = dot(sg_hash3(i), f);
    let c100 = dot(sg_hash3(i + vec3<f32>(1.0, 0.0, 0.0)), f - vec3<f32>(1.0, 0.0, 0.0));
    let c010 = dot(sg_hash3(i + vec3<f32>(0.0, 1.0, 0.0)), f - vec3<f32>(0.0, 1.0, 0.0));
    let c110 = dot(sg_hash3(i + vec3<f32>(1.0, 1.0, 0.0)), f - vec3<f32>(1.0, 1.0, 0.0));
    let c001 = dot(sg_hash3(i + vec3<f32>(0.0, 0.0, 1.0)), f - vec3<f32>(0.0, 0.0, 1.0));
    let c101 = dot(sg_hash3(i + vec3<f32>(1.0, 0.0, 1.0)), f - vec3<f32>(1.0, 0.0, 1.0));
    let c011 = dot(sg_hash3(i + vec3<f32>(0.0, 1.0, 1.0)), f - vec3<f32>(0.0, 1.0, 1.0));
    let c111 = dot(sg_hash3(i + vec3<f32>(1.0, 1.0, 1.0)), f - vec3<f32>(1.0, 1.0, 1.0));
    let near = mix(mix(c000, c100, u.x), mix(c010, c110, u.x), u.y);
    let far = mix(mix(c001, c101, u.x), mix(c011, c111, u.x), u.y);
    return clamp(mix(near, far, u.z) * 0.8 + 0.5, 0.0, 1.0);
}
"
        }
        Helper::Voronoi => {
            "fn sg_voronoi(p: vec2<f32>) -> vec2<f32> {
    let i = floor(p);
    let f = fract(p);
    var nearest = vec2<f32>(8.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let cell = vec2<f32>(f32(x), f32(y));
            let point = sg_hash3(vec3<f32>(i + cell, 7.0)).xy * 0.5 + 0.5;
            let d = length(cell + point - f);
            nearest = select(vec2<f32>(nearest.x, min(nearest.y, d)), vec2<f32>(d, nearest.x), d < nearest.x);
        }
    }
    return clamp(nearest, vec2<f32>(0.0), vec2<f32>(1.5));
}
"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;
    impl Context for Plain {
        fn builtin(&self, name: &str) -> Option<Value> {
            match name {
                "uv" => Some(Value::new("in.uv", Ty::F2)),
                "time" => Some(Value::new("in.time", Ty::F1)),
                _ => None,
            }
        }
        fn builtins(&self) -> Vec<String> {
            vec!["uv".into(), "time".into()]
        }
    }

    fn nodes(text: &str) -> BTreeMap<String, Node> {
        from_ron(text).unwrap()
    }

    #[test]
    fn nodes_come_out_in_the_order_they_are_read_and_only_those_reached() {
        let n = nodes(
            r#"{ "b": Add(a: "a", b: 1.0), "a": Multiply(a: "uv", b: "time"), "lost": Sine(of: "time") }"#,
        );
        let c = compile(&n, &[("out".into(), &Input::from("b"))], &Plain).unwrap();
        let a = c.body.find("let n_a").unwrap();
        let b = c.body.find("let n_b").unwrap();
        assert!(a < b, "{}", c.body);
        assert_eq!(
            c.values["b"].ty,
            Ty::F2,
            "a number spreads over the vec2 it meets"
        );
        assert!(c.body.contains("vec2<f32>(1.0)"), "{}", c.body);
        assert_eq!(c.unused, vec!["lost".to_string()]);
    }

    #[test]
    fn a_misspelt_name_says_where_and_what_was_meant() {
        let n = nodes(r#"{ "wave": Sine(of: "tme") }"#);
        let e = compile(&n, &[("out".into(), &Input::from("wave"))], &Plain).unwrap_err();
        assert!(e.contains("node `wave`, input `of`"), "{e}");
        assert!(e.contains("did you mean `time`"), "{e}");
    }

    #[test]
    fn a_circle_is_named_and_mismatched_vectors_say_how_to_fix_them() {
        let n = nodes(r#"{ "a": Add(a: "b", b: 1.0), "b": Add(a: "a", b: 1.0) }"#);
        let e = compile(&n, &[("out".into(), &Input::from("a"))], &Plain).unwrap_err();
        assert!(e.contains("circle") && e.contains("a → b → a"), "{e}");
        let n = nodes(r#"{ "m": Add(a: "uv", b: (1.0, 2.0, 3.0)) }"#);
        let e = compile(&n, &[("out".into(), &Input::from("m"))], &Plain).unwrap_err();
        assert!(e.contains("`a` is a vec2") && e.contains(".xyz"), "{e}");
    }

    #[test]
    fn swizzles_pick_and_spread_and_are_checked() {
        let n = nodes(
            r#"{ "c": Combine(x: "uv.y", y: "time.x", z: 0.5), "s": Add(a: "c.rg", b: "uv.yx") }"#,
        );
        let c = compile(&n, &[("out".into(), &Input::from("s"))], &Plain).unwrap();
        assert_eq!(c.values["c"].ty, Ty::F3);
        assert!(c.body.contains("n_c.xy"), "{}", c.body);
        let n = nodes(r#"{ "bad": Add(a: "uv.z", b: 1.0) }"#);
        let e = compile(&n, &[("out".into(), &Input::from("bad"))], &Plain).unwrap_err();
        assert!(e.contains("has no `z`"), "{e}");
    }

    #[test]
    fn a_texture_or_an_eye_where_the_graph_has_none_is_refused() {
        let n = nodes(r#"{ "t": Texture(name: "_Main"), "f": Fresnel() }"#);
        assert!(compile(&n, &[("out".into(), &Input::from("t"))], &Plain)
            .unwrap_err()
            .contains("no textures"));
        assert!(compile(&n, &[("out".into(), &Input::from("f"))], &Plain)
            .unwrap_err()
            .contains("no eye"));
    }

    #[test]
    fn numbers_are_written_as_wgsl_floats() {
        assert_eq!(number(1.0).unwrap(), "1.0");
        assert_eq!(number(0.25).unwrap(), "0.25");
        assert!(number(f32::NAN).is_err());
    }
}
