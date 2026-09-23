//! What in a scene names an asset, and renaming it everywhere at once.
//!
//! A scene names things the way a person would: a model, a material and a
//! prefab each by file stem. That is what keeps a line readable in
//! `git diff` (DNA, postulate 2) — and what makes renaming a file dangerous, because every line that said the old name now names
//! nothing. Worse for materials: a project's `stone` shadows the engine's,
//! so renaming it away leaves every `material: "stone"` quietly drawing the
//! builtin instead. Unity keeps references through a rename with GUIDs in
//! the file; here the rename is an operation that rewrites the names, and
//! the diff says exactly which lines pointed at the file.
//!
//! What is not rewritten: a game's own components are text the engine does
//! not read (see [`crate::components`]), so a path inside one is the game's
//! to keep. `builtin:stone` names the engine's material, never the
//! project's, and stays.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::id::EntityId;
use crate::scene::{EntityDesc, MaterialRef, Scene};

/// A name a scene can hold, and what kind of thing it names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssetRef {
    /// A model — a mesh or terrain source in `assets/` — by file stem.
    Model(String),
    /// A material by file stem.
    Material(String),
    /// A prefab by file stem.
    Prefab(String),
}

impl AssetRef {
    /// The name as a scene writes it.
    pub fn name(&self) -> &str {
        match self {
            AssetRef::Model(name) | AssetRef::Material(name) | AssetRef::Prefab(name) => name,
        }
    }

    /// The same kind of reference, to another name.
    pub fn renamed(&self, to: impl Into<String>) -> AssetRef {
        match self {
            AssetRef::Model(_) => AssetRef::Model(to.into()),
            AssetRef::Material(_) => AssetRef::Material(to.into()),
            AssetRef::Prefab(_) => AssetRef::Prefab(to.into()),
        }
    }
}

impl std::fmt::Display for AssetRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetRef::Model(name) => write!(f, "model `{name}`"),
            AssetRef::Material(name) => write!(f, "material `{name}`"),
            AssetRef::Prefab(name) => write!(f, "prefab `{name}`"),
        }
    }
}

/// One place an asset is named.
#[derive(Debug, Clone, PartialEq)]
pub struct Use {
    /// The entity whose line names it.
    pub entity: EntityId,
    pub name: String,
    /// The field: `model`, `material`, `prefab`, or an override's
    /// `overrides.<part id>.model` / `.material`.
    pub field: String,
}

/// Every place under `roots` that names `what`, depth first.
pub fn uses(roots: &[EntityDesc], what: &AssetRef) -> Vec<Use> {
    let mut out = Vec::new();
    for root in roots {
        visit(root, &mut |desc| {
            let mut found = |field: String| {
                out.push(Use {
                    entity: desc.id,
                    name: desc.name.clone(),
                    field,
                })
            };
            for field in fields(desc, what) {
                found(field.to_string());
            }
            for (part, change) in &desc.overrides {
                let hit = match what {
                    AssetRef::Model(name) => change.model().as_deref() == Some(name),
                    AssetRef::Material(name) => change
                        .material()
                        .as_ref()
                        .is_some_and(|m| names_material(m, name)),
                    AssetRef::Prefab(_) => false,
                };
                if hit {
                    let field = match what {
                        AssetRef::Model(_) => "model",
                        _ => "material",
                    };
                    found(format!("overrides.{part}.{field}"));
                }
            }
        });
    }
    out
}

/// Every place in a scene that names `what`.
pub fn uses_in_scene(scene: &Scene, what: &AssetRef) -> Vec<Use> {
    uses(&scene.entities, what)
}

/// Point every reference to `from` at `to` instead. How many were changed.
pub fn rewrite(roots: &mut [EntityDesc], from: &AssetRef, to: &str) -> usize {
    let mut count = 0;
    for root in roots {
        visit_mut(root, &mut |desc| {
            match from {
                AssetRef::Model(name) => {
                    // An instance draws its prefab, so its own `model` is
                    // ignored; it is still a name in the file, and leaving
                    // it stale would only confuse the next reader.
                    if desc.model() == *name {
                        // The same asset under its new name: the ID stays.
                        let mut model = desc.model();
                        model.name = to.to_string();
                        desc.set_model(model);
                        count += 1;
                    }
                }
                AssetRef::Material(name) => {
                    if names_material(&desc.material_ref(), name) {
                        // The same material under its new name: the ID stays.
                        let mut material = desc.material_ref();
                        if let MaterialRef::Named(link) = &mut material {
                            link.name = to.to_string();
                        }
                        desc.set_material(material);
                        count += 1;
                    }
                }
                AssetRef::Prefab(name) => {
                    if desc.prefab == *name {
                        desc.prefab.name = to.to_string();
                        count += 1;
                    }
                }
            }
            for change in desc.overrides.values_mut() {
                match from {
                    AssetRef::Model(name) if change.model().as_deref() == Some(name) => {
                        if let Some(mut model) = change.model() {
                            model.name = to.to_string();
                            change.set_part(&crate::scene::ModelRef(model));
                        }
                        count += 1;
                    }
                    AssetRef::Material(name)
                        if change
                            .material()
                            .as_ref()
                            .is_some_and(|m| names_material(m, name)) =>
                    {
                        if let Some(MaterialRef::Named(mut link)) = change.material() {
                            link.name = to.to_string();
                            change.set_part(&MaterialRef::Named(link));
                        }
                        count += 1;
                    }
                    _ => {}
                }
            }
        });
    }
    count
}

/// [`rewrite`] over a whole scene.
pub fn rewrite_scene(scene: &mut Scene, from: &AssetRef, to: &str) -> usize {
    rewrite(&mut scene.entities, from, to)
}

fn fields(desc: &EntityDesc, what: &AssetRef) -> Vec<&'static str> {
    let hit = match what {
        AssetRef::Model(name) => desc.model() == *name,
        AssetRef::Material(name) => names_material(&desc.material_ref(), name),
        AssetRef::Prefab(name) => desc.prefab == *name,
    };
    match (hit, what) {
        (false, _) => Vec::new(),
        (true, AssetRef::Model(_)) => vec!["model"],
        (true, AssetRef::Material(_)) => vec!["material"],
        (true, AssetRef::Prefab(_)) => vec!["prefab"],
    }
}

/// `material: "stone"` names the project's `stone`; `builtin:stone` does
/// not, whatever the project has.
fn names_material(material: &MaterialRef, name: &str) -> bool {
    matches!(material, MaterialRef::Named(n) if n == name)
}

fn visit(desc: &EntityDesc, f: &mut impl FnMut(&EntityDesc)) {
    f(desc);
    for child in &desc.children {
        visit(child, f);
    }
}

fn visit_mut(desc: &mut EntityDesc, f: &mut impl FnMut(&mut EntityDesc)) {
    f(desc);
    for child in &mut desc.children {
        visit_mut(child, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Override;

    fn scene() -> Scene {
        ron::from_str(
            r#"(entities: [
                (id: "00000000000000a1", name: "rock", model: "boulder", material: "stone"),
                (id: "00000000000000a2", name: "old rock", model: "builtin:sphere", material: "builtin:stone"),
                (id: "00000000000000a3", name: "fire", prefab: "campfire",
                 overrides: { "00000000000000c2": (material: "stone") }),
                (id: "00000000000000a4", name: "cart", model: "", children: [
                    (id: "00000000000000a5", name: "load", model: "boulder", material: "moss"),
                ]),
            ])"#,
        )
        .unwrap()
    }

    #[test]
    fn finds_every_use_including_children_and_overrides() {
        let scene = scene();
        let stone = uses_in_scene(&scene, &AssetRef::Material("stone".into()));
        let fields: Vec<(&str, &str)> = stone
            .iter()
            .map(|u| (u.name.as_str(), u.field.as_str()))
            .collect();
        assert_eq!(
            fields,
            [
                ("rock", "material"),
                ("fire", "overrides.00000000000000c2.material")
            ],
            "builtin:stone is the engine's, not the project's"
        );
        let boulder = uses_in_scene(&scene, &AssetRef::Model("boulder".into()));
        assert_eq!(boulder.len(), 2);
        assert_eq!(boulder[1].name, "load");
        let campfire = uses_in_scene(&scene, &AssetRef::Prefab("campfire".into()));
        assert_eq!(campfire.len(), 1);
    }

    #[test]
    fn rewrites_the_names_and_nothing_else() {
        let mut scene = scene();
        let before = scene.clone();
        let changed = rewrite_scene(&mut scene, &AssetRef::Material("stone".into()), "granite");
        assert_eq!(changed, 2);
        assert_eq!(
            scene.find("rock").unwrap().material_ref(),
            MaterialRef::Named("granite".into())
        );
        assert_eq!(
            scene.find("old rock").unwrap().material_ref(),
            MaterialRef::Named("builtin:stone".into())
        );
        let part: EntityId = "00000000000000c2".parse().unwrap();
        assert_eq!(
            scene.find("fire").unwrap().overrides[&part],
            Override {
                ..Override::default()
            }
            .with(MaterialRef::Named("granite".into()))
        );
        assert_eq!(
            rewrite_scene(&mut scene, &AssetRef::Material("granite".into()), "stone"),
            2
        );
        assert_eq!(scene, before, "and back again is where it started");
    }
}

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

/// Point every model, material, sound and prefab link under `roots` at what it names now:
/// its ID, and the name its file has. How a scene opened in the editor
/// gets IDs for lines written by name, and names that follow files renamed
/// since; the next save writes them. A link that finds nothing is left as
/// it is, for `check` to name. Returns how many links changed.
pub fn settle(
    roots: &mut [EntityDesc],
    library: Option<&crate::Library>,
    prefabs: &crate::Prefabs,
) -> usize {
    use crate::asset::AssetKind;
    let mut changed = 0;
    let model = |link: &mut AssetLink, changed: &mut usize| {
        if link.is_empty() || crate::builtin::by_name(link).is_some() {
            return;
        }
        if let Some((id, name)) = library.and_then(|l| l.find(link, AssetKind::Mesh)) {
            *changed += usize::from(link.settle(name, id));
        }
    };
    // A material by name: a builtin stays a name (the engine's own, and
    // `builtin:` says so); a project's gets its ID.
    let material = |reference: &mut MaterialRef, changed: &mut usize| {
        let MaterialRef::Named(link) = reference else {
            return;
        };
        if link.starts_with("builtin:") {
            return;
        }
        if let Some((id, name)) = library.and_then(|l| l.find(link, AssetKind::Material)) {
            *changed += usize::from(link.settle(name, id));
        }
    };
    let mut stack: Vec<&mut EntityDesc> = roots.iter_mut().collect();
    while let Some(desc) = stack.pop() {
        // Each field read, settled and written back only when it changed,
        // so a line nobody touched keeps its text.
        let before = changed;
        let mut link = desc.model();
        model(&mut link, &mut changed);
        if changed != before {
            desc.set_model(link);
        }
        let before = changed;
        let mut reference = desc.material_ref();
        material(&mut reference, &mut changed);
        if changed != before {
            desc.set_material(reference);
        }
        if let Some(mut along) = desc.along() {
            let before = changed;
            model(&mut along.model, &mut changed);
            if changed != before {
                desc.set_part(&along);
            }
        }
        if let Some(mut sound) = desc.sound() {
            if let Some((id, name)) = library.and_then(|l| l.find(&sound.clip, AssetKind::Sound)) {
                if sound.clip.settle(name, id) {
                    changed += 1;
                    desc.set_part(&sound);
                }
            }
        }
        for part in desc.overrides.values_mut() {
            if let Some(mut link) = part.model() {
                let before = changed;
                model(&mut link, &mut changed);
                if changed != before {
                    part.set_part(&crate::scene::ModelRef(link));
                }
            }
            if let Some(mut reference) = part.material() {
                let before = changed;
                material(&mut reference, &mut changed);
                if changed != before {
                    part.set_part(&reference);
                }
            }
        }
        if !desc.prefab.is_empty() {
            let found = prefabs
                .find(&desc.prefab)
                .and_then(|(name, _)| Some((prefabs.id_of(name)?, name.to_string())));
            if let Some((id, name)) = found {
                changed += usize::from(desc.prefab.settle(&name, id));
            }
        }
        stack.extend(desc.children.iter_mut());
    }
    changed
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
