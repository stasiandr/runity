//! What the Hierarchy and the Inspector show, as data a window draws.
//!
//! The panels of Unity's editor, without a UI toolkit: a list of rows with
//! their depth, and a list of an entity's fields as text, each settable by
//! name as one undo step. Whatever draws the editor — GPUI, when the
//! viewport question is settled (DNA, open question 1), or a test, or an
//! agent — draws these, so the panel's logic is written and tested once.
//! Components are shown and set as the RON the scene holds; a form built
//! from the game's types waits on how modules reach the editor (open
//! question 2).

#[allow(unused_imports)]
use runity::prelude::*;
use runity::scene::{Body, BodyProps, Collider, EntityDesc, Joint, Lens, MaterialRef};
use runity::EntityId;

use crate::{EditError, EditResult, Session};

/// One line of the Hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: EntityId,
    pub name: String,
    pub depth: usize,
    /// Whether it has anything under it to open.
    pub has_children: bool,
    pub open: bool,
    pub selected: bool,
    /// The prefab this line is an instance of.
    pub prefab: Option<String>,
    /// A part a prefab brought: edited as an override on its instance.
    pub part: bool,
    /// Not drawn in the Scene view: hidden, or outside what is isolated.
    pub hidden: bool,
    /// Drawn, but a click or a box does not take it.
    pub locked: bool,
}

/// What a field of a new entity says, as the Inspector writes it: what a
/// reset goes back to. `None` for fields that are the entity's own (name,
/// model, prefab) and for a game's components, whose defaults are the
/// game's.
pub fn default_text(field: &str) -> Option<String> {
    let blank = EntityDesc::default();
    Some(match field {
        "position" => ron(&blank.transform.position),
        "rotation" => ron(&blank.transform.rotation_deg),
        "scale" => ron(&blank.transform.scale),
        "material" => ron(&blank.material_ref()),
        "body" => ron(&blank.body()),
        "collider" => ron(&blank.collider()),
        "physics" => ron(&blank.physics()),
        "joint" => ron(&blank.joint()),
        "layer" | "bone" | "animator" => String::new(),
        "inactive" => "false".into(),
        "bends_grass" => ron(&blank.bends_grass()),
        "camera" | "light" | "particles" | "reflection_probe" | "post_volume" | "decal"
        | "footprints" | "terrain" | "render_texture" | "sound" | "route" | "rope" | "cloth" | "hair" | "soft_body" | "jiggle" | "fluid" | "fracture" | "dents" | "mpm" | "shallow_water" | "ripples" | "ocean" | "floats" | "smoke" | "grains" | "distance_field" | "snow_cover" | "ragdoll" | "crawler" | "spline" | "along"
        | "joint_break" => "None".into(),
        _ => return None,
    })
}

/// One field of the Inspector.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// What [`Session::set_field`] calls it: `position`, `material`,
    /// `components.door`.
    pub name: String,
    /// Its value as text: plain for names, RON for the rest.
    pub value: String,
    /// On a prefab's part: this instance says something else than the
    /// prefab does.
    pub overridden: bool,
    /// Worth a reset arrow, as Unreal's Details panel draws one: overridden
    /// on a prefab's part, or not what a new entity has.
    pub resettable: bool,
    /// For a game component, what it holds — `(open_angle: number,
    /// locked: bool)` — when the game has written its components' shapes
    /// (`library/components.ron`); empty otherwise.
    pub shape: String,
}

/// The fields every entity has, in the order the Inspector shows them.
pub const FIELDS: [&str; 41] = [
    "name",
    "model",
    "prefab",
    "position",
    "rotation",
    "scale",
    "material",
    "body",
    "collider",
    "physics",
    "layer",
    "joint",
    "camera",
    "light",
    "particles",
    "reflection_probe",
    "decal",
    "footprints",
    "terrain",
    "bends_grass",
    "route",
    "rope",
    "cloth",
    "hair",
    "soft_body",
    "jiggle",
    "fluid",
    "fracture",
    "dents",
    "mpm",
    "shallow_water",
    "ripples",
    "ocean",
    "floats",
    "smoke",
    "grains",
    "distance_field",
    "snow_cover",
    "ragdoll",
    "crawler",
    "components.<name>",
];

/// What a field of several things shows when they disagree.
pub const MIXED: &str = "—";

/// Move the one field `field` names out of an override: what it said, as
/// an override of its own (empty when it said nothing about it).
fn take_field(
    from: &mut runity::scene::Override,
    field: &str,
) -> EditResult<runity::scene::Override> {
    let mut one = runity::scene::Override::default();
    match field {
        "name" => one.name = from.name.take(),
        "position" | "rotation" | "scale" => one.transform = from.transform.take(),
        "inactive" => one.inactive = from.inactive.take(),
        other => match other.strip_prefix("components.") {
            Some(name) => {
                if let Some(value) = from.components.remove(name) {
                    one.components.insert(name.to_string(), value);
                }
            }
            // A module's field: moved as the text it is.
            None if FIELDS.contains(&other) => {
                if let Some(text) = from.parts.raw(other).map(str::to_string) {
                    from.parts.remove(other);
                    let _ = one.parts.set_raw(other, &text);
                }
            }
            None => {
                return Err(EditError::Scene(format!(
                    "`{other}` is not a field a part overrides — there are {}",
                    FIELDS.join(", ")
                )))
            }
        },
    }
    Ok(one)
}

fn ron<T: serde::Serialize>(value: &T) -> String {
    runity::ron::to_string(value).unwrap_or_default()
}

fn parse<T: serde::de::DeserializeOwned>(field: &str, text: &str) -> EditResult<T> {
    runity::ron::from_str(text).map_err(|e| EditError::Scene(format!("{field}: {e}")))
}

impl Session {
    /// The Hierarchy: every line of the document in tree order, under the
    /// ones that are open. The document's lines start open; an instance
    /// starts closed and opens onto its parts.
    pub fn hierarchy(&self) -> Vec<Row> {
        let selection = self.selection();
        let unseen = self.unseen();
        // Every document line by id, once: looking each up by walking the
        // tree made the panel quadratic — two thousand lines took tens of
        // milliseconds a frame.
        let index: std::collections::HashMap<EntityId, &EntityDesc> = self
            .scene()
            .flatten()
            .into_iter()
            .map(|(d, _)| (d.id, d))
            .collect();
        let mut out = Vec::new();
        #[allow(clippy::too_many_arguments)]
        fn walk(
            session: &Session,
            index: &std::collections::HashMap<EntityId, &EntityDesc>,
            entities: &[EntityDesc],
            depth: usize,
            part: bool,
            selection: &[EntityId],
            unseen: &std::collections::HashSet<EntityId>,
            out: &mut Vec<Row>,
        ) {
            for e in entities {
                let prefab = index
                    .get(&e.id)
                    .map(|l| l.prefab.clone())
                    .filter(|p| !p.is_empty());
                let open = if prefab.is_some() {
                    session.opened.contains(&e.id)
                } else {
                    !session.folded.contains(&e.id)
                };
                out.push(Row {
                    id: e.id,
                    name: e.name.clone(),
                    depth,
                    has_children: !e.children.is_empty(),
                    open,
                    selected: selection.contains(&e.id),
                    prefab: prefab.as_ref().map(ToString::to_string),
                    part,
                    hidden: unseen.contains(&e.id),
                    locked: !session.is_pickable(session.instanced_owner(e.id)),
                });
                if open {
                    walk(
                        session,
                        index,
                        &e.children,
                        depth + 1,
                        part || prefab.is_some(),
                        selection,
                        unseen,
                        out,
                    );
                }
            }
        }
        walk(
            self,
            &index,
            &self.expanded().entities,
            0,
            false,
            &selection,
            &unseen,
            &mut out,
        );
        out
    }

    /// Open or close a line of the Hierarchy. A view setting, not an edit.
    pub fn set_open(&mut self, id: EntityId, open: bool) {
        let instance = self
            .scene()
            .get(id)
            .is_some_and(|line| !line.prefab.is_empty());
        let (set, keep) = if instance {
            (&mut self.opened, open)
        } else {
            (&mut self.folded, !open)
        };
        if keep {
            set.insert(id);
        } else {
            set.remove(&id);
        }
    }

    /// Open or close every line of the Hierarchy that has lines under it:
    /// Expand All and Collapse All. A view setting, not an edit.
    pub fn set_all_open(&mut self, open: bool) {
        let mut ids = Vec::new();
        parents(&self.expanded().entities, &mut ids);
        for id in ids {
            self.set_open(id, open);
        }
    }

    /// Open or close a line and every line under it, as Alt and the arrow
    /// do in Unity.
    pub fn set_open_below(&mut self, id: EntityId, open: bool) {
        let mut ids = Vec::new();
        if let Some(e) = find_desc(&self.expanded().entities, id) {
            parents(std::slice::from_ref(e), &mut ids);
        }
        for id in ids {
            self.set_open(id, open);
        }
    }

    /// The Inspector for one entity: its fields as text.
    pub fn inspect(&self, id: EntityId) -> Option<Vec<Field>> {
        let desc = self.line(id)?;
        let overrides = self.part_override(id);
        let changed = |name: &str| {
            overrides.as_ref().is_some_and(|o| match name {
                "name" => o.name.is_some(),
                "position" | "rotation" | "scale" => o.transform.is_some(),
                "inactive" => o.inactive.is_some(),
                other => match other.strip_prefix("components.") {
                    Some(c) => o.components.contains_key(c),
                    None => o.parts.contains(other),
                },
            })
        };
        // During play the Inspector shows where things are, as Unity's
        // does: the document says where they start, the world where they
        // fell to. The document is what comes back when play stops.
        let t = self.live_transform(id).unwrap_or(desc.transform);
        let mut fields: Vec<(String, String)> = vec![
            ("name".into(), desc.name.clone()),
            ("model".into(), desc.model().to_string()),
            ("prefab".into(), desc.prefab.to_string()),
            ("position".into(), ron(&t.position)),
            ("rotation".into(), ron(&t.rotation_deg)),
            ("scale".into(), ron(&t.scale)),
            ("material".into(), ron(&desc.material_ref())),
            ("body".into(), ron(&desc.body())),
            ("collider".into(), ron(&desc.collider())),
            ("physics".into(), ron(&desc.physics())),
            ("layer".into(), desc.layer().clone()),
            ("inactive".into(), desc.inactive.to_string()),
            ("animator".into(), desc.animator().clone()),
            ("bends_grass".into(), ron(&desc.bends_grass())),
            ("bone".into(), desc.bone().clone()),
            ("joint".into(), ron(&desc.joint())),
            (
                "joint_break".into(),
                desc.joint_break().map_or("None".to_string(), |f| ron(&f)),
            ),
            (
                "camera".into(),
                desc.camera().map_or("None".to_string(), |c| ron(&c)),
            ),
            (
                "light".into(),
                desc.light().map_or("None".to_string(), |l| ron(&l)),
            ),
            (
                "particles".into(),
                desc.particles().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "reflection_probe".into(),
                desc.reflection_probe()
                    .map_or("None".to_string(), |p| ron(&p)),
            ),
            (
                "render_texture".into(),
                desc.render_texture()
                    .as_ref()
                    .map_or("None".to_string(), ron),
            ),
            (
                "sound".into(),
                desc.sound().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "post_volume".into(),
                desc.post_volume().map_or("None".to_string(), |v| ron(&v)),
            ),
            (
                "decal".into(),
                desc.decal().map_or("None".to_string(), |d| ron(&d)),
            ),
            (
                "footprints".into(),
                desc.footprints().map_or("None".to_string(), |f| ron(&f)),
            ),
            (
                "terrain".into(),
                desc.terrain().map_or("None".to_string(), |t| ron(&t)),
            ),
            (
                "route".into(),
                desc.route().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "rope".into(),
                desc.rope().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "cloth".into(),
                desc.cloth().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "hair".into(),
                desc.hair().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "soft_body".into(),
                desc.soft_body().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "jiggle".into(),
                desc.jiggle().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "fluid".into(),
                desc.fluid().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "fracture".into(),
                desc.fracture().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "dents".into(),
                desc.dents().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "mpm".into(),
                desc.mpm().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "shallow_water".into(),
                desc.shallow_water().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "ripples".into(),
                desc.ripples().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "ocean".into(),
                desc.ocean().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "floats".into(),
                desc.floats().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "smoke".into(),
                desc.smoke().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "grains".into(),
                desc.grains().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "distance_field".into(),
                desc.distance_field().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "snow_cover".into(),
                desc.snow_cover().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "ragdoll".into(),
                desc.ragdoll().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "crawler".into(),
                desc.crawler().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "spline".into(),
                desc.spline().as_ref().map_or("None".to_string(), ron),
            ),
            (
                "along".into(),
                desc.along().as_ref().map_or("None".to_string(), ron),
            ),
        ];
        for (name, value) in &desc.components {
            fields.push((format!("components.{name}"), value.get_ron().to_string()));
        }
        // What the running game says about it, read-only: the document is
        // where it starts, this is where it is.
        if let Some(state) = self.game_state() {
            if let Some(saved) = state.entities.iter().find(|s| s.id == id) {
                let t = saved.transform;
                fields.push(("game.position".into(), ron(&t.position)));
                fields.push(("game.rotation".into(), ron(&t.rotation_deg)));
                fields.push(("game.scale".into(), ron(&t.scale)));
                for (name, value) in &saved.components {
                    fields.push((format!("game.components.{name}"), value.clone()));
                }
                if !saved.animator.is_empty() {
                    fields.push(("game.animator".into(), saved.animator.clone()));
                }
            } else if state.gone.contains(&id) {
                fields.push(("game".into(), "gone".into()));
            }
        }
        let shapes = self.component_shapes();
        Some(
            fields
                .into_iter()
                .map(|(name, value)| Field {
                    overridden: changed(&name),
                    resettable: changed(&name)
                        || default_text(&name).is_some_and(|default| default != value),
                    shape: name
                        .strip_prefix("components.")
                        .and_then(|c| shapes.get(c))
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    name,
                    value,
                })
                .collect(),
        )
    }

    /// The entity's transform in the running world, while playing.
    fn live_transform(&self, id: EntityId) -> Option<runity::scene::Transform> {
        if !self.is_playing() {
            return None;
        }
        self.world
            .query::<(&runity::SceneId, &runity::scene::Transform)>()
            .iter()
            .find(|(scene_id, _)| scene_id.0 == id)
            .map(|(_, t)| *t)
    }

    /// What the game's components look like, by name, as the game last
    /// wrote them (`library/components.ron`, written when the game or its
    /// tests run). Empty when it has not: the Inspector then edits
    /// components as plain RON, as before.
    ///
    /// DNA, open question 2 (how modules ship) is not decided by this: the
    /// editor still links nothing of the game; the game describes itself in
    /// a file, the way the importer describes assets in `library/`.
    pub fn component_shapes(&self) -> std::collections::BTreeMap<String, runity::shape::Shape> {
        self.project()
            .map(|p| p.root().join(runity::project::SHAPES))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| runity::ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Add a component the game has with a value of its shape to start
    /// from — Unity's Add Component — as one undo step. The value is the
    /// shortest that parses; the game's defaults are its own.
    pub fn add_component(&mut self, id: EntityId, name: &str) -> EditResult<String> {
        let shapes = self.component_shapes();
        let shape = shapes.get(name).ok_or_else(|| {
            EditError::Scene(format!(
                "the game has no component `{name}`{}",
                runity::spelling::closest(name, shapes.keys().map(String::as_str))
                    .map(|n| format!(" — did you mean `{n}`?"))
                    .unwrap_or_default()
            ))
        })?;
        let value = shape.example();
        self.set_component(id, name, Some(&value))?;
        Ok(value)
    }

    /// The override this entity's instance keeps for it, when it is a part.
    fn part_override(&self, id: EntityId) -> Option<runity::scene::Override> {
        if self.scene().get(id).is_some() {
            return None;
        }
        let (instance, part) = self.instanced_parts().get(&id).copied()?;
        Some(
            self.scene()
                .get(instance)?
                .overrides
                .get(&part)
                .cloned()
                .unwrap_or_default(),
        )
    }

    /// Revert one overridden field of a prefab's part to what the prefab
    /// says — Unity's Revert on a single property — as one undo step.
    /// `false` when that field was not overridden. `position`, `rotation`
    /// and `scale` are one override, the transform, and revert together.
    pub fn revert_field(&mut self, id: EntityId, field: &str) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let (instance, part) = self.part_of(id)?;
        let mut rest = self
            .scene()
            .get(instance)
            .and_then(|l| l.overrides.get(&part).cloned())
            .unwrap_or_default();
        if take_field(&mut rest, field)?.is_empty() {
            return Ok(false);
        }
        let line = self.edit_entity(instance)?;
        if rest.is_empty() {
            line.overrides.remove(&part);
        } else {
            line.overrides.insert(part, rest);
        }
        self.respawn();
        Ok(true)
    }

    /// Apply one overridden field of a prefab's part to the prefab file —
    /// Unity's Apply on a single property: every instance gets it, this one
    /// stops overriding it, its other overrides stay. One undo step in the
    /// scene (the file stays applied, as with [`Session::apply_overrides`]).
    pub fn apply_field(&mut self, id: EntityId, field: &str) -> EditResult<bool> {
        self.refuse_while_playing()?;
        let (instance, part) = self.part_of(id)?;
        let all = self
            .scene()
            .get(instance)
            .map(|l| l.overrides.clone())
            .unwrap_or_default();
        let mut rest = all.get(&part).cloned().unwrap_or_default();
        let one = take_field(&mut rest, field)?;
        if one.is_empty() {
            return Ok(false);
        }
        let before = self.history.depth();
        // Only this field goes to the file; then the others come back.
        self.edit_entity(instance)?.overrides = [(part, one)].into_iter().collect();
        self.apply_overrides(instance)?;
        let mut others = all;
        if rest.is_empty() {
            others.remove(&part);
        } else {
            others.insert(part, rest);
        }
        self.edit_entity(instance)?.overrides = others;
        self.respawn();
        self.history.squash(self.history.depth() - before);
        Ok(true)
    }

    /// The instance a prefab's part came with, and the part's id in it.
    fn part_of(&self, id: EntityId) -> EditResult<(EntityId, EntityId)> {
        if self.scene().get(id).is_some() {
            return Err(EditError::Scene(format!(
                "{id} is a line of the scene, not a part of a prefab: it has nothing to revert to"
            )));
        }
        self.instanced_parts()
            .get(&id)
            .copied()
            .ok_or(EditError::NoEntity(id))
    }

    /// The Hierarchy while its search box has text: the lines that match,
    /// flat and in tree order, parts of prefabs included — Unity's
    /// Hierarchy search. An empty query is the whole tree.
    pub fn hierarchy_matching(&self, query: &str) -> EditResult<Vec<Row>> {
        if query.trim().is_empty() {
            return Ok(self.hierarchy());
        }
        let found = self.search(query)?;
        let selection = self.selection();
        let unseen = self.unseen();
        let document = self.scene();
        Ok(self
            .expanded()
            .flatten()
            .into_iter()
            .filter(|(e, _)| found.contains(&e.id))
            .map(|(e, _)| {
                let line = document.get(e.id);
                Row {
                    id: e.id,
                    name: e.name.clone(),
                    depth: 0,
                    has_children: false,
                    open: false,
                    selected: selection.contains(&e.id),
                    prefab: line.map(|l| l.prefab.to_string()).filter(|p| !p.is_empty()),
                    part: line.is_none(),
                    hidden: unseen.contains(&e.id),
                    locked: !self.is_pickable(self.instanced_owner(e.id)),
                }
            })
            .collect())
    }

    /// Put a field back as a new entity has it — the Inspector's Reset; on
    /// a prefab's part, back to what the prefab says. One undo step.
    pub fn reset_field(&mut self, id: EntityId, field: &str) -> EditResult<()> {
        // What a new entity has is the default a reset goes back to.
        if self.scene().get(id).is_none() {
            return self.revert_field(id, field).map(|_| ());
        }
        let text = match field {
            other if default_text(other).is_some() => default_text(other).unwrap_or_default(),
            other if other.starts_with("components.") => {
                let name = &other["components.".len()..];
                return match self.component_shapes().get(name) {
                    Some(shape) => {
                        let value = shape.example();
                        self.set_component(id, name, Some(&value))
                    }
                    None => Err(EditError::Scene(format!(
                        "`{name}`: the game has not said what it looks like (library/components.ron), so there is nothing to reset it to"
                    ))),
                };
            }
            other => {
                return Err(EditError::Scene(format!(
                    "`{other}` has no default to go back to — name and model are the entity's own"
                )))
            }
        };
        self.set_field(id, field, &text)
    }

    /// The Inspector for several things at once: the first one's fields,
    /// with `—` where the others say something else — Unity's mixed value.
    pub fn inspect_all(&self, ids: &[EntityId]) -> Option<Vec<Field>> {
        let (first, rest) = ids.split_first()?;
        let mut fields = self.inspect(*first)?;
        for id in rest {
            let theirs = self.inspect(*id)?;
            for field in &mut fields {
                let same = theirs
                    .iter()
                    .find(|f| f.name == field.name)
                    .is_some_and(|f| f.value == field.value);
                if !same {
                    field.value = MIXED.to_string();
                }
                field.overridden |= theirs.iter().any(|f| f.name == field.name && f.overridden);
            }
        }
        Some(fields)
    }

    /// Set one field of several things to the same text, as one undo step:
    /// typing into an Inspector showing all of them. Nothing changes when
    /// any of them is not there or the text does not parse.
    pub fn set_field_all(&mut self, ids: &[EntityId], field: &str, text: &str) -> EditResult<()> {
        for id in ids {
            self.require(*id)?;
        }
        let before = self.history.depth();
        for id in ids {
            if let Err(e) = self.set_field(*id, field, text) {
                // Only the first can fail on the text; take back what went.
                while self.history.depth() > before {
                    self.undo()?;
                }
                return Err(e);
            }
        }
        self.history
            .squash(self.history.depth().saturating_sub(before));
        Ok(())
    }

    /// Set one field from its text, as one undo step — what typing into
    /// the Inspector does. On a prefab's part it is an override, as any
    /// edit of a part is. Text that does not parse costs no step, and an
    /// unknown field names the ones there are.
    pub fn set_field(&mut self, id: EntityId, field: &str, text: &str) -> EditResult<()> {
        if field == "game" || field.starts_with("game.") {
            return Err(EditError::Scene(format!(
                "`{field}` is what the running game says; edit the field without `game.` and the game is given it"
            )));
        }
        if let Some(component) = field.strip_prefix("components.") {
            let value = (!text.trim().is_empty() && text.trim() != "None").then_some(text);
            return self.set_component(id, component, value);
        }
        let current = self.line(id).ok_or(EditError::NoEntity(id))?.clone();
        let mut next = current.clone();
        match field {
            "name" => next.name = text.to_string(),
            "model" => next.set_part(&runity::scene::ModelRef(text.into())),
            "prefab" => next.prefab = text.into(),
            "layer" => next.set_part(&runity::scene::LayerName(text.to_string())),
            "inactive" => next.inactive = parse::<bool>(field, text)?,
            "animator" => next.set_part(&runity::scene::AnimatorRef(text.trim().to_string())),
            "bends_grass" => next.set_part(&runity::scene::BendsGrass(
                parse::<f32>(field, text)?.max(0.0),
            )),
            "bone" => next.set_part(&runity::scene::BoneName(text.trim().to_string())),
            "position" => next.transform.position = parse(field, text)?,
            "rotation" => next.transform.rotation_deg = parse(field, text)?,
            "scale" => next.transform.scale = parse(field, text)?,
            "material" => next.set_part(&if text.starts_with('"') || text.starts_with('(') {
                parse::<MaterialRef>(field, text)?
            } else {
                MaterialRef::Named(text.into())
            }),
            "body" => next.set_part(&parse::<Body>(field, text)?),
            "collider" => next.set_part(&parse::<Collider>(field, text)?),
            "physics" => next.set_part(&parse::<BodyProps>(field, text)?),
            "joint" => next.set_part(&parse::<Joint>(field, text)?),
            "joint_break" => next.set_joint_break(if text.trim() == "None" {
                None
            } else {
                Some(parse::<f32>(field, text)?)
            }),
            "camera" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<Lens>(field, text)?)
                })
                .as_ref(),
            ),
            "light" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Light>(field, text)?)
                })
                .as_ref(),
            ),
            "spline" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::Spline>(field, text)?)
                })
                .as_ref(),
            ),
            "along" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::Along>(field, text)?)
                })
                .as_ref(),
            ),
            "route" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Route>(field, text)?)
                })
                .as_ref(),
            ),
            "soft_body" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::SoftBody>(field, text)?)
                })
                .as_ref(),
            ),
            "jiggle" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Jiggle>(field, text)?)
                })
                .as_ref(),
            ),
            "fluid" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Fluid>(field, text)?)
                })
                .as_ref(),
            ),
            "fracture" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Fracture>(field, text)?)
                })
                .as_ref(),
            ),
            "dents" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Dents>(field, text)?)
                })
                .as_ref(),
            ),
            "mpm" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Mpm>(field, text)?)
                })
                .as_ref(),
            ),
            "shallow_water" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::ShallowWater>(field, text)?)
                })
                .as_ref(),
            ),
            "ripples" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Ripples>(field, text)?)
                })
                .as_ref(),
            ),
            "ocean" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Ocean>(field, text)?)
                })
                .as_ref(),
            ),
            "floats" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Floats>(field, text)?)
                })
                .as_ref(),
            ),
            "smoke" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Smoke>(field, text)?)
                })
                .as_ref(),
            ),
            "grains" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Grains>(field, text)?)
                })
                .as_ref(),
            ),
            "distance_field" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::DistanceField>(field, text)?)
                })
                .as_ref(),
            ),
            "snow_cover" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::SnowCover>(field, text)?)
                })
                .as_ref(),
            ),
            "ragdoll" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Ragdoll>(field, text)?)
                })
                .as_ref(),
            ),
            "crawler" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Crawler>(field, text)?)
                })
                .as_ref(),
            ),
            "rope" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Rope>(field, text)?)
                })
                .as_ref(),
            ),
            "cloth" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Cloth>(field, text)?)
                })
                .as_ref(),
            ),
            "hair" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Hair>(field, text)?)
                })
                .as_ref(),
            ),
            "particles" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Emitter>(field, text)?)
                })
                .as_ref(),
            ),
            "decal" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Decal>(field, text)?)
                })
                .as_ref(),
            ),
            "sound" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::SoundSource>(field, text)?)
                })
                .as_ref(),
            ),
            "footprints" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::footprints::Footprints>(field, text)?)
                })
                .as_ref(),
            ),
            "terrain" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::terrain::Terrain>(field, text)?)
                })
                .as_ref(),
            ),
            "render_texture" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::RenderTexture>(field, text)?)
                })
                .as_ref(),
            ),
            "post_volume" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::PostVolume>(field, text)?)
                })
                .as_ref(),
            ),
            "reflection_probe" => next.set_part_opt(
                (if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<runity::scene::Probe>(field, text)?)
                })
                .as_ref(),
            ),
            other => {
                return Err(EditError::Scene(format!(
                    "no field `{other}` — there are {}",
                    FIELDS.join(", ")
                )))
            }
        }
        if next == current {
            return Ok(());
        }
        self.update(id, |desc| *desc = next)
    }
}

/// The entities among `entities`, at any depth, that have children.
fn parents(entities: &[EntityDesc], out: &mut Vec<EntityId>) {
    for e in entities {
        if !e.children.is_empty() {
            out.push(e.id);
            parents(&e.children, out);
        }
    }
}

fn find_desc(entities: &[EntityDesc], id: EntityId) -> Option<&EntityDesc> {
    entities.iter().find_map(|e| {
        if e.id == id {
            Some(e)
        } else {
            find_desc(&e.children, id)
        }
    })
}
