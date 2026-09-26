//! The rest of the nodes, turned into WGSL: more of the math, logic,
//! derivatives, vectors, colour, normals, UVs, shapes and the scene —
//! Unity's Shader Graph library, as far as scrap's surfaces go.

use super::{BlendMode, Compare, Helper, Node, State, Ty, Value};

impl State<'_> {
    pub(super) fn more(
        &mut self,
        name: &str,
        node: &Node,
        args: &[Value],
    ) -> Result<Value, String> {
        use Node::*;
        let at = |what: String| format!("node `{name}` ({}): {what}", node.kind());
        let fields: Vec<&str> = node.inputs().iter().map(|(f, _)| *f).collect();
        // All of these inputs as one type: the widest, numbers spread.
        let same = |which: &[usize]| -> Result<(Ty, Vec<String>), String> {
            let ty = which.iter().map(|i| args[*i].ty).max().unwrap_or(Ty::F1);
            let mut out = Vec::new();
            for i in which {
                out.push(args[*i].to(ty).ok_or_else(|| {
                    at(format!(
                        "`{}` is {} where the others make {} — take part of it (`.xy`, `.xyz`) or give a number",
                        fields[*i],
                        args[*i].ty.said(),
                        ty.said()
                    ))
                })?);
            }
            Ok((ty, out))
        };
        // An input that has to be one type.
        let exactly = |i: usize, ty: Ty| -> Result<String, String> {
            if args[i].ty == ty {
                Ok(args[i].code.clone())
            } else if args[i].ty == Ty::F1 && ty != Ty::F1 {
                Ok(format!("{}({})", ty.wgsl(), args[i].code))
            } else {
                let hint = if ty == Ty::F3 && args[i].ty == Ty::F4 {
                    " — take `.rgb`"
                } else {
                    ""
                };
                Err(at(format!(
                    "`{}` has to be {}, not {}{hint}",
                    fields[i],
                    ty.said(),
                    args[i].ty.said()
                )))
            }
        };
        let number = |i: usize| exactly(i, Ty::F1);
        let one = |f: &str| Value::new(format!("{f}({})", args[0].code), args[0].ty);
        let spread = |n: &str, ty: Ty| {
            if ty == Ty::F1 {
                n.to_string()
            } else {
                format!("{}({n})", ty.wgsl())
            }
        };
        let pixels = |this: &Self| -> Result<(), String> {
            if this.context.derivatives() {
                Ok(())
            } else {
                Err(at("this graph has no pixels to tell a change between — only a material's surface has".into()))
            }
        };
        let v = match node {
            Reciprocal { .. } => Value::new(
                format!("({} / {})", spread("1.0", args[0].ty), args[0].code),
                args[0].ty,
            ),
            Log { .. } => one("log"),
            Truncate { .. } => one("trunc"),
            Tangent { .. } => one("tan"),
            Arcsin { .. } => one("asin"),
            Arccos { .. } => one("acos"),
            Arctan { .. } => one("atan"),
            Arctan2 { .. } => {
                let (ty, a) = same(&[0, 1])?;
                Value::new(format!("atan2({}, {})", a[0], a[1]), ty)
            }
            Radians { .. } => one("radians"),
            Degrees { .. } => one("degrees"),
            InverseLerp { .. } => {
                let (ty, a) = same(&[0, 1, 2])?;
                Value::new(format!("(({2} - {0}) / ({1} - {0}))", a[0], a[1], a[2]), ty)
            }
            RandomRange { .. } => {
                let seed = match args[0].ty {
                    Ty::F1 => format!("vec4<f32>({}, 0.0, 0.0, 0.0)", args[0].code),
                    Ty::F2 => format!("vec4<f32>({}, 0.0, 0.0)", args[0].code),
                    Ty::F3 => format!("vec4<f32>({}, 0.0)", args[0].code),
                    Ty::F4 => args[0].code.clone(),
                };
                let (ty, a) = same(&[1, 2])?;
                let r = format!("fract(sin(dot({seed}, vec4<f32>(12.9898, 78.233, 37.719, 4.581))) * 43758.5453)");
                Value::new(format!("mix({}, {}, {})", a[0], a[1], spread(&r, ty)), ty)
            }
            Comparison { op, .. } => {
                let (ty, a) = same(&[0, 1])?;
                let o = match op {
                    Compare::Less => "<",
                    Compare::LessOrEqual => "<=",
                    Compare::Equal => "==",
                    Compare::NotEqual => "!=",
                    Compare::Greater => ">",
                    Compare::GreaterOrEqual => ">=",
                };
                Value::new(
                    format!(
                        "select({}, {}, {} {o} {})",
                        spread("0.0", ty),
                        spread("1.0", ty),
                        a[0],
                        a[1]
                    ),
                    ty,
                )
            }
            Branch { .. } => {
                let (ty, a) = same(&[1, 2])?;
                let when = if args[0].ty == Ty::F1 {
                    format!("{} > 0.5", args[0].code)
                } else if args[0].ty == ty {
                    format!("{} > {}", args[0].code, spread("0.5", ty))
                } else {
                    return Err(at(format!(
                        "`when` is {} where `yes` and `no` are {}: give a number, or one the size of them",
                        args[0].ty.said(),
                        ty.said()
                    )));
                };
                Value::new(format!("select({}, {}, {when})", a[1], a[0]), ty)
            }
            And { .. } | Or { .. } => {
                let (ty, a) = same(&[0, 1])?;
                let half = spread("0.5", ty);
                let (x, y) = (
                    format!("step({half}, {})", a[0]),
                    format!("step({half}, {})", a[1]),
                );
                let code = if matches!(node, And { .. }) {
                    format!("({x} * {y})")
                } else {
                    format!("max({x}, {y})")
                };
                Value::new(code, ty)
            }
            Not { .. } => {
                let ty = args[0].ty;
                Value::new(
                    format!(
                        "({} - step({}, {}))",
                        spread("1.0", ty),
                        spread("0.5", ty),
                        args[0].code
                    ),
                    ty,
                )
            }
            Ddx { .. } | Ddy { .. } | Fwidth { .. } => {
                pixels(self)?;
                one(match node {
                    Ddx { .. } => "dpdx",
                    Ddy { .. } => "dpdy",
                    _ => "fwidth",
                })
            }
            Reflect { .. } => Value::new(
                format!("reflect({}, {})", exactly(0, Ty::F3)?, exactly(1, Ty::F3)?),
                Ty::F3,
            ),
            Refract { .. } => Value::new(
                format!(
                    "refract({}, {}, {})",
                    exactly(0, Ty::F3)?,
                    exactly(1, Ty::F3)?,
                    number(2)?
                ),
                Ty::F3,
            ),
            Project { .. } | Reject { .. } => {
                let (ty, a) = same(&[0, 1])?;
                if ty == Ty::F1 {
                    return Err(at("`of` and `onto` are vectors".into()));
                }
                let along = format!("({1} * (dot({0}, {1}) / dot({1}, {1})))", a[0], a[1]);
                let code = if matches!(node, Project { .. }) {
                    along
                } else {
                    format!("({} - {along})", a[0])
                };
                Value::new(code, ty)
            }
            RotateAboutAxis { .. } => {
                let (v, k, angle) = (exactly(0, Ty::F3)?, exactly(1, Ty::F3)?, number(2)?);
                self.helpers.push(Helper::Rotate);
                Value::new(format!("sg_rotate_about({v}, {k}, {angle})"), Ty::F3)
            }
            SphereMask { .. } => {
                let (_, a) = same(&[0, 1])?;
                let (r, h) = (number(2)?, number(3)?);
                Value::new(
                    format!(
                        "(1.0 - saturate((distance({}, {}) - {r}) / max(1.0 - {h}, 1e-5)))",
                        a[0], a[1]
                    ),
                    Ty::F1,
                )
            }
            Luminance { .. } => Value::new(
                format!(
                    "dot({}, vec3<f32>(0.2126729, 0.7151522, 0.0721750))",
                    exactly(0, Ty::F3)?
                ),
                Ty::F1,
            ),
            Blend { mode, .. } => {
                let (ty, a) = same(&[0, 1])?;
                let (base, blend) = (&a[0], &a[1]);
                let opacity = args[2]
                    .to(ty)
                    .ok_or_else(|| at("`opacity` is a number".into()))?;
                let (one_, half) = (spread("1.0", ty), spread("0.5", ty));
                let code = match mode {
                    BlendMode::Burn => format!("({one_} - ({one_} - {blend}) / {base})"),
                    BlendMode::Darken => format!("min({blend}, {base})"),
                    BlendMode::Difference => format!("abs({blend} - {base})"),
                    BlendMode::Dodge => format!("({base} / ({one_} - {blend}))"),
                    BlendMode::Divide => format!("({base} / ({blend} + {}))", spread("1e-9", ty)),
                    BlendMode::Exclusion => format!("({blend} + {base} - 2.0 * {blend} * {base})"),
                    BlendMode::HardLight => format!(
                        "mix(({one_} - 2.0 * ({one_} - {base}) * ({one_} - {blend})), (2.0 * {base} * {blend}), step({blend}, {half}))"
                    ),
                    BlendMode::Lighten => format!("max({blend}, {base})"),
                    BlendMode::LinearBurn => format!("({base} + {blend} - {one_})"),
                    BlendMode::LinearDodge => format!("({base} + {blend})"),
                    BlendMode::Multiply => format!("({base} * {blend})"),
                    BlendMode::Overlay => format!(
                        "mix(({one_} - 2.0 * ({one_} - {base}) * ({one_} - {blend})), (2.0 * {base} * {blend}), step({base}, {half}))"
                    ),
                    BlendMode::Screen => format!("({one_} - ({one_} - {blend}) * ({one_} - {base}))"),
                    BlendMode::SoftLight => format!(
                        "mix((2.0 * {base} * {blend} + {base} * {base} * ({one_} - 2.0 * {blend})), (sqrt({base}) * (2.0 * {blend} - {one_}) + 2.0 * {base} * ({one_} - {blend})), step({half}, {blend}))"
                    ),
                    BlendMode::Subtract => format!("({base} - {blend})"),
                    BlendMode::Overwrite => blend.clone(),
                };
                Value::new(format!("mix({base}, {code}, {opacity})"), ty)
            }
            Hue { .. } => {
                let (c, o) = (exactly(0, Ty::F3)?, number(1)?);
                self.helpers.push(Helper::Hsv);
                Value::new(
                    format!("sg_hsv_to_rgb(sg_rgb_to_hsv({c}) + vec3<f32>({o}, 0.0, 0.0))"),
                    Ty::F3,
                )
            }
            Saturation { .. } => {
                let (c, amount) = (exactly(0, Ty::F3)?, number(1)?);
                let luma = format!("dot({c}, vec3<f32>(0.2126729, 0.7151522, 0.0721750))");
                Value::new(format!("mix(vec3<f32>({luma}), {c}, {amount})"), Ty::F3)
            }
            Contrast { .. } => {
                let (c, amount) = (exactly(0, Ty::F3)?, number(1)?);
                // Unity's middle grey, in linear numbers.
                Value::new(
                    format!("(({c} - vec3<f32>(0.21763764)) * {amount} + vec3<f32>(0.21763764))"),
                    Ty::F3,
                )
            }
            Invert { .. } => match args[0].ty {
                Ty::F4 => Value::new(
                    format!("vec4<f32>(vec3<f32>(1.0) - {0}.rgb, {0}.a)", args[0].code),
                    Ty::F4,
                ),
                ty => Value::new(format!("({} - {})", spread("1.0", ty), args[0].code), ty),
            },
            ChannelMixer { .. } => {
                let c = exactly(0, Ty::F3)?;
                let (r, g, b) = (
                    exactly(1, Ty::F3)?,
                    exactly(2, Ty::F3)?,
                    exactly(3, Ty::F3)?,
                );
                Value::new(
                    format!("vec3<f32>(dot({c}, {r}), dot({c}, {g}), dot({c}, {b}))"),
                    Ty::F3,
                )
            }
            ReplaceColor { .. } => {
                let (c, from, to) = (
                    exactly(0, Ty::F3)?,
                    exactly(1, Ty::F3)?,
                    exactly(2, Ty::F3)?,
                );
                let (range, fuzz) = (number(3)?, number(4)?);
                Value::new(
                    format!("mix({to}, {c}, saturate((distance({from}, {c}) - {range}) / max({fuzz}, 1e-5)))"),
                    Ty::F3,
                )
            }
            RgbToHsv { .. } | HsvToRgb { .. } => {
                let c = exactly(0, Ty::F3)?;
                self.helpers.push(Helper::Hsv);
                let f = if matches!(node, RgbToHsv { .. }) {
                    "sg_rgb_to_hsv"
                } else {
                    "sg_hsv_to_rgb"
                };
                Value::new(format!("{f}({c})"), Ty::F3)
            }
            LinearToSrgb { .. } | SrgbToLinear { .. } => {
                let c = exactly(0, Ty::F3)?;
                self.helpers.push(Helper::Srgb);
                let f = if matches!(node, LinearToSrgb { .. }) {
                    "sg_to_srgb"
                } else {
                    "sg_to_linear"
                };
                Value::new(format!("{f}({c})"), Ty::F3)
            }
            Gradient { keys, .. } => {
                let t = number(0)?;
                let mut keys = keys.clone();
                keys.sort_by(|a, b| a.0.total_cmp(&b.0));
                let colour = |c: (f32, f32, f32)| -> Result<String, String> {
                    Ok(format!(
                        "vec3<f32>({}, {}, {})",
                        super::number(c.0)?,
                        super::number(c.1)?,
                        super::number(c.2)?
                    ))
                };
                let Some(first) = keys.first() else {
                    return Err(at("`keys` is empty: give it (where, colour) pairs, `[(0.0, (0.0, 0.0, 0.0)), (1.0, (1.0, 1.0, 1.0))]`".into()));
                };
                let mut code = colour(first.1).map_err(at)?;
                for pair in keys.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let span = (b.0 - a.0).max(1e-5);
                    code = format!(
                        "mix({code}, {}, saturate(({t} - {}) / {}))",
                        colour(b.1).map_err(at)?,
                        super::number(a.0).map_err(at)?,
                        super::number(span).map_err(at)?
                    );
                }
                Value::new(code, Ty::F3)
            }
            NormalStrength { .. } => {
                let (n, s, base) = (exactly(0, Ty::F3)?, number(1)?, exactly(2, Ty::F3)?);
                Value::new(format!("normalize(mix({base}, {n}, {s}))"), Ty::F3)
            }
            NormalBlend { .. } => {
                let (a, b, base) = (
                    exactly(0, Ty::F3)?,
                    exactly(1, Ty::F3)?,
                    exactly(2, Ty::F3)?,
                );
                Value::new(format!("normalize({a} + {b} - {base})"), Ty::F3)
            }
            NormalFromHeight { .. } => {
                pixels(self)?;
                let (h, s) = (number(0)?, number(1)?);
                self.context.normal_from_height(&h, &s).map_err(at)?
            }
            NormalFromTexture { name: texture, .. } => {
                let (uv, s) = (exactly(0, Ty::F2)?, number(1)?);
                self.context
                    .normal_from_texture(texture, &uv, &s)
                    .map_err(at)?
            }
            Triplanar { name: texture, .. } => {
                let (p, n, scale, sharp) = (
                    exactly(0, Ty::F3)?,
                    exactly(1, Ty::F3)?,
                    number(2)?,
                    number(3)?,
                );
                let q = format!("({p} * {scale})");
                let x = self
                    .context
                    .texture(texture, &format!("{q}.zy"))
                    .map_err(at)?
                    .code;
                let y = self
                    .context
                    .texture(texture, &format!("{q}.xz"))
                    .map_err(at)?
                    .code;
                let z = self
                    .context
                    .texture(texture, &format!("{q}.xy"))
                    .map_err(at)?
                    .code;
                let w = format!("pow(abs({n}), vec3<f32>({sharp}))");
                Value::new(
                    format!("(({x}) * ({w}).x + ({y}) * ({w}).y + ({z}) * ({w}).z) / max(({w}).x + ({w}).y + ({w}).z, 1e-5)"),
                    Ty::F4,
                )
            }
            Flipbook { .. } => {
                let uv = exactly(0, Ty::F2)?;
                let (c, r, f) = (number(1)?, number(2)?, number(3)?);
                let n = format!("floor(({f}) - ({c}) * ({r}) * floor(({f}) / (({c}) * ({r}))))");
                Value::new(
                    format!("(({uv}) + vec2<f32>({n} - ({c}) * floor({n} / ({c})), floor({n} / ({c})))) / vec2<f32>({c}, {r})"),
                    Ty::F2,
                )
            }
            PolarCoordinates { .. } => {
                let (uv, c) = (exactly(0, Ty::F2)?, exactly(1, Ty::F2)?);
                let (rs, ls) = (number(2)?, number(3)?);
                let d = format!("({uv} - {c})");
                Value::new(
                    format!("vec2<f32>(length({d}) * 2.0 * {rs}, atan2({d}.x, {d}.y) / 6.28318530718 * {ls})"),
                    Ty::F2,
                )
            }
            Twirl { .. } | Spherize { .. } | RadialShear { .. } => {
                let (uv, c) = (exactly(0, Ty::F2)?, exactly(1, Ty::F2)?);
                let s = number(2)?;
                let o = exactly(3, Ty::F2)?;
                let d = format!("({uv} - {c})");
                let code = match node {
                    Twirl { .. } => {
                        let a = format!("({s} * length({d}))");
                        format!("(vec2<f32>(cos({a}) * {d}.x - sin({a}) * {d}.y, sin({a}) * {d}.x + cos({a}) * {d}.y) + {c} + {o})")
                    }
                    Spherize { .. } => {
                        format!("({uv} + {d} * (dot({d}, {d}) * dot({d}, {d}) * {s}) + {o})")
                    }
                    _ => format!("({uv} + vec2<f32>({d}.y, -{d}.x) * (dot({d}, {d}) * {s}) + {o})"),
                };
                Value::new(code, Ty::F2)
            }
            Ellipse { .. } | Rectangle { .. } | RoundedRectangle { .. } | Polygon { .. } => {
                pixels(self)?;
                let uv = exactly(0, Ty::F2)?;
                let code = match node {
                    Ellipse { .. } => {
                        let (w, h) = (number(1)?, number(2)?);
                        let d = format!("length(({uv} * 2.0 - 1.0) / vec2<f32>({w}, {h}))");
                        format!("saturate((1.0 - {d}) / max(fwidth({d}), 1e-6))")
                    }
                    Rectangle { .. } => {
                        let (w, h) = (number(1)?, number(2)?);
                        let d = format!("(abs({uv} * 2.0 - 1.0) - vec2<f32>({w}, {h}))");
                        let e =
                            format!("(vec2<f32>(1.0) - {d} / max(fwidth({d}), vec2<f32>(1e-6)))");
                        format!("saturate(min({e}.x, {e}.y))")
                    }
                    RoundedRectangle { .. } => {
                        let (w, h, r) = (number(1)?, number(2)?, number(3)?);
                        let radius =
                            format!("max(min(min(abs({r} * 2.0), abs({w})), abs({h})), 1e-5)");
                        let q = format!(
                            "(abs({uv} * 2.0 - 1.0) - vec2<f32>({w}, {h}) + vec2<f32>({radius}))"
                        );
                        let d = format!("(length(max(vec2<f32>(0.0), {q})) / {radius})");
                        format!("saturate((1.0 - {d}) / max(fwidth({d}), 1e-6))")
                    }
                    _ => {
                        let (sides, w, h) = (number(1)?, number(2)?, number(3)?);
                        self.helpers.push(Helper::Polygon);
                        format!("sg_polygon({uv}, {sides}, {w}, {h})")
                    }
                };
                Value::new(code, Ty::F1)
            }
            SimpleNoise { .. } => {
                let (p, scale) = (exactly(0, Ty::F2)?, number(1)?);
                self.helpers.push(Helper::ValueNoise);
                Value::new(format!("sg_simple_noise({p} * {scale})"), Ty::F1)
            }
            SceneColor { .. } => {
                let p = exactly(0, Ty::F2)?;
                self.context.scene_color(&p).map_err(at)?
            }
            Subgraph { name: sub, .. } => {
                return Err(at(format!("subgraph `{sub}` was not put in: this graph was compiled without its subgraphs")));
            }
            other => unreachable!("`{}` is compiled in expr.rs", other.kind()),
        };
        Ok(v)
    }
}

pub(super) const HSV: &str = "fn sg_rgb_to_hsv(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4<f32>(c.bg, k.wz), vec4<f32>(c.gb, k.xy), step(c.b, c.g));
    let q = mix(vec4<f32>(p.xyw, c.r), vec4<f32>(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1.0e-10;
    return vec3<f32>(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}
fn sg_hsv_to_rgb(c: vec3<f32>) -> vec3<f32> {
    let k = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(vec3<f32>(c.x) + k.xyz) * 6.0 - vec3<f32>(k.w));
    return c.z * mix(vec3<f32>(k.x), saturate(p - vec3<f32>(k.x)), c.y);
}
";

pub(super) const SRGB: &str = "fn sg_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let c0 = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(c0, vec3<f32>(1.0 / 2.4)) - 0.055, c0 * 12.92, c0 <= vec3<f32>(0.0031308));
}
fn sg_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
";

pub(super) const ROTATE: &str =
    "fn sg_rotate_about(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let k = normalize(axis);
    let c = cos(angle);
    let s = sin(angle);
    return v * c + cross(k, v) * s + k * dot(k, v) * (1.0 - c);
}
";

pub(super) const POLYGON: &str =
    "fn sg_polygon(uv: vec2<f32>, sides: f32, width: f32, height: f32) -> f32 {
    let pi = 3.14159265359;
    let w = width * cos(pi / sides);
    let h = height * cos(pi / sides);
    var p = (uv * 2.0 - 1.0) / vec2<f32>(w, h);
    p.y = -p.y;
    let angle = atan2(p.x, p.y);
    let r = 2.0 * pi / sides;
    let d = cos(floor(0.5 + angle / r) * r - angle) * length(p);
    return saturate((1.0 - d) / max(fwidth(d), 1e-6));
}
";

pub(super) const VALUE_NOISE: &str = "fn sg_value_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}
fn sg_value(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = sg_value_hash(i);
    let b = sg_value_hash(i + vec2<f32>(1.0, 0.0));
    let c = sg_value_hash(i + vec2<f32>(0.0, 1.0));
    let d = sg_value_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn sg_simple_noise(p: vec2<f32>) -> f32 {
    var t = 0.0;
    for (var k = 0; k < 3; k++) {
        let frequency = pow(2.0, f32(k));
        let amplitude = pow(0.5, f32(3 - k));
        t += sg_value(p / frequency) * amplitude;
    }
    return t;
}
";
