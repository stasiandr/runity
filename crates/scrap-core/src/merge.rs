//! Merging two people's changes to one scene.
//!
//! DNA, postulate 2: a merge of scenes is a merge of things, not of lines.
//! Two branches that each moved a different tree merge cleanly, whatever
//! lines those trees share. Two that moved the same tree conflict, and the
//! conflict says so in those words — "both changed the position of `tree
//! mid`" — rather than as `<<<<<<<` in the middle of a file that no longer
//! loads.
//!
//! The merge is three-way, over the values: entities are matched by `id`,
//! and each field of each entity — name, model, placement, material, body,
//! each of the game's components, the parent it hangs under — is taken
//! from whichever side changed it. Where both changed it differently, ours
//! is kept and the conflict is reported; the scene that comes out is always
//! a scene that loads.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use crate::id::EntityId;
use crate::scene::{EntityDesc, Scene};

/// Both sides changed the same thing differently. Ours was kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    /// `None` for the scene's own fields: view, sun, fog.
    pub entity: Option<EntityId>,
    pub entity_name: String,
    pub field: String,
    pub ours: String,
    pub theirs: String,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.entity {
            Some(id) if self.field == EXISTS => write!(
                f,
                "`{}` ({id}): ours {}, theirs {}; kept it, since a delete can be redone",
                self.entity_name, self.ours, self.theirs
            ),
            Some(id) if self.field == PARENT_LOST => write!(
                f,
                "`{}` ({id}): its parent was deleted or the moves made a loop; placed where ours had it, or at the top",
                self.entity_name
            ),
            Some(id) => write!(
                f,
                "`{}` ({id}): both changed {} — ours {}, theirs {}; kept ours",
                self.entity_name, self.field, self.ours, self.theirs
            ),
            None => write!(
                f,
                "the scene's {}: both changed it — ours {}, theirs {}; kept ours",
                self.field, self.ours, self.theirs
            ),
        }
    }
}

impl Conflict {
    /// Settle this conflict the other way: put theirs into `scene` — a
    /// merge result, which holds ours — for this one field. `false` when
    /// there is nothing to take, such as a parent the merge had to drop.
    pub fn take_theirs(&self, scene: &mut Scene, theirs: &Scene) -> bool {
        let Some(id) = self.entity else {
            // A field of the scene's look, by its name.
            take_part(&mut scene.parts, &theirs.parts, &self.field);
            return true;
        };
        match self.field.as_str() {
            // Theirs deleted it: taking theirs deletes it. Theirs changed
            // it: the merge already kept theirs' version.
            EXISTS => {
                if self.theirs == "deleted it" {
                    crate::edit::remove(scene, id).is_some()
                } else {
                    true
                }
            }
            PARENT_LOST => false,
            "its parent" => {
                let parent = parent_of(&theirs.entities, id, None).flatten();
                crate::edit::reparent(scene, id, parent)
            }
            field => {
                let (Some(t), Some(e)) = (theirs.get(id), scene.get_mut(id)) else {
                    return false;
                };
                match field {
                    "its name" => e.name = t.name.clone(),
                    "its prefab" => e.prefab = t.prefab.clone(),
                    "its position" => e.transform.position = t.transform.position,
                    "its rotation" => e.transform.rotation_deg = t.transform.rotation_deg,
                    "its scale" => e.transform.scale = t.transform.scale,
                    "its overrides" => e.overrides = t.overrides.clone(),
                    other if other.starts_with("its ") && !other.starts_with("its component `") => {
                        take_part(&mut e.parts, &t.parts, &other["its ".len()..]);
                    }
                    other => {
                        let Some(name) = other
                            .strip_prefix("its component `")
                            .and_then(|rest| rest.strip_suffix('`'))
                        else {
                            return false;
                        };
                        match t.components.get(name) {
                            Some(value) => {
                                e.components.insert(name.to_string(), value.clone());
                            }
                            None => {
                                e.components.remove(name);
                            }
                        }
                    }
                }
                true
            }
        }
    }
}

/// `Some(parent)` for the entity `id` in a tree: `Some(None)` at the top,
/// `None` when it is not there.
fn parent_of(
    entities: &[EntityDesc],
    id: EntityId,
    parent: Option<EntityId>,
) -> Option<Option<EntityId>> {
    entities.iter().find_map(|e| {
        if e.id == id {
            Some(parent)
        } else {
            parent_of(&e.children, id, Some(e.id))
        }
    })
}

const EXISTS: &str = "whether it exists";
const PARENT_LOST: &str = "where it hangs";

/// What a merge produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub scene: Scene,
    pub conflicts: Vec<Conflict>,
}

/// One entity without its children, with where it hangs.
#[derive(Clone)]
struct Flat {
    desc: EntityDesc,
    parent: Option<EntityId>,
}

fn flatten(entities: &[EntityDesc], parent: Option<EntityId>, out: &mut Vec<(EntityId, Flat)>) {
    for desc in entities {
        let mut bare = desc.clone();
        bare.children = Vec::new();
        out.push((desc.id, Flat { desc: bare, parent }));
        flatten(&desc.children, Some(desc.id), out);
    }
}

fn show<T: serde::Serialize + fmt::Debug>(value: &T) -> String {
    ron::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

struct Merger<'a> {
    conflicts: Vec<Conflict>,
    entity: Option<EntityId>,
    name: &'a str,
}

impl Merger<'_> {
    /// [`Merger::pick`] for a field's text: shown as written, and as
    /// "nothing" where a side has none.
    fn pick_text(
        &mut self,
        field: &str,
        base: Option<&str>,
        ours: Option<&str>,
        theirs: Option<&str>,
    ) -> Option<String> {
        if ours == theirs || theirs == base {
            return ours.map(str::to_string);
        }
        if ours == base {
            return theirs.map(str::to_string);
        }
        self.conflicts.push(Conflict {
            entity: self.entity,
            entity_name: self.name.to_string(),
            field: field.to_string(),
            ours: ours.unwrap_or("nothing").to_string(),
            theirs: theirs.unwrap_or("nothing").to_string(),
        });
        ours.map(str::to_string)
    }

    /// Whichever side changed it; ours, and a conflict, if both did.
    fn pick<T: PartialEq + Clone + serde::Serialize + fmt::Debug>(
        &mut self,
        field: &str,
        base: &T,
        ours: &T,
        theirs: &T,
    ) -> T {
        if ours == theirs || theirs == base {
            return ours.clone();
        }
        if ours == base {
            return theirs.clone();
        }
        self.conflicts.push(Conflict {
            entity: self.entity,
            entity_name: self.name.to_string(),
            field: field.to_string(),
            ours: show(ours),
            theirs: show(theirs),
        });
        ours.clone()
    }
}

/// Merge `ours` and `theirs`, both descended from `base`.
pub fn merge_scenes(base: &Scene, ours: &Scene, theirs: &Scene) -> Merged {
    let mut conflicts = Vec::new();
    let mut scene = Scene::default();
    {
        let mut top = Merger {
            conflicts: Vec::new(),
            entity: None,
            name: "",
        };
        scene.parts = merge_parts(
            &mut top,
            |name| name.to_string(),
            &base.parts,
            &ours.parts,
            &theirs.parts,
        );
        conflicts.extend(top.conflicts);
    }

    let (mut b, mut o, mut t) = (Vec::new(), Vec::new(), Vec::new());
    flatten(&base.entities, None, &mut b);
    flatten(&ours.entities, None, &mut o);
    flatten(&theirs.entities, None, &mut t);
    let base_map: HashMap<EntityId, &Flat> = b.iter().map(|(id, f)| (*id, f)).collect();
    let ours_map: HashMap<EntityId, &Flat> = o.iter().map(|(id, f)| (*id, f)).collect();
    let theirs_map: HashMap<EntityId, &Flat> = t.iter().map(|(id, f)| (*id, f)).collect();

    // Every entity that survives, with its merged line and parent.
    let mut merged: HashMap<EntityId, Flat> = HashMap::new();
    let ids: Vec<EntityId> = o
        .iter()
        .map(|(id, _)| *id)
        .chain(t.iter().map(|(id, _)| *id))
        .chain(b.iter().map(|(id, _)| *id))
        .collect();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let (base_e, ours_e, theirs_e) =
            (base_map.get(&id), ours_map.get(&id), theirs_map.get(&id));
        let result = match (base_e, ours_e, theirs_e) {
            (_, Some(o), Some(t)) => {
                // In both: merged field by field. Added on both sides with
                // one id, the base is "nothing", so every difference is a
                // conflict.
                let base = base_e.copied().unwrap_or(o);
                Some(merge_entity(base, o, t, &mut conflicts))
            }
            // Added on one side only.
            (None, Some(only), None) | (None, None, Some(only)) => Some((*only).clone()),
            // Deleted on one side: gone, unless the other side changed it,
            // which is a conflict — and the entity is kept, because a
            // delete can be redone and lost work cannot.
            (Some(base), Some(kept), None) | (Some(base), None, Some(kept)) => {
                let deleted_by_theirs = ours_e.is_some();
                if flat_eq(base, kept) {
                    None
                } else {
                    let (ours_word, theirs_word) = if deleted_by_theirs {
                        ("changed it".to_string(), "deleted it".to_string())
                    } else {
                        ("deleted it".to_string(), "changed it".to_string())
                    };
                    conflicts.push(Conflict {
                        entity: Some(id),
                        entity_name: kept.desc.name.clone(),
                        field: EXISTS.into(),
                        ours: ours_word,
                        theirs: theirs_word,
                    });
                    Some((*kept).clone())
                }
            }
            (Some(_), None, None) | (None, None, None) => None,
        };
        if let Some(flat) = result {
            merged.insert(id, flat);
        }
    }

    // Order: ours', with what only theirs has placed after the sibling it
    // follows there.
    let mut order: Vec<EntityId> = o
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| merged.contains_key(id))
        .collect();
    for (index, (id, flat)) in t.iter().enumerate() {
        if order.contains(id) || !merged.contains_key(id) {
            continue;
        }
        let previous = t[..index]
            .iter()
            .rev()
            .find(|(_, f)| f.parent == flat.parent)
            .map(|(id, _)| *id)
            .or(flat.parent);
        let at = previous
            .and_then(|p| order.iter().position(|x| *x == p))
            .map(|i| i + 1)
            .unwrap_or(order.len());
        order.insert(at, *id);
    }

    // A parent that is gone, or a loop made by two moves, puts the entity
    // back where ours had it — or at the top — and says so.
    let ids: Vec<EntityId> = order.clone();
    for id in &ids {
        let parent = merged[id].parent;
        let broken = match parent {
            Some(p) if !merged.contains_key(&p) => true,
            Some(_) => loops(&merged, *id),
            None => false,
        };
        if broken {
            let fallback = ours_map
                .get(id)
                .and_then(|f| f.parent)
                .filter(|p| merged.contains_key(p) && *p != *id);
            let flat = merged.get_mut(id).expect("in order means merged");
            flat.parent = fallback;
            if loops(&merged, *id) {
                merged.get_mut(id).expect("still there").parent = None;
            }
            conflicts.push(Conflict {
                entity: Some(*id),
                entity_name: merged[id].desc.name.clone(),
                field: PARENT_LOST.into(),
                ours: String::new(),
                theirs: String::new(),
            });
        }
    }

    scene.entities = build(&order, &merged, None);
    Merged { scene, conflicts }
}

/// Merge `ours` and `theirs` of a prefab: a scene of one entity.
pub fn merge_prefabs(base: &EntityDesc, ours: &EntityDesc, theirs: &EntityDesc) -> Merged {
    let wrap = |desc: &EntityDesc| Scene {
        entities: vec![desc.clone()],
        ..Scene::default()
    };
    merge_scenes(&wrap(base), &wrap(ours), &wrap(theirs))
}

fn flat_eq(a: &Flat, b: &Flat) -> bool {
    a.desc == b.desc && a.parent == b.parent
}

fn loops(merged: &HashMap<EntityId, Flat>, id: EntityId) -> bool {
    let mut at = merged.get(&id).and_then(|f| f.parent);
    for _ in 0..=merged.len() {
        match at {
            None => return false,
            Some(p) if p == id => return true,
            Some(p) => at = merged.get(&p).and_then(|f| f.parent),
        }
    }
    true
}

fn build(
    order: &[EntityId],
    merged: &HashMap<EntityId, Flat>,
    parent: Option<EntityId>,
) -> Vec<EntityDesc> {
    order
        .iter()
        .filter(|id| merged[*id].parent == parent)
        .map(|id| {
            let mut desc = merged[id].desc.clone();
            desc.children = build(order, merged, Some(*id));
            desc
        })
        .collect()
}

fn merge_entity(base: &Flat, ours: &Flat, theirs: &Flat, conflicts: &mut Vec<Conflict>) -> Flat {
    let (b, o, t) = (&base.desc, &ours.desc, &theirs.desc);
    let mut m = Merger {
        conflicts: Vec::new(),
        entity: Some(o.id),
        name: &o.name,
    };
    let mut desc = o.clone();
    desc.name = m.pick("its name", &b.name, &o.name, &t.name);
    desc.prefab = m.pick("its prefab", &b.prefab, &o.prefab, &t.prefab);
    desc.transform.position = m.pick(
        "its position",
        &b.transform.position,
        &o.transform.position,
        &t.transform.position,
    );
    desc.transform.rotation_deg = m.pick(
        "its rotation",
        &b.transform.rotation_deg,
        &o.transform.rotation_deg,
        &t.transform.rotation_deg,
    );
    desc.transform.scale = m.pick(
        "its scale",
        &b.transform.scale,
        &o.transform.scale,
        &t.transform.scale,
    );
    // Every module field one by one, by name, as the components are: one
    // side lighting a lamp and the other moving its sound is no conflict.
    desc.parts = merge_parts(
        &mut m,
        |name| format!("its {name}"),
        &b.parts,
        &o.parts,
        &t.parts,
    );
    desc.overrides = m.pick("its overrides", &b.overrides, &o.overrides, &t.overrides);

    // The game's components one by one: one side adding `loot` and the
    // other changing `door` is no conflict.
    let names: HashSet<&String> = b
        .components
        .keys()
        .chain(o.components.keys())
        .chain(t.components.keys())
        .collect();
    let mut components = BTreeMap::new();
    for name in names {
        let text = |map: &BTreeMap<String, crate::scene::ComponentValue>| {
            map.get(name).map(|v| v.get_ron().to_string())
        };
        let field = format!("its component `{name}`");
        let picked = m.pick(
            &field,
            &text(&b.components),
            &text(&o.components),
            &text(&t.components),
        );
        if let Some(text) = picked {
            let side = if text.as_str() == o.components.get(name).map_or("", |v| v.get_ron()) {
                &o.components
            } else {
                &t.components
            };
            components.insert(name.clone(), side[name].clone());
        }
    }
    desc.components = components;

    let parent = m.pick(
        "its parent",
        &base.parent.map(|p| p.to_string()),
        &ours.parent.map(|p| p.to_string()),
        &theirs.parent.map(|p| p.to_string()),
    );
    let parent = parent.and_then(|p| p.parse().ok());
    conflicts.extend(m.conflicts);
    Flat { desc, parent }
}


/// Put theirs' field `name` in place of ours — or take ours away where
/// theirs has none.
fn take_part(ours: &mut crate::parts::Parts, theirs: &crate::parts::Parts, name: &str) {
    match theirs.raw(name) {
        Some(text) => {
            let _ = ours.set_raw(name, text);
        }
        None => {
            ours.remove(name);
        }
    }
}

/// Three sides' module fields merged one by one, by name and text: a field
/// only one side changed takes that side; both changing it the same way
/// is no conflict; both changing it differently keeps ours and says so,
/// as `label(name)`.
fn merge_parts(
    m: &mut Merger,
    label: impl Fn(&str) -> String,
    base: &crate::parts::Parts,
    ours: &crate::parts::Parts,
    theirs: &crate::parts::Parts,
) -> crate::parts::Parts {
    let mut names: Vec<&str> = ours.names().collect();
    for name in theirs.names().chain(base.names()) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let mut out = crate::parts::Parts::default();
    for name in names {
        let picked = m.pick_text(
            &label(name),
            base.raw(name),
            ours.raw(name),
            theirs.raw(name),
        );
        if let Some(text) = picked {
            let _ = out.set_raw(name, &text);
        }
    }
    out
}
