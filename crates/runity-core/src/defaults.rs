//! Small defaults the scene's types share in their serde attributes:
//! `#[serde(default = "unit", skip_serializing_if = "is_one")]`. Here so
//! that a module's types spell a field the way the core's do.
#![allow(dead_code)]

pub fn half() -> f32 {
    0.5
}

pub fn unit() -> f32 {
    1.0
}

pub fn is_half(v: &f32) -> bool {
    *v == 0.5
}

pub fn is_one(v: &f32) -> bool {
    *v == 1.0
}

pub fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

pub fn is_true(v: &bool) -> bool {
    *v
}

pub fn is_false(v: &bool) -> bool {
    !*v
}

/// An optional field written as its value, not as `Some(value)`: absent
/// means `None`, present means `Some`. RON would otherwise want `Some(…)`
/// spelled out around every override, which is noise to read and a trap to
/// write.
pub mod plain {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(
        value: &Option<T>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => value.serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<T>, D::Error> {
        T::deserialize(d).map(Some)
    }
}

pub fn is_zero_vec3(v: &glam::Vec3) -> bool {
    *v == glam::Vec3::ZERO
}
