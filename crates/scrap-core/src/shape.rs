//! The shape of a type, read off its `Deserialize`: field names, what each
//! holds, an enum's variants. What lets an editor that does not link the
//! game still know that `door` is `(open_angle: number, locked: bool)`.
//!
//! No derive and no reflection crate: the type is asked to deserialize
//! itself from a tracer that answers every question with a placeholder
//! and writes the question down — a struct says its field names, an enum
//! all its variants' names, a number that it is a number. What serde never
//! asks (a value read as "anything") is [`Shape::Any`].

use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, IntoDeserializer, Visitor};
use serde::{Deserialize, Serialize};

/// What a value of a type looks like, as written in RON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Bool,
    Int,
    Float,
    /// A number from 0 to 1 — an amount, a share, an opacity: what an
    /// editor draws as a slider. Serde cannot tell it from any number; the
    /// type says so itself ([`with_fractions`]).
    Fraction,
    Char,
    Text,
    Unit,
    Option(Box<Shape>),
    List(Box<Shape>),
    Map(Box<Shape>, Box<Shape>),
    Tuple(Vec<Shape>),
    /// A struct's fields, in order.
    Struct(Vec<(String, Shape)>),
    /// Every variant's name.
    Enum(Vec<String>),
    /// An enum some of whose variants hold something — `Crate(Food)`,
    /// `Box(half: …)` — each variant with what it holds (`Unit` for a
    /// bare one). An enum of bare names only is [`Shape::Enum`].
    Tagged(Vec<(String, Shape)>),
    /// Serde asks nothing that says: raw RON, a value of any shape.
    Any,
    /// A link to another entity of the scene ([`crate::EntityRef`]): what
    /// an editor shows as a picker.
    Entity,
    /// A link to an asset of a kind — `model`, `prefab`, `sound`… — by the
    /// typed links in [`crate::links`], or an [`crate::AssetLink`] in a field
    /// whose name says the kind ([`crate::links::kind_of_field`]): a picker
    /// of that kind's assets.
    Asset(String),
    /// Any one of these, told apart by how it is written — serde's
    /// untagged enum: a material by name, or one spelled out. Serde asks
    /// nothing that says so; a type says it itself ([`crate::parts::Part::shape`]).
    OneOf(Vec<Shape>),
    /// An entity of the scene by its bare id (`to: "4f1c…"`), where
    /// [`Shape::Entity`] is a component's `EntityRef("4f1c…")`.
    EntityId,
    /// An asset by its ID alone, of the kind its field's name says — a
    /// material's `base_map` a texture, its `shader` a shader.
    AssetId(String),
}

/// The shape of `T` as the value of a field called `field`: a link to an
/// asset in it is of the kind the name says (`model`, `clip`…).
pub fn of_field<'de, T: Deserialize<'de>>(field: &'static str) -> Shape {
    let before = FIELD.replace(field);
    let shape = of::<T>();
    FIELD.set(before);
    shape
}

std::thread_local! {
    /// The name of the field being traced: what a link in it is a link to.
    static FIELD: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
}

/// What text read by a visitor expecting `expecting` is: an entity's id, an
/// asset's, or text.
fn text_shape(expecting: &str) -> Shape {
    if expecting == crate::id::ID_EXPECTING {
        Shape::EntityId
    } else if expecting == crate::asset::ID_EXPECTING {
        Shape::AssetId(
            crate::links::kind_of_field(FIELD.get())
                .unwrap_or("asset")
                .to_string(),
        )
    } else {
        Shape::Text
    }
}

/// A visitor's `expecting`, as text.
struct Expecting<'a, V>(&'a V);

impl<'de, V: Visitor<'de>> fmt::Display for Expecting<'_, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }
}

/// The shape of `T`: every enum in it with what each of its variants
/// holds, found by tracing `T` again with that variant chosen (a trace
/// takes one variant of each enum, the first, as a value can hold only
/// one).
pub fn of<'de, T: Deserialize<'de>>() -> Shape {
    let (mut shape, mut sites) = trace::<T>(Vec::new());
    let mut done: Vec<Vec<String>> = Vec::new();
    // Enums inside a variant's content are found when that variant is
    // traced; their own variants are traced with it chosen.
    let mut i = 0;
    while i < sites.len() && i < SITES_AT_MOST {
        let site = sites[i].clone();
        i += 1;
        if done.contains(&site.path) {
            continue;
        }
        done.push(site.path.clone());
        let mut contents = Vec::new();
        for variant in &site.variants {
            let mut steer = site.steer.clone();
            steer.push((site.path.clone(), variant.clone()));
            let (_, found) = trace::<T>(steer.clone());
            let content = CAUGHT
                .with(|c| c.borrow_mut().take())
                .unwrap_or(Shape::Unit);
            for mut nested in found {
                if nested.path.len() > site.path.len() && nested.path.starts_with(&site.path) {
                    nested.steer = steer.clone();
                    sites.push(nested);
                }
            }
            contents.push((variant.clone(), content));
        }
        if contents.iter().any(|(_, c)| *c != Shape::Unit) {
            replace_at(&mut shape, &site.path, Shape::Tagged(contents));
        }
    }
    shape
}

/// Enums a trace looks into, at most: a type that holds itself stops.
const SITES_AT_MOST: usize = 64;

/// An enum met in a trace: where, its variants, and the variants chosen
/// on the way there.
#[derive(Clone)]
struct Site {
    path: Vec<String>,
    variants: Vec<String>,
    steer: Vec<(Vec<String>, String)>,
}

std::thread_local! {
    /// Where in the value the trace is: field names, item places, the
    /// variant whose content it is in.
    static PATH: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Which variant to take at which enum, instead of the first.
    static STEER: std::cell::RefCell<Vec<(Vec<String>, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The enums met.
    static SITES: std::cell::RefCell<Vec<Site>> = const { std::cell::RefCell::new(Vec::new()) };
    /// What the last steered variant held.
    static CAUGHT: std::cell::RefCell<Option<Shape>> = const { std::cell::RefCell::new(None) };
}

/// One trace of `T`, the variants in `steer` taken: its shape, and the
/// enums it met.
fn trace<'de, T: Deserialize<'de>>(steer: Vec<(Vec<String>, String)>) -> (Shape, Vec<Site>) {
    let saved = (
        PATH.with(|p| std::mem::take(&mut *p.borrow_mut())),
        STEER.with(|s| std::mem::replace(&mut *s.borrow_mut(), steer)),
        SITES.with(|s| std::mem::take(&mut *s.borrow_mut())),
    );
    CAUGHT.with(|c| c.borrow_mut().take());
    let mut out = Shape::Any;
    let _ = T::deserialize(Tracer {
        out: &mut out,
        depth: 0,
    });
    let sites = SITES.with(|s| std::mem::replace(&mut *s.borrow_mut(), saved.2));
    PATH.with(|p| *p.borrow_mut() = saved.0);
    STEER.with(|s| *s.borrow_mut() = saved.1);
    (out, sites)
}

/// Run `f` a step further into the value.
fn at<R>(step: impl Into<String>, f: impl FnOnce() -> R) -> R {
    PATH.with(|p| p.borrow_mut().push(step.into()));
    let r = f();
    PATH.with(|p| p.borrow_mut().pop());
    r
}

/// `shape` with what is at `path` — steps as the trace takes them —
/// replaced by `with`.
fn replace_at(shape: &mut Shape, path: &[String], with: Shape) {
    let Some((step, rest)) = path.split_first() else {
        *shape = with;
        return;
    };
    match shape {
        Shape::Option(inner) => replace_at(inner, path, with),
        Shape::Struct(fields) => {
            if let Some((_, s)) = fields.iter_mut().find(|(k, _)| k == step) {
                replace_at(s, rest, with);
            }
        }
        Shape::List(item) => replace_at(item, rest, with),
        Shape::Tuple(items) => {
            if let Some(s) = step.parse::<usize>().ok().and_then(|i| items.get_mut(i)) {
                replace_at(s, rest, with);
            }
        }
        Shape::Map(_, value) => replace_at(value, rest, with),
        Shape::Tagged(variants) => {
            if let Some((_, s)) = variants.iter_mut().find(|(v, _)| v == step) {
                replace_at(s, rest, with);
            }
        }
        _ => {}
    }
}

/// `shape` with the numbers at `paths` said to be 0 to 1
/// ([`Shape::Fraction`]). A path is field names with dots between —
/// `rain`, `bloom.scatter` — stepping through an option, a list's items,
/// every variant of an enum and every shape of an untagged one, so
/// `angle` is the `angle` of whichever variant has one.
///
/// A path that names no number is a typo in the type's declaration: it
/// panics in a debug build, where every part's shape is traced by a test.
pub fn with_fractions(mut shape: Shape, paths: &[&str]) -> Shape {
    for path in paths {
        let steps: Vec<&str> = path.split('.').collect();
        let found = mark_fraction(&mut shape, &steps);
        debug_assert!(found, "`{path}` is no number of {shape}");
    }
    shape
}

fn mark_fraction(shape: &mut Shape, steps: &[&str]) -> bool {
    match shape {
        Shape::Float if steps.is_empty() => {
            *shape = Shape::Fraction;
            true
        }
        Shape::Fraction if steps.is_empty() => true,
        Shape::Option(inner) | Shape::List(inner) => mark_fraction(inner, steps),
        Shape::Struct(fields) => {
            let Some((step, rest)) = steps.split_first() else {
                return false;
            };
            fields
                .iter_mut()
                .find(|(k, _)| k == step)
                .is_some_and(|(_, s)| mark_fraction(s, rest))
        }
        Shape::Tagged(variants) => variants
            .iter_mut()
            .fold(false, |found, (_, s)| mark_fraction(s, steps) | found),
        Shape::OneOf(shapes) => shapes
            .iter_mut()
            .fold(false, |found, s| mark_fraction(s, steps) | found),
        _ => false,
    }
}

/// What each variant of the enum `T` holds, by name: `Unit` for a bare
/// one, the fields of one written `Box(half: …)`. Empty when `T` is not an
/// enum. [`Shape::Enum`] names the variants only; this is what an editor
/// needs to switch a value to another variant with fields to fill in.
pub fn variants_of<'de, T: Deserialize<'de>>() -> Vec<(String, Shape)> {
    match of::<T>() {
        Shape::Tagged(variants) => variants,
        Shape::Enum(names) => names.into_iter().map(|n| (n, Shape::Unit)).collect(),
        _ => Vec::new(),
    }
}

impl Shape {
    /// A value of this shape in RON, as short as it can be written: what a
    /// component added in an inspector starts as.
    pub fn example(&self) -> String {
        match self {
            Shape::Bool => "false".into(),
            Shape::Int => "0".into(),
            Shape::Float | Shape::Fraction => "0.0".into(),
            Shape::Char => "'a'".into(),
            Shape::Text => "\"\"".into(),
            Shape::Unit | Shape::Any => "()".into(),
            Shape::Option(_) => "None".into(),
            Shape::List(_) => "[]".into(),
            Shape::Map(_, _) => "{}".into(),
            Shape::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(Shape::example)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Shape::Struct(fields) => format!(
                "({})",
                fields
                    .iter()
                    .map(|(name, shape)| format!("{name}: {}", shape.example()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Shape::Enum(variants) => variants.first().cloned().unwrap_or_default(),
            Shape::Tagged(variants) => match variants.first() {
                Some((name, Shape::Unit)) => name.clone(),
                Some((name, content @ (Shape::Struct(_) | Shape::Tuple(_)))) => {
                    format!("{name}{}", content.example())
                }
                Some((name, content)) => format!("{name}({})", content.example()),
                None => String::new(),
            },
            Shape::OneOf(shapes) => shapes.first().map(Shape::example).unwrap_or_default(),
            // No entity: the id no entity has.
            Shape::EntityId => "\"0\"".into(),
            Shape::AssetId(_) => "\"0\"".into(),
            Shape::Entity => format!("{}(\"\")", crate::EntityRef::NAME),
            Shape::Asset(kind) => {
                let name = crate::links::LINK_KINDS
                    .iter()
                    .find(|(_, k)| k == kind)
                    .map_or("ModelLink", |(n, _)| n);
                format!("{name}(\"\")")
            }
        }
    }

    /// What is wrong with a RON value for this shape, in words: a field it
    /// does not have (with the nearest it does), a number where a flag
    /// goes. Fields left out are not wrong — they may have defaults.
    pub fn problems(&self, text: &str) -> Vec<String> {
        match ron::from_str::<ron::Value>(text) {
            Ok(value) => {
                let mut out = Vec::new();
                self.check(&value, "", &mut out);
                out
            }
            Err(e) => vec![e.to_string()],
        }
    }

    fn check(&self, value: &ron::Value, at: &str, out: &mut Vec<String>) {
        use ron::Value as V;
        let here = if at.is_empty() {
            "the value".to_string()
        } else {
            format!("`{at}`")
        };
        let wrong = |out: &mut Vec<String>, wanted: &str| {
            out.push(format!("{here} is {wanted}, not {}", describe(value)))
        };
        match (self, value) {
            (
                Shape::Any
                | Shape::Enum(_)
                | Shape::Entity
                | Shape::Asset(_)
                | Shape::OneOf(_)
                | Shape::EntityId
                | Shape::AssetId(_)
                | Shape::Tagged(_),
                _,
            ) => {}
            (Shape::Bool, V::Bool(_)) => {}
            (Shape::Bool, _) => wrong(out, "true or false"),
            (Shape::Int, V::Number(n)) if n.into_f64().fract() == 0.0 => {}
            (Shape::Int, _) => wrong(out, "a whole number"),
            (Shape::Float | Shape::Fraction, V::Number(_)) => {}
            (Shape::Float, _) => wrong(out, "a number"),
            (Shape::Fraction, _) => wrong(out, "a number from 0 to 1"),
            (Shape::Text, V::String(_)) => {}
            (Shape::Text, _) => wrong(out, "text in quotes"),
            (Shape::Option(inner), V::Option(Some(v))) => inner.check(v, at, out),
            (Shape::Option(_), V::Option(None)) => {}
            (Shape::Option(inner), v) => inner.check(v, at, out),
            (Shape::List(item), V::Seq(items)) => {
                for (i, v) in items.iter().enumerate() {
                    item.check(v, &format!("{at}[{i}]"), out);
                }
            }
            (Shape::List(_), _) => wrong(out, "a list [..]"),
            (Shape::Struct(fields), V::Map(map)) => {
                let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
                for (key, v) in map.iter() {
                    let V::String(key) = key else { continue };
                    let path = if at.is_empty() {
                        key.clone()
                    } else {
                        format!("{at}.{key}")
                    };
                    match fields.iter().find(|(n, _)| n == key) {
                        Some((_, shape)) => shape.check(v, &path, out),
                        None => out.push(format!(
                            "no field `{path}`{} — there are {}",
                            crate::spelling::closest(key, names.iter().copied())
                                .map(|n| format!(" (did you mean `{n}`?)"))
                                .unwrap_or_default(),
                            names.join(", ")
                        )),
                    }
                }
            }
            (Shape::Struct(fields), V::Unit) if fields.is_empty() => {}
            (Shape::Struct(_), V::Unit) => {}
            (Shape::Struct(_), _) => wrong(out, "(field: value, …)"),
            _ => {}
        }
    }
}

fn describe(value: &ron::Value) -> &'static str {
    use ron::Value as V;
    match value {
        V::Bool(_) => "true or false",
        V::Number(_) => "a number",
        V::String(_) | V::Char(_) => "text",
        V::Seq(_) => "a list",
        V::Map(_) => "a struct or map",
        V::Option(_) => "an option",
        V::Unit => "()",
        V::Bytes(_) => "bytes",
    }
}

impl fmt::Display for Shape {
    /// The way an inspector labels it: `(open_angle: number, locked: bool)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Shape::Bool => write!(f, "bool"),
            Shape::Int => write!(f, "whole number"),
            Shape::Float => write!(f, "number"),
            Shape::Fraction => write!(f, "number 0 to 1"),
            Shape::Char => write!(f, "char"),
            Shape::Text => write!(f, "text"),
            Shape::Unit => write!(f, "()"),
            Shape::Any => write!(f, "any"),
            Shape::Option(inner) => write!(f, "{inner} or None"),
            Shape::List(item) => write!(f, "[{item}]"),
            Shape::Map(k, v) => write!(f, "{{{k}: {v}}}"),
            Shape::Tuple(items) => {
                let items: Vec<String> = items.iter().map(ToString::to_string).collect();
                write!(f, "({})", items.join(", "))
            }
            Shape::Struct(fields) => {
                let fields: Vec<String> = fields.iter().map(|(n, s)| format!("{n}: {s}")).collect();
                write!(f, "({})", fields.join(", "))
            }
            Shape::Enum(variants) => write!(f, "{}", variants.join(" | ")),
            Shape::Tagged(variants) => {
                let variants: Vec<String> = variants
                    .iter()
                    .map(|(v, s)| match s {
                        Shape::Unit => v.clone(),
                        Shape::Struct(_) | Shape::Tuple(_) => format!("{v}{s}"),
                        s => format!("{v}({s})"),
                    })
                    .collect();
                write!(f, "{}", variants.join(" | "))
            }
            Shape::OneOf(shapes) => {
                let shapes: Vec<String> = shapes.iter().map(ToString::to_string).collect();
                write!(f, "{}", shapes.join(" or "))
            }
            Shape::Entity | Shape::EntityId => write!(f, "entity"),
            Shape::AssetId(kind) => write!(f, "{kind} id"),
            Shape::Asset(kind) => write!(f, "{kind}"),
        }
    }
}

// --- the tracer ---------------------------------------------------------

#[derive(Debug)]
struct Stop(String);

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Stop {}
impl de::Error for Stop {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Stop(msg.to_string())
    }
}

/// Deep enough for any component; a type that holds itself stops here.
const DEPTH: usize = 12;

struct Tracer<'a> {
    out: &'a mut Shape,
    depth: usize,
}

impl<'a> Tracer<'a> {
    fn child<'b>(&self, out: &'b mut Shape) -> Tracer<'b> {
        Tracer {
            out,
            depth: self.depth + 1,
        }
    }
}

macro_rules! simple {
    ($($method:ident => $shape:expr, $visit:ident($($value:expr)?);)*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
            *self.out = $shape;
            visitor.$visit($($value)?)
        }
    )*};
}

impl<'de> Deserializer<'de> for Tracer<'_> {
    type Error = Stop;

    simple! {
        deserialize_bool => Shape::Bool, visit_bool(false);
        deserialize_i8 => Shape::Int, visit_i8(0);
        deserialize_i16 => Shape::Int, visit_i16(0);
        deserialize_i32 => Shape::Int, visit_i32(0);
        deserialize_i64 => Shape::Int, visit_i64(0);
        deserialize_u8 => Shape::Int, visit_u8(0);
        deserialize_u16 => Shape::Int, visit_u16(0);
        deserialize_u32 => Shape::Int, visit_u32(0);
        deserialize_u64 => Shape::Int, visit_u64(0);
        deserialize_f32 => Shape::Float, visit_f32(0.0);
        deserialize_f64 => Shape::Float, visit_f64(0.0);
        deserialize_char => Shape::Char, visit_char('a');
        // `0`, not nothing: text a type parses further — an entity's or an
        // asset's id — reads it, and the trace goes on past it.

        deserialize_bytes => Shape::Any, visit_bytes(&[]);
        deserialize_byte_buf => Shape::Any, visit_byte_buf(Vec::new());
        deserialize_unit => Shape::Unit, visit_unit();
        deserialize_identifier => Shape::Text, visit_str("");
        deserialize_ignored_any => Shape::Any, visit_unit();
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        *self.out = text_shape(&Expecting(&visitor).to_string());
        // `1`, not nothing: an entity's or an asset's id reads it, and the
        // trace goes on past it.
        visitor.visit_str("1")
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        *self.out = text_shape(&Expecting(&visitor).to_string());
        visitor.visit_string("1".to_string())
    }

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        // A link to an asset reads anything a link is written as: an
        // `AssetLink` says so by what it expects, and its kind is the
        // field's it is the value of.
        if Expecting(&visitor).to_string() == crate::links::LINK_EXPECTING {
            *self.out = Shape::Asset(
                crate::links::kind_of_field(FIELD.get())
                    .unwrap_or("asset")
                    .to_string(),
            );
            return visitor.visit_str("");
        }
        *self.out = Shape::Any;
        visitor.visit_unit()
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        if self.depth > DEPTH {
            *self.out = Shape::Option(Box::new(Shape::Any));
            return visitor.visit_none();
        }
        let mut inner = Shape::Any;
        let value = visitor.visit_some(self.child(&mut inner));
        *self.out = Shape::Option(Box::new(inner));
        value
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Stop> {
        *self.out = Shape::Unit;
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Stop> {
        let Tracer { out, depth } = self;
        // A typed link reads its inside as anything at all, which a tracer
        // cannot answer: its inside is an empty name, and its shape its kind.
        if let Some((_, kind)) = crate::links::LINK_KINDS.iter().find(|(n, _)| *n == name) {
            *out = Shape::Asset(kind.to_string());
            return visitor.visit_newtype_struct(IntoDeserializer::<Stop>::into_deserializer(""));
        }
        let value = visitor.visit_newtype_struct(Tracer {
            out: &mut *out,
            depth,
        });
        // A link is text inside; what it means is its name.
        if name == crate::EntityRef::NAME {
            *out = Shape::Entity;
        }
        value
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        let mut item = Shape::Any;
        let count = usize::from(self.depth <= DEPTH);
        let value = visitor.visit_seq(Items {
            shapes: std::slice::from_mut(&mut item),
            left: count,
            depth: self.depth + 1,
            repeat: true,
        });
        *self.out = Shape::List(Box::new(item));
        value
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Stop> {
        let mut items = vec![Shape::Any; len];
        let value = visitor.visit_seq(Items {
            shapes: &mut items,
            left: len,
            depth: self.depth + 1,
            repeat: false,
        });
        *self.out = Shape::Tuple(items);
        value
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Stop> {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Stop> {
        let (mut key, mut value_shape) = (Shape::Any, Shape::Any);
        let value = visitor.visit_map(Entries {
            key: &mut key,
            value: &mut value_shape,
            left: usize::from(self.depth <= DEPTH),
            depth: self.depth + 1,
        });
        *self.out = Shape::Map(Box::new(key), Box::new(value_shape));
        value
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Stop> {
        let mut shapes = vec![Shape::Any; fields.len()];
        let value = visitor.visit_map(Fields {
            names: fields,
            shapes: &mut shapes,
            next: 0,
            depth: self.depth + 1,
        });
        *self.out = Shape::Struct(fields.iter().map(|f| f.to_string()).zip(shapes).collect());
        value
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Stop> {
        *self.out = Shape::Enum(variants.iter().map(|v| v.to_string()).collect());
        let path = PATH.with(|p| p.borrow().clone());
        let steered = STEER.with(|s| {
            s.borrow()
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, v)| v.clone())
        });
        if self.depth <= DEPTH {
            SITES.with(|s| {
                s.borrow_mut().push(Site {
                    path: path.clone(),
                    variants: variants.iter().map(|v| v.to_string()).collect(),
                    steer: Vec::new(),
                })
            });
        }
        // The steered one when this is the enum being looked into, and
        // what it holds caught: the steering's last step is its site.
        let catch = STEER.with(|s| s.borrow().last().is_some_and(|(p, _)| *p == path));
        let name = steered
            .and_then(|v| variants.iter().find(|n| **n == v).copied())
            .or(variants.first().copied())
            .unwrap_or("");
        visitor.visit_enum(Variant {
            name,
            depth: self.depth + 1,
            catch,
        })
    }
}

struct Items<'a> {
    shapes: &'a mut [Shape],
    left: usize,
    depth: usize,
    /// One shape for every item: a list, not a tuple.
    repeat: bool,
}

impl<'de> de::SeqAccess<'de> for Items<'_> {
    type Error = Stop;
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Stop> {
        if self.left == 0 {
            return Ok(None);
        }
        let index = if self.repeat {
            0
        } else {
            self.shapes.len() - self.left
        };
        self.left -= 1;
        let out = &mut self.shapes[index];
        let depth = self.depth;
        at(index.to_string(), || {
            seed.deserialize(Tracer { out, depth })
        })
        .map(Some)
    }
}

struct Entries<'a> {
    key: &'a mut Shape,
    value: &'a mut Shape,
    left: usize,
    depth: usize,
}

impl<'de> de::MapAccess<'de> for Entries<'_> {
    type Error = Stop;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Stop> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        seed.deserialize(Tracer {
            out: self.key,
            depth: self.depth,
        })
        .map(Some)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Stop> {
        seed.deserialize(Tracer {
            out: self.value,
            depth: self.depth,
        })
    }
}

struct Fields<'a> {
    names: &'static [&'static str],
    shapes: &'a mut [Shape],
    next: usize,
    depth: usize,
}

impl<'de> de::MapAccess<'de> for Fields<'_> {
    type Error = Stop;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Stop> {
        let Some(name) = self.names.get(self.next) else {
            return Ok(None);
        };
        seed.deserialize(name.into_deserializer()).map(Some)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Stop> {
        let index = self.next;
        self.next += 1;
        let before = FIELD.replace(self.names[index]);
        let (out, depth) = (&mut self.shapes[index], self.depth);
        let value = at(self.names[index], || {
            seed.deserialize(Tracer { out, depth })
        });
        FIELD.set(before);
        value
    }
}

struct Variant {
    name: &'static str,
    depth: usize,
    /// What it holds is what a steered trace looks for.
    catch: bool,
}

impl Variant {
    /// Trace what the variant holds, a step in by its name; keep it when
    /// it is what the trace is for.
    fn content<R>(&self, f: impl FnOnce(&mut Shape) -> R) -> R {
        let mut content = Shape::Unit;
        let r = at(self.name, || f(&mut content));
        if self.catch {
            CAUGHT.with(|c| *c.borrow_mut() = Some(content));
        }
        r
    }
}

impl<'de> de::EnumAccess<'de> for Variant {
    type Error = Stop;
    type Variant = Self;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Stop> {
        let value = seed.deserialize(self.name.into_deserializer())?;
        Ok((value, self))
    }
}

impl<'de> de::VariantAccess<'de> for Variant {
    type Error = Stop;
    fn unit_variant(self) -> Result<(), Stop> {
        self.content(|_| Ok(()))
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Stop> {
        let depth = self.depth;
        self.content(|out| seed.deserialize(Tracer { out, depth }))
    }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Stop> {
        let depth = self.depth;
        self.content(|out| Tracer { out, depth }.deserialize_tuple(len, visitor))
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Stop> {
        let depth = self.depth;
        self.content(|out| Tracer { out, depth }.deserialize_struct("", fields, visitor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Door {
        open_angle: f32,
        #[serde(default)]
        locked: bool,
        kind: Kind,
        name: Option<String>,
        tags: Vec<String>,
        hinge: glam::Vec3,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    enum Kind {
        Wood,
        Iron { weight: f32 },
    }

    #[test]
    fn a_link_to_an_entity_is_its_own_shape() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Door {
            switch: crate::EntityRef,
            open: bool,
        }
        let Shape::Struct(fields) = of::<Door>() else {
            panic!("a struct")
        };
        assert_eq!(fields[0], ("switch".to_string(), Shape::Entity));
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Spawner {
            what: crate::PrefabLink,
            sound: crate::SoundLink,
        }
        let Shape::Struct(spawner) = of::<Spawner>() else {
            panic!("a struct")
        };
        assert_eq!(spawner[0].1, Shape::Asset("prefab".into()));
        assert_eq!(spawner[1].1, Shape::Asset("sound".into()));
        assert_eq!(Shape::Asset("prefab".into()).example(), r#"PrefabLink("")"#);
        assert_eq!(fields[1].1, Shape::Bool);
        assert_eq!(Shape::Entity.example(), r#"EntityRef("")"#);
        let example: crate::EntityRef = ron::from_str(&Shape::Entity.example()).unwrap();
        assert_eq!(example, crate::EntityRef(None));
    }

    #[test]
    fn a_struct_says_its_fields_and_an_enum_every_variant() {
        let shape = of::<Door>();
        let Shape::Struct(fields) = &shape else {
            panic!("{shape:?}");
        };
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["open_angle", "locked", "kind", "name", "tags", "hinge"]
        );
        assert_eq!(fields[0].1, Shape::Float);
        assert_eq!(
            fields[2].1,
            Shape::Tagged(vec![
                ("Wood".into(), Shape::Unit),
                (
                    "Iron".into(),
                    Shape::Struct(vec![("weight".into(), Shape::Float)])
                ),
            ])
        );
        assert_eq!(fields[3].1, Shape::Option(Box::new(Shape::Text)));
        assert_eq!(fields[4].1, Shape::List(Box::new(Shape::Text)));
        assert_eq!(
            shape.to_string(),
            "(open_angle: number, locked: bool, kind: Wood | Iron(weight: number), name: text or None, tags: [text], hinge: (number, number, number))"
        );
        let example = shape.example();
        assert!(ron::from_str::<Door>(&example).is_ok(), "{example}");
    }

    #[test]
    fn a_link_to_an_asset_is_of_the_kind_its_field_names() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Emitter {
            model: crate::AssetLink,
            clip: Option<crate::AssetLink>,
            rate: f32,
        }
        let Shape::Struct(fields) = of::<Emitter>() else {
            panic!("a struct")
        };
        assert_eq!(fields[0].1, Shape::Asset("model".into()));
        assert_eq!(
            fields[1].1,
            Shape::Option(Box::new(Shape::Asset("sound".into())))
        );
        assert_eq!(fields[2].1, Shape::Float, "traced on past the links");
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Joint {
            to: crate::EntityId,
            base_map: Option<crate::asset::AssetId>,
            name: String,
        }
        assert_eq!(
            of::<Joint>(),
            Shape::Struct(vec![
                ("to".into(), Shape::EntityId),
                (
                    "base_map".into(),
                    Shape::Option(Box::new(Shape::AssetId("texture".into())))
                ),
                ("name".into(), Shape::Text),
            ])
        );
        assert_eq!(
            of_field::<crate::AssetLink>("material"),
            Shape::Asset("material".into())
        );
    }

    #[test]
    fn an_enum_inside_a_variant_says_its_own_variants() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        enum Food {
            Cabbage,
            Tomato,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        enum Kind {
            Counter,
            Crate(Food),
            Oven { heat: Heat },
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        enum Heat {
            Low,
            High(f32),
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Station {
            kind: Kind,
        }
        let food = Shape::Enum(vec!["Cabbage".into(), "Tomato".into()]);
        let heat = Shape::Tagged(vec![
            ("Low".into(), Shape::Unit),
            ("High".into(), Shape::Float),
        ]);
        assert_eq!(
            of::<Station>(),
            Shape::Struct(vec![(
                "kind".into(),
                Shape::Tagged(vec![
                    ("Counter".into(), Shape::Unit),
                    ("Crate".into(), food),
                    ("Oven".into(), Shape::Struct(vec![("heat".into(), heat)])),
                ])
            )])
        );
        assert_eq!(of::<Station>().example(), "(kind: Counter)");
    }

    #[test]
    fn each_variant_says_what_it_holds() {
        assert_eq!(
            variants_of::<Kind>(),
            vec![
                ("Wood".to_string(), Shape::Unit),
                (
                    "Iron".to_string(),
                    Shape::Struct(vec![("weight".into(), Shape::Float)])
                ),
            ]
        );
        assert!(variants_of::<Door>().is_empty(), "not an enum");
    }

    #[test]
    fn a_type_says_which_of_its_numbers_go_from_0_to_1() {
        let shape = with_fractions(of::<Door>(), &["open_angle", "kind.weight"]);
        let Shape::Struct(fields) = &shape else {
            panic!("{shape:?}");
        };
        assert_eq!(fields[0].1, Shape::Fraction);
        assert_eq!(
            fields[2].1,
            Shape::Tagged(vec![
                ("Wood".into(), Shape::Unit),
                (
                    "Iron".into(),
                    Shape::Struct(vec![("weight".into(), Shape::Fraction)])
                ),
            ]),
            "through every variant that has it"
        );
        assert!(shape.to_string().starts_with("(open_angle: number 0 to 1,"));
        assert!(ron::from_str::<Door>(&shape.example()).is_ok());
    }

    #[test]
    fn a_fraction_that_names_nothing_is_a_typo() {
        // Not by a panic: `crash`'s test owns the panic hook meanwhile.
        let mut shape = of::<Door>();
        assert!(!mark_fraction(&mut shape, &["open_angel"]));
        assert!(
            !mark_fraction(&mut shape, &["locked"]),
            "a flag is no number"
        );
        assert!(mark_fraction(&mut shape, &["open_angle"]));
    }

    #[test]
    fn a_value_that_does_not_fit_says_where() {
        let shape = of::<Door>();
        assert!(shape
            .problems("(open_angle: 90.0, locked: true)")
            .is_empty());
        let problems = shape.problems("(open_angel: 90.0, locked: 1)");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("no field `open_angel` (did you mean `open_angle`?)")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`locked` is true or false, not a number")),
            "{problems:?}"
        );
    }
}
