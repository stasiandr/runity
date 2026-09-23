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
            match self.field.as_str() {
                "view" => scene.view = theirs.view,
                "sun" => scene.sun = theirs.sun,
                "fog" => scene.fog = theirs.fog,
                _ => return false,
            }
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
                    "its model" => e.model = t.model.clone(),
                    "its prefab" => e.prefab = t.prefab.clone(),
                    "its position" => e.transform.position = t.transform.position,
                    "its rotation" => e.transform.rotation_deg = t.transform.rotation_deg,
                    "its scale" => e.transform.scale = t.transform.scale,
                    "its material" => e.material = t.material.clone(),
                    "its body" => e.body = t.body,
                    "its collider" => e.collider = t.collider,
                    "its overrides" => e.overrides = t.overrides.clone(),
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
        scene.view = top.pick("view", &base.view, &ours.view, &theirs.view);
        scene.sun = top.pick("sun", &base.sun, &ours.sun, &theirs.sun);
        scene.fog = top.pick("fog", &base.fog, &ours.fog, &theirs.fog);
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
    desc.model = m.pick("its model", &b.model, &o.model, &t.model);
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
    desc.material = m.pick("its material", &b.material, &o.material, &t.material);
    desc.body = m.pick("its body", &b.body, &o.body, &t.body);
    desc.collider = m.pick("its collider", &b.collider, &o.collider, &t.collider);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(text: &str) -> Scene {
        let mut scene: Scene = ron::from_str(text).unwrap();
        scene.assign_ids();
        scene
    }

    const BASE: &str = r#"(entities: [
        (id: "a1", name: "tree", model: "m", transform: (position: (0.0, 0.0, 0.0))),
        (id: "b2", name: "rock", model: "m", material: "stone"),
        (id: "c3", name: "hut", model: "m", children: [(id: "d4", name: "door", model: "m")]),
    ])"#;

    #[test]
    fn different_entities_changed_on_each_side_merge_cleanly() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)"));
        let theirs = scene(&BASE.replace("\"stone\"", "\"moss\""));
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let tree = merged.scene.find("tree").unwrap();
        assert_eq!(tree.transform.position.x, 5.0);
        assert_eq!(
            merged.scene.find("rock").unwrap().material,
            crate::scene::MaterialRef::Named("moss".into())
        );
    }

    #[test]
    fn different_fields_of_one_entity_merge_and_the_same_field_conflicts_in_words() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)"));
        let theirs = scene(
            &BASE
                .replace("\"tree\"", "\"pine\"")
                .replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 9.0)"),
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        let tree = merged.scene.get("a1".parse().unwrap()).unwrap();
        assert_eq!(tree.name, "pine", "their rename");
        assert_eq!(tree.transform.position.x, 5.0, "our move, kept");
        assert_eq!(merged.conflicts.len(), 1);
        let said = merged.conflicts[0].to_string();
        assert!(said.contains("both changed its position"), "{said}");
        assert!(said.contains("kept ours"), "{said}");
    }

    #[test]
    fn additions_on_both_sides_and_a_deletion_all_land() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace(
            r#"(id: "b2", name: "rock", model: "m", material: "stone"),"#,
            r#"(id: "b2", name: "rock", model: "m", material: "stone"), (id: "e5", name: "bush", model: "m"),"#,
        ));
        let theirs = scene(
            &BASE
                .replace(r#"(id: "b2", name: "rock", model: "m", material: "stone"),"#, "")
                .replace(
                    r#"[(id: "d4", name: "door", model: "m")]"#,
                    r#"[(id: "d4", name: "door", model: "m"), (id: "f6", name: "window", model: "m")]"#,
                ),
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let names: Vec<&str> = merged
            .scene
            .flatten()
            .iter()
            .map(|(e, _)| e.name.as_str())
            .collect();
        assert_eq!(names, ["tree", "bush", "hut", "door", "window"]);
    }

    #[test]
    fn deleting_what_the_other_side_changed_keeps_it_and_says_so() {
        let base = scene(BASE);
        let ours = scene(&BASE.replace(
            r#"(id: "b2", name: "rock", model: "m", material: "stone"),"#,
            "",
        ));
        let theirs = scene(&BASE.replace("\"stone\"", "\"moss\""));
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(
            merged.scene.find("rock").is_some(),
            "kept: a delete can be redone"
        );
        assert_eq!(merged.conflicts.len(), 1);
        let said = merged.conflicts[0].to_string();
        assert!(
            said.contains("ours deleted it, theirs changed it; kept it"),
            "{said}"
        );
    }

    #[test]
    fn components_merge_one_by_one() {
        let base = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m", components: { "door": (open: false) })])"#,
        );
        let ours = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m", components: { "door": (open: true) })])"#,
        );
        let theirs = scene(
            r#"(entities: [(id: "a1", name: "gate", model: "m", components: { "door": (open: false), "loot": ("chest") })])"#,
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let gate = &merged.scene.entities[0];
        assert_eq!(gate.components["door"].get_ron(), "(open: true)");
        assert_eq!(gate.components["loot"].get_ron(), "(\"chest\")");
    }

    #[test]
    fn two_moves_that_would_make_a_loop_do_not() {
        let base = scene(
            r#"(entities: [(id: "a1", name: "a", model: "m"), (id: "b2", name: "b", model: "m")])"#,
        );
        let ours = scene(
            r#"(entities: [(id: "b2", name: "b", model: "m", children: [(id: "a1", name: "a", model: "m")])])"#,
        );
        let theirs = scene(
            r#"(entities: [(id: "a1", name: "a", model: "m", children: [(id: "b2", name: "b", model: "m")])])"#,
        );
        let merged = merge_scenes(&base, &ours, &theirs);
        assert_eq!(
            merged.scene.flatten().len(),
            2,
            "both still there: {:?}",
            merged.scene
        );
        assert!(!merged.conflicts.is_empty());
    }

    #[test]
    fn a_conflict_can_be_settled_theirs_way_field_by_field() {
        let base = scene(BASE);
        let ours = scene(
            &BASE
                .replace("(0.0, 0.0, 0.0)", "(5.0, 0.0, 0.0)")
                .replace("\"stone\"", "\"bark\""),
        );
        let theirs = scene(
            &BASE
                .replace("(0.0, 0.0, 0.0)", "(0.0, 0.0, 9.0)")
                .replace("\"stone\"", "\"moss\""),
        );
        let mut merged = merge_scenes(&base, &ours, &theirs);
        assert_eq!(merged.conflicts.len(), 2);
        let position = merged
            .conflicts
            .iter()
            .find(|c| c.field == "its position")
            .unwrap()
            .clone();
        assert!(position.take_theirs(&mut merged.scene, &theirs));
        assert_eq!(merged.scene.find("tree").unwrap().transform.position.z, 9.0);
        assert_eq!(
            merged.scene.find("rock").unwrap().material,
            crate::scene::MaterialRef::Named("bark".into()),
            "the other conflict is still ours"
        );
    }
}
