//! A `.unity` or `.prefab` file as scrap entities.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

use scrap::glam::{Quat, Vec3};
use scrap::scene::{Body, BodyProps, Collider, EntityDesc, Joint, Lens, Light, MaterialRef};
use scrap::{AssetLink, EntityId, Transform};
use yaml_rust2::Yaml;

use super::yaml::{self, Doc, Get, Ref};
use super::{snake, Report, Unity};

/// Unity's built-in meshes live in one file with this GUID.
const BUILTIN: &str = "0000000000000000e000000000000000";

const GAME_OBJECT: u32 = 1;
const TRANSFORM: u32 = 4;
const RECT_TRANSFORM: u32 = 224;
const PREFAB_INSTANCE: u32 = 1001;
/// The fileID Unity gives a model's root GameObject, in every model.
const MODEL_ROOT: i64 = 919132149155446097;
/// And its root's Transform: what a placed model's move, turn and scale
/// name. Any other of a model's transforms is a node inside it, which a
/// model brought over whole has no part for.
const MODEL_ROOT_TRANSFORM: i64 = -8679921383154817045;

/// An entity's ID from a Unity fileID: the same object, the same ID, every
/// time the file is imported (docs/unity-import.md).
pub fn entity_id(file_id: i64) -> EntityId {
    let mut z = (file_id as u64) ^ 0x9e37_79b9_7f4a_7c15;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    EntityId::from_raw(if z == 0 { 1 } else { z })
}

/// Unity is left-handed, scrap right-handed: the Z axis is mirrored.
fn position(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], -v[2])
}

fn rotation(q: [f32; 4]) -> Quat {
    Quat::from_xyzw(-q[0], -q[1], q[2], q[3]).normalize()
}

/// An axis of turning under the same mirror: a pseudo-vector.
fn axis(v: [f32; 3]) -> Vec3 {
    Vec3::new(-v[0], -v[1], v[2])
}

/// A scene's directional light as scrap's sun: the hour whose sun
/// shines the way it does, and how bright.
pub fn sun(text: &str) -> Option<scrap::scene::Sun> {
    let docs = yaml::documents(text);
    let light = docs
        .iter()
        .find(|d| d.kind == "Light" && d.body.i64("m_Type") == Some(1))?;
    let object = light.body.reference("m_GameObject")?.file_id;
    let transform = docs.iter().find(|d| {
        matches!(d.class, TRANSFORM)
            && d.body
                .reference("m_GameObject")
                .is_some_and(|r| r.file_id == object)
    })?;
    let turn = transform.body.quat("m_LocalRotation").map(rotation)?;
    // Unity's light shines along its +z; mirrored, scrap's −z.
    let travel = turn * Vec3::NEG_Z;
    let up = (-travel.y).clamp(-1.0, 1.0).asin().max(0.05);
    // Morning when the light travels toward +x, as scrap's sun does.
    let angle = if travel.x > 0.0 {
        up
    } else {
        std::f32::consts::PI - up
    };
    let tint = light.body.color("m_Color").map(|c| [c[0], c[1], c[2]]);
    Some(scrap::scene::Sun {
        hour: 6.0 + angle / std::f32::consts::PI * 12.0,
        intensity: light.body.f32("m_Intensity").unwrap_or(1.0),
        // Exactly where Unity's stood, and its colour: the hour is only
        // near it.
        toward: Some(travel),
        tint: tint.filter(|c| *c != [1.0, 1.0, 1.0]),
        shadow_strength: light.body["m_Shadows"].f32("m_Strength").unwrap_or(1.0),
        temperature: temperature(&light.body),
        ..scrap::scene::Sun::default()
    })
}

/// A light's colour temperature, when it uses one: URP always honours it
/// (it sets `GraphicsSettings.lightsUseColorTemperature`).
fn temperature(light: &yaml_rust2::Yaml) -> Option<f32> {
    (light.i64("m_UseColorTemperature") == Some(1)).then(|| light.f32("m_ColorTemperature").unwrap_or(6570.0))
}

/// The roots of a Unity file as entities, children under them.
pub fn convert_file(unity: &Unity, text: &str, report: &mut Report) -> Vec<EntityDesc> {
    let docs = yaml::documents(text);
    let mut parts = Parts::default();

    // Where each object is: a transform's GameObject, a stripped
    // transform's prefab instance.
    let mut game_object_of: HashMap<i64, i64> = HashMap::new();
    let mut instance_of: HashMap<i64, i64> = HashMap::new();
    for d in &docs {
        if matches!(d.class, TRANSFORM | RECT_TRANSFORM) {
            if d.stripped {
                if let Some(r) = d.body.reference("m_PrefabInstance") {
                    instance_of.insert(d.file_id, r.file_id);
                }
            } else if let Some(r) = d.body.reference("m_GameObject") {
                game_object_of.insert(d.file_id, r.file_id);
            }
        }
    }
    // A transform's entity: its GameObject's, or its prefab instance's.
    let entity_of_transform = |t: i64| -> Option<i64> {
        game_object_of
            .get(&t)
            .or_else(|| instance_of.get(&t))
            .copied()
    };
    // Components of a GameObject, stripped ones included (an added
    // component on a prefab's part names a stripped GameObject).
    let mut components: HashMap<i64, Vec<&Doc>> = HashMap::new();
    for d in &docs {
        if let Some(go) = d.body.reference("m_GameObject") {
            if !matches!(d.class, TRANSFORM | RECT_TRANSFORM) {
                components.entry(go.file_id).or_default().push(d);
            }
        }
    }
    // Rigidbodies' GameObjects: what a joint's connected body names.
    let body_object: HashMap<i64, i64> = docs
        .iter()
        .filter(|d| d.kind == "Rigidbody")
        .filter_map(|d| Some((d.file_id, d.body.reference("m_GameObject")?.file_id)))
        .collect();
    let refs = Refs {
        unity,
        entity_of: docs
            .iter()
            .filter_map(|d| match d.class {
                // A placeholder for an object inside a prefab instance: in
                // scrap the instance is one line, so it is the instance.
                _ if d.stripped => d
                    .body
                    .reference("m_PrefabInstance")
                    .map(|i| (d.file_id, i.file_id)),
                GAME_OBJECT => Some((d.file_id, d.file_id)),
                TRANSFORM | RECT_TRANSFORM => {
                    entity_of_transform(d.file_id).map(|e| (d.file_id, e))
                }
                PREFAB_INSTANCE => Some((d.file_id, d.file_id)),
                _ => d
                    .body
                    .reference("m_GameObject")
                    .map(|g| (d.file_id, g.file_id)),
            })
            .collect(),
        body_object,
        scoped: HashMap::new(),
    };

    // A stripped object stands for a part of a prefab instance: which
    // part, as the prefab's source names it.
    let stripped_source: HashMap<i64, (i64, String)> = docs
        .iter()
        .filter(|d| d.stripped)
        .filter_map(|d| {
            let r = d.body.reference("m_CorrespondingSourceObject")?;
            Some((d.file_id, (r.file_id, r.guid?)))
        })
        .collect();
    let part_of_stripped = |id: i64| stripped_source.get(&id).cloned();
    // What hangs on a part of an instance rather than on the instance.
    let mut on_part: HashMap<i64, (i64, String)> = HashMap::new();
    // Every entity, and its parent entity.
    // Each GameObject's RectTransform's size: a TextMeshPro's box.
    let rect_size: HashMap<i64, [f32; 2]> = docs
        .iter()
        .filter(|d| d.class == RECT_TRANSFORM && !d.stripped)
        .filter_map(|d| {
            let go = d.body.reference("m_GameObject")?.file_id;
            let s = &d.body["m_SizeDelta"];
            Some((go, [s.f32("x")?, s.f32("y")?]))
        })
        .collect();
    let mut entities: BTreeMap<i64, EntityDesc> = BTreeMap::new();
    let mut parent: HashMap<i64, i64> = HashMap::new();
    let mut order: HashMap<i64, usize> = HashMap::new();
    for d in &docs {
        match d.class {
            GAME_OBJECT if !d.stripped => {
                let mut desc = EntityDesc {
                    id: entity_id(d.file_id),
                    name: d.body.str("m_Name").unwrap_or("GameObject").to_string(),
                    ..Default::default()
                };
                if let Some(layer) = d
                    .body
                    .i64("m_Layer")
                    .filter(|l| *l != 0)
                    .and_then(|l| unity.layers.get(&l))
                {
                    desc.set_part(&scrap::scene::LayerName(layer.clone()));
                }
                if d.body.i64("m_IsActive") == Some(0) {
                    desc.inactive = true;
                }
                for c in components.get(&d.file_id).into_iter().flatten() {
                    if is_text_mesh_pro(c) {
                        text_mesh_pro(&mut desc, &c.body, rect_size.get(&d.file_id).copied(), report);
                        continue;
                    }
                    component(&mut desc, c, &refs, report);
                }
                // A switched-off renderer draws nothing, and a mesh with no
                // renderer at all (a trigger's shape, a collider's) is not
                // drawn either: its mesh is not brought over to be drawn.
                let own = || components.get(&d.file_id).into_iter().flatten();
                let renderer = |c: &&Doc| matches!(c.kind.as_str(), "MeshRenderer" | "SkinnedMeshRenderer");
                let hidden = own().any(|c| renderer(&c) && c.body.i64("m_Enabled") == Some(0))
                    || (own().any(|c| c.kind == "MeshFilter") && !own().any(|c| renderer(&c)));
                // A mesh collider keeps its mesh as its collision model.
                if hidden {
                    if desc.part::<Collider>() == Some(Collider::Model)
                        && desc.part::<scrap::scene::CollisionModel>().is_none()
                    {
                        if let Some(drawn) = desc.part::<scrap::scene::ModelRef>() {
                            desc.set_part(&scrap::scene::CollisionModel(drawn.0));
                        }
                    }
                    desc.clear_part::<scrap::scene::ModelRef>();
                    desc.clear_part::<MaterialRef>();
                }
                entities.insert(d.file_id, desc);
            }
            TRANSFORM | RECT_TRANSFORM if !d.stripped => {
                let Some(go) = game_object_of.get(&d.file_id) else {
                    continue;
                };
                if let Some(father) = d.body.reference("m_Father").filter(|r| r.file_id != 0) {
                    if let Some(p) = entity_of_transform(father.file_id) {
                        parent.insert(*go, p);
                    }
                    if let Some(part) = part_of_stripped(father.file_id) {
                        on_part.insert(*go, part);
                    }
                }
                for (i, child) in d.body.list("m_Children").iter().enumerate() {
                    if let Some(c) =
                        yaml::reference(child).and_then(|r| entity_of_transform(r.file_id))
                    {
                        order.insert(c, i);
                    }
                }
            }
            PREFAB_INSTANCE => {
                let Some((desc, into)) =
                    instance(d, &mut parts, &entity_of_transform, unity, report)
                else {
                    continue;
                };
                if let Some(p) = into {
                    parent.insert(d.file_id, p);
                }
                let father = d.body["m_Modification"].reference("m_TransformParent");
                if let Some(part) = father.and_then(|r| part_of_stripped(r.file_id)) {
                    on_part.insert(d.file_id, part);
                }
                entities.insert(d.file_id, desc);
            }
            _ => {}
        }
    }
    // Transforms come to entities last: a GameObject's position is its
    // transform's — a RectTransform's from its anchors in its parent's.
    let rect_docs: HashMap<i64, Yaml> = docs
        .iter()
        .filter(|d| d.class == RECT_TRANSFORM && !d.stripped)
        .map(|d| (d.file_id, d.body.clone()))
        .collect();
    let rects = |id: i64| rect_docs.get(&id).cloned();
    for d in docs
        .iter()
        .filter(|d| matches!(d.class, TRANSFORM | RECT_TRANSFORM) && !d.stripped)
    {
        let Some(go) = game_object_of.get(&d.file_id) else {
            continue;
        };
        if let Some(desc) = entities.get_mut(go) {
            desc.transform = transform(&d.body);
            if d.class == RECT_TRANSFORM {
                desc.transform.position = position(rect_position(&d.body, &rects));
            }
        }
    }
    // Components added to a prefab instance's parts in this file land on
    // the instance: scrap's instance is one line.
    // Components added to a part go on that part, as an override of it;
    // on the prefab's root, on the instance line itself.
    for d in docs.iter().filter(|d| d.class == GAME_OBJECT && d.stripped) {
        let Some(instance) = d.body.reference("m_PrefabInstance") else {
            continue;
        };
        // What the prefab's part has already: a script added again beside
        // its own is not the one that counts — Dacha's providers contribute
        // only what is absent, and the prefab's comes first — so the
        // prefab's stays (a heap's own drops, its burst of scrap with them).
        let had: HashSet<String> = part_of_stripped(d.file_id)
            .map(|(fid, guid)| {
                let of = parts.of(unity, &guid, 0);
                let key = of.keys.get(&fid).copied();
                of.components
                    .values()
                    .filter(|(on, _)| Some(*on) == key)
                    .map(|(_, name)| name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let added = components
            .get(&d.file_id)
            .into_iter()
            .flatten()
            .filter(|c| {
                let again = c.kind == "MonoBehaviour" && removal(c, unity).is_some_and(|name| had.contains(&name));
                if again {
                    report.skip("a script added to a prefab's part that has one already (the prefab's kept)");
                }
                !again
            })
            .collect::<Vec<_>>();
        let part = part_of_stripped(d.file_id).and_then(|(fid, guid)| {
            let of = parts.of(unity, &guid, 0);
            of.keys.get(&fid).copied().filter(|k| Some(*k) != of.root)
        });
        let Some(desc) = entities.get_mut(&instance.file_id) else {
            continue;
        };
        match part {
            None => {
                for c in added {
                    component(desc, c, &refs, report);
                }
            }
            Some(key) => {
                let mut given = EntityDesc::default();
                for c in added {
                    component(&mut given, c, &refs, report);
                }
                let change = scrap::scene::Override::between(&EntityDesc::default(), &given);
                if !change.is_empty() {
                    desc.overrides.entry(key).or_default().merge(change);
                }
            }
        }
    }
    // Hung on a part: the part's key, where it is not the prefab's root.
    for (child, (fid, guid)) in on_part {
        let of = parts.of(unity, &guid, 0);
        let key = of.keys.get(&fid).copied().filter(|k| Some(*k) != of.root);
        if let (Some(key), Some(desc)) = (key, entities.get_mut(&child)) {
            desc.in_part = Some(key);
        }
    }

    // Children under their parents, in the order Unity keeps them.
    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut roots = Vec::new();
    for id in entities.keys() {
        match parent.get(id) {
            Some(p) if entities.contains_key(p) => children.entry(*p).or_default().push(*id),
            _ => roots.push(*id),
        }
    }
    for list in children.values_mut() {
        list.sort_by_key(|c| order.get(c).copied().unwrap_or(usize::MAX));
    }
    // Scenes list their roots in order; prefabs have one.
    let root_order: HashMap<i64, usize> = docs
        .iter()
        .find(|d| d.kind == "SceneRoots")
        .map(|d| {
            d.body
                .list("m_Roots")
                .iter()
                .enumerate()
                .filter_map(|(i, r)| Some((entity_of_transform(yaml::reference(r)?.file_id)?, i)))
                .collect()
        })
        .unwrap_or_default();
    roots.sort_by_key(|r| root_order.get(r).copied().unwrap_or(usize::MAX));

    fn build(
        id: i64,
        entities: &mut BTreeMap<i64, EntityDesc>,
        children: &HashMap<i64, Vec<i64>>,
    ) -> Option<EntityDesc> {
        let mut desc = entities.remove(&id)?;
        for c in children.get(&id).into_iter().flatten() {
            if let Some(child) = build(*c, entities, children) {
                desc.children.push(child);
            }
        }
        Some(desc)
    }
    let mut roots: Vec<EntityDesc> = roots
        .into_iter()
        .filter_map(|r| build(r, &mut entities, &children))
        .collect();
    parts_under_bodies(&mut roots, false);
    roots
}

/// Colliders under a Rigidbody are parts of its body, as in Unity: a
/// collider with no Rigidbody of its own, below one that has it, becomes a
/// `Part` (a trigger a `TriggerPart`) rather than standing still on its own.
fn parts_under_bodies(lines: &mut [EntityDesc], under_body: bool) {
    for line in lines {
        let body = line.body();
        if under_body {
            match body {
                Body::Static => line.set_part(&Body::Part),
                Body::Trigger => line.set_part(&Body::TriggerPart),
                _ => {}
            }
        }
        let below = under_body || matches!(body, Body::Dynamic | Body::Kinematic);
        parts_under_bodies(&mut line.children, below);
    }
}

/// What a component's references resolve to.
struct Refs<'a> {
    unity: &'a Unity,
    /// Any object of the file → the entity it is on.
    entity_of: HashMap<i64, i64>,
    body_object: HashMap<i64, i64>,
    /// Objects of another file (a prefab whose component an instance
    /// changes) → the id its part has here, looked up first.
    scoped: HashMap<i64, EntityId>,
}

/// A RectTransform's local position, Unity's hand: its anchored position
/// from the point its anchors pick in its parent's rectangle (the anchors'
/// span taken at its pivot), and its z. `rects` finds a RectTransform by
/// its fileID; a parent that is not one has no size.
fn rect_position(t: &Yaml, rects: &dyn Fn(i64) -> Option<Yaml>) -> [f32; 3] {
    let v2 = |y: &Yaml, key: &str, or: [f32; 2]| -> [f32; 2] {
        let v = &y[key];
        [v.f32("x").unwrap_or(or[0]), v.f32("y").unwrap_or(or[1])]
    };
    // A rectangle's size: its size delta plus its anchors' share of its
    // parent's.
    fn size(t: &Yaml, rects: &dyn Fn(i64) -> Option<Yaml>, depth: usize) -> [f32; 2] {
        let v2 = |key: &str, or: f32| -> [f32; 2] {
            let v = &t[key];
            [v.f32("x").unwrap_or(or), v.f32("y").unwrap_or(or)]
        };
        let (delta, lo, hi) = (v2("m_SizeDelta", 0.0), v2("m_AnchorMin", 0.5), v2("m_AnchorMax", 0.5));
        let parent = (depth < 32)
            .then(|| t.reference("m_Father").filter(|r| r.file_id != 0))
            .flatten()
            .and_then(|r| rects(r.file_id))
            .map(|p| size(&p, rects, depth + 1))
            .unwrap_or([0.0, 0.0]);
        [delta[0] + (hi[0] - lo[0]) * parent[0], delta[1] + (hi[1] - lo[1]) * parent[1]]
    }
    let parent = t.reference("m_Father").filter(|r| r.file_id != 0).and_then(|r| rects(r.file_id));
    let (parent_size, parent_pivot) = match &parent {
        Some(p) => (size(p, rects, 0), v2(p, "m_Pivot", [0.5, 0.5])),
        None => ([0.0, 0.0], [0.5, 0.5]),
    };
    let (lo, hi, pivot) = (v2(t, "m_AnchorMin", [0.5, 0.5]), v2(t, "m_AnchorMax", [0.5, 0.5]), v2(t, "m_Pivot", [0.5, 0.5]));
    let anchored = v2(t, "m_AnchoredPosition", [0.0, 0.0]);
    let at = |i: usize| {
        let min = -parent_pivot[i] * parent_size[i];
        let anchor = lo[i] + (hi[i] - lo[i]) * pivot[i];
        min + parent_size[i] * anchor + anchored[i]
    };
    let z = t.vec3("m_LocalPosition").map_or(0.0, |p| p[2]);
    [at(0), at(1), z]
}

fn transform(t: &Yaml) -> Transform {
    let mut out = Transform {
        position: t
            .vec3("m_LocalPosition")
            .map(position)
            .unwrap_or(Vec3::ZERO),
        scale: t
            .vec3("m_LocalScale")
            .map(Vec3::from_array)
            .unwrap_or(Vec3::ONE),
        ..Default::default()
    };
    out.set_rotation(
        t.quat("m_LocalRotation")
            .map(rotation)
            .unwrap_or(Quat::IDENTITY),
    );
    out
}

/// A prefab instance: the prefab by name, where it stands and what it is
/// called, from its modifications. And the entity it is under.
fn instance(
    d: &Doc,
    parts: &mut Parts,
    entity_of_transform: &impl Fn(i64) -> Option<i64>,
    unity: &Unity,
    report: &mut Report,
) -> Option<(EntityDesc, Option<i64>)> {
    let source = d.body.reference("m_SourcePrefab")?;
    let (kind, name) = unity.named(source.guid.as_deref()?)?;
    if !matches!(kind, "prefab" | "model") {
        report.skip(format!(
            "an instance of a {kind} (only prefabs and models are placed)"
        ));
        return None;
    }
    let modification = &d.body["m_Modification"];
    let into = modification
        .reference("m_TransformParent")
        .filter(|r| r.file_id != 0)
        .and_then(|r| entity_of_transform(r.file_id));
    let mut local = Transform::default();
    let (mut p, mut q, mut s) = ([0.0f32; 3], [0.0, 0.0, 0.0, 1.0f32], [1.0f32; 3]);
    // A model placed as it is — an FBX dragged into the scene — is one
    // entity drawing it; a prefab is an instance of it.
    let mut desc = EntityDesc {
        id: entity_id(d.file_id),
        name: name.to_string(),
        ..Default::default()
    };
    if kind == "model" {
        desc.set_part(&scrap::scene::ModelRef(AssetLink::named(name)));
    } else {
        desc.prefab = AssetLink::named(name);
    }
    // The root's transform is what the modifications say. Which target is
    // the root: the one the prefab's own root is, or else the first carrying
    // m_LocalPosition, as Unity always writes the root's.
    let of = source.guid.as_deref().map(|g| parts.of(unity, g, 0));
    let key_of = |t: Option<i64>| of.as_ref().and_then(|o| o.keys.get(&t?).copied());
    let root_key = of.as_ref().and_then(|o| o.root);
    // What the modifications leave out is as the prefab has its root —
    // Unity writes a scale only where it differs from the prefab's.
    if let Some(base) = root_key.and_then(|k| of.as_ref()?.places.get(&k).copied()) {
        let (bp, bq) = (base.position, base.rotation());
        p = [bp.x, bp.y, -bp.z];
        q = [-bq.x, -bq.y, bq.z, bq.w];
        s = base.scale.to_array();
    }
    let mut root_target = modification
        .list("m_Modifications")
        .iter()
        .filter_map(|m| m.reference("target").map(|r| r.file_id))
        .find(|t| root_key.is_some() && key_of(Some(*t)) == root_key);
    // A part moved, turned or scaled: its axes as Unity says them.
    type Axes = ([Option<f32>; 3], [Option<f32>; 4], [Option<f32>; 3]);
    let mut moved: BTreeMap<EntityId, Axes> = BTreeMap::new();
    // A component's fields changed: the prefab's component, changed.
    let mut behaviours: BTreeMap<i64, Behaviour> = BTreeMap::new();
    // A built-in component's fields changed: it, changed, and which fields.
    let mut builtins: BTreeMap<i64, (Builtin, Vec<String>)> = BTreeMap::new();
    // Whether an anchored position is said: then it, not the local
    // position's x and y, is where a RectTransform stands.
    let anchored_too = modification
        .list("m_Modifications")
        .iter()
        .any(|m| m.str("propertyPath").is_some_and(|p| p.starts_with("m_AnchoredPosition")));
    for m in modification.list("m_Modifications") {
        let Some(path) = m.str("propertyPath") else {
            continue;
        };
        let target = m.reference("target").map(|r| r.file_id);
        let behaviour = target.and_then(|t| Some((t, of.as_ref()?.behaviours.get(&t)?)));
        if let Some((t, b)) = behaviour.filter(|_| kind == "prefab" && !path.starts_with("m_")) {
            let b = behaviours.entry(t).or_insert_with(|| b.clone());
            modify(&mut b.body, path, m);
            continue;
        }
        let builtin = target.and_then(|t| Some((t, of.as_ref()?.builtins.get(&t)?)));
        if let Some((t, b)) = builtin.filter(|_| {
            kind == "prefab" && path != "m_Enabled" && !path.starts_with("m_Materials")
        }) {
            let (b, fields) = builtins.entry(t).or_insert_with(|| (b.clone(), Vec::new()));
            modify(&mut b.doc.body, path, m);
            fields.push(field_of(path).to_string());
            continue;
        }
        let value = yaml::number(&m["value"]).unwrap_or(0.0) as f32;
        let axis = |name: &str| -> Option<usize> {
            let rest = path.strip_prefix(name)?.strip_prefix('.')?;
            ["x", "y", "z", "w"].iter().position(|a| *a == rest)
        };
        // A RectTransform's place is its anchored position (x, y) and its
        // local z: taken as the position, as it is under a parent that is
        // not a rectangle.
        let transforming = ["m_LocalPosition", "m_LocalRotation", "m_LocalScale", "m_AnchoredPosition"]
            .iter()
            .find_map(|f| Some((*f, axis(f)?)))
            .map(|(f, i)| if f == "m_AnchoredPosition" { ("m_LocalPosition", i) } else { (f, i) })
            .filter(|(f, i)| !(path.starts_with("m_LocalPosition") && *f == "m_LocalPosition" && *i < 2 && anchored_too));
        if let Some((field, i)) = transforming {
            if field == "m_LocalPosition" {
                root_target = root_target.or(target);
            }
            let on_root = target == root_target || root_target.is_none();
            if on_root {
                match field {
                    "m_LocalPosition" if i < 3 => p[i] = value,
                    "m_LocalRotation" => q[i] = value,
                    "m_LocalScale" if i < 3 => s[i] = value,
                    _ => {}
                }
            } else if let Some(part) = key_of(target).filter(|_| kind == "prefab") {
                let axes = moved.entry(part).or_default();
                match field {
                    "m_LocalPosition" if i < 3 => axes.0[i] = Some(value),
                    "m_LocalRotation" => axes.1[i] = Some(value),
                    "m_LocalScale" if i < 3 => axes.2[i] = Some(value),
                    _ => {}
                }
            } else if kind == "model" {
                report.skip("a placed model's part moved (a model is one entity)");
            } else {
                report.skip("a prefab instance's part moved, where the part is not found");
            }
        } else if path == "m_Materials.Array.data[0]" {
            // The first material swapped: on the placed thing, or on the
            // part whose renderer it is.
            let material = m
                .reference("objectReference")
                .and_then(|r| r.guid)
                .and_then(|g| unity.named(&g))
                .filter(|(k, _)| *k == "material");
            if let Some((_, name)) = material {
                let reference = MaterialRef::Named(AssetLink::named(name));
                let on = target
                    .and_then(|t| of.as_ref()?.components.get(&t))
                    .map(|(part, _)| *part)
                    .filter(|part| Some(*part) != root_key);
                match on {
                    Some(part) => desc.overrides.entry(part).or_default().set_part(&reference),
                    None => desc.set_part(&reference),
                }
            }
        } else if path.starts_with("m_Materials") {
            report.skip("a prefab modification of `m_Materials` past the first");
        } else if path == "m_Name" {
            if let Some(n) = m.str("value") {
                desc.name = n.to_string();
            }
        } else if path == "m_Controller" && kind == "model" {
            // A placed model given an animator graph for its own Animator —
            // a variant of a hand model that plays its grab and point: the
            // model is one entity, so the graph is the line's.
            let graph = m
                .reference("objectReference")
                .and_then(|r| r.guid)
                .and_then(|g| unity.named(&g))
                .filter(|(k, _)| *k == "animator");
            match graph {
                Some((_, name)) => desc.set_part(&scrap::scene::AnimatorRef(name.to_string())),
                None => report.skip("a placed model's Animator given no graph (or an override controller)"),
            }
        } else if path == "m_Enabled" && value == 0.0 && kind == "model" {
            // A placed model's renderer or collider switched off: drawn
            // not at all, solid by its mesh still if its collider is on.
            desc.set_part(&scrap::scene::CollisionModel(AssetLink::named(name)));
            desc.clear_part::<scrap::scene::ModelRef>();
            report.skip("a placed model's renderer switched off (drawn not at all)");
        } else if path == "m_Enabled" && value == 0.0 {
            // A part's renderer or collider switched off in this instance:
            // taken off it.
            let found = target.and_then(|t| of.as_ref()?.components.get(&t).cloned());
            match found {
                Some((part, what)) if what == "collider" || what == "model" => {
                    let change = desc.overrides.entry(part).or_default();
                    if !change.removed.contains(&what) {
                        change.removed.push(what);
                    }
                }
                _ => report.skip(format!("a prefab modification of `{path}`")),
            }
        } else if path == "m_IsActive" || path == "m_Layer" {
            // On the part it names — the prefab's root being the instance
            // itself, which the engine applies an override of the root to.
            let Some(part) = key_of(target) else {
                report.skip(format!(
                    "a prefab modification of `{path}` on something not a GameObject of it"
                ));
                continue;
            };
            // A placed model has no parts to override: it is its root.
            let mut own = scrap::scene::Override::default();
            let change = if kind == "model" {
                &mut own
            } else {
                desc.overrides.entry(part).or_default()
            };
            if path == "m_IsActive" {
                change.inactive = Some(value == 0.0);
            } else {
                change.set_part(&scrap::scene::LayerName(
                    unity
                        .layers
                        .get(&(value as i64))
                        .cloned()
                        .unwrap_or_default(),
                ));
            }
            if kind == "model" {
                own.apply(&mut desc);
            }
        } else if path.starts_with("m_LocalEulerAnglesHint") || path == "m_RootOrder" {
            // Editor hints: nothing to carry.
        } else {
            report.skip(format!("a prefab modification of `{}`", field_of(path)));
        }
    }
    local.position = position(p);
    local.scale = Vec3::from_array(s);
    local.set_rotation(rotation(q));
    desc.transform = local;
    // Each part moved: the axes said, the rest where the prefab has it.
    for (part, (at, turn, size)) in moved {
        let base = of
            .as_ref()
            .and_then(|o| o.places.get(&part))
            .copied()
            .unwrap_or_default();
        // The base back in Unity's hand, to fill the axes not said.
        let (bp, bq) = (base.position, base.rotation());
        let unity_p = [bp.x, bp.y, -bp.z];
        let unity_q = [-bq.x, -bq.y, bq.z, bq.w];
        let p: [f32; 3] = std::array::from_fn(|i| at[i].unwrap_or(unity_p[i]));
        let q: [f32; 4] = std::array::from_fn(|i| turn[i].unwrap_or(unity_q[i]));
        let s: [f32; 3] = std::array::from_fn(|i| size[i].unwrap_or(base.scale[i]));
        let mut t = Transform {
            position: position(p),
            scale: Vec3::from_array(s),
            ..Default::default()
        };
        t.set_rotation(rotation(q));
        desc.overrides.entry(part).or_default().transform = Some(t);
    }
    // Each component changed, whole, on its part, naming the parts of this
    // instance as this file has them.
    for b in behaviours.into_values() {
        let refs = Refs {
            unity,
            entity_of: HashMap::new(),
            body_object: HashMap::new(),
            // A link to the prefab's root is to the instance itself, as
            // the engine has it (its root takes the instance's id).
            scoped: b
                .links
                .iter()
                .map(|(k, e)| (*k, if Some(*e) == root_key { desc.id } else { desc.id.within(*e) }))
                .collect(),
        };
        let value = mono_behaviour(&b.body, &refs);
        let Ok(raw) = scrap::ron::value::RawValue::from_boxed_ron(value.into_boxed_str()) else {
            report.skip(format!("component `{}` changed in an instance, whose fields did not make RON", b.name));
            continue;
        };
        // The root's too: the engine applies an override of the root to
        // the instance itself.
        desc.overrides
            .entry(b.part)
            .or_default()
            .components
            .insert(b.name, raw);
    }
    // Each built-in component changed: made again from its changed fields,
    // and what it makes of the line set on its part — its body only when
    // what makes the body (a trigger, kinematic) is what changed, since
    // the part's other components have their say in it too.
    for (b, fields) in builtins.into_values() {
        let refs = Refs {
            unity,
            entity_of: HashMap::new(),
            body_object: HashMap::new(),
            scoped: HashMap::new(),
        };
        // The part made again from all it has — its colliders first, then
        // its body, as a whole object is — the changed one as changed, so
        // a trigger turned solid on a Rigidbody stays that Rigidbody's.
        let mut made = EntityDesc::default();
        made.name = desc.name.clone();
        let siblings: Vec<&Builtin> = of
            .as_ref()
            .map(|o| o.builtins.iter().filter(|(_, x)| x.part == b.part).map(|(_, x)| x).collect())
            .unwrap_or_default();
        let order = |kind: &str| match kind {
            k if k.ends_with("Collider") => 0,
            "Rigidbody" => 1,
            _ => 2,
        };
        let mut docs: Vec<&Doc> = siblings
            .iter()
            .filter(|x| x.doc.file_id != b.doc.file_id && x.doc.kind != b.doc.kind)
            .map(|x| &x.doc)
            .chain(std::iter::once(&b.doc))
            .collect();
        docs.sort_by_key(|d| order(&d.kind));
        let mut quiet = Report::default();
        for d in docs {
            let say = if d.file_id == b.doc.file_id { &mut *report } else { &mut quiet };
            let mut own = EntityDesc::default();
            if d.file_id == b.doc.file_id {
                component(&mut made, d, &refs, say);
            } else {
                // A sibling only for the body it makes.
                own.name = made.name.clone();
                if let Some(body) = made.parts.raw("body") {
                    let _ = own.parts.set_raw("body", body);
                }
                component(&mut own, d, &refs, say);
                if let Some(body) = own.parts.raw("body") {
                    let _ = made.parts.set_raw("body", body);
                }
            }
        }
        let body_changed = fields.iter().any(|f| f == "m_IsTrigger" || f == "m_IsKinematic");
        let joint = b.doc.kind.ends_with("Joint");
        let change = desc.overrides.entry(b.part).or_default();
        for (name, text) in made.parts.iter() {
            if name == "body" && !body_changed {
                continue;
            }
            if name == "joint" {
                continue;
            }
            let _ = change.parts.set_raw(name, text);
        }
        if joint {
            // What it is joined to, as a part of this instance; the world
            // when it names nothing.
            let to = b
                .doc
                .body
                .reference("m_ConnectedBody")
                .filter(|r| r.file_id != 0)
                .and_then(|r| b.links.get(&r.file_id).copied());
            let to = match to {
                Some(part) if Some(part) == root_key => desc.id,
                Some(part) => desc.id.within(part),
                None => EntityId::UNASSIGNED,
            };
            if let Some(j) = made.parts.try_get::<Joint>().ok().flatten() {
                change.set_part(&j.with_to(to));
            }
        }
    }
    // Components taken off a part: what each takes off the line.
    for removed in modification.list("m_RemovedComponents") {
        let found =
            yaml::reference(removed).and_then(|r| of.as_ref()?.components.get(&r.file_id).cloned());
        match found {
            Some((part, what)) => {
                let change = desc.overrides.entry(part).or_default();
                if !change.removed.contains(&what) {
                    change.removed.push(what);
                }
            }
            None => report.skip(
                "a component removed from a prefab instance, of a kind scrap has no place for",
            ),
        }
    }
    Some((desc, into))
}

/// A prefab file's GameObjects as a modification's `target` names them —
/// its own, and through the prefabs inside it theirs, whose id in the file is
/// Unity's `(instance ^ source) & i64::MAX` — to the key scrap's overrides
/// name each part by: its own id, or its instance's within its own.
#[derive(Default)]
struct Parts {
    files: HashMap<String, std::rc::Rc<PrefabParts>>,
}

#[derive(Default)]
struct PrefabParts {
    keys: HashMap<i64, EntityId>,
    /// The key the prefab's root has — for a variant, its instance's.
    root: Option<EntityId>,
    /// Each part's own place, as the prefab puts it.
    places: HashMap<EntityId, Transform>,
    /// Its components by the id a `m_RemovedComponents` names them: the
    /// part they are on, and what a removal takes off it.
    components: HashMap<i64, (EntityId, String)>,
    /// Its MonoBehaviours by the id a modification's `target` names them:
    /// what an instance changes a field of.
    behaviours: HashMap<i64, Behaviour>,
    /// Its colliders, bodies, joints and lamps by the id a modification's
    /// `target` names them: what an instance changes a size or an axis of.
    builtins: HashMap<i64, Builtin>,
}

/// A prefab's built-in component as it stands in the prefab: the part it
/// is on, its document, and what the objects it names are as parts.
#[derive(Clone)]
struct Builtin {
    part: EntityId,
    doc: Doc,
    links: std::rc::Rc<HashMap<i64, EntityId>>,
}

/// The built-in components an instance's change of a field is carried
/// for: what `component` makes of them, again, with the change.
fn changeable(kind: &str) -> bool {
    matches!(
        kind,
        "BoxCollider" | "SphereCollider" | "CapsuleCollider" | "MeshCollider" | "Rigidbody"
            | "HingeJoint" | "SpringJoint" | "ConfigurableJoint" | "CharacterJoint" | "FixedJoint"
            | "Light"
    )
}

/// A prefab's MonoBehaviour as it stands in the prefab: the part it is on,
/// its component name, its fields, and what the objects its fields name
/// are as parts of the prefab.
#[derive(Clone)]
struct Behaviour {
    part: EntityId,
    name: String,
    body: Yaml,
    links: std::rc::Rc<HashMap<i64, EntityId>>,
}

/// What taking a component away takes off a line: the field, or the
/// game component by its name.
fn removal(d: &Doc, unity: &Unity) -> Option<String> {
    Some(
        match d.kind.as_str() {
            "MeshFilter" | "MeshRenderer" | "SkinnedMeshRenderer" => "model",
            "BoxCollider" | "SphereCollider" | "CapsuleCollider" | "MeshCollider" => "collider",
            "Rigidbody" => "body",
            "Light" => "light",
            "Camera" => "camera",
            "ParticleSystem" | "ParticleSystemRenderer" => "particles",
            "AudioSource" => "sound",
            "Animator" => "animator",
            "NavMeshAgent" => "nav_mesh_agent",
            "MonoBehaviour" => {
                let script = d.body.reference("m_Script")?;
                let path = unity.guids.get(script.guid.as_deref()?)?;
                return Some(snake(&super::stem(path)));
            }
            _ => return None,
        }
        .to_string(),
    )
}

/// Where a prefab instance puts its prefab's root, from its modifications:
/// the target that carries `m_LocalPosition` is the root, as Unity writes.
fn placement(modification: &Yaml) -> Transform {
    let (mut p, mut q, mut s) = ([0.0f32; 3], [0.0, 0.0, 0.0, 1.0f32], [1.0f32; 3]);
    let mut root = None;
    for m in modification.list("m_Modifications") {
        let (Some(path), target) = (
            m.str("propertyPath"),
            m.reference("target").map(|r| r.file_id),
        ) else {
            continue;
        };
        let value = yaml::number(&m["value"]).unwrap_or(0.0) as f32;
        let Some((field, axis)) = path.split_once('.') else {
            continue;
        };
        let Some(i) = ["x", "y", "z", "w"].iter().position(|a| *a == axis) else {
            continue;
        };
        if field == "m_LocalPosition" {
            root = root.or(target);
        }
        if root.is_some() && target != root {
            continue;
        }
        match field {
            "m_LocalPosition" if i < 3 => p[i] = value,
            "m_LocalRotation" => q[i] = value,
            "m_LocalScale" if i < 3 => s[i] = value,
            _ => {}
        }
    }
    let mut out = Transform {
        position: position(p),
        scale: Vec3::from_array(s),
        ..Default::default()
    };
    out.set_rotation(rotation(q));
    out
}

impl Parts {
    fn of(&mut self, unity: &Unity, guid: &str, depth: usize) -> std::rc::Rc<PrefabParts> {
        if let Some(done) = self.files.get(guid) {
            return done.clone();
        }
        let mut out = PrefabParts::default();
        // A model is one entity: of its objects only its root is anything.
        if unity.named(guid).is_some_and(|(kind, _)| kind == "model") {
            let root = entity_id(MODEL_ROOT);
            out.keys.insert(MODEL_ROOT, root);
            out.keys.insert(MODEL_ROOT_TRANSFORM, root);
            out.root = Some(root);
        }
        let text = unity
            .named(guid)
            .filter(|(kind, _)| *kind == "prefab" && depth < 16)
            .and_then(|_| unity.guids.get(guid))
            .and_then(|path| std::fs::read_to_string(path).ok());
        let docs = text.as_deref().map(yaml::documents).unwrap_or_default();
        let rect_docs: HashMap<i64, Yaml> = docs
            .iter()
            .filter(|d| d.class == RECT_TRANSFORM && !d.stripped)
            .map(|d| (d.file_id, d.body.clone()))
            .collect();
        let rects = |id: i64| rect_docs.get(&id).cloned();
        // What each object of the file is a part of: a GameObject itself,
        // a component its GameObject, a placeholder its prefab instance.
        let links: std::rc::Rc<HashMap<i64, EntityId>> = std::rc::Rc::new(
            docs.iter()
                .filter_map(|d| {
                    let of = match d.class {
                        _ if d.stripped => d.body.reference("m_PrefabInstance")?.file_id,
                        GAME_OBJECT | PREFAB_INSTANCE => d.file_id,
                        _ => d.body.reference("m_GameObject")?.file_id,
                    };
                    Some((d.file_id, entity_id(of)))
                })
                .collect(),
        );
        for d in docs {
            match d.class {
                GAME_OBJECT if !d.stripped => {
                    out.keys.insert(d.file_id, entity_id(d.file_id));
                }
                TRANSFORM | RECT_TRANSFORM if !d.stripped => {
                    let go = d
                        .body
                        .reference("m_GameObject")
                        .map(|r| entity_id(r.file_id));
                    let top = d.body.reference("m_Father").is_none_or(|r| r.file_id == 0);
                    if top {
                        out.root = go;
                    }
                    if let Some(go) = go {
                        let mut place = transform(&d.body);
                        if d.class == RECT_TRANSFORM {
                            place.position = position(rect_position(&d.body, &rects));
                        }
                        out.places.insert(go, place);
                        // A move names the transform: the same part.
                        out.keys.insert(d.file_id, go);
                    }
                }
                PREFAB_INSTANCE => {
                    let Some(inner) = d
                        .body
                        .reference("m_SourcePrefab")
                        .and_then(|r| r.guid)
                        .map(|g| self.of(unity, &g, depth + 1))
                    else {
                        continue;
                    };
                    let n = entity_id(d.file_id);
                    // A variant's base is expanded straight into its scope.
                    let variant = d.body["m_Modification"]
                        .reference("m_TransformParent")
                        .is_none_or(|r| r.file_id == 0);
                    if variant {
                        out.root = Some(n);
                    }
                    let key_of = |e: EntityId| {
                        if Some(e) == inner.root {
                            n
                        } else if variant {
                            e
                        } else {
                            n.within(e)
                        }
                    };
                    for (x, e) in &inner.keys {
                        out.keys.insert((d.file_id ^ x) & i64::MAX, key_of(*e));
                    }
                    for (e, place) in &inner.places {
                        out.places.insert(key_of(*e), *place);
                    }
                    // Its root stands where this file puts it.
                    out.places.insert(n, placement(&d.body["m_Modification"]));
                    for (x, (e, what)) in &inner.components {
                        out.components
                            .insert((d.file_id ^ x) & i64::MAX, (key_of(*e), what.clone()));
                    }
                    // Its behaviours as this file sees them: with what this
                    // instance changes in them, and their links as parts
                    // here.
                    let mut changed: HashMap<i64, Behaviour> = inner.behaviours.clone();
                    for m in d.body["m_Modification"].list("m_Modifications") {
                        let (Some(path), Some(target)) =
                            (m.str("propertyPath"), m.reference("target"))
                        else {
                            continue;
                        };
                        if let Some(b) = changed.get_mut(&target.file_id) {
                            modify(&mut b.body, path, m);
                        }
                    }
                    let mut builtins: HashMap<i64, Builtin> = inner.builtins.clone();
                    for m in d.body["m_Modification"].list("m_Modifications") {
                        let (Some(path), Some(target)) =
                            (m.str("propertyPath"), m.reference("target"))
                        else {
                            continue;
                        };
                        if let Some(b) = builtins.get_mut(&target.file_id) {
                            modify(&mut b.doc.body, path, m);
                        }
                    }
                    for (x, b) in builtins {
                        let links = b.links.iter().map(|(k, e)| (*k, key_of(*e))).collect();
                        out.builtins.insert(
                            (d.file_id ^ x) & i64::MAX,
                            Builtin {
                                part: key_of(b.part),
                                links: std::rc::Rc::new(links),
                                ..b
                            },
                        );
                    }
                    for (x, b) in changed {
                        let links = b.links.iter().map(|(k, e)| (*k, key_of(*e))).collect();
                        out.behaviours.insert(
                            (d.file_id ^ x) & i64::MAX,
                            Behaviour {
                                part: key_of(b.part),
                                links: std::rc::Rc::new(links),
                                ..b
                            },
                        );
                    }
                }
                _ if !d.stripped => {
                    let go = d.body.reference("m_GameObject").filter(|r| r.file_id != 0);
                    if let Some(go) = go.as_ref().filter(|_| changeable(&d.kind)) {
                        out.builtins.insert(
                            d.file_id,
                            Builtin {
                                part: entity_id(go.file_id),
                                doc: d.clone(),
                                links: links.clone(),
                            },
                        );
                    }
                    if let (Some(go), Some(what)) = (go, removal(&d, unity)) {
                        if d.kind == "MonoBehaviour" {
                            out.behaviours.insert(
                                d.file_id,
                                Behaviour {
                                    part: entity_id(go.file_id),
                                    name: what.clone(),
                                    body: d.body.clone(),
                                    links: links.clone(),
                                },
                            );
                        }
                        out.components
                            .insert(d.file_id, (entity_id(go.file_id), what));
                    }
                }
                _ => {}
            }
        }
        let out = std::rc::Rc::new(out);
        self.files.insert(guid.to_string(), out.clone());
        out
    }
}

/// An AudioSource: its clip, volume, loop, whether it plays on awake, how
/// far it carries and its mixer group, by the group's name.
fn audio_source(desc: &mut EntityDesc, b: &Yaml, unity: &Unity, report: &mut Report) {
    // Unity 6 keeps the clip as `m_Resource`, before it `m_audioClip`.
    let clip = ["m_Resource", "m_audioClip"]
        .iter()
        .filter_map(|key| b.reference(key))
        .find_map(|r| unity.named(r.guid.as_deref()?))
        .filter(|(kind, _)| *kind == "sound");
    let Some((_, clip)) = clip else {
        report.skip("an AudioSource with no clip (the game gives it one)");
        return;
    };
    if desc.sound().is_some() {
        report.skip("a second AudioSource on one object (one sound an entity)");
        return;
    }
    if b.i64("Mute") == Some(1) {
        report.skip("a muted AudioSource");
    }
    // Spatial blend: the first key of its curve, 0 flat and 1 in the world.
    let blend = b["panLevelCustomCurve"]
        .list("m_Curve")
        .first()
        .and_then(|k| k.f32("value"))
        .unwrap_or(0.0);
    let group = b
        .reference("OutputAudioMixerGroup")
        .filter(|r| r.file_id != 0)
        .and_then(|r| {
            let text = std::fs::read_to_string(unity.guids.get(r.guid.as_deref()?)?).ok()?;
            yaml::documents(&text)
                .into_iter()
                .find(|d| d.file_id == r.file_id)
                .and_then(|d| d.body.str("m_Name").map(snake))
        })
        .unwrap_or_default();
    desc.set_part(&scrap::scene::SoundSource {
        clip: AssetLink::named(clip),
        volume: b.f32("m_Volume").unwrap_or(1.0),
        looped: b.i64("Loop") == Some(1),
        pitch: b.f32("m_Pitch").unwrap_or(1.0),
        // A switched-off AudioSource plays only when the game says so.
        on_start: b.i64("m_PlayOnAwake") != Some(0) && b.i64("m_Enabled") != Some(0),
        group,
        spatial: blend >= 0.5,
        near: b.f32("MinDistance").unwrap_or(1.0),
        far: b.f32("MaxDistance").unwrap_or(40.0),
    });
}

/// The field a property path is about: `hunts.any.Array.data[0]` → `hunts`.
fn field_of(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// One component of a GameObject, onto its entity.
fn component(desc: &mut EntityDesc, c: &Doc, refs: &Refs, report: &mut Report) {
    let b = &c.body;
    let collider = matches!(
        c.kind.as_str(),
        "BoxCollider" | "SphereCollider" | "CapsuleCollider" | "MeshCollider"
    );
    if collider && b.i64("m_Enabled") == Some(0) {
        // Switched off, it touches nothing.
        report.skip("a switched-off collider");
        return;
    }
    match c.kind.as_str() {
        "MeshFilter" => {
            if let Some(r) = b.reference("m_Mesh") {
                if let Some(model) = model(&r, refs.unity) {
                    let model = piece_of(refs.unity, model, &desc.name, &r);
                    desc.set_part(&scrap::scene::ModelRef(AssetLink::named(model)));
                }
            }
        }
        "MeshRenderer" | "SkinnedMeshRenderer" => {
            if c.kind == "SkinnedMeshRenderer" {
                // Its piece, with its skin: bent by the bones of the scene
                // it is under, found by name when it is spawned.
                if let Some(r) = b.reference("m_Mesh") {
                    if let Some(model) = model(&r, refs.unity) {
                        let model = piece_of(refs.unity, model, &desc.name, &r);
                        desc.set_part(&scrap::scene::ModelRef(AssetLink::named(model)));
                    }
                }
            }
            let materials = b.list("m_Materials");
            if let Some(first) = materials.first().and_then(yaml::reference) {
                if let Some((kind, name)) = first.guid.as_deref().and_then(|g| refs.unity.named(g))
                {
                    if kind == "material" {
                        desc.set_part(&MaterialRef::Named(AssetLink::named(name)));
                    }
                }
            }
            if materials.len() > 1 {
                report.skip("a renderer's second and later materials (one per entity)");
            }
        }
        "BoxCollider" => {
            let size = b.vec3("m_Size").unwrap_or([1.0; 3]);
            let center = b.vec3("m_Center").map(position).unwrap_or(Vec3::ZERO);
            desc.set_part(&Collider::Box {
                half: Vec3::from_array(size) * 0.5,
                center,
            });
            solid(desc, b);
        }
        "SphereCollider" => {
            desc.set_part(&Collider::Sphere {
                radius: b.f32("m_Radius").unwrap_or(0.5),
                center: b.vec3("m_Center").map(position).unwrap_or(Vec3::ZERO),
            });
            solid(desc, b);
        }
        "CapsuleCollider" => {
            let radius = b.f32("m_Radius").unwrap_or(0.5);
            let height = b.f32("m_Height").unwrap_or(2.0);
            desc.set_part(&Collider::Capsule {
                half_height: (height * 0.5 - radius).max(0.0),
                radius,
                center: b.vec3("m_Center").map(position).unwrap_or(Vec3::ZERO),
                // Unity's direction: 0 along x, 1 along y, 2 along z.
                axis: b.i64("m_Direction").map_or(1, |d| d.clamp(0, 2) as u8),
            });
            solid(desc, b);
        }
        "MeshCollider" => {
            desc.set_part(&Collider::Model);
            // Its own mesh, which need not be the one drawn.
            if let Some(r) = b.reference("m_Mesh") {
                if let Some(model) = model(&r, refs.unity) {
                    let model = piece_of(refs.unity, model, &desc.name, &r);
                    desc.set_part(&scrap::scene::CollisionModel(AssetLink::named(model)));
                }
            }
            solid(desc, b);
        }
        "Rigidbody" => {
            // A kinematic Rigidbody beside a trigger collider is still a
            // trigger — one that follows its transform, which a `Trigger`
            // does: a shop counter's volume, not a solid box over it.
            let kinematic = b.i64("m_IsKinematic") == Some(1);
            if !(kinematic && desc.body() == Body::Trigger) {
                desc.set_part(&if kinematic { Body::Kinematic } else { Body::Dynamic });
            }
            desc.set_part(&BodyProps {
                drag: b.f32("m_Drag").or(b.f32("m_LinearDamping")).unwrap_or(0.0),
                spin_drag: b
                    .f32("m_AngularDrag")
                    .or(b.f32("m_AngularDamping"))
                    .unwrap_or(0.05),
                gravity: if b.i64("m_UseGravity") == Some(0) {
                    0.0
                } else {
                    1.0
                },
                // Unity's own default is a kilogram.
                mass: Some(b.f32("m_Mass").unwrap_or(1.0)),
                // Continuous, speculative or dynamic: checked between steps.
                fast: b.i64("m_CollisionDetection").is_some_and(|m| m != 0),
                // How a frame between steps draws it, as authored: None,
                // Interpolate, Extrapolate. Unity's own default is None.
                drawn: match b.i64("m_Interpolate") {
                    Some(1) => scrap::scene::Drawn::Between,
                    Some(2) => scrap::scene::Drawn::Ahead,
                    _ => scrap::scene::Drawn::AtStep,
                },
                ..BodyProps::default()
            });
        }
        "Light" => {
            let color = b.color("m_Color").unwrap_or([1.0; 4]);
            let kind = b.i64("m_Type").unwrap_or(2);
            if kind == 1 {
                report.skip("a directional light (the scene's sun is its own setting)");
                return;
            }
            desc.set_part(&Light {
                color: (color[0], color[1], color[2]),
                intensity: b.f32("m_Intensity").unwrap_or(1.0),
                range: b.f32("m_Range").unwrap_or(10.0),
                cone_deg: (kind == 0).then(|| b.f32("m_SpotAngle").unwrap_or(30.0)),
                inner_cone_deg: (kind == 0).then(|| b.f32("m_InnerSpotAngle")).flatten(),
                shadows: b["m_Shadows"].i64("m_Type").unwrap_or(0) != 0,
                flare: 0.0,
                // URP's lamps fall off as the square of the distance.
                falloff: scrap::render::Falloff::InverseSquare,
                temperature: temperature(b),
            });
        }
        "Camera" => {
            let mut lens: Lens = scrap::ron::from_str("()").expect("a lens of defaults");
            lens.fov_deg = b.f32("field of view").unwrap_or(60.0);
            if b.i64("orthographic") == Some(1) {
                lens.ortho = b.f32("orthographic size");
            }
            desc.set_part(&lens);
        }
        "HingeJoint" | "FixedJoint" | "CharacterJoint" | "ConfigurableJoint" | "SpringJoint" => {
            let to = b
                .reference("m_ConnectedBody")
                .filter(|r| r.file_id != 0)
                .and_then(|r| refs.body_object.get(&r.file_id))
                .map(|go| entity_id(*go))
                .unwrap_or(EntityId::UNASSIGNED);
            let anchor = b.vec3("m_Anchor").map(position).unwrap_or(Vec3::ZERO);
            // Set by hand, the other body's end is where Unity says, not
            // where the two happen to stand: a rope's links held apart.
            let connected = (b.i64("m_AutoConfigureConnectedAnchor") == Some(0))
                .then(|| b.vec3("m_ConnectedAnchor").map(position))
                .flatten();
            desc.set_part(&match c.kind.as_str() {
                "FixedJoint" => Joint::Fixed { to },
                "HingeJoint" => Joint::Hinge {
                    to,
                    anchor,
                    axis: b.vec3("m_Axis").map(axis).unwrap_or(Vec3::X),
                    limits_deg: (b.i64("m_UseLimits") == Some(1)).then(|| {
                        let l = &b["m_Limits"];
                        // The axis is mirrored as a pseudo-vector, which
                        // keeps the sense of a turn about it: the angles
                        // are Unity's.
                        (l.f32("min").unwrap_or(0.0), l.f32("max").unwrap_or(0.0))
                    }),
                    motor: hinge_drive(b),
                    connected,
                },
                "SpringJoint" => Joint::Spring {
                    to,
                    anchor,
                    stiffness: b.f32("m_Spring").unwrap_or(10.0),
                    damping: b.f32("m_Damper").unwrap_or(0.2),
                    connected,
                },
                "ConfigurableJoint" => {
                    // Its turns, each locked (0), limited (1) or free (2):
                    // x between its low and high limits, y and z within
                    // their one limit either way.
                    let motion = |k: &str| b.i64(k).unwrap_or(2);
                    let limit = |k: &str| b[k].f32("limit").unwrap_or(0.0);
                    let about = |m: i64, low: f32, high: f32| match m {
                        0 => (0.0, 0.0),
                        1 => (low, high),
                        _ => (-180.0, 180.0),
                    };
                    let (mx, my, mz) = (motion("m_AngularXMotion"), motion("m_AngularYMotion"), motion("m_AngularZMotion"));
                    let limits_deg = (mx != 2 || my != 2 || mz != 2).then(|| {
                        [
                            about(mx, limit("m_LowAngularXLimit"), limit("m_HighAngularXLimit")),
                            about(my, -limit("m_AngularYLimit"), limit("m_AngularYLimit")),
                            about(mz, -limit("m_AngularZLimit"), limit("m_AngularZLimit")),
                        ]
                    });
                    let slides = [motion("m_XMotion"), motion("m_YMotion"), motion("m_ZMotion")];
                    let free: Vec<usize> = (0..3).filter(|i| slides[*i] != 0).collect();
                    if free.len() == 1 && [mx, my, mz] == [0, 0, 0] {
                        // Slides along one of its axes and turns not at
                        // all: a slider. Its axes as Unity has them: x the
                        // joint's axis, y its secondary, z the two crossed;
                        // a direction, so mirrored as a place is.
                        let i = free[0];
                        let primary = Vec3::from_array(b.vec3("m_Axis").unwrap_or([1.0, 0.0, 0.0])).normalize_or(Vec3::X);
                        let secondary = Vec3::from_array(b.vec3("m_SecondaryAxis").unwrap_or([0.0, 1.0, 0.0])).normalize_or(Vec3::Y);
                        let along = [primary, secondary, primary.cross(secondary)][i].normalize_or(Vec3::X);
                        let reach = limit("m_LinearLimit");
                        let limits = (slides[i] == 1).then_some((-reach, reach));
                        // Its drive along that axis, held at the target —
                        // which Unity drives the joint toward negated.
                        let drive = &b[["m_XDrive", "m_YDrive", "m_ZDrive"][i]];
                        let spring = drive.f32("positionSpring").unwrap_or(0.0);
                        let target = b.vec3("m_TargetPosition").map_or(0.0, |t| -t[i]);
                        let motor = (spring > 0.0).then(|| scrap::scene::Motor {
                            speed: 0.0,
                            hold: Some(target),
                            strength: spring,
                            damping: Some(drive.f32("positionDamper").unwrap_or(0.0)),
                        });
                        Joint::Slider { to, axis: Vec3::new(along.x, along.y, -along.z), limits, motor }
                    } else {
                        if slides != [0, 0, 0] {
                            report.skip("a ConfigurableJoint that slides and turns (brought over as a ball joint)");
                        }
                        Joint::Ball { to, anchor, connected, limits_deg }
                    }
                }
                _ => {
                    report.skip(format!("{} (brought over as a ball joint)", c.kind));
                    Joint::Ball { to, anchor, connected, limits_deg: None }
                }
            });
            // Unity writes an unbreakable joint's force as infinity.
            desc.set_joint_break(b.f32("m_BreakForce").filter(|f| f.is_finite() && *f < 1e30));
        }
        "MonoBehaviour" => {
            let Some(script) = b.reference("m_Script") else {
                return;
            };
            if is_text_mesh_pro(c) {
                text_mesh_pro(desc, b, None, report);
                return;
            }
            let Some(path) = script.guid.as_deref().and_then(|g| refs.unity.guids.get(g)) else {
                report.skip("a MonoBehaviour whose script is not in Assets/ (a package's)");
                return;
            };
            // Dacha's planar mirror: scrap's own, a camera reflected in
            // the plane the mirror's material shows.
            if super::stem(path) == "PlanarReflectionMirror" {
                // Which local axis its glass looks along (its `facing`:
                // Back, Forward, Up, Down, Right, Left), z mirrored as
                // the scene is: Back, −z, right for Unity's Quad, is +z.
                let facing = match b.i64("facing").unwrap_or(0) {
                    1 => Vec3::NEG_Z,
                    2 => Vec3::Y,
                    3 => Vec3::NEG_Y,
                    4 => Vec3::X,
                    5 => Vec3::NEG_X,
                    _ => Vec3::Z,
                };
                desc.set_part(&scrap::scene::RenderTexture {
                    name: "mirror".into(),
                    hide: Vec::new(),
                    mirror: true,
                    facing,
                });
                return;
            }
            let name = snake(&super::stem(path));
            let value = mono_behaviour(b, refs);
            match scrap::ron::value::RawValue::from_boxed_ron(value.into_boxed_str()) {
                Ok(raw) => {
                    desc.components.insert(name, raw);
                }
                Err(_) => report.skip(format!("component `{name}` whose fields did not make RON")),
            }
        }
        "Animator" => {
            let graph = b
                .reference("m_Controller")
                .and_then(|r| refs.unity.named(r.guid.as_deref()?))
                .filter(|(kind, _)| *kind == "animator");
            match graph {
                Some((_, name)) if b.i64("m_Enabled") != Some(0) => {
                    desc.set_part(&scrap::scene::AnimatorRef(name.to_string()))
                }
                Some(_) => report.skip("a switched-off Animator"),
                None => report.skip("an Animator with no controller (or an override controller)"),
            }
        }
        "AudioSource" => audio_source(desc, b, refs.unity, report),
        "NavMeshAgent" => nav_mesh_agent(desc, b, report),
        "ParticleSystem" => {
            shuriken(desc, b, report);
            bind_custom(desc, refs.unity);
        }
        "ParticleSystemRenderer" => {
            let mut emitter = desc.particles().unwrap_or_default();
            let e = &mut emitter;
            match b.i64("m_RenderMode").unwrap_or(0) {
                // Billboards, and stretched ones: longer the faster.
                0 | 2 | 3 => e.facing = true,
                1 => {
                    e.facing = true;
                    e.stretch = b.f32("m_VelocityScale").unwrap_or(0.0).max(0.1);
                }
                // Mesh.
                4 => {
                    if let Some(m) = b.reference("m_Mesh").and_then(|r| model(&r, refs.unity)) {
                        e.model = AssetLink::named(m);
                    }
                }
                _ => {}
            }
            if let Some(first) = b.list("m_Materials").first().and_then(yaml::reference) {
                if let Some(("material", name)) =
                    first.guid.as_deref().and_then(|g| refs.unity.named(g))
                {
                    e.material = Some(AssetLink::named(name));
                }
            }
            desc.set_part(&emitter);
            bind_custom(desc, refs.unity);
        }
        other => report.skip(other.to_string()),
    }
}

/// A NavMeshAgent as a `nav_mesh_agent` component: how the thing walks —
/// its speed, acceleration and turn (degrees a second), where it counts
/// as arrived, and its radius and height. Unity's navigation runs it; the
/// game that reads these walks it here. A switched-off one is not brought.
fn nav_mesh_agent(desc: &mut EntityDesc, b: &Yaml, report: &mut Report) {
    if b.i64("m_Enabled") == Some(0) {
        report.skip("a switched-off NavMeshAgent");
        return;
    }
    let f = |key: &str, default: f32| b.f32(key).unwrap_or(default);
    let value = format!(
        "(speed: {}, acceleration: {}, angularSpeed: {}, stoppingDistance: {}, radius: {}, height: {}, baseOffset: {})",
        f("m_Speed", 3.5),
        f("m_Acceleration", 8.0),
        f("m_AngularSpeed", 120.0),
        f("m_StoppingDistance", 0.0),
        f("m_Radius", 0.5),
        f("m_Height", 2.0),
        f("m_BaseOffset", 0.0),
    );
    match scrap::ron::value::RawValue::from_boxed_ron(value.into_boxed_str()) {
        Ok(raw) => {
            desc.components.insert("nav_mesh_agent".into(), raw);
        }
        Err(_) => report.skip("a NavMeshAgent whose fields did not make RON"),
    }
}

/// TextMeshPro's world-space text component (the package's `TMPro.TextMeshPro`;
/// not `TextMeshProUGUI`, a canvas's, which a game's own screens draw).
const TEXT_MESH_PRO: &str = "9541d86e2fd84c1d9990edf0852d74ab";

fn is_text_mesh_pro(c: &Doc) -> bool {
    c.kind == "MonoBehaviour"
        && c.body.reference("m_Script").and_then(|r| r.guid).as_deref() == Some(TEXT_MESH_PRO)
}

/// Words a TextMeshPro puts in the world, as a `text_mesh_pro` component:
/// its text as written (rich-text tags and all), its font size (TMP's
/// last fitted one when it sizes itself), its colour and — when its
/// vertex gradient is on — the four corners' tints over it (top left,
/// top right, bottom left, bottom right), its RectTransform's box (`None`
/// when the line changes only the component, in an instance), its
/// alignments as TMP numbers them and its margins (left, top, right,
/// bottom). The package's code draws it in Unity, so the game draws it
/// here; a switched-off one is not brought.
fn text_mesh_pro(desc: &mut EntityDesc, b: &Yaml, size: Option<[f32; 2]>, report: &mut Report) {
    if b.i64("m_Enabled") == Some(0) {
        report.skip("a switched-off TextMeshPro");
        return;
    }
    let text = match &b["m_text"] {
        Yaml::String(s) | Yaml::Real(s) => s.clone(),
        Yaml::Integer(i) => i.to_string(),
        Yaml::Boolean(v) => v.to_string(),
        _ => String::new(),
    };
    let color = b.color("m_fontColor").unwrap_or([1.0; 4]);
    let gradient: Vec<[f32; 4]> = if b.i64("m_enableVertexGradient") == Some(1) {
        let g = &b["m_fontColorGradient"];
        ["topLeft", "topRight", "bottomLeft", "bottomRight"]
            .iter()
            .map(|k| g.color(k).unwrap_or([1.0; 4]))
            .collect()
    } else {
        Vec::new()
    };
    let m = &b["m_margin"];
    let margin = [m.f32("x"), m.f32("y"), m.f32("z"), m.f32("w")].map(|v| v.unwrap_or(0.0));
    let f = |v: f32| format!("{v:?}");
    let c4 = |c: [f32; 4]| format!("({}, {}, {}, {})", f(c[0]), f(c[1]), f(c[2]), f(c[3]));
    let value = format!(
        "(text: {}, fontSize: {}, color: {}, gradient: [{}], size: {}, horizontal: {}, vertical: {}, margin: {}, style: {})",
        ron::to_string(&text).unwrap_or_else(|_| "\"\"".into()),
        f(b.f32("m_fontSize").unwrap_or(36.0)),
        c4(color),
        gradient.into_iter().map(c4).collect::<Vec<_>>().join(", "),
        size.map_or("None".into(), |s| format!("Some(({}, {}))", f(s[0]), f(s[1]))),
        b.i64("m_HorizontalAlignment").unwrap_or(1),
        b.i64("m_VerticalAlignment").unwrap_or(256),
        c4(margin),
        b.i64("m_fontStyle").unwrap_or(0),
    );
    match scrap::ron::value::RawValue::from_boxed_ron(value.into_boxed_str()) {
        Ok(raw) => {
            desc.components.insert("text_mesh_pro".into(), raw);
        }
        Err(_) => report.skip("a TextMeshPro whose text did not make RON"),
    }
}

/// A Shuriken value — a MinMaxCurve — as one number: a constant as it is,
/// two constants as their middle, a curve by where it ends up.
fn min_max(v: &Yaml) -> Option<f32> {
    let scalar = v.f32("scalar")?;
    Some(match v.i64("minMaxState").unwrap_or(0) {
        3 => (scalar + v.f32("minScalar").unwrap_or(scalar)) * 0.5,
        1 | 2 => scalar * curve_end(&v["maxCurve"]).unwrap_or(1.0),
        _ => scalar,
    })
}

/// A Shuriken value over the whole of a play — a rate that swells and
/// dies away — as its average: a curve evaluated as Unity does (Hermite
/// between keys) and averaged.
fn min_max_mean(v: &Yaml) -> Option<f32> {
    let scalar = v.f32("scalar")?;
    Some(match v.i64("minMaxState").unwrap_or(0) {
        3 => (scalar + v.f32("minScalar").unwrap_or(scalar)) * 0.5,
        1 => scalar * curve_mean(&v["maxCurve"]).unwrap_or(1.0),
        2 => {
            scalar
                * (curve_mean(&v["maxCurve"]).unwrap_or(1.0) + curve_mean(&v["minCurve"]).unwrap_or(1.0))
                * 0.5
        }
        _ => scalar,
    })
}

/// An AnimationCurve's value at `t`, Hermite between keys as Unity has
/// it, a stepped key held; `None` for a curve with no keys.
fn curve_at(curve: &Yaml, t: f32) -> Option<f32> {
    let keys: Vec<[f32; 4]> = curve
        .list("m_Curve")
        .iter()
        .filter_map(|k| Some([k.f32("time")?, k.f32("value")?, k.f32("inSlope").unwrap_or(0.0), k.f32("outSlope").unwrap_or(0.0)]))
        .collect();
    match keys.len() {
        0 => return None,
        1 => return Some(keys[0][1]),
        _ => {}
    }
    let i = keys.iter().rposition(|k| k[0] <= t).unwrap_or(0).min(keys.len() - 2);
    let (a, b) = (keys[i], keys[i + 1]);
    if t <= keys[0][0] {
        return Some(keys[0][1]);
    }
    if t >= keys[keys.len() - 1][0] {
        return Some(keys[keys.len() - 1][1]);
    }
    if !a[3].is_finite() || !b[2].is_finite() {
        return Some(a[1]);
    }
    let span = (b[0] - a[0]).max(1e-6);
    let u = ((t - a[0]) / span).clamp(0.0, 1.0);
    let (u2, u3) = (u * u, u * u * u);
    Some(
        (2.0 * u3 - 3.0 * u2 + 1.0) * a[1]
            + (u3 - 2.0 * u2 + u) * span * a[3]
            + (-2.0 * u3 + 3.0 * u2) * b[1]
            + (u3 - u2) * span * b[2],
    )
    .filter(|v| v.is_finite())
}

/// A MinMaxCurve over a particle's life, as (share of life, value) keys:
/// a constant as one key, a curve (or two, averaged) sampled.
fn min_max_keys(v: &Yaml) -> Vec<(f32, f32)> {
    let scalar = v.f32("scalar").unwrap_or(1.0);
    match v.i64("minMaxState").unwrap_or(0) {
        1 | 2 => (0..=KEYS)
            .map(|i| {
                let t = i as f32 / KEYS as f32;
                let max = curve_at(&v["maxCurve"], t).unwrap_or(1.0);
                let value = if v.i64("minMaxState") == Some(2) {
                    (max + curve_at(&v["minCurve"], t).unwrap_or(1.0)) * 0.5
                } else {
                    max
                };
                (t, scalar * value)
            })
            .collect(),
        3 => vec![(0.0, (scalar + v.f32("minScalar").unwrap_or(scalar)) * 0.5)],
        _ => vec![(0.0, scalar)],
    }
}

/// Keys a curve over a life is sampled into.
const KEYS: usize = 12;

/// A Gradient's colour and alpha at `t`: straight between its keys.
fn gradient_at(g: &Yaml, t: f32) -> Option<[f32; 4]> {
    let colours = g.i64("m_NumColorKeys").unwrap_or(2).clamp(1, 8) as usize;
    let alphas = g.i64("m_NumAlphaKeys").unwrap_or(2).clamp(1, 8) as usize;
    let key = |i: usize| g.color(&format!("key{i}"));
    let blend = |n: usize, time: &str, pick: &dyn Fn([f32; 4]) -> [f32; 4]| -> Option<[f32; 4]> {
        let at = |i: usize| g.f32(&format!("{time}{i}")).unwrap_or(0.0) / 65535.0;
        let mut out = pick(key(0)?);
        for i in 0..n {
            if at(i) <= t {
                out = pick(key(i)?);
            }
            if i + 1 < n && at(i) <= t && t <= at(i + 1) {
                let u = (t - at(i)) / (at(i + 1) - at(i)).max(1e-6);
                let (a, b) = (pick(key(i)?), pick(key(i + 1)?));
                return Some(std::array::from_fn(|c| a[c] + (b[c] - a[c]) * u));
            }
        }
        Some(out)
    };
    let rgb = blend(colours, "ctime", &|c| c)?;
    let a = blend(alphas, "atime", &|c| [c[3]; 4])?;
    Some([rgb[0], rgb[1], rgb[2], a[0]])
}

/// A MinMaxGradient over a particle's life as keys: a gradient sampled, a
/// colour as one key.
fn gradient_keys(v: &Yaml) -> Vec<(f32, [f32; 4])> {
    match v.i64("minMaxState").unwrap_or(0) {
        1 => (0..=KEYS)
            .filter_map(|i| {
                let t = i as f32 / KEYS as f32;
                Some((t, gradient_at(&v["maxGradient"], t)?))
            })
            .collect(),
        _ => v.color("maxColor").map(|c| vec![(0.0, c)]).unwrap_or_default(),
    }
}

/// A colour Unity saved to linear light: sRGB up to white, and past it (an
/// HDR glow) as it is.
fn linear_hdr(c: f32) -> f32 {
    if c <= 1.0 {
        scrap::material::srgb_to_linear(c.max(0.0))
    } else {
        c
    }
}

/// An AnimationCurve's average over its keys' span (0 to 1 for Shuriken).
fn curve_mean(curve: &Yaml) -> Option<f32> {
    let keys: Vec<[f32; 4]> = curve
        .list("m_Curve")
        .iter()
        .filter_map(|k| Some([k.f32("time")?, k.f32("value")?, k.f32("inSlope").unwrap_or(0.0), k.f32("outSlope").unwrap_or(0.0)]))
        .collect();
    match keys.len() {
        0 => return None,
        1 => return Some(keys[0][1]),
        _ => {}
    }
    let at = |t: f32| {
        let i = keys.iter().rposition(|k| k[0] <= t).unwrap_or(0).min(keys.len() - 2);
        let (a, b) = (keys[i], keys[i + 1]);
        let span = (b[0] - a[0]).max(1e-6);
        let u = ((t - a[0]) / span).clamp(0.0, 1.0);
        // An infinite tangent is a step: the value held to the next key.
        if !a[3].is_finite() || !b[2].is_finite() {
            return a[1];
        }
        let (u2, u3) = (u * u, u * u * u);
        (2.0 * u3 - 3.0 * u2 + 1.0) * a[1]
            + (u3 - 2.0 * u2 + u) * span * a[3]
            + (-2.0 * u3 + 3.0 * u2) * b[1]
            + (u3 - u2) * span * b[2]
    };
    let (start, end) = (keys[0][0], keys[keys.len() - 1][0]);
    let steps = 64;
    let sum: f32 = (0..steps).map(|i| at(start + (end - start) * (i as f32 + 0.5) / steps as f32)).sum();
    Some(sum / steps as f32).filter(|m| m.is_finite())
}

fn curve_end(curve: &Yaml) -> Option<f32> {
    curve.list("m_Curve").last().and_then(|k| k.f32("value"))
}

/// A MinMaxGradient's colour at its start and at its end.
fn gradient(v: &Yaml) -> Option<([f32; 4], [f32; 4])> {
    match v.i64("minMaxState").unwrap_or(0) {
        0 => v.color("maxColor").map(|c| (c, c)),
        2 => {
            let (a, b) = (v.color("minColor")?, v.color("maxColor")?);
            let mid = std::array::from_fn(|i| (a[i] + b[i]) * 0.5);
            Some((mid, mid))
        }
        _ => {
            let g = &v["maxGradient"];
            let last = g.i64("m_NumColorKeys").unwrap_or(2).clamp(1, 8) - 1;
            Some((g.color("key0")?, g.color(&format!("key{last}"))?))
        }
    }
}

/// An emitter's custom data streams onto the numbers its material's
/// shader names them by (`custom0.x`…): from the first such name on. A
/// stream its shader does not name stays unbound, and is not drawn with.
fn bind_custom(desc: &mut EntityDesc, unity: &Unity) {
    let Some(mut emitter) = desc.particles() else { return };
    let Some(material) = emitter.material.as_ref().map(|m| m.to_string()) else { return };
    let Some(names) = unity.declared_params.get(&material) else { return };
    let mut changed = false;
    for stream in &mut emitter.custom {
        let prefix = format!("{}.", stream.name);
        if let Some(slot) = names.iter().position(|n| n.starts_with(&prefix)) {
            // As many of its components as the shader names in a row.
            let named = names[slot..].iter().take_while(|n| n.starts_with(&prefix)).count();
            for (_, values) in &mut stream.keys {
                values.truncate(named);
            }
            stream.slot = slot as u8;
            changed = true;
        }
    }
    if changed {
        desc.set_part(&emitter);
    }
}

/// A Shuriken ParticleSystem onto the entity's emitter: the Main module's
/// life, speed, size, colour and gravity, the emission rate, the cone,
/// the simulation space, size and colour over lifetime. What has no
/// counterpart (bursts, noise, collision, trails…) the report names.
fn shuriken(desc: &mut EntityDesc, b: &Yaml, report: &mut Report) {
    let mut emitter = desc.particles().unwrap_or_default();
    let e = &mut emitter;
    let main = &b["InitialModule"];
    let rgb = |c: [f32; 4]| (c[0], c[1], c[2]);
    e.life = min_max_mean(&main["startLifetime"]).unwrap_or(5.0);
    e.speed = min_max_mean(&main["startSpeed"]).unwrap_or(5.0);
    e.size = min_max_mean(&main["startSize"]).unwrap_or(1.0);
    if let Some((start, _)) = gradient(&main["startColor"]) {
        e.color = rgb(start);
        e.alpha = start[3];
    }
    e.gravity = -9.81 * min_max(&main["gravityModifier"]).unwrap_or(0.0);
    // 0 Hierarchy, 1 Local: sizes by the transform's scale; 2 Shape: not.
    e.scaled = b.i64("scalingMode").unwrap_or(1) != 2;
    // 0 is Local, 1 World.
    e.local = b.i64("moveWithTransform") == Some(0);
    let emission = &b["EmissionModule"];
    e.rate = if emission.i64("enabled") == Some(0) {
        0.0
    } else {
        min_max_mean(&emission["rateOverTime"]).unwrap_or(10.0)
    };
    e.bursts = emission
        .list("m_Bursts")
        .iter()
        .map(|burst| {
            let count = min_max(&burst["countCurve"])
                .or_else(|| burst.f32("maxCount"))
                .unwrap_or(0.0);
            (
                burst.f32("time").unwrap_or(0.0),
                count.round().max(0.0) as u32,
            )
        })
        .collect();
    if emission
        .list("m_Bursts")
        .iter()
        .any(|burst| burst.i64("cycleCount").is_some_and(|c| c != 1))
    {
        report.skip("a ParticleSystem burst that repeats within a play (it goes off once)");
    }
    e.duration = b.f32("lengthInSec").unwrap_or(5.0);
    e.once = b.i64("looping") == Some(0);
    e.waits = b.i64("playOnAwake") == Some(0);
    let shape = &b["ShapeModule"];
    // Unity's cone points along forward, and Z is mirrored.
    e.direction = Some(Vec3::new(0.0, 0.0, -1.0));
    if shape.i64("enabled") != Some(0) {
        let kind = shape.i64("type").unwrap_or(4);
        e.spread_deg = match kind {
            // Sphere, hemisphere.
            0 | 1 => 180.0,
            2 | 3 => 90.0,
            // A box gives off straight along its forward.
            5 | 15 | 16 => 0.0,
            _ => shape.f32("angle").unwrap_or(25.0),
        };
        // Where the shape is and how it is turned, mirrored in Z as the
        // scene is.
        let mirror = |v: [f32; 3]| Vec3::new(v[0], v[1], -v[2]);
        let from = shape.vec3("m_Position").map(mirror).unwrap_or(Vec3::ZERO);
        e.from = (from != Vec3::ZERO).then_some(from);
        let turn = shape.vec3("m_Rotation").map(|r| Vec3::new(-r[0], -r[1], r[2])).unwrap_or(Vec3::ZERO);
        let scale = shape.vec3("m_Scale").map(Vec3::from_array).unwrap_or(Vec3::ONE);
        if turn != Vec3::ZERO {
            e.shape_turn_deg = Some(turn);
            let q = scrap::glam::Quat::from_euler(
                scrap::glam::EulerRot::YXZ,
                turn.y.to_radians(),
                turn.x.to_radians(),
                turn.z.to_radians(),
            );
            e.direction = Some(q * Vec3::new(0.0, 0.0, -1.0));
        }
        match kind {
            5 | 15 | 16 => e.box_size = Some(scale),
            // Cones, circles and spheres: their radius, as scaled.
            0..=4 | 7..=11 => {
                e.radius = shape["radius"].f32("value").unwrap_or(1.0) * scale.x.abs().max(scale.z.abs());
            }
            _ => {}
        }
    } else {
        e.spread_deg = 0.0;
    }
    let size = &b["SizeModule"];
    if size.i64("enabled") == Some(1) {
        e.end_size = Some(e.size * min_max(&size["curve"]).unwrap_or(1.0));
    } else {
        e.end_size = Some(e.size);
    }
    let colour = &b["ColorModule"];
    if colour.i64("enabled") == Some(1) {
        if let Some((_, end)) = gradient(&colour["gradient"]) {
            e.end_color = Some((e.color.0 * end[0], e.color.1 * end[1], e.color.2 * end[2]));
            e.end_alpha = Some(e.alpha * end[3]);
        }
    }
    e.time_scale = b.f32("simulationSpeed").unwrap_or(1.0);
    // Limit Velocity: slowed past a speed, as a whole (not by axis).
    let clamp = &b["ClampVelocityModule"];
    if clamp.i64("enabled") == Some(1) && clamp.i64("separateAxis") != Some(1) {
        e.speed_limit = min_max_mean(&clamp["magnitude"]);
        e.dampen = clamp.f32("dampen").unwrap_or(0.0);
    }
    // Size and colour over each one's life, as keys.
    if size.i64("enabled") == Some(1) {
        e.size_keys = min_max_keys(&size["curve"]);
        e.end_size = e.size_keys.last().map(|(_, f)| e.size * f);
    }
    if colour.i64("enabled") == Some(1) {
        let keys = gradient_keys(&colour["gradient"]);
        if keys.len() > 1 {
            e.color_keys = keys.iter().map(|(t, c)| (*t, (linear_hdr(c[0]), linear_hdr(c[1]), linear_hdr(c[2])))).collect();
            e.alpha_keys = keys.iter().map(|(t, c)| (*t, c[3])).collect();
        }
    }
    // Custom Data: numbers each carries to its shader over its life. Which
    // of the material's numbers they fill, the shader says (`custom0.x`,
    // `custom1.r` in its params); the renderer's material is bound to
    // them once both are read (`bind_custom`).
    let data = &b["CustomDataModule"];
    if data.i64("enabled") == Some(1) {
        e.custom.clear();
        for stream in 0..2 {
            let name = format!("custom{stream}");
            let keys: Vec<(f32, Vec<f32>)> = match data.i64(&format!("mode{stream}")).unwrap_or(0) {
                // Vector: each component its own curve.
                1 => {
                    let count = data.i64(&format!("vectorComponentCount{stream}")).unwrap_or(4).clamp(1, 4) as usize;
                    let comps: Vec<Vec<(f32, f32)>> = (0..count)
                        .map(|c| min_max_keys(&data[format!("vector{stream}_{c}").as_str()]))
                        .collect();
                    (0..=KEYS)
                        .map(|i| {
                            let t = i as f32 / KEYS as f32;
                            (t, comps.iter().map(|k| scrap::look::keyed(k, t)).collect())
                        })
                        .collect()
                }
                // Colour: a gradient, linear, a glow past white kept.
                2 => gradient_keys(&data[format!("color{stream}").as_str()])
                    .into_iter()
                    .map(|(t, c)| (t, vec![linear_hdr(c[0]), linear_hdr(c[1]), linear_hdr(c[2]), c[3]]))
                    .collect(),
                _ => continue,
            };
            if !keys.is_empty() {
                e.custom.push(scrap::look::CustomStream { slot: u8::MAX, name, keys });
            }
        }
    }
    // Texture Sheet Animation, grid mode: which frame of the sheet each
    // is, as a share of the whole sheet.
    let uv = &b["UVModule"];
    if uv.i64("enabled") == Some(1) && uv.i64("mode").unwrap_or(0) == 0 {
        let (across, down) = (uv.i64("tilesX").unwrap_or(1).max(1) as u32, uv.i64("tilesY").unwrap_or(1).max(1) as u32);
        let count = (across * down) as f32;
        let start = min_max(&uv["startFrame"]).unwrap_or(0.0) / count;
        let over = &uv["frameOverTime"];
        let scalar = over.f32("scalar").unwrap_or(0.0);
        let (from, to, random) = match over.i64("minMaxState").unwrap_or(0) {
            3 => (over.f32("minScalar").unwrap_or(0.0), scalar, true),
            1 | 2 => {
                let keys = over["maxCurve"].list("m_Curve");
                let first = keys.first().and_then(|k| k.f32("value")).unwrap_or(0.0);
                let last = keys.last().and_then(|k| k.f32("value")).unwrap_or(1.0);
                (first * scalar, last * scalar, false)
            }
            _ => (scalar, scalar, false),
        };
        e.sheet = Some((across, down));
        e.frames = (start + from, start + to);
        e.frames_random = random;
    }
    // Trails, of particles: a streak behind each, as long as it goes in
    // the trail's lifetime (a share of the particle's).
    let trails = &b["TrailModule"];
    if trails.i64("enabled") == Some(1) {
        if trails.i64("mode").unwrap_or(0) == 0 {
            e.trail = min_max_mean(&trails["lifetime"]).unwrap_or(1.0) * e.life;
            if trails.f32("ratio").unwrap_or(1.0) < 0.5 {
                report.skip("a ParticleSystem's trails on only some of its particles (brought on all)");
            }
        } else {
            report.skip("a ParticleSystem's ribbon trails");
        }
    }
    for module in [
        "NoiseModule",
        "CollisionModule",
        "SubModule",
        "VelocityModule",
        "ForceModule",
        "RotationModule",
        "LightsModule",
    ] {
        if b[module].i64("enabled") == Some(1) {
            report.skip(format!("a ParticleSystem's {module}"));
        }
    }
    desc.set_part(&emitter);
}

/// A collider's GameObject is solid when nothing else says, a trigger when
/// Unity said so.
fn solid(desc: &mut EntityDesc, b: &Yaml) {
    if b.i64("m_IsTrigger") == Some(1) {
        desc.set_part(&Body::Trigger);
    } else if desc.body() == Body::None {
        desc.set_part(&Body::Static);
    }
}

/// The model a mesh reference names: Unity's builtins by fileID, anything
/// else by its file's scrap name.
fn model(r: &Ref, unity: &Unity) -> Option<String> {
    let guid = r.guid.as_deref()?;
    if guid == BUILTIN {
        return Some(
            match r.file_id {
                10202 => "builtin:cube",
                10206 => "builtin:cylinder",
                10207 => "builtin:sphere",
                10208 => "builtin:capsule",
                10209 => "builtin:unity_plane",
                10210 => "builtin:unity_quad",
                _ => return None,
            }
            .to_string(),
        );
    }
    let (kind, name) = unity.named(guid)?;
    (kind == "model").then(|| name.to_string())
}

/// [`piece`], or — when the object's own name is none of the model's
/// pieces — the piece the same mesh (the model's GUID and the mesh's
/// fileID) is drawn as elsewhere, by an object named as that piece: a
/// model's root object carries its body under the file's own name
/// (Dacha's multitool: `SM_Multitool_01` draws `SM_Multitool_Body_01`,
/// which its art prefab of that name draws too). Not the whole model: that
/// draws every tool head and fan of it, still, over the ones its clips move.
fn piece_of(unity: &Unity, model: String, object: &str, mesh: &Ref) -> String {
    let named = piece(unity, model.clone(), object);
    if named != model {
        return named;
    }
    let key = (mesh.guid.clone().unwrap_or_default(), mesh.file_id);
    match unity.mesh_pieces.get(&key) {
        Some(p) => format!("{model}@{p}"),
        None => model,
    }
}

/// Which mesh of a model a renderer on `object` draws, in that object's
/// frame: the piece named as the object is (Unity's "(1)" copies aside),
/// or a model's only piece. The whole model, in its root's frame, when it
/// cannot tell — or has no pieces converted.
fn piece(unity: &Unity, model: String, object: &str) -> String {
    let Some(pieces) = unity.pieces.get(&model) else {
        return model;
    };
    let bare = object.trim_end_matches(|c: char| c == ')' || c.is_ascii_digit());
    let bare = bare.strip_suffix(" (").unwrap_or(object).trim();
    for name in [object, bare] {
        let wanted = super::piece_name(name);
        if pieces.iter().any(|p| *p == wanted) {
            return format!("{model}@{wanted}");
        }
    }
    match pieces.as_slice() {
        [only] => format!("{model}@{only}"),
        _ => model,
    }
}

/// Fields a MonoBehaviour's document has that are Unity's, not the game's.
const UNITY_FIELDS: [&str; 10] = [
    "m_ObjectHideFlags",
    "m_CorrespondingSourceObject",
    "m_PrefabInstance",
    "m_PrefabAsset",
    "m_GameObject",
    "m_Enabled",
    "m_EditorHideFlags",
    "m_Script",
    "m_Name",
    "m_EditorClassIdentifier",
];

/// A HingeJoint's spring (held at an angle) or motor (turning at a
/// speed), in Unity's angles as its limits are. The spring wins when both are on,
/// as a lever that springs back is what the two together usually mean.
fn hinge_drive(b: &Yaml) -> Option<scrap::scene::Motor> {
    if b.i64("m_UseSpring") == Some(1) {
        let s = &b["m_Spring"];
        return Some(scrap::scene::Motor {
            speed: 0.0,
            hold: Some(s.f32("targetPosition").unwrap_or(0.0)),
            strength: s.f32("spring").unwrap_or(0.0),
            damping: Some(s.f32("damper").unwrap_or(0.0)),
        });
    }
    if b.i64("m_UseMotor") == Some(1) {
        let m = &b["m_Motor"];
        return Some(scrap::scene::Motor {
            speed: m.f32("targetVelocity").unwrap_or(0.0),
            hold: None,
            strength: m.f32("force").unwrap_or(0.0),
            damping: None,
        });
    }
    None
}

/// Change one field of a MonoBehaviour's YAML as a prefab modification
/// says: `a.b`, `list.Array.size`, `list.Array.data[2].count`. The value
/// takes the type the field had (a number stays a number); an object
/// field takes the modification's `objectReference`.
fn modify(body: &mut Yaml, path: &str, m: &Yaml) {
    let mut steps: Vec<&str> = path.split('.').collect();
    let mut at = body;
    while let Some(step) = (!steps.is_empty()).then(|| steps.remove(0)) {
        if step == "Array" {
            continue;
        }
        if step == "size" {
            // `list.Array.size`: longer copies the last item, shorter cuts.
            let n = yaml::number(&m["value"]).unwrap_or(0.0).max(0.0) as usize;
            if !matches!(at, Yaml::Array(_)) {
                *at = Yaml::Array(Vec::new());
            }
            if let Yaml::Array(items) = at {
                let fill = items.last().cloned().unwrap_or(Yaml::Hash(Default::default()));
                items.resize(n, fill);
            }
            return;
        }
        if let Some(i) = step
            .strip_prefix("data[")
            .and_then(|r| r.strip_suffix(']'))
            .and_then(|i| i.parse::<usize>().ok())
        {
            let Yaml::Array(items) = at else { return };
            if i >= items.len() {
                let fill = items.last().cloned().unwrap_or(Yaml::Hash(Default::default()));
                items.resize(i + 1, fill);
            }
            at = &mut items[i];
        } else {
            if !matches!(at, Yaml::Hash(_)) {
                *at = Yaml::Hash(Default::default());
            }
            let Yaml::Hash(h) = at else { return };
            at = h
                .entry(Yaml::String(step.to_string()))
                .or_insert(Yaml::Null);
        }
    }
    let reference = m["objectReference"].clone();
    let is_reference = yaml::reference(at).is_some()
        || yaml::reference(&reference).is_some_and(|r| !r.is_none());
    *at = if is_reference {
        reference
    } else {
        let text = match &m["value"] {
            Yaml::String(t) => t.clone(),
            Yaml::Integer(i) => i.to_string(),
            Yaml::Real(r) => r.clone(),
            Yaml::Boolean(b) => (*b as i64).to_string(),
            _ => String::new(),
        };
        match at {
            Yaml::Integer(_) | Yaml::Boolean(_) => text
                .parse::<i64>()
                .map(Yaml::Integer)
                .unwrap_or(Yaml::Real(text)),
            Yaml::Real(_) => Yaml::Real(text),
            Yaml::String(_) => Yaml::String(text),
            _ => {
                if let Ok(i) = text.parse::<i64>() {
                    Yaml::Integer(i)
                } else if text.parse::<f64>().is_ok() {
                    Yaml::Real(text)
                } else {
                    Yaml::String(text)
                }
            }
        }
    };
}

/// A ScriptableObject `.asset` as data: the script it is an instance of,
/// and its own fields as a RON struct. `None` when the file is not one, or
/// its script is not the project's (a package's: fonts, render settings).
pub fn data_asset(unity: &Unity, text: &str) -> Option<(String, String)> {
    let docs = yaml::documents(text);
    let doc = docs.iter().find(|d| d.kind == "MonoBehaviour")?;
    let script = doc
        .body
        .reference("m_Script")
        .and_then(|r| r.guid)
        .and_then(|g| unity.guids.get(&g))
        .map(|p| super::stem(p))?;
    let refs = Refs {
        unity,
        entity_of: HashMap::new(),
        body_object: HashMap::new(),
        scoped: HashMap::new(),
    };
    Some((script, mono_behaviour(&doc.body, &refs)))
}

/// A MonoBehaviour's own fields as a RON struct.
fn mono_behaviour(b: &Yaml, refs: &Refs) -> String {
    let Yaml::Hash(hash) = b else {
        return "()".into();
    };
    let mut fields = Vec::new();
    for (k, v) in hash {
        let Some(key) = k.as_str() else { continue };
        if UNITY_FIELDS.contains(&key) {
            continue;
        }
        if let Some(value) = ron_of(v, refs, 0) {
            fields.push(format!("{}: {value}", ron_key(key)));
        }
    }
    format!("({})", fields.join(", "))
}

/// A field name RON takes as it is, or raw when it is a keyword or odd.
fn ron_key(key: &str) -> String {
    let plain = key
        .chars()
        .enumerate()
        .all(|(i, c)| c.is_ascii_alphabetic() || c == '_' || (i > 0 && c.is_ascii_digit()));
    if plain && !matches!(key, "true" | "false" | "Some" | "None") {
        key.to_string()
    } else {
        format!(
            "r#{}",
            key.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_")
        )
    }
}

/// A YAML value as RON: numbers, text, lists and structs as they are;
/// vectors and colours as tuples; a reference to an object of this file as
/// an `EntityRef`, to an asset as a typed link. `None` for what there is
/// nothing to say about (an empty reference).
fn ron_of(v: &Yaml, refs: &Refs, depth: usize) -> Option<String> {
    if depth > 16 {
        return None;
    }
    Some(match v {
        Yaml::Integer(i) => i.to_string(),
        Yaml::Real(r) => {
            let n: f64 = r.parse().ok()?;
            let s = format!("{n:?}");
            if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
                s
            } else {
                format!("{s}.0")
            }
        }
        Yaml::Boolean(b) => b.to_string(),
        Yaml::String(s) => format!("{s:?}"),
        Yaml::Null => return None,
        Yaml::Array(items) => {
            let items: Vec<String> = items
                .iter()
                .filter_map(|i| ron_of(i, refs, depth + 1))
                .collect();
            format!("[{}]", items.join(", "))
        }
        Yaml::Hash(h) => {
            if let Some(r) = yaml::reference(v) {
                return link(&r, refs);
            }
            let keys: HashSet<&str> = h.keys().filter_map(Yaml::as_str).collect();
            let tuple = |names: &[&str]| -> Option<String> {
                let parts: Option<Vec<String>> = names
                    .iter()
                    .map(|n| ron_of(&v[*n], refs, depth + 1))
                    .collect();
                Some(format!("({})", parts?.join(", ")))
            };
            if keys == HashSet::from(["x", "y", "z"]) {
                return tuple(&["x", "y", "z"]);
            }
            if keys == HashSet::from(["x", "y", "z", "w"]) {
                return tuple(&["x", "y", "z", "w"]);
            }
            if keys == HashSet::from(["x", "y"]) {
                return tuple(&["x", "y"]);
            }
            if keys == HashSet::from(["r", "g", "b", "a"]) {
                return tuple(&["r", "g", "b", "a"]);
            }
            let mut fields = Vec::new();
            for (k, value) in h {
                let Some(key) = k.as_str() else { continue };
                if let Some(value) = ron_of(value, refs, depth + 1) {
                    fields.push(format!("{}: {value}", ron_key(key)));
                }
            }
            format!("({})", fields.join(", "))
        }
        _ => return None,
    })
}

/// A reference as a link: to an entity of this file, or to an asset.
fn link(r: &Ref, refs: &Refs) -> Option<String> {
    if r.is_none() {
        return None;
    }
    match r.guid.as_deref() {
        None => {
            if let Some(id) = refs.scoped.get(&r.file_id) {
                return Some(format!("EntityRef(\"{id}\")"));
            }
            let entity = refs.entity_of.get(&r.file_id)?;
            Some(format!("EntityRef(\"{}\")", entity_id(*entity)))
        }
        Some(guid) => {
            let (kind, name) = refs.unity.named(guid)?;
            let type_name = match kind {
                "model" => "ModelLink",
                "material" => "MaterialLink",
                "prefab" => "PrefabLink",
                "sound" => "SoundLink",
                "texture" => "TextureLink",
                "scene" => "SceneLink",
                _ => return Some(format!("{name:?}")),
            };
            Some(format!("{type_name}({name:?})"))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_rate_that_swells_and_dies_away_comes_over_as_its_average() {
        let hump = super::yaml::documents(
            "--- !u!1 &1\nX:\n  v:\n    scalar: 20\n    minMaxState: 1\n    maxCurve:\n      m_Curve:\n      - time: 0\n        value: 0\n        inSlope: 4\n        outSlope: 4\n      - time: 1\n        value: 0\n        inSlope: -4\n        outSlope: -4\n",
        );
        // A parabola 4t(1-t) has slopes 4 and -4 at its ends, and averages 2/3.
        let rate = min_max_mean(&hump[0].body["v"]).unwrap();
        assert!((rate - 20.0 * 2.0 / 3.0).abs() < 0.1, "{rate}");
        let step = super::yaml::documents(
            "--- !u!1 &1\nX:\n  v:\n    scalar: 10\n    minMaxState: 1\n    maxCurve:\n      m_Curve:\n      - time: 0\n        value: 1\n        inSlope: Infinity\n        outSlope: Infinity\n      - time: 1\n        value: 0\n        inSlope: Infinity\n        outSlope: Infinity\n",
        );
        assert_eq!(min_max_mean(&step[0].body["v"]), Some(10.0), "a step holds its value, never NaN");
    }

    use super::*;

    fn unity() -> Unity {
        Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            declared_params: Default::default(),
            root: Default::default(),
            guids: [
                ("aaa".to_string(), "Assets/Models/crate.fbx".into()),
                ("mmm".to_string(), "Assets/Mats/wood.mat".into()),
                ("sss".to_string(), "Assets/Scripts/Door.cs".into()),
                ("ppp".to_string(), "Assets/Prefabs/Lamp.prefab".into()),
                ("www".to_string(), "Assets/Sounds/Radio.ogg".into()),
            ]
            .into_iter()
            .collect(),
            layers: Default::default(),
            names: [
                ("aaa", "crate"),
                ("mmm", "wood"),
                ("sss", "Door"),
                ("ppp", "Lamp"),
                ("www", "radio"),
            ]
            .into_iter()
            .map(|(g, n)| (g.to_string(), n.to_string()))
            .collect(),
        }
    }

    const SCENE: &str = "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Crate
  m_IsActive: 1
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 1, y: 2, z: 3}
  m_LocalRotation: {x: 0, y: 0.7071068, z: 0, w: 0.7071068}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 0}
  m_Children:
  - {fileID: 21}
--- !u!33 &12
MeshFilter:
  m_GameObject: {fileID: 10}
  m_Mesh: {fileID: 4300000, guid: aaa, type: 3}
--- !u!23 &13
MeshRenderer:
  m_GameObject: {fileID: 10}
  m_Materials:
  - {fileID: 2100000, guid: mmm, type: 2}
--- !u!65 &14
BoxCollider:
  m_GameObject: {fileID: 10}
  m_IsTrigger: 0
  m_Size: {x: 2, y: 1, z: 1}
  m_Center: {x: 0, y: 0.5, z: 0}
--- !u!54 &15
Rigidbody:
  m_GameObject: {fileID: 10}
  m_IsKinematic: 0
  m_UseGravity: 1
--- !u!1 &20
GameObject:
  m_Name: Lid
--- !u!4 &21
Transform:
  m_GameObject: {fileID: 20}
  m_LocalPosition: {x: 0, y: 1, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 11}
--- !u!114 &22
MonoBehaviour:
  m_GameObject: {fileID: 20}
  m_Enabled: 1
  m_Script: {fileID: 11500000, guid: sss, type: 3}
  m_Name:
  openAngle: 90
  locked: 1
  hinge: {fileID: 10}
  spawns: {fileID: 100100000, guid: ppp, type: 3}
  empty: {fileID: 0}
  tint: {r: 1, g: 0.5, b: 0, a: 1}
--- !u!1001 &30
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 555, guid: ppp, type: 3}
      propertyPath: m_LocalPosition.x
      value: 5
      objectReference: {fileID: 0}
    - target: {fileID: 555, guid: ppp, type: 3}
      propertyPath: m_LocalPosition.z
      value: 2
      objectReference: {fileID: 0}
    - target: {fileID: 777, guid: ppp, type: 3}
      propertyPath: m_Name
      value: Porch lamp
      objectReference: {fileID: 0}
    - target: {fileID: 888, guid: ppp, type: 3}
      propertyPath: brightness
      value: 3
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: ppp, type: 3}
";

    const SMOKE: &str = "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Chimney smoke
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 0}
--- !u!198 &12
ParticleSystem:
  m_GameObject: {fileID: 10}
  looping: 1
  moveWithTransform: 0
  InitialModule:
    startLifetime: {minMaxState: 3, scalar: 6, minScalar: 4}
    startSpeed: {minMaxState: 0, scalar: 1.5}
    startSize: {minMaxState: 0, scalar: 0.4}
    startColor:
      minMaxState: 0
      maxColor: {r: 0.8, g: 0.8, b: 0.8, a: 1}
    gravityModifier: {minMaxState: 0, scalar: -0.1}
  ShapeModule:
    enabled: 1
    type: 4
    angle: 12
  EmissionModule:
    enabled: 1
    rateOverTime: {minMaxState: 0, scalar: 20}
    m_Bursts:
    - time: 0.5
      countCurve: {minMaxState: 3, scalar: 40, minScalar: 20}
      cycleCount: 1
  SizeModule:
    enabled: 1
    curve:
      minMaxState: 1
      scalar: 1
      maxCurve:
        m_Curve:
        - {time: 0, value: 1}
        - {time: 1, value: 3}
  ColorModule:
    enabled: 1
    gradient:
      minMaxState: 1
      maxGradient:
        key0: {r: 1, g: 1, b: 1, a: 1}
        key1: {r: 0.5, g: 0.5, b: 0.5, a: 0}
        m_NumColorKeys: 2
  NoiseModule:
    enabled: 1
--- !u!199 &13
ParticleSystemRenderer:
  m_GameObject: {fileID: 10}
  m_RenderMode: 4
  m_Mesh: {fileID: 10207, guid: 0000000000000000e000000000000000, type: 0}
  m_Materials:
  - {fileID: 2100000, guid: mmm, type: 2}
";

    #[test]
    fn a_hinge_springs_to_its_target_in_unitys_angles() {
        let text = "a: 1\nm_UseSpring: 1\nm_Spring:\n  spring: 20\n  damper: 5\n  targetPosition: 90\nm_UseMotor: 0\n";
        let b = &yaml_rust2::YamlLoader::load_from_str(text).unwrap()[0];
        let motor = hinge_drive(b).unwrap();
        assert_eq!(motor.hold, Some(90.0));
        assert_eq!((motor.strength, motor.damping()), (20.0, 5.0));
        let off = &yaml_rust2::YamlLoader::load_from_str("m_UseSpring: 0\nm_UseMotor: 0\n").unwrap()[0];
        assert!(hinge_drive(off).is_none());
    }

    #[test]
    fn an_instance_changes_a_components_fields_as_its_modifications_say() {
        let load = |t: &str| yaml_rust2::YamlLoader::load_from_str(t).unwrap().remove(0);
        let mut body = load("required:\n- tag: plank\n  count: 3\n- tag: scrap\n  count: 2\nrideSeconds: 31\nvolume: {fileID: 5}\n");
        let change = |path: &str, value: &str| load(&format!("propertyPath: {path}\nvalue: {value}\nobjectReference: {{fileID: 0}}\n"));
        modify(&mut body, "required.Array.size", &change("required.Array.size", "1"));
        modify(&mut body, "required.Array.data[0].tag", &change("", "sponge"));
        modify(&mut body, "required.Array.data[0].count", &change("", "4"));
        modify(&mut body, "rideSeconds", &change("", "12.5"));
        modify(&mut body, "volume", &load("value: \nobjectReference: {fileID: 9}\n"));
        let items = body["required"].as_vec().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["tag"].as_str(), Some("sponge"));
        assert_eq!(items[0]["count"].as_i64(), Some(4));
        assert_eq!(yaml::number(&body["rideSeconds"]), Some(12.5));
        assert_eq!(body["volume"]["fileID"].as_i64(), Some(9));
    }

    #[test]
    fn a_renderer_draws_the_piece_of_a_model_named_as_its_object() {
        let mut unity = unity();
        unity.pieces.insert("level".into(), vec!["Sand".into(), "Rock_2".into()]);
        unity.pieces.insert("sheet".into(), vec!["Cube_054".into()]);
        assert_eq!(piece(&unity, "level".into(), "Sand (3)"), "level@Sand");
        assert_eq!(piece(&unity, "level".into(), "Rock.2"), "level@Rock_2");
        assert_eq!(piece(&unity, "level".into(), "Tree"), "level", "not one of its pieces: the whole");
        assert_eq!(piece(&unity, "sheet".into(), "SM_Sheet_01 (7)"), "sheet@Cube_054", "its only piece");
        assert_eq!(piece(&unity, "crate".into(), "Crate"), "crate", "no pieces converted");
    }

    #[test]
    fn a_models_root_draws_the_piece_its_mesh_is_elsewhere() {
        // Dacha's multitool: the object `SM_Multitool_01` draws the mesh
        // its art prefab `SM_Multitool_Body_01` draws — its body alone,
        // not the whole model with every tool head in it.
        let mut unity = unity();
        unity.pieces.insert("tool".into(), vec!["Body".into(), "Fan".into(), "Head".into()]);
        unity.mesh_pieces.insert(("g".into(), 868), "Body".into());
        let mesh = |id| Ref { file_id: id, guid: Some("g".into()) };
        assert_eq!(piece_of(&unity, "tool".into(), "SM_Tool", &mesh(868)), "tool@Body");
        assert_eq!(piece_of(&unity, "tool".into(), "Fan (1)", &mesh(868)), "tool@Fan", "its own name first");
        assert_eq!(piece_of(&unity, "tool".into(), "SM_Tool", &mesh(5)), "tool", "a mesh no prefab names: the whole");
    }

    #[test]
    fn a_textmeshpro_in_the_world_keeps_its_words_box_and_colour() {
        let text = "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Label
  m_IsActive: 1
--- !u!224 &11
RectTransform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 0}
  m_AnchoredPosition: {x: 0, y: 0}
  m_SizeDelta: {x: 20, y: 5}
--- !u!114 &12
MonoBehaviour:
  m_GameObject: {fileID: 10}
  m_Enabled: 1
  m_Script: {fileID: 11500000, guid: 9541d86e2fd84c1d9990edf0852d74ab, type: 3}
  m_Name: 
  m_text: \"\\u041F\\u0438\\u043B\\u0430\"
  m_fontColor: {r: 0, g: 1, b: 0.5, a: 1}
  m_enableVertexGradient: 0
  m_fontSize: 37.5
  m_HorizontalAlignment: 2
  m_VerticalAlignment: 512
  m_margin: {x: 0, y: 0, z: 0, w: 0}
--- !u!1 &20
GameObject:
  m_Name: Count
  m_IsActive: 1
--- !u!224 &21
RectTransform:
  m_GameObject: {fileID: 20}
  m_Father: {fileID: 0}
  m_SizeDelta: {x: 4, y: 2}
--- !u!114 &22
MonoBehaviour:
  m_GameObject: {fileID: 20}
  m_Enabled: 1
  m_Script: {fileID: 11500000, guid: 9541d86e2fd84c1d9990edf0852d74ab, type: 3}
  m_text: 321
  m_fontSize: 10
";
        let mut report = Report::default();
        let lines = convert_file(&unity(), text, &mut report);
        let label = lines.iter().find(|l| l.name == "Label").unwrap();
        let tmp = label.components.get("text_mesh_pro").expect("the text brought").get_ron().to_string();
        assert!(tmp.contains("text: \"Пила\""), "{tmp}");
        assert!(tmp.contains("fontSize: 37.5") && tmp.contains("size: Some((20.0, 5.0))"), "{tmp}");
        assert!(tmp.contains("color: (0.0, 1.0, 0.5, 1.0)") && tmp.contains("horizontal: 2"), "{tmp}");
        let count = lines.iter().find(|l| l.name == "Count").unwrap();
        let tmp = count.components.get("text_mesh_pro").unwrap().get_ron().to_string();
        assert!(tmp.contains("text: \"321\""), "a number's words are words: {tmp}");
    }

    #[test]
    fn a_scriptable_object_becomes_data() {
        let text = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_ObjectHideFlags: 0\n  m_Script: {fileID: 11500000, guid: sss, type: 3}\n  m_Name: GardenSoil\n  graphName: garden_soil\n  nodes:\n  - prefab: {fileID: 100, guid: ppp, type: 3}\n    branches:\n    - condition: 2\n      requires: [seeds]\n";
        let (script, body) = data_asset(&unity(), text).unwrap();
        assert_eq!(script, "Door");
        assert!(body.contains("graphName: \"garden_soil\""), "{body}");
        assert!(body.contains("PrefabLink(\"Lamp\")"), "{body}");
        assert!(body.contains("condition: 2"), "{body}");
        assert!(!body.contains("m_Name"), "{body}");
        assert!(data_asset(&unity(), "%YAML 1.1\n--- !u!29 &1\nOcclusionCullingSettings:\n  m_ObjectHideFlags: 0\n").is_none());
    }

    #[test]
    fn a_script_added_again_to_an_instance_leaves_the_prefabs_own() {
        let dir = std::env::temp_dir().join(format!("scrap-again-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let heap = dir.join("Heap.prefab");
        std::fs::write(
            &heap,
            "%YAML 1.1
--- !u!1 &100
GameObject:
  m_Name: Heap
--- !u!4 &101
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!114 &102
MonoBehaviour:
  m_GameObject: {fileID: 100}
  m_Enabled: 1
  m_Script: {fileID: 11500000, guid: sss, type: 3}
  drops: 2
",
        )
        .unwrap();
        let mut unity = unity();
        unity.guids.insert("hhh".into(), heap);
        unity.names.insert("hhh".into(), "Heap".into());
        let scene = "%YAML 1.1
--- !u!1001 &900
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications: []
    m_AddedComponents:
    - targetCorrespondingSourceObject: {fileID: 100, guid: hhh, type: 3}
      insertIndex: -1
      addedObject: {fileID: 903}
  m_SourcePrefab: {fileID: 100100000, guid: hhh, type: 3}
--- !u!1 &902 stripped
GameObject:
  m_CorrespondingSourceObject: {fileID: 100, guid: hhh, type: 3}
  m_PrefabInstance: {fileID: 900}
--- !u!4 &901 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 101, guid: hhh, type: 3}
  m_PrefabInstance: {fileID: 900}
--- !u!114 &903
MonoBehaviour:
  m_GameObject: {fileID: 902}
  m_Enabled: 1
  m_Script: {fileID: 11500000, guid: sss, type: 3}
  drops: 1
";
        let mut report = Report::default();
        let roots = convert_file(&unity, scene, &mut report);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(roots[0].components.get("door").is_none(), "the prefab's own door counts: {:?}", roots[0].components);
    }

    #[test]
    fn an_instance_changes_a_parts_collider_and_hinge() {
        let dir = std::env::temp_dir().join(format!("scrap-builtins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lever = dir.join("Lever.prefab");
        std::fs::write(
            &lever,
            "%YAML 1.1
--- !u!1 &100
GameObject:
  m_Name: Lever
--- !u!4 &101
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!1 &200
GameObject:
  m_Name: Handle
--- !u!4 &201
Transform:
  m_GameObject: {fileID: 200}
  m_Father: {fileID: 101}
--- !u!65 &202
BoxCollider:
  m_GameObject: {fileID: 200}
  m_IsTrigger: 1
  m_Enabled: 1
  m_Size: {x: 1, y: 1, z: 1}
  m_Center: {x: 0, y: 0, z: 0}
--- !u!54 &203
Rigidbody:
  m_GameObject: {fileID: 200}
  m_Mass: 1
--- !u!59 &204
HingeJoint:
  m_GameObject: {fileID: 200}
  m_ConnectedBody: {fileID: 0}
  m_Anchor: {x: 0, y: 0, z: 0}
  m_Axis: {x: 1, y: 0, z: 0}
",
        )
        .unwrap();
        let mut unity = unity();
        unity.guids.insert("lll".into(), lever);
        unity.names.insert("lll".into(), "Lever".into());
        let scene = "%YAML 1.1
--- !u!1001 &900
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 101, guid: lll, type: 3}
      propertyPath: m_LocalPosition.x
      value: 5
    - target: {fileID: 202, guid: lll, type: 3}
      propertyPath: m_Size.y
      value: 3
    - target: {fileID: 202, guid: lll, type: 3}
      propertyPath: m_IsTrigger
      value: 0
    - target: {fileID: 204, guid: lll, type: 3}
      propertyPath: m_Axis.x
      value: 0
    - target: {fileID: 204, guid: lll, type: 3}
      propertyPath: m_Axis.z
      value: 1
  m_SourcePrefab: {fileID: 100100000, guid: lll, type: 3}
--- !u!4 &901 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 101, guid: lll, type: 3}
  m_PrefabInstance: {fileID: 900}
";
        let mut report = Report::default();
        let roots = convert_file(&unity, scene, &mut report);
        let _ = std::fs::remove_dir_all(&dir);
        let handle = &roots[0].overrides[&entity_id(200)];
        let collider: Collider = handle.parts.try_get().unwrap().expect("the collider changed");
        assert!(matches!(collider, Collider::Box { half, .. } if half == Vec3::new(0.5, 1.5, 0.5)), "{collider:?}");
        assert_eq!(
            handle.parts.raw("body"),
            Some("Dynamic"),
            "a trigger made solid is its Rigidbody's body, not a static one"
        );
        let joint: Joint = handle.parts.try_get().unwrap().expect("the hinge changed");
        match joint {
            Joint::Hinge { to, axis, .. } => {
                assert!(to.is_unassigned(), "still to the world");
                assert!((axis - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-6 || (axis - Vec3::Z).length() < 1e-6, "{axis}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_instance_moves_a_part_and_takes_its_light_away() {
        let dir = std::env::temp_dir().join(format!("scrap-parts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lamp = dir.join("Lamp.prefab");
        std::fs::write(
            &lamp,
            "%YAML 1.1
--- !u!1 &100
GameObject:
  m_Name: Lamp
--- !u!4 &101
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!1 &200
GameObject:
  m_Name: Bulb
--- !u!4 &201
Transform:
  m_GameObject: {fileID: 200}
  m_Father: {fileID: 101}
  m_LocalPosition: {x: 0, y: 2, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 1, y: 1, z: 1}
--- !u!108 &202
Light:
  m_GameObject: {fileID: 200}
",
        )
        .unwrap();
        let mut unity = unity();
        unity.guids.insert("ppp".into(), lamp);
        let scene = "%YAML 1.1
--- !u!1001 &900
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 101, guid: ppp, type: 3}
      propertyPath: m_LocalPosition.x
      value: 5
    - target: {fileID: 201, guid: ppp, type: 3}
      propertyPath: m_LocalPosition.x
      value: 1
    m_RemovedComponents:
    - {fileID: 202, guid: ppp, type: 3}
  m_SourcePrefab: {fileID: 100100000, guid: ppp, type: 3}
--- !u!4 &901 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 201, guid: ppp, type: 3}
  m_PrefabInstance: {fileID: 900}
--- !u!1 &902 stripped
GameObject:
  m_CorrespondingSourceObject: {fileID: 200, guid: ppp, type: 3}
  m_PrefabInstance: {fileID: 900}
--- !u!114 &903
MonoBehaviour:
  m_GameObject: {fileID: 902}
  m_Script: {fileID: 11500000, guid: sss, type: 3}
  locked: 1
--- !u!1 &910
GameObject:
  m_Name: Moth
--- !u!4 &911
Transform:
  m_GameObject: {fileID: 910}
  m_Father: {fileID: 901}
";
        let mut report = Report::default();
        let roots = convert_file(&unity, scene, &mut report);
        let _ = std::fs::remove_dir_all(&dir);
        let lamp = &roots[0];
        assert_eq!(
            lamp.transform.position.x, 5.0,
            "the root where the scene puts it"
        );
        let bulb = &lamp.overrides[&entity_id(200)];
        let t = bulb.transform.expect("the bulb moved");
        assert_eq!(
            t.position,
            Vec3::new(1.0, 2.0, 0.0),
            "x said, y as the prefab has it"
        );
        assert_eq!(bulb.removed, ["light"]);
        assert!(
            bulb.components.contains_key("door"),
            "added to the bulb, not the lamp"
        );
        assert!(lamp.components.is_empty());
        let moth = lamp
            .children
            .iter()
            .find(|c| c.name == "Moth")
            .expect("under the lamp");
        assert_eq!(moth.in_part, Some(entity_id(200)), "on the bulb");
    }

    #[test]
    fn an_audio_source_becomes_the_entitys_sound() {
        let text = "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Radio
  m_Component:
  - component: {fileID: 11}
  - component: {fileID: 12}
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Father: {fileID: 0}
--- !u!82 &12
AudioSource:
  m_GameObject: {fileID: 10}
  m_Enabled: 1
  m_Resource: {fileID: 8300000, guid: www, type: 3}
  m_PlayOnAwake: 1
  m_Volume: 0.5
  m_Pitch: 0.8
  Loop: 1
  MinDistance: 2
  MaxDistance: 12
  panLevelCustomCurve:
    m_Curve:
    - time: 0
      value: 1
";
        let mut report = Report::default();
        let roots = convert_file(&unity(), text, &mut report);
        let sound = roots[0].sound().clone().expect("a sound");
        assert_eq!(sound.clip.as_str(), "radio");
        assert_eq!((sound.volume, sound.pitch), (0.5, 0.8));
        assert!(sound.looped && sound.on_start && sound.spatial);
        assert_eq!((sound.near, sound.far), (2.0, 12.0));
    }

    #[test]
    fn a_nav_mesh_agent_says_how_its_thing_walks() {
        let text = "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Mouse
  m_Component:
  - component: {fileID: 11}
  - component: {fileID: 12}
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_Father: {fileID: 0}
--- !u!195 &12
NavMeshAgent:
  m_GameObject: {fileID: 10}
  m_Enabled: 1
  m_Radius: 0.18
  m_Speed: 2
  m_Acceleration: 20
  avoidancePriority: 50
  m_AngularSpeed: 600
  m_StoppingDistance: 0.25
  m_Height: 0.3
  m_BaseOffset: 0
";
        let mut report = Report::default();
        let roots = convert_file(&unity(), text, &mut report);
        let agent = roots[0].components.get("nav_mesh_agent").expect("its agent").get_ron();
        assert_eq!(
            agent,
            "(speed: 2, acceleration: 20, angularSpeed: 600, stoppingDistance: 0.25, radius: 0.18, height: 0.3, baseOffset: 0)"
        );
    }

    #[test]
    fn a_shuriken_system_becomes_an_emitter() {
        let mut report = Report::default();
        let roots = convert_file(&unity(), SMOKE, &mut report);
        let e = roots[0].particles().clone().expect("an emitter");
        assert_eq!(e.life, 5.0, "two constants: the middle");
        assert_eq!(e.rate, 20.0);
        assert_eq!(e.bursts, vec![(0.5, 30)]);
        assert!(!e.once && !e.waits);
        assert_eq!(e.spread_deg, 12.0);
        assert!((e.gravity - 0.981).abs() < 1e-4, "{}", e.gravity);
        assert!(e.local);
        assert_eq!(e.end_size, Some(1.2), "size over life: three times");
        assert_eq!(e.end_color, Some((0.4, 0.4, 0.4)));
        assert_eq!(e.end_alpha, Some(0.0), "thins to nothing");
        assert_eq!(e.direction, Some(Vec3::new(0.0, 0.0, -1.0)));
        assert_eq!(e.model.as_str(), "builtin:sphere");
        assert_eq!(e.material.as_ref().map(|m| m.as_str()), Some("wood"));
        assert!(format!("{report:?}").contains("NoiseModule"), "{report:?}");
        // And the text it makes reads back.
        let text = ron::to_string(&e).unwrap();
        let back: scrap::scene::Emitter = ron::from_str(&text).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn a_trigger_with_a_kinematic_rigidbody_stays_a_trigger() {
        let volume = "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Counter
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 0}
--- !u!65 &12
BoxCollider:
  m_GameObject: {fileID: 10}
  m_IsTrigger: 1
  m_Size: {x: 1, y: 1, z: 1}
--- !u!54 &13
Rigidbody:
  m_GameObject: {fileID: 10}
  m_IsKinematic: 1
  m_UseGravity: 0
";
        let mut report = Report::default();
        let roots = convert_file(&unity(), volume, &mut report);
        assert_eq!(roots[0].body(), Body::Trigger);
        let solid = volume.replace("m_IsTrigger: 1", "m_IsTrigger: 0");
        let roots = convert_file(&unity(), &solid, &mut report);
        assert_eq!(roots[0].body(), Body::Kinematic);
    }

    #[test]
    fn a_rigidbody_is_drawn_between_its_steps_as_its_interpolation_says() {
        use scrap::scene::Drawn;
        let drawn = |scene: &str| {
            let roots = convert_file(&unity(), scene, &mut Report::default());
            let crate_ = roots.iter().find(|r| r.name == "Crate").unwrap();
            crate_.parts.try_get::<BodyProps>().ok().flatten().unwrap_or_default().drawn
        };
        // Unity's own default, None: where the step left it.
        assert_eq!(drawn(SCENE), Drawn::AtStep);
        let with = |mode: &str| SCENE.replace("  m_UseGravity: 1\n", &format!("  m_UseGravity: 1\n  m_Interpolate: {mode}\n"));
        assert_eq!(drawn(&with("1")), Drawn::Between);
        assert_eq!(drawn(&with("2")), Drawn::Ahead);
        assert_eq!(drawn(&with("0")), Drawn::AtStep);
    }

    #[test]
    fn a_scene_comes_over_with_its_hierarchy_parts_and_components() {
        let mut report = Report::default();
        let roots = convert_file(&unity(), SCENE, &mut report);
        assert_eq!(roots.len(), 2, "{roots:#?}");
        let crate_ = roots.iter().find(|r| r.name == "Crate").unwrap();
        assert_eq!(crate_.id, entity_id(10), "the same object, the same ID");
        assert_eq!(
            crate_.transform.position,
            Vec3::new(1.0, 2.0, -3.0),
            "Z mirrored"
        );
        assert_eq!(crate_.model().as_str(), "crate");
        assert!(matches!(&crate_.material_ref(), MaterialRef::Named(l) if l.as_str() == "wood"));
        assert_eq!(crate_.body(), Body::Dynamic);
        assert!(matches!(crate_.collider(), Collider::Box { half, center }
            if half == Vec3::new(1.0, 0.5, 0.5) && center == Vec3::new(0.0, 0.5, 0.0)));

        let lid = &crate_.children[0];
        assert_eq!(lid.name, "Lid");
        let door = lid.components["door"].get_ron();
        assert!(door.contains("openAngle: 90"), "{door}");
        assert!(
            door.contains(&format!("hinge: EntityRef(\"{}\")", entity_id(10))),
            "{door}"
        );
        assert!(door.contains(r#"spawns: PrefabLink("Lamp")"#), "{door}");
        assert!(!door.contains("empty"), "{door}");
        assert!(door.contains("tint: (1, 0.5, 0, 1)"), "{door}");

        let lamp = roots.iter().find(|r| r.prefab.as_str() == "Lamp").unwrap();
        assert_eq!(lamp.name, "Porch lamp");
        assert_eq!(lamp.transform.position, Vec3::new(5.0, 0.0, -2.0));
        assert_eq!(
            report.skipped.get("a prefab modification of `brightness`"),
            Some(&1)
        );

        // It reads as a scene.
        let scene = scrap::Scene {
            entities: roots,
            ..Default::default()
        };
        let text = scrap::ron::to_string(&scene).unwrap();
        let back: scrap::Scene = scrap::ron::from_str(&text).unwrap();
        assert_eq!(back.entities.len(), 2);
    }
}
