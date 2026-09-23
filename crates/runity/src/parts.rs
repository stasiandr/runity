//! What a line of a scene carries for the modules: `body: Dynamic`,
//! `light: (…)`, `sound: (…)` — every field the core does not know, kept as
//! the text it was written as (docs/modules.md).
//!
//! The core knows a line's identity, its place and its tree: `id`, `name`,
//! `prefab`, `transform`, `inactive`, the game's `components`, the prefab's
//! `overrides` and the `children`. A module knows the rest, as a type that
//! says which field it is ([`Part`]), and reads it off a line by that type
//! — `desc.part::<Light>()`. The core never parses a part: it checks that
//! it is RON and keeps its text, so a scene with a field of a module this
//! build does not have opens, saves byte for byte, and `check` names the
//! field (DNA, postulate 3: a disabled module's component is not lost).
//!
//! The same holds for the fields of a scene's look (`sun`, `fog`, `post`…)
//! and of a prefab instance's overrides.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// A part's value, in RON, as written.
pub type PartValue = Box<ron::value::RawValue>;

/// A field of a scene's line that a module owns: its name in the file, and
/// its type. `impl Part for Light { const NAME: &'static str = "light"; }`.
pub trait Part: Serialize + DeserializeOwned {
    /// The field's name in the file.
    const NAME: &'static str;

    /// Whether this value is what an absent field means: set to it, the
    /// field is left out of the file, as a default was before.
    fn is_default(&self) -> bool {
        false
    }
}

/// A field some module of this build reads: its name, and how to tell
/// whether a text fits it. `check` goes through a scene with the kinds of
/// every module the build has, and names a field none of them reads.
#[derive(Clone, Copy)]
pub struct PartKind {
    pub name: &'static str,
    pub check: fn(&str) -> Result<(), String>,
}

impl PartKind {
    /// The kind of `T`.
    pub fn of<T: Part>() -> Self {
        PartKind {
            name: T::NAME,
            check: |text| {
                ron::from_str::<T>(text)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            },
        }
    }
}

impl std::fmt::Debug for PartKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

/// The parts of one line, in the order the file wrote them (new ones at
/// the end), so that writing a line back out keeps it as it was.
#[derive(Debug, Clone, Default)]
pub struct Parts(Vec<(String, PartValue)>);

impl PartialEq for Parts {
    /// The same fields with the same text, in any order.
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .all(|(name, value)| other.raw(name) == Some(value.get_ron()))
    }
}

impl Parts {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The fields, by name, in the file's order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(n, _)| n.as_str())
    }

    /// Every field and its text.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(n, v)| (n.as_str(), v.get_ron()))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| n == name)
    }

    /// A field's text, as written.
    pub fn raw(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.get_ron())
    }

    /// Set a field from RON text. The text is checked to be RON, not to fit
    /// the field: the module that owns it says so when it reads it.
    pub fn set_raw(&mut self, name: &str, ron: &str) -> Result<(), String> {
        let value = ron::value::RawValue::from_boxed_ron(ron.trim().into())
            .map_err(|e| format!("{name}: {e}"))?;
        self.put(name, value);
        Ok(())
    }

    /// A field as it was read: `value` is kept, trimmed of the whitespace
    /// around it in the file.
    pub(crate) fn put(&mut self, name: &str, value: PartValue) {
        let value = value.trim_boxed();
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = value,
            None => self.0.push((name.to_string(), value)),
        }
    }

    /// Take a field off; whether there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|(n, _)| n != name);
        self.0.len() != before
    }

    /// A field read as its type: `None` when absent, the error in words
    /// when its text does not fit.
    pub fn try_get<T: Part>(&self) -> Result<Option<T>, String> {
        match self.raw(T::NAME) {
            None => Ok(None),
            Some(text) => ron::from_str(text)
                .map(Some)
                .map_err(|e| format!("`{}`: {e}", T::NAME)),
        }
    }

    /// A field read as its type; `None` when absent or when its text does
    /// not fit (which [`Parts::try_get`] and `check` say in words).
    pub fn get<T: Part>(&self) -> Option<T> {
        self.try_get().ok().flatten()
    }

    /// Set a field to `value`; a default value takes the field out, as it
    /// is not written.
    pub fn set<T: Part>(&mut self, value: &T) {
        if value.is_default() {
            self.remove(T::NAME);
            return;
        }
        let raw = ron::value::RawValue::from_boxed_ron(to_text(value).into_boxed_str())
            .expect("what RON writes, RON reads");
        self.put(T::NAME, raw);
    }

    /// Set a field, or take it out with `None`.
    pub fn set_opt<T: Part>(&mut self, value: Option<&T>) {
        match value {
            Some(value) => self.set(value),
            None => {
                self.remove(T::NAME);
            }
        }
    }

    /// Every field from `other` on top of these: the prefab's line under an
    /// instance's overrides.
    pub fn overlay(&mut self, other: &Parts) {
        for (name, value) in &other.0 {
            self.put(name, value.clone());
        }
    }

    pub(crate) fn entries(&self) -> &[(String, PartValue)] {
        &self.0
    }
}

/// A value as a person writes it on a line: one line, a space after each
/// comma and colon — `Box(half: (0.5, 0.5, 0.5))`.
pub fn to_text<T: Serialize>(value: &T) -> String {
    ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::new().depth_limit(0))
        .expect("a part serializes")
}

/// A name for a serializer that wants `&'static str` field names: a part's
/// name is one of a few, each leaked once.
pub(crate) fn static_name(name: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap();
    if let Some(n) = names.get(name) {
        return n;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    struct Glow(f32);
    impl Part for Glow {
        const NAME: &'static str = "glow";
        fn is_default(&self) -> bool {
            self.0 == 0.0
        }
    }

    #[test]
    fn a_part_is_its_text_until_a_module_reads_it_by_type() {
        let mut parts = Parts::default();
        parts.set_raw("glow", " 2.5 ").unwrap();
        parts.set_raw("rigid", "Dynamic(mass: 3.0)").unwrap();
        assert_eq!(parts.raw("glow"), Some("2.5"));
        assert_eq!(parts.get::<Glow>(), Some(Glow(2.5)));
        assert_eq!(
            parts.raw("rigid"),
            Some("Dynamic(mass: 3.0)"),
            "nobody's field, kept"
        );
        parts.set(&Glow(0.0));
        assert!(!parts.contains("glow"), "a default is left out");
        parts.set_raw("glow", "\"bright\"").unwrap();
        assert!(parts.try_get::<Glow>().unwrap_err().contains("glow"));
        assert!(parts.set_raw("glow", "(").is_err(), "not RON");
    }

    #[test]
    fn parts_are_equal_in_any_order() {
        let mut a = Parts::default();
        a.set_raw("x", "1").unwrap();
        a.set_raw("y", "2").unwrap();
        let mut b = Parts::default();
        b.set_raw("y", "2").unwrap();
        b.set_raw("x", "1").unwrap();
        assert_eq!(a, b);
        b.set_raw("x", "3").unwrap();
        assert_ne!(a, b);
    }
}
