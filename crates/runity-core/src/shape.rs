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

/// A visitor's `expecting`, as text.
struct Expecting<'a, V>(&'a V);

impl<'de, V: Visitor<'de>> fmt::Display for Expecting<'_, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }
}

/// The shape of `T`.
pub fn of<'de, T: Deserialize<'de>>() -> Shape {
    let mut out = Shape::Any;
    let _ = T::deserialize(Tracer {
        out: &mut out,
        depth: 0,
    });
    out
}

/// What each variant of the enum `T` holds, by name: `Unit` for a bare
/// one, the fields of one written `Box(half: …)`. Empty when `T` is not an
/// enum. [`Shape::Enum`] names the variants only; this is what an editor
/// needs to switch a value to another variant with fields to fill in.
pub fn variants_of<'de, T: Deserialize<'de>>() -> Vec<(String, Shape)> {
    let Shape::Enum(names) = of::<T>() else {
        return Vec::new();
    };
    names
        .into_iter()
        .map(|name| {
            let mut content = Shape::Unit;
            let _ = T::deserialize(Picker {
                name: &name,
                content: &mut content,
            });
            (name, content)
        })
        .collect()
}

impl Shape {
    /// A value of this shape in RON, as short as it can be written: what a
    /// component added in an inspector starts as.
    pub fn example(&self) -> String {
        match self {
            Shape::Bool => "false".into(),
            Shape::Int => "0".into(),
            Shape::Float => "0.0".into(),
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
            Shape::OneOf(shapes) => shapes.first().map(Shape::example).unwrap_or_default(),
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
                Shape::Any | Shape::Enum(_) | Shape::Entity | Shape::Asset(_) | Shape::OneOf(_),
                _,
            ) => {}
            (Shape::Bool, V::Bool(_)) => {}
            (Shape::Bool, _) => wrong(out, "true or false"),
            (Shape::Int, V::Number(n)) if n.into_f64().fract() == 0.0 => {}
            (Shape::Int, _) => wrong(out, "a whole number"),
            (Shape::Float, V::Number(_)) => {}
            (Shape::Float, _) => wrong(out, "a number"),
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
            Shape::OneOf(shapes) => {
                let shapes: Vec<String> = shapes.iter().map(ToString::to_string).collect();
                write!(f, "{}", shapes.join(" or "))
            }
            Shape::Entity => write!(f, "entity"),
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
        deserialize_str => Shape::Text, visit_str("0");
        deserialize_string => Shape::Text, visit_string("0".to_string());
        deserialize_bytes => Shape::Any, visit_bytes(&[]);
        deserialize_byte_buf => Shape::Any, visit_byte_buf(Vec::new());
        deserialize_unit => Shape::Unit, visit_unit();
        deserialize_identifier => Shape::Text, visit_str("");
        deserialize_ignored_any => Shape::Any, visit_unit();
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
        let first = variants.first().copied().unwrap_or("");
        visitor.visit_enum(Variant {
            name: first,
            depth: self.depth + 1,
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
        seed.deserialize(Tracer {
            out: &mut self.shapes[index],
            depth: self.depth,
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
        let value = seed.deserialize(Tracer {
            out: &mut self.shapes[index],
            depth: self.depth,
        });
        FIELD.set(before);
        value
    }
}

struct Variant {
    name: &'static str,
    depth: usize,
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
        Ok(())
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Stop> {
        let mut ignored = Shape::Any;
        seed.deserialize(Tracer {
            out: &mut ignored,
            depth: self.depth,
        })
    }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Stop> {
        let mut ignored = Shape::Any;
        Tracer {
            out: &mut ignored,
            depth: self.depth,
        }
        .deserialize_tuple(len, visitor)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Stop> {
        let mut ignored = Shape::Any;
        Tracer {
            out: &mut ignored,
            depth: self.depth,
        }
        .deserialize_struct("", fields, visitor)
    }
}

// --- one variant of an enum, traced -----------------------------------

/// Asked for an enum, answers with the variant `name` and traces what it
/// holds into `content`; asked for anything else, stops.
struct Picker<'a> {
    name: &'a str,
    content: &'a mut Shape,
}

impl<'de> Deserializer<'de> for Picker<'_> {
    type Error = Stop;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Stop> {
        Err(Stop("not an enum".into()))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Stop> {
        let name = variants
            .iter()
            .find(|v| **v == self.name)
            .copied()
            .ok_or_else(|| Stop(format!("no variant `{}`", self.name)))?;
        visitor.visit_enum(Picked {
            name,
            content: self.content,
        })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

struct Picked<'a> {
    name: &'static str,
    content: &'a mut Shape,
}

impl<'de> de::EnumAccess<'de> for Picked<'_> {
    type Error = Stop;
    type Variant = Self;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Stop> {
        let value = seed.deserialize(self.name.into_deserializer())?;
        Ok((value, self))
    }
}

impl<'de> de::VariantAccess<'de> for Picked<'_> {
    type Error = Stop;
    fn unit_variant(self) -> Result<(), Stop> {
        *self.content = Shape::Unit;
        Ok(())
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Stop> {
        seed.deserialize(Tracer {
            out: self.content,
            depth: 1,
        })
    }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Stop> {
        Tracer {
            out: self.content,
            depth: 1,
        }
        .deserialize_tuple(len, visitor)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Stop> {
        Tracer {
            out: self.content,
            depth: 1,
        }
        .deserialize_struct("", fields, visitor)
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
        assert_eq!(fields[2].1, Shape::Enum(vec!["Wood".into(), "Iron".into()]));
        assert_eq!(fields[3].1, Shape::Option(Box::new(Shape::Text)));
        assert_eq!(fields[4].1, Shape::List(Box::new(Shape::Text)));
        assert_eq!(
            shape.to_string(),
            "(open_angle: number, locked: bool, kind: Wood | Iron, name: text or None, tags: [text], hinge: (number, number, number))"
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
        assert_eq!(
            of_field::<crate::AssetLink>("material"),
            Shape::Asset("material".into())
        );
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
