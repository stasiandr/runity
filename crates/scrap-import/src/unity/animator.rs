//! Animator Controllers (`.controller`) as scrap animator graphs.
//!
//! A controller is documents: the controller (parameters, layers), state
//! machines (states, any-state transitions, the default state), states
//! (a motion, speed, their transitions), transitions (a destination and
//! conditions) and blend trees. The first layer comes over; clips are
//! named as the model's `.meta` names them.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use scrap::animgraph::{Condition, Graph, State, Transition};
use yaml_rust2::Yaml;

use super::yaml::{self, Get};
use super::Unity;

/// A clip's name, from the model its motion is in: the `.meta` file's
/// `internalIDToNameTable` (or `clipAnimations`) names it by fileID.
fn clip_name(
    unity: &Unity,
    r: &yaml::Ref,
    cache: &mut HashMap<String, HashMap<i64, String>>,
) -> Option<String> {
    let guid = r.guid.clone()?;
    let path = unity.guids.get(&guid)?;
    if path.extension().is_some_and(|e| e == "anim") {
        // As `clips/` names it.
        return Some(
            unity
                .names
                .get(&guid)
                .cloned()
                .unwrap_or_else(|| super::stem(path)),
        );
    }
    let table = cache.entry(guid).or_insert_with(|| {
        let mut out = HashMap::new();
        let Ok(text) = std::fs::read_to_string(super::meta_of(path)) else {
            return out;
        };
        // `- first: {74: 123}` then `second: Name` on the next line; or,
        // under `clipAnimations`, `name: Name` and later `internalID: 123`.
        // Mixamo names every clip `mixamo.com`: the model's own name says
        // more, and keeps the clips of different files apart.
        let model = super::stem(path);
        let mut pending: Option<i64> = None;
        let mut named: Option<String> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line
                .strip_prefix("name:")
                .filter(|_| line.starts_with("name:"))
            {
                named = Some(name.trim().to_string());
            } else if let Some(id) = line
                .strip_prefix("internalID:")
                .and_then(|n| n.trim().parse::<i64>().ok())
            {
                if let Some(name) = named.take() {
                    let name = if name == "mixamo.com" {
                        model.clone()
                    } else {
                        name
                    };
                    out.entry(id).or_insert(name);
                }
            } else if let Some(rest) = line.strip_prefix("- first:") {
                pending = rest
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .split(':')
                    .nth(1)
                    .and_then(|n| n.trim().parse().ok());
            } else if let (Some(id), Some(name)) = (pending, line.strip_prefix("second:")) {
                out.insert(id, name.trim().to_string());
                pending = None;
            }
        }
        // A take the `.meta` does not name — no `clipAnimations`, an empty
        // table: Unity names it by the take and gives it the fileID its
        // name hashes to.
        for take in fbx_takes(path) {
            let id = unity_file_id("AnimationClip", &take);
            let name = if take == "mixamo.com" { model.clone() } else { take };
            out.entry(id).or_insert(name);
        }
        out
    });
    table.get(&r.file_id).cloned()
}

/// Whether a model's clip loops: its `clipAnimations` entry's Loop Time.
/// A take the `.meta` does not list plays once, as Unity imports it.
fn model_clip_loops(unity: &Unity, r: &yaml::Ref) -> bool {
    let Some(path) = r.guid.as_ref().and_then(|g| unity.guids.get(g)) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(super::meta_of(path)) else {
        return false;
    };
    let mut this = false;
    for line in text.lines().map(str::trim) {
        if let Some(id) = line.strip_prefix("internalID:") {
            this = id.trim().parse::<i64>().ok() == Some(r.file_id);
        } else if let Some(v) = line.strip_prefix("loopTime:").filter(|_| this) {
            return v.trim() == "1";
        }
    }
    false
}

/// The takes (animation stacks) an FBX file holds, by name: a binary file's
/// `Name\0\x01AnimStack` strings, an ASCII one's `"AnimStack::Name"`.
fn fbx_takes(path: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let mut takes: Vec<String> = Vec::new();
    let mut add = |name: &[u8]| {
        if let Ok(name) = std::str::from_utf8(name) {
            if !name.is_empty() && !takes.iter().any(|t| t == name) {
                takes.push(name.to_string());
            }
        }
    };
    let binary_tail = b"\x00\x01AnimStack";
    let ascii_head = b"\"AnimStack::";
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(binary_tail) {
            // Back to the string's `S` and length: the name is what lies
            // between them and here.
            let end = i + binary_tail.len();
            let start = (0..i.saturating_sub(4)).rev().take(1024).find(|&j| {
                bytes[j] == b'S'
                    && u32::from_le_bytes([bytes[j + 1], bytes[j + 2], bytes[j + 3], bytes[j + 4]])
                        as usize
                        == end - (j + 5)
            });
            if let Some(j) = start {
                add(&bytes[j + 5..i]);
            }
            i = end;
        } else if bytes[i..].starts_with(ascii_head) {
            let from = i + ascii_head.len();
            if let Some(len) = bytes[from..].iter().position(|b| *b == b'"') {
                add(&bytes[from..from + len]);
            }
            i = from;
        } else {
            i += 1;
        }
    }
    takes
}

/// The fileID Unity gives an object a model importer makes of a class and
/// a name it does not list: xxHash64 of `Type:<class>-><name>0`.
fn unity_file_id(class: &str, name: &str) -> i64 {
    xxh64(format!("Type:{class}->{name}0").as_bytes()) as i64
}

/// xxHash64, seed 0.
fn xxh64(input: &[u8]) -> u64 {
    const P1: u64 = 11400714785074694791;
    const P2: u64 = 14029467366897019727;
    const P3: u64 = 1609587929392839161;
    const P4: u64 = 9650029242287828579;
    const P5: u64 = 2870177450012600261;
    let round = |acc: u64, lane: u64| {
        acc.wrapping_add(lane.wrapping_mul(P2))
            .rotate_left(31)
            .wrapping_mul(P1)
    };
    let merge = |acc: u64, v: u64| (acc ^ round(0, v)).wrapping_mul(P1).wrapping_add(P4);
    let u64_at = |i: usize| u64::from_le_bytes(input[i..i + 8].try_into().unwrap());
    let n = input.len();
    let mut i = 0;
    let mut h = if n >= 32 {
        let mut v = [
            P1.wrapping_add(P2),
            P2,
            0,
            0u64.wrapping_sub(P1),
        ];
        while i + 32 <= n {
            for (k, lane) in v.iter_mut().enumerate() {
                *lane = round(*lane, u64_at(i + 8 * k));
            }
            i += 32;
        }
        let mut h = v[0]
            .rotate_left(1)
            .wrapping_add(v[1].rotate_left(7))
            .wrapping_add(v[2].rotate_left(12))
            .wrapping_add(v[3].rotate_left(18));
        for lane in v {
            h = merge(h, lane);
        }
        h
    } else {
        P5
    };
    h = h.wrapping_add(n as u64);
    while i + 8 <= n {
        h = (h ^ round(0, u64_at(i))).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        i += 8;
    }
    if i + 4 <= n {
        let w = u32::from_le_bytes(input[i..i + 4].try_into().unwrap()) as u64;
        h = (h ^ w.wrapping_mul(P1)).rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        i += 4;
    }
    while i < n {
        h = (h ^ (input[i] as u64).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
        i += 1;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}

fn condition(c: &Yaml, kinds: &HashMap<String, i64>) -> Option<Condition> {
    let parameter = c.str("m_ConditionEvent")?.to_string();
    let threshold = c.f32("m_EventTreshold").unwrap_or(0.0);
    let mode = c.i64("m_ConditionMode")?;
    // Unity: 1 If, 2 IfNot, 3 Greater, 4 Less, 6 Equals, 7 NotEqual.
    // A trigger's `If` is the trigger being pulled.
    Some(match mode {
        1 if kinds.get(&parameter) == Some(&9) => Condition::Trigger(parameter),
        1 => Condition::Is(parameter),
        2 => Condition::Not(parameter),
        3 => Condition::Above(parameter, threshold),
        4 => Condition::Below(parameter, threshold),
        6 => Condition::Equals(parameter, threshold),
        7 => Condition::NotEquals(parameter, threshold),
        _ => return None,
    })
}

/// A controller as the text of an animator graph.
pub fn convert(unity: &Unity, path: &Path, report: &mut super::Report) -> Result<String> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let docs = yaml::documents(&text);
    let by_id = yaml::by_id(&docs);
    let controller = docs
        .iter()
        .find(|d| d.kind == "AnimatorController")
        .context("no AnimatorController in it")?;
    // Parameter kinds: 1 float, 3 int, 4 bool, 9 trigger.
    let kinds: HashMap<String, i64> = controller
        .body
        .list("m_AnimatorParameters")
        .iter()
        .filter_map(|p| Some((p.str("m_Name")?.to_string(), p.i64("m_Type")?)))
        .collect();
    let layer = controller
        .body
        .list("m_AnimatorLayers")
        .first()
        .context("no layers")?;
    let machine_id = layer
        .reference("m_StateMachine")
        .context("no state machine")?
        .file_id;
    let machine = by_id
        .get(&machine_id)
        .context("the state machine is missing")?;
    let mut names: HashMap<i64, String> = HashMap::new();
    let mut states = BTreeMap::new();
    let mut clips_cache = HashMap::new();
    let mut unnamed = 0;
    for child in machine.body.list("m_ChildStates") {
        let Some(id) = child.reference("m_State").map(|r| r.file_id) else {
            continue;
        };
        let Some(state) = by_id.get(&id) else {
            continue;
        };
        let named = state
            .body
            .str("m_Name")
            .map(str::to_string)
            .unwrap_or_else(|| {
                unnamed += 1;
                format!("state_{unnamed}")
            });
        // Unity keeps two states of one name apart by their fileIDs — a
        // Mixamo clip dragged in twice is `mixamo_com` both times (Dacha's
        // pawn, a jump standing and one running). Here a state is its
        // name: the second is `mixamo_com 1`, as Unity names a copy.
        let mut name = named.clone();
        let mut copy = 0;
        while states.contains_key(&name) {
            copy += 1;
            name = format!("{named} {copy}");
        }
        let motion = state.body.reference("m_Motion");
        let mut out = State {
            clip: String::new(),
            blend: Vec::new(),
            blend_by: String::new(),
            directional: Vec::new(),
            blend_by_y: String::new(),
            events: Vec::new(),
            looping: true,
            speed: state.body.f32("m_Speed").unwrap_or(1.0),
            speed_from: (state.body.i64("m_SpeedParameterActive") == Some(1))
                .then(|| state.body.str("m_SpeedParameter").map(str::to_string))
                .flatten(),
            time_from: (state.body.i64("m_TimeParameterActive") == Some(1))
                .then(|| state.body.str("m_TimeParameter").map(str::to_string))
                .flatten(),
        };
        if let Some(m) = motion.filter(|m| !m.is_none()) {
            if let Some(tree) = m.guid.is_none().then(|| by_id.get(&m.file_id)).flatten() {
                // A blend tree in this file: 1D children by threshold, 2D
                // ones (simple and freeform directional, freeform
                // cartesian) by position.
                out.blend_by = tree.body.str("m_BlendParameter").unwrap_or("").to_string();
                let two_d = matches!(tree.body.i64("m_BlendType"), Some(1..=3));
                if two_d {
                    out.blend_by_y = tree.body.str("m_BlendParameterY").unwrap_or("").to_string();
                }
                for c in tree.body.list("m_Childs") {
                    let Some(clip) = c
                        .reference("m_Motion")
                        .and_then(|r| clip_name(unity, &r, &mut clips_cache))
                    else {
                        continue;
                    };
                    if two_d {
                        let at = &c["m_Position"];
                        out.directional.push((
                            at.f32("x").unwrap_or(0.0),
                            at.f32("y").unwrap_or(0.0),
                            clip,
                        ));
                    } else {
                        out.blend.push((c.f32("m_Threshold").unwrap_or(0.0), clip));
                    }
                }
                out.blend.sort_by(|a, b| a.0.total_cmp(&b.0));
            } else if let Some(clip) = clip_name(unity, &m, &mut clips_cache) {
                out.clip = clip;
                // An `.anim` says whether it loops (Loop Time).
                let anim = m
                    .guid
                    .as_ref()
                    .and_then(|g| unity.guids.get(g))
                    .filter(|p| p.extension().is_some_and(|e| e == "anim"));
                if let Some(text) = anim.and_then(|p| std::fs::read_to_string(p).ok()) {
                    out.looping = !text.contains("m_LoopTime: 0");
                } else if anim.is_none() {
                    // A model's clip: its import settings' Loop Time.
                    out.looping = model_clip_loops(unity, &m);
                }
            }
        }
        if out.clip.is_empty() && out.blend.is_empty() && out.directional.is_empty() {
            out.clip = name.clone();
        }
        names.insert(id, name.clone());
        states.insert(name, out);
    }
    let start = machine
        .body
        .reference("m_DefaultState")
        .and_then(|r| names.get(&r.file_id).cloned())
        .or_else(|| states.keys().next().cloned())
        .unwrap_or_default();
    let mut transitions = Vec::new();
    let mut add = |from: &str, transition: &Yaml| {
        let Some(t) = transition_doc(transition, &by_id) else {
            return;
        };
        let Some(to) = t
            .reference("m_DstState")
            .and_then(|r| names.get(&r.file_id).cloned())
        else {
            return;
        };
        let mut when: Vec<Condition> = t
            .list("m_Conditions")
            .iter()
            .filter_map(|c| condition(c, &kinds))
            .collect();
        if when.is_empty() && t.i64("m_HasExitTime") == Some(1) {
            when.push(Condition::Finished);
        }
        transitions.push(Transition {
            from: from.to_string(),
            to,
            when,
            fade: t.f32("m_TransitionDuration").unwrap_or(0.2),
        });
    };
    for any in machine.body.list("m_AnyStateTransitions") {
        add("*", any);
    }
    for (id, name) in &names {
        if let Some(state) = by_id.get(id) {
            for t in state.body.list("m_Transitions") {
                add(name, t);
            }
        }
    }
    transitions.sort_by(|a, b| a.from.cmp(&b.from).then(a.to.cmp(&b.to)));
    // What a flat graph of the base layer has no place for, said rather
    // than dropped without a word (docs/animation.md: no nested machines).
    for _ in 1..controller.body.list("m_AnimatorLayers").len() {
        report.skip("an animator layer past the base one");
    }
    for doc in &docs {
        match doc.kind.as_str() {
            "AnimatorStateMachine" if doc.file_id != machine_id => {
                report.skip("an animator's sub-state machine")
            }
            "AnimatorState" if !names.contains_key(&doc.file_id) => {
                report.skip("an animator state inside a sub-state machine or a later layer")
            }
            "MonoBehaviour" => report.skip("a StateMachineBehaviour (state callbacks are game code)"),
            _ => {}
        }
    }
    let graph = Graph {
        start,
        states,
        transitions,
        // Layers and their Avatar Masks do not come over yet.
        layers: Vec::new(),
    };
    let pretty = ron::ser::PrettyConfig::new().depth_limit(3);
    Ok(format!(
        "// From {}.\n{}\n",
        path.strip_prefix(&unity.root).unwrap_or(path).display(),
        ron::ser::to_string_pretty(&graph, pretty)?
    ))
}

fn transition_doc<'a>(r: &Yaml, by_id: &HashMap<i64, &'a yaml::Doc>) -> Option<&'a Yaml> {
    let id = yaml::reference(r)?.file_id;
    by_id.get(&id).map(|d| &d.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLLER: &str = "%YAML 1.1
--- !u!91 &9100000
AnimatorController:
  m_Name: Hero
  m_AnimatorParameters:
  - m_Name: speed
    m_Type: 1
  - m_Name: jump
    m_Type: 9
  m_AnimatorLayers:
  - m_Name: Base Layer
    m_StateMachine: {fileID: 100}
--- !u!1107 &100
AnimatorStateMachine:
  m_ChildStates:
  - m_State: {fileID: 200}
  - m_State: {fileID: 300}
  m_AnyStateTransitions:
  - {fileID: 500}
  m_DefaultState: {fileID: 200}
--- !u!1102 &200
AnimatorState:
  m_Name: Idle
  m_Speed: 1
  m_Transitions:
  - {fileID: 400}
  m_Motion: {fileID: 7400000, guid: idleclip, type: 2}
--- !u!1102 &300
AnimatorState:
  m_Name: Jump
  m_Speed: 1
  m_TimeParameterActive: 1
  m_TimeParameter: speed
  m_Transitions: []
  m_Motion: {fileID: 0}
--- !u!1101 &400
AnimatorStateTransition:
  m_Conditions:
  - m_ConditionMode: 3
    m_ConditionEvent: speed
    m_EventTreshold: 0.1
  m_DstState: {fileID: 300}
  m_TransitionDuration: 0.25
  m_HasExitTime: 0
--- !u!1101 &500
AnimatorStateTransition:
  m_Conditions:
  - m_ConditionMode: 1
    m_ConditionEvent: jump
    m_EventTreshold: 0
  m_DstState: {fileID: 300}
  m_TransitionDuration: 0.1
";

    #[test]
    fn a_clip_is_named_from_the_models_clip_animations() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("A_Run.fbx.meta"),
            "ModelImporter:\n  animations:\n    clipAnimations:\n    - serializedVersion: 16\n      name: mixamo.com\n      takeName: mixamo.com\n      internalID: -42\n    - serializedVersion: 16\n      name: Wave\n      takeName: Take 001\n      internalID: 7\n",
        )
        .unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            declared_params: Default::default(),
            layers: Default::default(),
            root: dir.clone(),
            guids: [("run".to_string(), dir.join("A_Run.fbx"))]
                .into_iter()
                .collect(),
            names: Default::default(),
        };
        let mut cache = HashMap::new();
        let clip = |id| yaml::Ref {
            file_id: id,
            guid: Some("run".into()),
        };
        assert_eq!(
            clip_name(&unity, &clip(-42), &mut cache).as_deref(),
            Some("A_Run"),
            "Mixamo's name for every clip: the model's instead"
        );
        assert_eq!(
            clip_name(&unity, &clip(7), &mut cache).as_deref(),
            Some("Wave")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A model whose `.meta` lists no clips: each take is named by itself
    /// and found by the fileID Unity hashes its name to (Dacha's hand,
    /// `SKM_Hand_01_Grab`, whose Grab state plays -4673120959766696371);
    /// and it plays once, as Unity imports a take it is told nothing of.
    #[test]
    fn a_models_unlisted_take_is_found_by_unitys_hash_of_its_name() {
        assert_eq!(unity_file_id("AnimationClip", "Armature|Armature|Grab|BaseLayer"), -4673120959766696371);
        assert_eq!(unity_file_id("AnimationClip", "Armature|Armature|Point|BaseLayer"), -2317691810437224117);
        let dir = std::env::temp_dir().join(format!("scrap-unity-take-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A binary FBX's stack: `S`, the length, `Name\0\x01AnimStack`.
        let mut fbx = b"Kaydara FBX Binary  \0junk".to_vec();
        for take in ["Armature|Armature|Grab|BaseLayer", "Armature|Armature|Point|BaseLayer"] {
            let name = format!("{take}\0\x01AnimStack");
            fbx.push(b'S');
            fbx.extend_from_slice(&(name.len() as u32).to_le_bytes());
            fbx.extend_from_slice(name.as_bytes());
            fbx.extend_from_slice(b"S\x09\0\0\0AnimStack");
        }
        std::fs::write(dir.join("Hand.fbx"), &fbx).unwrap();
        std::fs::write(dir.join("Hand.fbx.meta"), "ModelImporter:\n  internalIDToNameTable: []\n  animations:\n    clipAnimations: []\n").unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            declared_params: Default::default(),
            layers: Default::default(),
            root: dir.clone(),
            guids: [("hand".to_string(), dir.join("Hand.fbx"))].into_iter().collect(),
            names: Default::default(),
        };
        let mut cache = HashMap::new();
        let clip = |id| yaml::Ref { file_id: id, guid: Some("hand".into()) };
        assert_eq!(clip_name(&unity, &clip(-4673120959766696371), &mut cache).as_deref(), Some("Armature|Armature|Grab|BaseLayer"));
        assert_eq!(clip_name(&unity, &clip(-2317691810437224117), &mut cache).as_deref(), Some("Armature|Armature|Point|BaseLayer"));
        assert!(!model_clip_loops(&unity, &clip(-4673120959766696371)), "a take plays once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A clip the model's `.meta` lists loops as its Loop Time says.
    #[test]
    fn a_listed_model_clip_loops_as_its_loop_time_says() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-loop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Run.fbx.meta"),
            "ModelImporter:\n  animations:\n    clipAnimations:\n    - name: Run\n      internalID: 5\n      loopTime: 1\n    - name: Jump\n      internalID: 6\n      loopTime: 0\n",
        )
        .unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            declared_params: Default::default(),
            layers: Default::default(),
            root: dir.clone(),
            guids: [("run".to_string(), dir.join("Run.fbx"))].into_iter().collect(),
            names: Default::default(),
        };
        let clip = |id| yaml::Ref { file_id: id, guid: Some("run".into()) };
        assert!(model_clip_loops(&unity, &clip(5)));
        assert!(!model_clip_loops(&unity, &clip(6)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_controller_becomes_a_graph() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-ctl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Hero.controller");
        std::fs::write(&path, CONTROLLER).unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            declared_params: Default::default(),
            layers: Default::default(),
            root: dir.clone(),
            guids: [("idleclip".to_string(), dir.join("Idle.anim"))]
                .into_iter()
                .collect(),
            names: Default::default(),
        };
        let text = convert(&unity, &path, &mut Default::default()).unwrap();
        let graph: Graph = ron::from_str(
            &text
                .lines()
                .filter(|l| !l.starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert_eq!(graph.start, "Idle");
        assert_eq!(graph.states["Idle"].clip, "Idle");
        assert_eq!(graph.states["Jump"].time_from.as_deref(), Some("speed"));
        assert!(graph.transitions.iter().any(|t| t.from == "*"
            && t.to == "Jump"
            && t.when == vec![Condition::Trigger("jump".into())]));
        assert!(graph
            .transitions
            .iter()
            .any(|t| t.from == "Idle" && t.when == vec![Condition::Above("speed".into(), 0.1)]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two states Unity calls the same (Dacha's pawn has two `mixamo_com`,
    /// a jump standing and one running): both come over, the second as
    /// `mixamo_com 1`, each with its own clip and its own way in and out.
    #[test]
    fn two_states_of_one_name_both_come_over() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-twins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Pawn.controller");
        let text = CONTROLLER
            .replace("  - m_State: {fileID: 300}\n", "  - m_State: {fileID: 300}\n  - m_State: {fileID: 600}\n")
            .replace("  m_Name: Jump\n", "  m_Name: mixamo_com\n")
            + "--- !u!1102 &600
AnimatorState:
  m_Name: mixamo_com
  m_Speed: 1
  m_Transitions:
  - {fileID: 700}
  m_Motion: {fileID: 7400000, guid: runclip, type: 2}
--- !u!1101 &700
AnimatorStateTransition:
  m_Conditions: []
  m_DstState: {fileID: 200}
  m_TransitionDuration: 0.3
  m_HasExitTime: 1
";
        std::fs::write(&path, text).unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            declared_params: Default::default(),
            layers: Default::default(),
            root: dir.clone(),
            guids: [("idleclip".to_string(), dir.join("Idle.anim")), ("runclip".to_string(), dir.join("Run.anim"))]
                .into_iter()
                .collect(),
            names: Default::default(),
        };
        let text = convert(&unity, &path).unwrap();
        let graph: Graph = ron::from_str(&text.lines().filter(|l| !l.starts_with("//")).collect::<Vec<_>>().join("\n")).unwrap();
        assert_eq!(graph.states["mixamo_com"].time_from.as_deref(), Some("speed"), "the first keeps the name");
        assert_eq!(graph.states["mixamo_com 1"].clip, "Run", "the second is a state of its own");
        assert!(graph.transitions.iter().any(|t| t.from == "Idle" && t.to == "mixamo_com"));
        assert!(graph.transitions.iter().any(|t| t.from == "mixamo_com 1" && t.to == "Idle" && t.fade == 0.3));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
