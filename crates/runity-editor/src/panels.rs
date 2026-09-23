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
}

/// The fields every entity has, in the order the Inspector shows them.
pub const FIELDS: [&str; 14] = [
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
    "components.<name>",
];

/// What a field of several things shows when they disagree.
pub const MIXED: &str = "—";

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
        let mut out = Vec::new();
        fn walk(
            session: &Session,
            entities: &[EntityDesc],
            depth: usize,
            part: bool,
            selection: &[EntityId],
            unseen: &std::collections::HashSet<EntityId>,
            out: &mut Vec<Row>,
        ) {
            let document = session.scene();
            for e in entities {
                let line = document.get(e.id);
                let prefab = line.map(|l| l.prefab.clone()).filter(|p| !p.is_empty());
                let open = session.is_open(e.id);
                out.push(Row {
                    id: e.id,
                    name: e.name.clone(),
                    depth,
                    has_children: !e.children.is_empty(),
                    open,
                    selected: selection.contains(&e.id),
                    prefab: prefab.clone(),
                    part,
                    hidden: unseen.contains(&e.id),
                });
                if open {
                    walk(
                        session,
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
            &self.expanded().entities,
            0,
            false,
            &selection,
            &unseen,
            &mut out,
        );
        out
    }

    fn is_open(&self, id: EntityId) -> bool {
        let instance = self
            .scene()
            .get(id)
            .is_some_and(|line| !line.prefab.is_empty());
        if instance {
            self.opened.contains(&id)
        } else {
            !self.folded.contains(&id)
        }
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

    /// The Inspector for one entity: its fields as text.
    pub fn inspect(&self, id: EntityId) -> Option<Vec<Field>> {
        let desc = self.line(id)?;
        let overrides = self.part_override(id);
        let changed = |name: &str| {
            overrides.as_ref().is_some_and(|o| match name {
                "name" => o.name.is_some(),
                "model" => o.model.is_some(),
                "position" | "rotation" | "scale" => o.transform.is_some(),
                "material" => o.material.is_some(),
                "body" => o.body.is_some(),
                "collider" => o.collider.is_some(),
                "physics" => o.physics.is_some(),
                "layer" => o.layer.is_some(),
                other => other
                    .strip_prefix("components.")
                    .is_some_and(|c| o.components.contains_key(c)),
            })
        };
        let t = desc.transform;
        let mut fields: Vec<(String, String)> = vec![
            ("name".into(), desc.name.clone()),
            ("model".into(), desc.model.clone()),
            ("prefab".into(), desc.prefab.clone()),
            ("position".into(), ron(&t.position)),
            ("rotation".into(), ron(&t.rotation_deg)),
            ("scale".into(), ron(&t.scale)),
            ("material".into(), ron(&desc.material)),
            ("body".into(), ron(&desc.body)),
            ("collider".into(), ron(&desc.collider)),
            ("physics".into(), ron(&desc.physics)),
            ("layer".into(), desc.layer.clone()),
            ("joint".into(), ron(&desc.joint)),
            (
                "camera".into(),
                desc.camera.map_or("None".to_string(), |c| ron(&c)),
            ),
        ];
        for (name, value) in &desc.components {
            fields.push((format!("components.{name}"), value.get_ron().to_string()));
        }
        Some(
            fields
                .into_iter()
                .map(|(name, value)| Field {
                    overridden: changed(&name),
                    name,
                    value,
                })
                .collect(),
        )
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
        if let Some(component) = field.strip_prefix("components.") {
            let value = (!text.trim().is_empty() && text.trim() != "None").then_some(text);
            return self.set_component(id, component, value);
        }
        let current = self.line(id).ok_or(EditError::NoEntity(id))?.clone();
        let mut next = current.clone();
        match field {
            "name" => next.name = text.to_string(),
            "model" => next.model = text.to_string(),
            "prefab" => next.prefab = text.to_string(),
            "layer" => next.layer = text.to_string(),
            "position" => next.transform.position = parse(field, text)?,
            "rotation" => next.transform.rotation_deg = parse(field, text)?,
            "scale" => next.transform.scale = parse(field, text)?,
            "material" => {
                next.material = if text.starts_with('"') || text.starts_with('(') {
                    parse::<MaterialRef>(field, text)?
                } else {
                    MaterialRef::Named(text.to_string())
                }
            }
            "body" => next.body = parse::<Body>(field, text)?,
            "collider" => next.collider = parse::<Collider>(field, text)?,
            "physics" => next.physics = parse::<BodyProps>(field, text)?,
            "joint" => next.joint = parse::<Joint>(field, text)?,
            "camera" => {
                next.camera = if text.trim() == "None" {
                    None
                } else {
                    Some(parse::<Lens>(field, text)?)
                }
            }
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
