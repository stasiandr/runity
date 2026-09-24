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

pub use crate::links::*;

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
