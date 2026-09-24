//! Links to assets: an asset by its ID and the name it had when the link
//! was written ([`AssetLink`]), and the typed links a game's component
//! holds (`ModelLink`, `SoundLink`…) — what `check` follows and the editor
//! shows as a picker (docs/refs.md). The core's: a link names an asset,
//! not what a module does with it.

/// A link to an asset: its name, for a person reading the line, and its
/// ID, for finding it (docs/refs.md).
///
/// In a file it is `("rock", "fc5513a0…")` — name, then ID — or a bare
/// `"rock"`
/// as a person or an agent writes it by hand — found by name, and given
/// its ID the first time it is saved. Following a link tries the ID first
/// and the name second, so a link survives its file being renamed or moved
/// anywhere: the ID travels in the sidecar, and a file renamed outside the
/// editor is found by the name the link keeps. Whoever follows it brings
/// the name up to date with the file, so the name read in a diff is the
/// file's name now.
///
/// It reads as its name (`Deref<Target = str>`): `link.is_empty()`,
/// `builtin::by_name(&link)`, `format!("{link}")`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct AssetLink {
    pub name: String,
    pub id: Option<crate::AssetId>,
}

impl AssetLink {
    /// A link by name only, to be given its ID when it is followed.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: None,
        }
    }

    pub fn to(name: impl Into<String>, id: crate::AssetId) -> Self {
        Self {
            name: name.into(),
            id: Some(id),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }

    /// Point it at what it was found to be: the ID, and the file's name now.
    /// `true` when that changed the link.
    pub fn settle(&mut self, name: &str, id: crate::AssetId) -> bool {
        let changed = self.id != Some(id) || self.name != name;
        self.id = Some(id);
        if self.name != name {
            self.name = name.to_string();
        }
        changed
    }
}

impl std::ops::Deref for AssetLink {
    type Target = str;

    fn deref(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for AssetLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

impl From<String> for AssetLink {
    fn from(name: String) -> Self {
        Self::named(name)
    }
}

impl From<&str> for AssetLink {
    fn from(name: &str) -> Self {
        Self::named(name)
    }
}

impl PartialEq<str> for AssetLink {
    fn eq(&self, other: &str) -> bool {
        self.name == other
    }
}

impl PartialEq<&str> for AssetLink {
    fn eq(&self, other: &&str) -> bool {
        self.name == *other
    }
}

impl PartialEq<String> for AssetLink {
    fn eq(&self, other: &String) -> bool {
        &self.name == other
    }
}

impl serde::Serialize for AssetLink {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        match self.id {
            // As it was written: a line a person wrote stays as written
            // until the engine has found what it names.
            None => serializer.serialize_str(&self.name),
            // A pair, not a struct: a tuple stays on its line, so an entity
            // written on one line stays on one line.
            Some(id) => {
                let mut s = serializer.serialize_tuple(2)?;
                s.serialize_element(&self.name)?;
                s.serialize_element(&id.to_string())?;
                s.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for AssetLink {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Link;
        impl<'de> serde::de::Visitor<'de> for Link {
            type Value = AssetLink;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "an asset's name, or (name: \"…\", id: \"…\")")
            }

            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<AssetLink, E> {
                Ok(AssetLink::named(name))
            }

            fn visit_string<E: serde::de::Error>(self, name: String) -> Result<AssetLink, E> {
                Ok(AssetLink::named(name))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<AssetLink, A::Error> {
                let name: String = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let id: Option<String> = seq.next_element()?;
                let id = id
                    .map(|text| text.parse().map_err(serde::de::Error::custom))
                    .transpose()?;
                Ok(AssetLink { name, id })
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<AssetLink, A::Error> {
                let mut link = AssetLink::default();
                let mut named = false;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" => {
                            link.name = map.next_value()?;
                            named = true;
                        }
                        "id" => {
                            let text: String = map.next_value()?;
                            link.id = Some(text.parse().map_err(serde::de::Error::custom)?);
                        }
                        other => {
                            // Not a link: an inline material beside one in
                            // an untagged enum is a map too, and must not
                            // read as a link with no name.
                            return Err(serde::de::Error::unknown_field(other, &["name", "id"]));
                        }
                    }
                }
                if !named {
                    return Err(serde::de::Error::missing_field("name"));
                }
                Ok(link)
            }
        }
        deserializer.deserialize_any(Link)
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn a_link_is_a_bare_name_until_found_and_then_a_name_and_an_id() {
        let id: crate::AssetId = "fc55".parse().unwrap();
        let bare: AssetLink = ron::from_str(r#""rock""#).unwrap();
        assert_eq!(bare, AssetLink::named("rock"));
        assert_eq!(ron::to_string(&bare).unwrap(), r#""rock""#);
        let full: AssetLink = ron::from_str(r#"("rock", "fc55")"#).unwrap();
        assert_eq!(full, AssetLink::to("rock", id));
        let spelled: AssetLink = ron::from_str(r#"(name: "rock", id: "fc55")"#).unwrap();
        assert_eq!(spelled, full, "the long form, written by hand, reads too");
        let back: AssetLink = ron::from_str(&ron::to_string(&full).unwrap()).unwrap();
        assert_eq!(back, full);
        assert!(full == "rock" && !full.is_empty());

        let mut moved = full.clone();
        assert!(moved.settle("boulder", id), "renamed: the name follows");
        assert_eq!(moved.name, "boulder");
        assert!(!moved.settle("boulder", id));
    }
}

/// A game component's link to an asset of one kind: what its field is
/// shown as — a picker of that kind's assets — and what `check` follows
/// (docs/refs.md, stage 3). In a scene: `ModelLink(("rock", "fc55…"))`,
/// or `ModelLink("rock")` written by hand.
macro_rules! asset_link_kind {
    ($(#[$doc:meta])* $name:ident, $kind:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
        pub struct $name(pub AssetLink);

        impl $name {
            /// The type's name in a file, which is how an editor knows it.
            pub const NAME: &'static str = stringify!($name);
            /// The kind of asset it links: what its picker lists.
            pub const KIND: &'static str = $kind;
        }

        impl std::ops::Deref for $name {
            type Target = AssetLink;
            fn deref(&self) -> &AssetLink {
                &self.0
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_newtype_struct(Self::NAME, &self.0)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Visit;
                impl<'de> serde::de::Visitor<'de> for Visit {
                    type Value = $name;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        write!(f, "{}(\"name\") or {}((\"name\", \"id\"))", $name::NAME, $name::NAME)
                    }
                    fn visit_newtype_struct<D: serde::Deserializer<'de>>(
                        self,
                        d: D,
                    ) -> Result<$name, D::Error> {
                        serde::Deserialize::deserialize(d).map($name)
                    }
                }
                d.deserialize_newtype_struct(Self::NAME, Visit)
            }
        }
    };
}

asset_link_kind!(
    /// A link to a model: a mesh or a terrain in `assets/`, or a builtin.
    ModelLink,
    "model"
);
asset_link_kind!(
    /// A link to a material in `materials/`.
    MaterialLink,
    "material"
);
asset_link_kind!(
    /// A link to a prefab: what a spawner spawns.
    PrefabLink,
    "prefab"
);
asset_link_kind!(
    /// A link to a sound: what a door plays when it opens.
    SoundLink,
    "sound"
);
asset_link_kind!(
    /// A link to a texture.
    TextureLink,
    "texture"
);
asset_link_kind!(
    /// A link to a scene: where a door leads.
    SceneLink,
    "scene"
);

/// Every typed link: its name in a file, and the kind of asset it links.
pub const LINK_KINDS: [(&str, &str); 6] = [
    (ModelLink::NAME, ModelLink::KIND),
    (MaterialLink::NAME, MaterialLink::KIND),
    (PrefabLink::NAME, PrefabLink::KIND),
    (SoundLink::NAME, SoundLink::KIND),
    (TextureLink::NAME, TextureLink::KIND),
    (SceneLink::NAME, SceneLink::KIND),
];

/// Every typed link in a value's RON text, with its kind: what `check`
/// follows in a game's components.
pub fn links_in(text: &str) -> Vec<(&'static str, AssetLink)> {
    let mut out = Vec::new();
    for (name, kind) in LINK_KINDS {
        let mut rest = text;
        let open = format!("{name}(");
        while let Some(at) = rest.find(&open) {
            let start = at + open.len() - 1;
            let Some(span) = crate::ron_edit::value_span(rest, start) else {
                break;
            };
            let inner = &rest[span.start + 1..span.end.saturating_sub(1)];
            if let Ok(link) = ron::from_str::<AssetLink>(inner.trim()) {
                out.push((kind, link));
            }
            rest = &rest[span.end..];
        }
    }
    out
}

#[cfg(test)]
mod typed_link_tests {
    use super::*;

    #[test]
    fn a_typed_link_reads_by_name_or_by_name_and_id() {
        let bare: ModelLink = ron::from_str(r#"ModelLink("rock")"#).unwrap();
        assert_eq!(bare.name, "rock");
        let full: PrefabLink = ron::from_str(r#"PrefabLink(("door", "fc55"))"#).unwrap();
        assert_eq!(full.id, Some("fc55".parse().unwrap()));
        let found = links_in(r#"(open: SoundLink("creak"), leads: SceneLink(("cellar", "a1")))"#);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, "sound");
        assert_eq!(found[1].1.name, "cellar");
    }
}

