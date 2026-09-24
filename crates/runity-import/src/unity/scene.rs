//! A `.unity` or `.prefab` file as runity entities.

#[allow(unused_imports)]
use runity::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

use runity::glam::{Quat, Vec3};
use runity::scene::{Body, BodyProps, Collider, EntityDesc, Joint, Lens, Light, MaterialRef};
use runity::{AssetLink, EntityId, Transform};
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

/// An entity's ID from a Unity fileID: the same object, the same ID, every
/// time the file is imported (docs/unity-import.md).
pub fn entity_id(file_id: i64) -> EntityId {
    let mut z = (file_id as u64) ^ 0x9e37_79b9_7f4a_7c15;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    EntityId::from_raw(if z == 0 { 1 } else { z })
}

/// Unity is left-handed, runity right-handed: the Z axis is mirrored.
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

/// A scene's directional light as runity's sun: the hour whose sun
/// shines the way it does, and how bright.
pub fn sun(text: &str) -> Option<runity::scene::Sun> {
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
    // Unity's light shines along its +z; mirrored, runity's −z.
    let travel = turn * Vec3::NEG_Z;
    let up = (-travel.y).clamp(-1.0, 1.0).asin().max(0.05);
    // Morning when the light travels toward +x, as runity's sun does.
    let angle = if travel.x > 0.0 {
        up
    } else {
        std::f32::consts::PI - up
    };
    let tint = light.body.color("m_Color").map(|c| [c[0], c[1], c[2]]);
    Some(runity::scene::Sun {
        hour: 6.0 + angle / std::f32::consts::PI * 12.0,
        intensity: light.body.f32("m_Intensity").unwrap_or(1.0),
        // Exactly where Unity's stood, and its colour: the hour is only
        // near it.
        toward: Some(travel),
        tint: tint.filter(|c| *c != [1.0, 1.0, 1.0]),
        ..runity::scene::Sun::default()
    })
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
                // runity the instance is one line, so it is the instance.
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
                    desc.set_part(&runity::scene::LayerName(layer.clone()));
                }
                if d.body.i64("m_IsActive") == Some(0) {
                    desc.inactive = true;
                }
                for c in components.get(&d.file_id).into_iter().flatten() {
                    component(&mut desc, c, &refs, report);
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
    // transform's.
    for d in docs
        .iter()
        .filter(|d| matches!(d.class, TRANSFORM | RECT_TRANSFORM) && !d.stripped)
    {
        let Some(go) = game_object_of.get(&d.file_id) else {
            continue;
        };
        if let Some(desc) = entities.get_mut(go) {
            desc.transform = transform(&d.body);
        }
    }
    // Components added to a prefab instance's parts in this file land on
    // the instance: runity's instance is one line.
    // Components added to a part go on that part, as an override of it;
    // on the prefab's root, on the instance line itself.
    for d in docs.iter().filter(|d| d.class == GAME_OBJECT && d.stripped) {
        let Some(instance) = d.body.reference("m_PrefabInstance") else {
            continue;
        };
        let added = components.get(&d.file_id).into_iter().flatten();
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
                let change = runity::scene::Override::between(&EntityDesc::default(), &given);
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
        desc.set_part(&runity::scene::ModelRef(AssetLink::named(name)));
    } else {
        desc.prefab = AssetLink::named(name);
    }
    // The root's transform is what the modifications say. Which target is
    // the root: the one the prefab's own root is, or else the first carrying
    // m_LocalPosition, as Unity always writes the root's.
    let of = source.guid.as_deref().map(|g| parts.of(unity, g, 0));
    let key_of = |t: Option<i64>| of.as_ref().and_then(|o| o.keys.get(&t?).copied());
    let root_key = of.as_ref().and_then(|o| o.root);
    let mut root_target = modification
        .list("m_Modifications")
        .iter()
        .filter_map(|m| m.reference("target").map(|r| r.file_id))
        .find(|t| root_key.is_some() && key_of(Some(*t)) == root_key);
    // A part moved, turned or scaled: its axes as Unity says them.
    type Axes = ([Option<f32>; 3], [Option<f32>; 4], [Option<f32>; 3]);
    let mut moved: BTreeMap<EntityId, Axes> = BTreeMap::new();
    for m in modification.list("m_Modifications") {
        let Some(path) = m.str("propertyPath") else {
            continue;
        };
        let target = m.reference("target").map(|r| r.file_id);
        let value = yaml::number(&m["value"]).unwrap_or(0.0) as f32;
        let axis = |name: &str| -> Option<usize> {
            let rest = path.strip_prefix(name)?.strip_prefix('.')?;
            ["x", "y", "z", "w"].iter().position(|a| *a == rest)
        };
        let transforming = ["m_LocalPosition", "m_LocalRotation", "m_LocalScale"]
            .iter()
            .find_map(|f| Some((*f, axis(f)?)));
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
            let mut own = runity::scene::Override::default();
            let change = if kind == "model" {
                &mut own
            } else {
                desc.overrides.entry(part).or_default()
            };
            if path == "m_IsActive" {
                change.inactive = Some(value == 0.0);
            } else {
                change.set_part(&runity::scene::LayerName(
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
                "a component removed from a prefab instance, of a kind runity has no place for",
            ),
        }
    }
    Some((desc, into))
}

/// A prefab file's GameObjects as a modification's `target` names them —
/// its own, and through the prefabs inside it theirs, whose id in the file is
/// Unity's `(instance ^ source) & i64::MAX` — to the key runity's overrides
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
            out.root = Some(root);
        }
        let text = unity
            .named(guid)
            .filter(|(kind, _)| *kind == "prefab" && depth < 16)
            .and_then(|_| unity.guids.get(guid))
            .and_then(|path| std::fs::read_to_string(path).ok());
        for d in text.as_deref().map(yaml::documents).unwrap_or_default() {
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
                        out.places.insert(go, transform(&d.body));
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
                }
                _ if !d.stripped => {
                    let go = d.body.reference("m_GameObject").filter(|r| r.file_id != 0);
                    if let (Some(go), Some(what)) = (go, removal(&d, unity)) {
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
    desc.set_part(&runity::scene::SoundSource {
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
    match c.kind.as_str() {
        "MeshFilter" => {
            if let Some(model) = b.reference("m_Mesh").and_then(|r| model(&r, refs.unity)) {
                desc.set_part(&runity::scene::ModelRef(AssetLink::named(model)));
            }
        }
        "MeshRenderer" | "SkinnedMeshRenderer" => {
            if c.kind == "SkinnedMeshRenderer" {
                if let Some(model) = b.reference("m_Mesh").and_then(|r| model(&r, refs.unity)) {
                    desc.set_part(&runity::scene::ModelRef(AssetLink::named(model)));
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
            });
            solid(desc, b);
        }
        "CapsuleCollider" => {
            let radius = b.f32("m_Radius").unwrap_or(0.5);
            let height = b.f32("m_Height").unwrap_or(2.0);
            desc.set_part(&Collider::Capsule {
                half_height: (height * 0.5 - radius).max(0.0),
                radius,
            });
            if b.i64("m_Direction").is_some_and(|d| d != 1) {
                report.skip("a capsule lying along x or z (brought over standing)");
            }
            solid(desc, b);
        }
        "MeshCollider" => {
            desc.set_part(&Collider::Model);
            solid(desc, b);
        }
        "Rigidbody" => {
            desc.set_part(&if b.i64("m_IsKinematic") == Some(1) {
                Body::Kinematic
            } else {
                Body::Dynamic
            });
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
                shadows: b["m_Shadows"].i64("m_Type").unwrap_or(0) != 0,
                flare: 0.0,
            });
        }
        "Camera" => {
            let mut lens: Lens = runity::ron::from_str("()").expect("a lens of defaults");
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
            desc.set_part(&match c.kind.as_str() {
                "FixedJoint" => Joint::Fixed { to },
                "HingeJoint" => Joint::Hinge {
                    to,
                    anchor,
                    axis: b.vec3("m_Axis").map(axis).unwrap_or(Vec3::X),
                    limits_deg: (b.i64("m_UseLimits") == Some(1)).then(|| {
                        let l = &b["m_Limits"];
                        // Mirrored: a turn one way is now the other.
                        (-l.f32("max").unwrap_or(0.0), -l.f32("min").unwrap_or(0.0))
                    }),
                    motor: None,
                },
                "SpringJoint" => Joint::Spring {
                    to,
                    anchor,
                    stiffness: b.f32("m_Spring").unwrap_or(10.0),
                    damping: b.f32("m_Damper").unwrap_or(0.2),
                },
                _ => {
                    report.skip(format!("{} (brought over as a ball joint)", c.kind));
                    Joint::Ball { to, anchor }
                }
            });
            // Unity writes an unbreakable joint's force as infinity.
            desc.set_joint_break(b.f32("m_BreakForce").filter(|f| f.is_finite() && *f < 1e30));
        }
        "MonoBehaviour" => {
            let Some(script) = b.reference("m_Script") else {
                return;
            };
            let Some(path) = script.guid.as_deref().and_then(|g| refs.unity.guids.get(g)) else {
                report.skip("a MonoBehaviour whose script is not in Assets/ (a package's)");
                return;
            };
            // Dacha's planar mirror: runity's own, a camera reflected in
            // the plane the mirror's material shows.
            if super::stem(path) == "PlanarReflectionMirror" {
                desc.set_part(&runity::scene::RenderTexture {
                    name: "mirror".into(),
                    hide: Vec::new(),
                    mirror: true,
                });
                return;
            }
            let name = snake(&super::stem(path));
            let value = mono_behaviour(b, refs);
            match runity::ron::value::RawValue::from_boxed_ron(value.into_boxed_str()) {
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
                    desc.set_part(&runity::scene::AnimatorRef(name.to_string()))
                }
                Some(_) => report.skip("a switched-off Animator"),
                None => report.skip("an Animator with no controller (or an override controller)"),
            }
        }
        "AudioSource" => audio_source(desc, b, refs.unity, report),
        "ParticleSystem" => shuriken(desc, b, report),
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
        }
        other => report.skip(other.to_string()),
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

/// A Shuriken ParticleSystem onto the entity's emitter: the Main module's
/// life, speed, size, colour and gravity, the emission rate, the cone,
/// the simulation space, size and colour over lifetime. What has no
/// counterpart (bursts, noise, collision, trails…) the report names.
fn shuriken(desc: &mut EntityDesc, b: &Yaml, report: &mut Report) {
    let mut emitter = desc.particles().unwrap_or_default();
    let e = &mut emitter;
    let main = &b["InitialModule"];
    let rgb = |c: [f32; 4]| (c[0], c[1], c[2]);
    e.life = min_max(&main["startLifetime"]).unwrap_or(5.0);
    e.speed = min_max(&main["startSpeed"]).unwrap_or(5.0);
    e.size = min_max(&main["startSize"]).unwrap_or(1.0);
    if let Some((start, _)) = gradient(&main["startColor"]) {
        e.color = rgb(start);
        e.alpha = start[3];
    }
    e.gravity = -9.81 * min_max(&main["gravityModifier"]).unwrap_or(0.0);
    // 0 is Local, 1 World.
    e.local = b.i64("moveWithTransform") == Some(0);
    let emission = &b["EmissionModule"];
    e.rate = if emission.i64("enabled") == Some(0) {
        0.0
    } else {
        min_max(&emission["rateOverTime"]).unwrap_or(10.0)
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
    if shape.i64("enabled") != Some(0) {
        e.spread_deg = match shape.i64("type").unwrap_or(4) {
            // Sphere, hemisphere.
            0 | 1 => 180.0,
            2 | 3 => 90.0,
            _ => shape.f32("angle").unwrap_or(25.0),
        };
    } else {
        e.spread_deg = 0.0;
    }
    // Unity's cone points along forward, and Z is mirrored.
    e.direction = Some(Vec3::new(0.0, 0.0, -1.0));
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
    for module in [
        "NoiseModule",
        "CollisionModule",
        "TrailModule",
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
/// else by its file's runity name.
fn model(r: &Ref, unity: &Unity) -> Option<String> {
    let guid = r.guid.as_deref()?;
    if guid == BUILTIN {
        return Some(
            match r.file_id {
                10202 => "builtin:cube",
                10206 => "builtin:cylinder",
                10207 => "builtin:sphere",
                10208 => "builtin:cylinder",
                10209 | 10210 => "builtin:plane",
                _ => return None,
            }
            .to_string(),
        );
    }
    let (kind, name) = unity.named(guid)?;
    (kind == "model").then(|| name.to_string())
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
    use super::*;

    fn unity() -> Unity {
        Unity {
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
    fn an_instance_moves_a_part_and_takes_its_light_away() {
        let dir = std::env::temp_dir().join(format!("runity-parts-{}", std::process::id()));
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
        let back: runity::scene::Emitter = ron::from_str(&text).unwrap();
        assert_eq!(back, e);
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
        let scene = runity::Scene {
            entities: roots,
            ..Default::default()
        };
        let text = runity::ron::to_string(&scene).unwrap();
        let back: runity::Scene = runity::ron::from_str(&text).unwrap();
        assert_eq!(back.entities.len(), 2);
    }
}
