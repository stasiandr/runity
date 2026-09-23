//! A `.unity` or `.prefab` file as runity entities.

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

/// The roots of a Unity file as entities, children under them.
pub fn convert_file(unity: &Unity, text: &str, report: &mut Report) -> Vec<EntityDesc> {
    let docs = yaml::documents(text);
    let by_id = yaml::by_id(&docs);

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
                if d.body.i64("m_IsActive") == Some(0) {
                    report.skip("an inactive GameObject (brought over active)");
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
                let Some((desc, into)) = instance(d, &by_id, &entity_of_transform, unity, report)
                else {
                    continue;
                };
                if let Some(p) = into {
                    parent.insert(d.file_id, p);
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
    for d in docs.iter().filter(|d| d.class == GAME_OBJECT && d.stripped) {
        if let Some(instance) = d.body.reference("m_PrefabInstance") {
            let added = components.get(&d.file_id).map_or(0, Vec::len);
            if added > 0 {
                if let Some(desc) = entities.get_mut(&instance.file_id) {
                    for c in components.get(&d.file_id).into_iter().flatten() {
                        component(desc, c, &refs, report);
                    }
                }
            }
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
    roots
        .into_iter()
        .filter_map(|r| build(r, &mut entities, &children))
        .collect()
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
    _by_id: &HashMap<i64, &Doc>,
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
        desc.model = AssetLink::named(name);
    } else {
        desc.prefab = AssetLink::named(name);
    }
    // The root's transform is what the modifications say; which target is
    // the root is the one carrying m_LocalPosition, as Unity always writes it.
    let mut root_target = None;
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
        if let Some(i) = axis("m_LocalPosition") {
            root_target = root_target.or(target);
            if target == root_target && i < 3 {
                p[i] = value;
            }
        } else if let Some(i) = axis("m_LocalRotation") {
            if target == root_target || root_target.is_none() {
                q[i] = value;
            }
        } else if let Some(i) = axis("m_LocalScale") {
            if target == root_target || root_target.is_none() {
                if i < 3 {
                    s[i] = value;
                }
            } else {
                report.skip("a prefab instance's part rescaled (m_LocalScale on a part)");
            }
        } else if path == "m_Materials.Array.data[0]" {
            // The first material swapped on the placed thing.
            let material = m
                .reference("objectReference")
                .and_then(|r| r.guid)
                .and_then(|g| unity.named(&g))
                .filter(|(k, _)| *k == "material");
            if let Some((_, name)) = material {
                desc.material = MaterialRef::Named(AssetLink::named(name));
            }
        } else if path.starts_with("m_Materials") {
            report.skip("a prefab modification of `m_Materials` past the first");
        } else if path == "m_Name" {
            if let Some(n) = m.str("value") {
                desc.name = n.to_string();
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
    for removed in modification.list("m_RemovedComponents") {
        let _ = removed;
        report.skip("a component removed from a prefab instance");
    }
    Some((desc, into))
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
                desc.model = AssetLink::named(model);
            }
        }
        "MeshRenderer" | "SkinnedMeshRenderer" => {
            if c.kind == "SkinnedMeshRenderer" {
                if let Some(model) = b.reference("m_Mesh").and_then(|r| model(&r, refs.unity)) {
                    desc.model = AssetLink::named(model);
                }
            }
            let materials = b.list("m_Materials");
            if let Some(first) = materials.first().and_then(yaml::reference) {
                if let Some((kind, name)) = first.guid.as_deref().and_then(|g| refs.unity.named(g))
                {
                    if kind == "material" {
                        desc.material = MaterialRef::Named(AssetLink::named(name));
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
            desc.collider = Collider::Box {
                half: Vec3::from_array(size) * 0.5,
                center,
            };
            solid(desc, b);
        }
        "SphereCollider" => {
            desc.collider = Collider::Sphere {
                radius: b.f32("m_Radius").unwrap_or(0.5),
            };
            solid(desc, b);
        }
        "CapsuleCollider" => {
            let radius = b.f32("m_Radius").unwrap_or(0.5);
            let height = b.f32("m_Height").unwrap_or(2.0);
            desc.collider = Collider::Capsule {
                half_height: (height * 0.5 - radius).max(0.0),
                radius,
            };
            if b.i64("m_Direction").is_some_and(|d| d != 1) {
                report.skip("a capsule lying along x or z (brought over standing)");
            }
            solid(desc, b);
        }
        "MeshCollider" => {
            desc.collider = Collider::Model;
            solid(desc, b);
        }
        "Rigidbody" => {
            desc.body = if b.i64("m_IsKinematic") == Some(1) {
                Body::Kinematic
            } else {
                Body::Dynamic
            };
            desc.physics = BodyProps {
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
                ..BodyProps::default()
            };
        }
        "Light" => {
            let color = b.color("m_Color").unwrap_or([1.0; 4]);
            let kind = b.i64("m_Type").unwrap_or(2);
            if kind == 1 {
                report.skip("a directional light (the scene's sun is its own setting)");
                return;
            }
            desc.light = Some(Light {
                color: (color[0], color[1], color[2]),
                intensity: b.f32("m_Intensity").unwrap_or(1.0),
                range: b.f32("m_Range").unwrap_or(10.0),
                cone_deg: (kind == 0).then(|| b.f32("m_SpotAngle").unwrap_or(30.0)),
                shadows: b["m_Shadows"].i64("m_Type").unwrap_or(0) != 0,
            });
        }
        "Camera" => {
            let mut lens: Lens = runity::ron::from_str("()").expect("a lens of defaults");
            lens.fov_deg = b.f32("field of view").unwrap_or(60.0);
            if b.i64("orthographic") == Some(1) {
                lens.ortho = b.f32("orthographic size");
            }
            desc.camera = Some(lens);
        }
        "HingeJoint" | "FixedJoint" | "CharacterJoint" | "ConfigurableJoint" | "SpringJoint" => {
            let to = b
                .reference("m_ConnectedBody")
                .filter(|r| r.file_id != 0)
                .and_then(|r| refs.body_object.get(&r.file_id))
                .map(|go| entity_id(*go))
                .unwrap_or(EntityId::UNASSIGNED);
            let anchor = b.vec3("m_Anchor").map(position).unwrap_or(Vec3::ZERO);
            desc.joint = match c.kind.as_str() {
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
                _ => {
                    report.skip(format!("{} (brought over as a ball joint)", c.kind));
                    Joint::Ball { to, anchor }
                }
            };
            if b.f32("m_BreakForce")
                .is_some_and(|f| f.is_finite() && f < 1e30)
            {
                report.skip("a joint's break force");
            }
        }
        "MonoBehaviour" => {
            let Some(script) = b.reference("m_Script") else {
                return;
            };
            let Some(path) = script.guid.as_deref().and_then(|g| refs.unity.guids.get(g)) else {
                report.skip("a MonoBehaviour whose script is not in Assets/ (a package's)");
                return;
            };
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
            report.skip("Animator (the graph comes over from animators/; the game attaches it)")
        }
        "AudioSource" => report.skip("AudioSource"),
        "ParticleSystem" | "ParticleSystemRenderer" => report.skip("ParticleSystem (Shuriken)"),
        other => report.skip(other.to_string()),
    }
}

/// A collider's GameObject is solid when nothing else says, a trigger when
/// Unity said so.
fn solid(desc: &mut EntityDesc, b: &Yaml) {
    if b.i64("m_IsTrigger") == Some(1) {
        desc.body = Body::Trigger;
    } else if desc.body == Body::None {
        desc.body = Body::Static;
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
            ]
            .into_iter()
            .collect(),
            names: [
                ("aaa", "crate"),
                ("mmm", "wood"),
                ("sss", "Door"),
                ("ppp", "Lamp"),
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
        assert_eq!(crate_.model.as_str(), "crate");
        assert!(matches!(&crate_.material, MaterialRef::Named(l) if l.as_str() == "wood"));
        assert_eq!(crate_.body, Body::Dynamic);
        assert!(matches!(crate_.collider, Collider::Box { half, center }
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
