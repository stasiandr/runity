//! Motion: clips that move things rather than bones — a door swinging
//! open, a hologram blinking on, a lamp's smoke thinning — Unity's
//! Animation Clips on a GameObject and its children. A clip is a file in
//! `clips/`, tracks of keys by the path of the thing under the entity
//! (`""` is the entity itself, `"Door/Hinge"` a grandchild):
//!
//! ```ron
//! (
//!     length: 1.0,
//!     tracks: [
//!         (path: "Door", what: TurnY, keys: [(0.0, 0.0), (1.0, -90.0)]),
//!         (path: "Light", what: Active, keys: [(0.0, 0.0), (0.5, 1.0)]),
//!     ],
//! )
//! ```
//!
//! A line's `animator: "door"` plays the graph `animators/door.ron` on it,
//! the same graph an Animator window shows and a skeleton plays: its clips
//! are these files by name. The game sets its parameters on the
//! [`Moving`] component — `moving.controller.trigger("open")`.
//!
//! Positions are metres and turns degrees, in scrap's space and the order
//! a line's `rotation` has (Y, then X, then Z); a transform track is keyed
//! linearly — or on a curve between every two keys, with the track's
//! `ease: OutBack` ([`crate::ease::Ease`]) — and what it does not key
//! stays where the scene put it. `Active`
//! is a switch, on from a key above one half to the next key; `Volume` is its sound's,
//! `ParticleRate` its particles'.

#[allow(unused_imports)]
use crate::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use glam::{Quat, Vec3};
use hecs::World;
use serde::{Deserialize, Serialize};

use crate::animation::{Channel, Clip, Joint, Path as Part, PoseTransform, Skeleton};
use crate::animator::Animator;
use crate::animgraph::{Controller, Graph};
use crate::id::EntityId;

/// The folder motion clips are in.
pub const DIR: &str = "clips";

/// What a track moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Property {
    X,
    Y,
    Z,
    TurnX,
    TurnY,
    TurnZ,
    ScaleX,
    ScaleY,
    ScaleZ,
    /// Switched on above 0.5.
    Active,
    /// Its `sound`'s volume.
    Volume,
    /// Its particles' rate.
    ParticleRate,
}

/// One property of one thing over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    pub what: Property,
    /// (seconds, value), in time order.
    pub keys: Vec<(f32, f32)>,
    /// The curve from each key to the next: straight by default.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub ease: crate::ease::Ease,
}

fn is_linear(ease: &crate::ease::Ease) -> bool {
    *ease == crate::ease::Ease::Linear
}

/// A clip of tracks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// Seconds.
    pub length: f32,
    pub tracks: Vec<Track>,
}

impl Motion {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = scrap_core::files::read_to_string(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The value of keys at a time: linear between, held past either end.
pub fn sample(keys: &[(f32, f32)], time: f32) -> Option<f32> {
    sample_eased(keys, time, crate::ease::Ease::Linear)
}

/// The value of keys at a time, on `ease` from each key to the next.
pub fn sample_eased(keys: &[(f32, f32)], time: f32, ease: crate::ease::Ease) -> Option<f32> {
    let first = keys.first()?;
    if time <= first.0 {
        return Some(first.1);
    }
    for pair in keys.windows(2) {
        let ((t0, v0), (t1, v1)) = (pair[0], pair[1]);
        if time <= t1 {
            let f = if t1 > t0 {
                (time - t0) / (t1 - t0)
            } else {
                1.0
            };
            return Some(ease.lerp(v0, v1, f));
        }
    }
    keys.last().map(|k| k.1)
}

/// The value of the last key at or before a time: a switch, which does not
/// pass through a half.
pub fn step(keys: &[(f32, f32)], time: f32) -> Option<f32> {
    let first = keys.first()?;
    Some(
        keys.iter()
            .take_while(|(t, _)| *t <= time)
            .last()
            .unwrap_or(first)
            .1,
    )
}

/// Which of the graph's clips a line animates with, by the path of each
/// thing under it — from the line's `animator`, when it was spawned.
#[derive(Debug, Clone, PartialEq)]
pub struct Animates {
    pub graph: String,
    /// The line's model: a skinned one plays the graph on its skeleton.
    pub model: crate::AssetLink,
    /// Every thing under the line by its path of names, the line itself `""`.
    pub parts: Vec<(String, EntityId)>,
}

/// A line's graph, playing: its controller for the game's parameters.
pub struct Moving {
    pub controller: Controller,
    animator: Animator,
    /// The entity each joint moves.
    joints: Vec<hecs::Entity>,
    /// Per clip, in the animator's order: the tracks that are not a place.
    #[allow(clippy::type_complexity)]
    others: Vec<Vec<(hecs::Entity, Property, Vec<(f32, f32)>, crate::ease::Ease)>>,
    /// What any clip switches on or off, as it was when the line was
    /// dressed: put back while a clip that does not switch it plays —
    /// Unity's Write Defaults. A saw's sparks one state turns off are on
    /// again in the next, which does not mention them.
    defaults: Vec<(hecs::Entity, Property, f32)>,
}

impl Moving {
    /// The clip playing now, by name: what Unity's
    /// `GetCurrentAnimatorClipInfo` says of an animator.
    pub fn playing_clip(&self) -> Option<&str> {
        let playing = self.animator.playing()?;
        self.animator.clips.get(playing.clip).map(|c| c.name.as_str())
    }
}

impl std::fmt::Debug for Moving {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Moving")
            .field("state", &self.controller.state())
            .finish()
    }
}

/// The project's graphs and motion clips, by name.
#[derive(Debug, Default, Clone)]
pub struct Motions {
    pub graphs: HashMap<String, Graph>,
    pub clips: HashMap<String, Motion>,
}

impl Motions {
    /// `animators/*.ron` and `clips/*.ron` under a project's root; what did
    /// not read, said.
    pub fn load(root: impl AsRef<Path>) -> (Self, Vec<String>) {
        let root = root.as_ref();
        let mut out = Self::default();
        let mut problems = Vec::new();
        let files = |dir: &Path| -> Vec<std::path::PathBuf> {
            let mut paths: Vec<_> = scrap_core::files::read_dir(dir)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.extension().is_some_and(|e| e == "ron")
                                && !p.to_string_lossy().ends_with(".cases.ron")
                        })
                        .collect()
                })
                .unwrap_or_default();
            paths.sort();
            paths
        };
        let stem = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().into_owned());
        for path in files(&root.join(crate::project::ANIMATORS)) {
            let read = scrap_core::files::read_to_string(&path).map_err(|e| e.to_string());
            match read.and_then(|t| ron::from_str::<Graph>(&t).map_err(|e| e.to_string())) {
                Ok(graph) => {
                    out.graphs.insert(stem(&path).unwrap_or_default(), graph);
                }
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        for path in files(&root.join(DIR)) {
            match Motion::load(&path) {
                Ok(motion) => {
                    out.clips.insert(stem(&path).unwrap_or_default(), motion);
                }
                Err(e) => problems.push(e),
            }
        }
        (out, problems)
    }
}

/// Every clip a graph names.
pub fn clips_of(graph: &Graph) -> BTreeSet<String> {
    graph.clips()
}

/// Start every line's graph that is not playing yet. A graph with clips in
/// `clips/` moves the things under the line; one without plays on the
/// line's model's skeleton, with the model's own clips and those of any
/// model named as a clip (a Mixamo file a clip) — `skins` finds a model's
/// skin by its link. Returns what it could not do.
pub fn attach(
    world: &mut World,
    motions: &Motions,
    skins: impl Fn(&crate::AssetLink) -> Option<crate::asset::MeshSkin>,
) -> Vec<String> {
    crate::animator::bind_skins(world, &skins);
    let waiting: Vec<(hecs::Entity, Animates)> = world
        .query::<(hecs::Entity, &Animates)>()
        .without::<&Moving>()
        .iter()
        .map(|(e, a)| (e, a.clone()))
        .collect();
    if waiting.is_empty() {
        return Vec::new();
    }
    // By the scene's ids and by a run-time prefab's own: a building stage
    // spawned mid-level has a drone its clips fly.
    let mut by_id: HashMap<EntityId, hecs::Entity> = world
        .query::<(hecs::Entity, &crate::world::SceneId)>()
        .iter()
        .map(|(e, s)| (s.0, e))
        .collect();
    by_id.extend(
        world
            .query::<(hecs::Entity, &crate::world::SpawnedId)>()
            .iter()
            .map(|(e, s)| (s.0, e)),
    );
    let mut problems = Vec::new();
    for (entity, animates) in waiting {
        let Some(graph) = motions.graphs.get(&animates.graph) else {
            problems.push(format!(
                "animator `{}`: no such graph in animators/",
                animates.graph
            ));
            // Not asked again every frame.
            let _ = world.remove_one::<Animates>(entity);
            continue;
        };
        let names: Vec<String> = clips_of(graph)
            .into_iter()
            .filter(|c| motions.clips.contains_key(c))
            .collect();
        if names.is_empty() {
            let _ = world.remove_one::<Animates>(entity);
            match skins(&animates.model) {
                Some(skin) => {
                    let mut animator = Animator::new(Arc::new(skin.skeleton), Arc::new(skin.clips));
                    for clip in clips_of(graph) {
                        if animator.clip_named(&clip).is_some() {
                            continue;
                        }
                        if let Some(from) = skins(&crate::AssetLink::named(clip.clone())) {
                            animator.take_clips(&clip, &from.skeleton, &from.clips);
                        }
                    }
                    // Masks and IK name joints: said once, with the nearest.
                    let joints: Vec<&str> = animator
                        .skeleton
                        .joints
                        .iter()
                        .map(|j| j.name.as_str())
                        .collect();
                    for problem in graph.mask_problems(&joints) {
                        problems.push(format!("animator `{}`: {problem}", animates.graph));
                    }
                    if let Ok(ik) = world.get::<&crate::ik::Ik>(entity) {
                        problems.extend(ik.problems(&animator.skeleton));
                    }
                    let _ = world.insert(entity, (animator, Controller::new(graph.clone())));
                }
                None => problems.push(format!(
                    "animator `{}`: none of its clips is in clips/, and `{}` has no skeleton",
                    animates.graph, animates.model
                )),
            }
            continue;
        }
        let part = |path: &str| -> Option<hecs::Entity> {
            animates
                .parts
                .iter()
                .find(|(p, _)| p == path)
                .and_then(|(_, id)| by_id.get(id).copied())
        };
        let mut joints: Vec<hecs::Entity> = Vec::new();
        let mut rest: Vec<crate::Transform> = Vec::new();
        let mut skeleton = Skeleton { joints: Vec::new() };
        let mut clips = Vec::new();
        let mut others = Vec::new();
        let mut unknown = BTreeSet::new();
        for name in &names {
            let motion = &motions.clips[name];
            let mut by_path: HashMap<&str, Vec<&Track>> = HashMap::new();
            let mut other = Vec::new();
            for track in &motion.tracks {
                let Some(target) = part(&track.path) else {
                    unknown.insert(track.path.clone());
                    continue;
                };
                match track.what {
                    Property::Active | Property::Volume | Property::ParticleRate => {
                        other.push((target, track.what, track.keys.clone(), track.ease));
                    }
                    _ => by_path.entry(track.path.as_str()).or_default().push(track),
                }
            }
            let mut channels = Vec::new();
            for (path, tracks) in by_path {
                let target = part(path).expect("found above");
                let joint = match joints.iter().position(|j| *j == target) {
                    Some(i) => i,
                    None => {
                        let at = world
                            .get::<&crate::Transform>(target)
                            .map(|t| *t)
                            .unwrap_or_default();
                        joints.push(target);
                        rest.push(at);
                        skeleton.joints.push(Joint {
                            name: path.to_string(),
                            parent: None,
                            inverse_bind: glam::Mat4::IDENTITY.to_cols_array_2d(),
                            rest: PoseTransform {
                                translation: at.position.to_array(),
                                rotation: at.rotation().to_array(),
                                scale: at.scale.to_array(),
                            },
                        });
                        joints.len() - 1
                    }
                };
                channels.extend(channels_for(joint as u16, &tracks, &rest[joint]));
            }
            clips.push(Clip {
                name: name.clone(),
                duration: motion.length,
                channels,
            });
            others.push(other);
        }
        // Said once for every copy of the thing: Unity skips a path it
        // cannot find, and so does this.
        for path in unknown {
            let problem = format!(
                "animator `{}`: its clips move `{path}`, which is not under the line (skipped)",
                animates.graph
            );
            if !problems.contains(&problem) {
                problems.push(problem);
            }
        }
        let mut defaults: Vec<(hecs::Entity, Property, f32)> = Vec::new();
        for (target, what, _, _) in others.iter().flatten() {
            if *what == Property::Active && !defaults.iter().any(|(t, w, _)| t == target && w == what) {
                let on = world.get::<&crate::world::Inactive>(*target).is_err();
                defaults.push((*target, *what, if on { 1.0 } else { 0.0 }));
            }
        }
        let animator = Animator::new(Arc::new(skeleton), Arc::new(clips));
        let _ = world.insert_one(
            entity,
            Moving {
                controller: Controller::new(graph.clone()),
                animator,
                joints,
                others,
                defaults,
            },
        );
    }
    problems
}

/// Samples baked between two keys of a track on a curve: the skeleton's
/// channels are straight between their samples.
const EASED_SAMPLES: usize = 16;

/// A joint's channels from its tracks: every axis a track does not key
/// holds the rest's value. A track on a curve is baked into samples
/// between its keys.
fn channels_for(joint: u16, tracks: &[&Track], rest: &crate::Transform) -> Vec<Channel> {
    let find = |what: Property| tracks.iter().find(|t| t.what == what);
    let mut out = Vec::new();
    for (axes, path) in [
        ([Property::X, Property::Y, Property::Z], Part::Translation),
        (
            [Property::TurnX, Property::TurnY, Property::TurnZ],
            Part::Rotation,
        ),
        (
            [Property::ScaleX, Property::ScaleY, Property::ScaleZ],
            Part::Scale,
        ),
    ] {
        let keyed: Vec<Option<&&Track>> = axes.iter().map(|a| find(*a)).collect();
        if keyed.iter().all(Option::is_none) {
            continue;
        }
        let mut times: Vec<f32> = keyed
            .iter()
            .flatten()
            .flat_map(|track| {
                let eased = track.ease != crate::ease::Ease::Linear;
                track
                    .keys
                    .windows(2)
                    .flat_map(move |pair| {
                        let (t0, t1) = (pair[0].0, pair[1].0);
                        let n = if eased { EASED_SAMPLES } else { 1 };
                        (0..n).map(move |i| t0 + (t1 - t0) * i as f32 / n as f32)
                    })
                    .chain(track.keys.iter().map(|(t, _)| *t))
            })
            .collect();
        times.sort_by(f32::total_cmp);
        times.dedup();
        // Angles are keyed as Unity keys them, in degrees, and a turn
        // may be more than half a circle between two keys — a saw's blade
        // from -360 to 0 each second. Rotations blend the short way, so
        // such a gap is cut into pieces of a quarter turn at most: else
        // -360 and 0, the same rotation, would not turn at all.
        if path == Part::Rotation {
            let mut finer = Vec::with_capacity(times.len());
            for pair in times.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                finer.push(a);
                let swing = keyed
                    .iter()
                    .flatten()
                    .filter_map(|k| Some((sample_eased(&k.keys, b, k.ease)? - sample_eased(&k.keys, a, k.ease)?).abs()))
                    .fold(0.0f32, f32::max);
                let pieces = (swing / 90.0).ceil().max(1.0) as usize;
                for n in 1..pieces {
                    finer.push(a + (b - a) * n as f32 / pieces as f32);
                }
            }
            finer.extend(times.last());
            times = finer;
        }
        let base = match path {
            Part::Translation => rest.position,
            Part::Rotation => rest.rotation_deg,
            Part::Scale => rest.scale,
        };
        let mut values = Vec::new();
        for t in &times {
            let v = Vec3::from_array(std::array::from_fn(|i| {
                keyed[i]
                    .and_then(|k| sample_eased(&k.keys, *t, k.ease))
                    .unwrap_or(base[i])
            }));
            match path {
                Part::Rotation => {
                    let turned = crate::Transform {
                        rotation_deg: v,
                        ..Default::default()
                    };
                    values.extend(turned.rotation().to_array());
                }
                _ => values.extend(v.to_array()),
            }
        }
        out.push(Channel {
            joint,
            path,
            times,
            values,
        });
    }
    out
}

/// Every [`Moving`] line one step on: its graph takes its transitions, its
/// clips move its parts and switch them. In the fixed step, before
/// [`crate::world::apply_hierarchy`].
///
/// A track of another module's property — a sound's volume, particles'
/// rate — is handed to `set`: this module reads the clip, the module that
/// owns the component writes it. The engine's `scrap::motion::run` passes
/// the sound and particle modules' setter.
pub fn run_with(
    world: &mut World,
    dt: f32,
    set: &mut dyn FnMut(&mut World, hecs::Entity, Property, f32),
) {
    let mut places: Vec<(hecs::Entity, PoseTransform)> = Vec::new();
    let mut others: Vec<(hecs::Entity, Property, f32)> = Vec::new();
    for moving in world.query_mut::<&mut Moving>() {
        moving.controller.update(&mut moving.animator);
        let pose = moving.animator.advance(dt);
        places.extend(moving.joints.iter().copied().zip(pose));
        if let Some(playing) = moving.animator.playing() {
            let length = moving
                .animator
                .clips
                .get(playing.clip)
                .map_or(0.0, |c| c.duration);
            let time = if playing.looping && length > 0.0 {
                playing.time.rem_euclid(length)
            } else {
                playing.time.min(length)
            };
            let keyed = moving.others.get(playing.clip);
            for (target, what, value) in &moving.defaults {
                let said = keyed
                    .into_iter()
                    .flatten()
                    .any(|(t, w, _, _)| t == target && w == what);
                if !said {
                    others.push((*target, *what, *value));
                }
            }
            for (target, what, keys, ease) in keyed.into_iter().flatten() {
                let value = if *what == Property::Active {
                    step(keys, time)
                } else {
                    sample_eased(keys, time, *ease)
                };
                if let Some(v) = value {
                    others.push((*target, *what, v));
                }
            }
        }
    }
    for (entity, pose) in places {
        if let Ok(mut t) = world.get::<&mut crate::Transform>(entity) {
            t.position = Vec3::from_array(pose.translation);
            t.set_rotation(Quat::from_array(pose.rotation));
            t.scale = Vec3::from_array(pose.scale);
        }
    }
    for (entity, what, value) in others {
        match what {
            Property::Active => {
                let on = value > 0.5;
                let off_now = world.get::<&crate::world::Inactive>(entity).is_ok();
                if off_now == on {
                    crate::world::set_active(world, entity, on);
                }
            }
            Property::Volume | Property::ParticleRate => set(world, entity, what, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Put a scene into a world with this module dressing it.
    fn spawn(scene: &crate::scene::Scene, world: &mut World) {
        crate::world::spawn_scene_dressed(scene, world, &mut [Box::new(MotionDress)]);
    }

    /// One step, with no sound or particles to hand tracks to.
    fn run(world: &mut World, dt: f32) {
        run_with(world, dt, &mut |_, _, _, _| {});
    }

    #[test]
    fn a_graph_with_no_clips_here_plays_on_the_models_skeleton() {
        use crate::animation::{Joint, Skeleton};
        let scene: crate::scene::Scene = ron::from_str(
            r#"(entities: [
                (id: "0000000000000001", name: "mouse", model: "mouse", animator: "mouse"),
                (id: "0000000000000002", name: "rock", model: "rock", animator: "mouse"),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut motions = Motions::default();
        motions.graphs.insert(
            "mouse".into(),
            ron::from_str(
                r#"(start: "idle", states: {
                    "idle": (clip: "idle", transitions: [(to: "wave", when: [Trigger("wave")])]),
                    "wave": (clip: "wave"),
                })"#,
            )
            .unwrap(),
        );
        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent: None,
                inverse_bind: glam::Mat4::IDENTITY.to_cols_array_2d(),
                rest: PoseTransform::default(),
            }],
        };
        let clip = |name: &str| Clip {
            name: name.into(),
            duration: 1.0,
            channels: Vec::new(),
        };
        let skins = |model: &crate::AssetLink| match model.as_str() {
            "mouse" => Some(crate::asset::MeshSkin {
                skeleton: skeleton.clone(),
                clips: vec![clip("idle")],
                joints: Vec::new(),
                weights: Vec::new(),
            }),
            // A Mixamo file: one clip, named as the file.
            "wave" => Some(crate::asset::MeshSkin {
                skeleton: skeleton.clone(),
                clips: vec![clip("mixamo.com")],
                joints: Vec::new(),
                weights: Vec::new(),
            }),
            _ => None,
        };
        let said = attach(&mut world, &motions, skins);
        assert!(
            said.iter().any(|p| p.contains("`rock` has no skeleton")),
            "{said:?}"
        );
        let mouse = crate::world::addressable(&world)[&EntityId::from_raw(1)];
        let names: Vec<String> = world
            .get::<&Animator>(mouse)
            .unwrap()
            .clips
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, ["idle", "wave"], "the wave from its own file");
        crate::animgraph::run_controllers(&mut world);
        world.get::<&mut Controller>(mouse).unwrap().trigger("wave");
        crate::animgraph::run_controllers(&mut world);
        assert_eq!(
            world.get::<&Controller>(mouse).unwrap().state(),
            Some("wave")
        );
    }

    #[test]
    fn a_full_turn_spins_and_what_one_state_switched_off_the_next_puts_back() {
        // A saw: starting, its sparks off; running, its blade from -360
        // to 0 each second — a whole turn — and nothing said of sparks.
        let scene: crate::scene::Scene = ron::from_str(
            r#"(entities: [
                (id: "0000000000000001", name: "saw", model: "builtin:cube", animator: "saw", children: [
                    (id: "0000000000000002", name: "Blade", model: "builtin:cube"),
                    (id: "0000000000000003", name: "Sparks", model: "builtin:cube"),
                ]),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut motions = Motions::default();
        motions.graphs.insert(
            "saw".into(),
            ron::from_str(
                r#"(start: "start", states: {
                    "start": (clip: "start", transitions: [(to: "on", when: [Trigger("on")])]),
                    "on": (clip: "on"),
                })"#,
            )
            .unwrap(),
        );
        motions.clips.insert(
            "start".into(),
            ron::from_str(r#"(length: 1.0, tracks: [(path: "Sparks", what: Active, keys: [(0.0, 0.0)])])"#).unwrap(),
        );
        motions.clips.insert(
            "on".into(),
            ron::from_str(r#"(length: 1.0, tracks: [(path: "Blade", what: TurnX, keys: [(0.0, -360.0), (1.0, 0.0)])])"#)
                .unwrap(),
        );
        assert!(attach(&mut world, &motions, |_| None).is_empty());
        let ids = crate::world::addressable(&world);
        let (saw, blade, sparks) = (ids[&EntityId::from_raw(1)], ids[&EntityId::from_raw(2)], ids[&EntityId::from_raw(3)]);
        run(&mut world, 0.1);
        assert!(!crate::world::is_active(&world, sparks), "starting: off");
        world.get::<&mut Moving>(saw).unwrap().controller.trigger("on");
        let mut turns = Vec::new();
        for _ in 0..12 {
            run(&mut world, 0.1);
            turns.push(world.get::<&crate::Transform>(blade).unwrap().rotation());
        }
        let most = turns.windows(2).map(|w| w[0].angle_between(w[1])).fold(0.0f32, f32::max);
        assert!(most > 0.5, "a tenth of a second is a tenth of a turn, {most} rad");
        assert!(crate::world::is_active(&world, sparks), "running: back on, as the scene had them");
    }

    #[test]
    fn a_lines_graph_moves_and_switches_what_is_under_it() {
        let scene: crate::scene::Scene = ron::from_str(
            r#"(entities: [
                (id: "0000000000000001", name: "well", model: "builtin:cube", animator: "well", children: [
                    (id: "0000000000000002", name: "Lid", model: "builtin:cube",
                     transform: (position: (0.0, 2.0, 0.0))),
                    (id: "0000000000000003", name: "Glow", model: "builtin:cube", inactive: true),
                ]),
            ])"#,
        )
        .unwrap();
        let mut world = World::new();
        spawn(&scene, &mut world);
        let mut motions = Motions::default();
        motions.graphs.insert(
            "well".into(),
            ron::from_str(
                r#"(start: "shut", states: {
                    "shut": (clip: "", transitions: [(to: "open", when: [Trigger("open")])]),
                    "open": (clip: "open", looping: false),
                })"#,
            )
            .unwrap(),
        );
        motions.clips.insert(
            "open".into(),
            ron::from_str(
                r#"(length: 1.0, tracks: [
                    (path: "Lid", what: TurnX, keys: [(0.0, 0.0), (1.0, -90.0)]),
                    (path: "Glow", what: Active, keys: [(0.0, 0.0), (0.5, 1.0)]),
                    (path: "Chimney", what: Y, keys: [(0.0, 0.0)]),
                ])"#,
            )
            .unwrap(),
        );
        let said = attach(&mut world, &motions, |_| None);
        assert!(said.iter().any(|p| p.contains("Chimney")), "{said:?}");
        let ids = crate::world::addressable(&world);
        let (well, lid, glow) = (
            ids[&EntityId::from_raw(1)],
            ids[&EntityId::from_raw(2)],
            ids[&EntityId::from_raw(3)],
        );
        run(&mut world, 0.1);
        assert!(!crate::world::is_active(&world, glow), "shut: nothing yet");
        world
            .get::<&mut Moving>(well)
            .unwrap()
            .controller
            .trigger("open");
        for _ in 0..20 {
            run(&mut world, 0.1);
        }
        let t = *world.get::<&crate::Transform>(lid).unwrap();
        assert!((t.rotation_deg.x + 90.0).abs() < 0.5, "{t:?}");
        assert_eq!(
            t.position,
            Vec3::new(0.0, 2.0, 0.0),
            "where the scene put it"
        );
        assert!(crate::world::is_active(&world, glow), "lit halfway through");
    }

    #[test]
    fn a_track_on_a_curve_eases_between_its_keys_and_a_straight_one_is_written_as_before() {
        let track: Track =
            ron::from_str(r#"(what: Y, keys: [(0.0, 0.0), (1.0, 10.0)], ease: InQuad)"#).unwrap();
        assert_eq!(sample_eased(&track.keys, 0.5, track.ease), Some(2.5));
        assert_eq!(
            sample(&track.keys, 0.5),
            Some(5.0),
            "straight when not asked"
        );
        let channels = channels_for(0, &[&track], &crate::Transform::default());
        let channel = &channels[0];
        let i = channel
            .times
            .iter()
            .position(|t| *t == 0.5)
            .expect("baked between the keys");
        assert!(
            (channel.values[i * 3 + 1] - 2.5).abs() < 1e-5,
            "{:?}",
            channel.values
        );
        assert_eq!(channel.times.last(), Some(&1.0));

        let plain: Track = ron::from_str(r#"(what: Y, keys: [(0.0, 0.0), (1.0, 10.0)])"#).unwrap();
        assert_eq!(plain.ease, crate::ease::Ease::Linear);
        assert!(
            !ron::to_string(&plain).unwrap().contains("ease"),
            "no new field in old files"
        );
        let channels = channels_for(0, &[&plain], &crate::Transform::default());
        assert_eq!(channels[0].times, [0.0, 1.0], "keys only");
    }
}

/// A line's `animator`, with every thing under it by its path of names.
pub(crate) fn animates(desc: &crate::scene::EntityDesc) -> crate::motion::Animates {
    fn walk(
        desc: &crate::scene::EntityDesc,
        path: &str,
        out: &mut Vec<(String, crate::id::EntityId)>,
    ) {
        for child in &desc.children {
            let at = if path.is_empty() {
                child.name.clone()
            } else {
                format!("{path}/{}", child.name)
            };
            out.push((at.clone(), child.id));
            walk(child, &at, out);
        }
    }
    let mut parts = vec![(String::new(), desc.id)];
    walk(desc, "", &mut parts);
    crate::motion::Animates {
        graph: desc.animator().clone(),
        model: desc.model().clone(),
        parts,
    }
}

/// The animation module's dresser ([`crate::world::Dress`]): the graph a
/// line's `animator` plays, and the bone of its parent's skeleton it rides
/// on.
pub struct MotionDress;

impl crate::world::Dress for MotionDress {
    fn parts(&self) -> &[&'static str] {
        &["animator", "bone", "model", "ik"]
    }

    fn dress(
        &mut self,
        line: &crate::scene::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        changed: crate::world::Changed,
        _: &mut Vec<crate::world::Unresolved>,
    ) {
        if changed.has("animator") {
            let _ = world.remove_one::<Moving>(entity);
            if line.animator().is_empty() {
                let _ = world.remove_one::<Animates>(entity);
            } else {
                let _ = world.insert_one(entity, animates(line));
            }
        }
        // A model that may be skinned: bound to its bones by name once
        // spawned (`animator::bind_skins`).
        if changed.has("model") {
            let _ = world.remove_one::<crate::animator::BoundSkin>(entity);
            let model = line.model();
            if model.is_empty() {
                let _ = world.remove_one::<crate::animator::SkinOf>(entity);
            } else {
                let _ = world.insert_one(entity, crate::animator::SkinOf(model.clone()));
            }
        }
        if changed.has("ik") {
            let _ = world.remove_one::<crate::ik::LookingAt>(entity);
            match line
                .part::<crate::ik::Ik>()
                .filter(|i| *i != crate::ik::Ik::default())
            {
                Some(ik) => {
                    let _ = world.insert_one(entity, ik);
                }
                None => {
                    let _ = world.remove_one::<crate::ik::Ik>(entity);
                }
            }
        }
        if changed.has("bone") {
            let bone = line.bone();
            if bone.is_empty() {
                let _ = world.remove_one::<crate::world::OnBone>(entity);
                let _ = world.remove_one::<crate::world::Between>(entity);
            } else {
                let _ = world.insert_one(entity, crate::world::OnBone(bone));
            }
        }
    }
}

/// `animator: "door"` — the graph in `animators/` that moves it and the
/// things under it, with the clips in `clips/` (see [`crate::motion`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnimatorRef(pub String);

/// `bone: "hand.R"` — held by this joint of the parent's skeleton, not by
/// the parent itself: a spade in a hand, a hat on a head. `transform` is
/// then relative to the bone.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BoneName(pub String);

crate::impl_parts! {
    // A graph in `animators/` by name: a picker of them, not text.
    AnimatorRef => "animator", default if |a| a.0.is_empty(), shape || {
        scrap_core::shape::Shape::Asset("animator".into())
    };
    BoneName => "bone", default if |b| b.0.is_empty();
    // Feet on the ground and a look, on the line's animated skeleton.
    crate::ik::Ik => "ik", default if |i| *i == crate::ik::Ik::default();
}

/// What moves a line of a scene, read off it: its graph, and the bone of
/// its parent's skeleton it rides on.
pub trait AnimationLine {
    fn animator(&self) -> String;
    fn bone(&self) -> String;
    fn set_animator(&mut self, animator: impl Into<String>);
    fn set_bone(&mut self, bone: impl Into<String>);
}

impl AnimationLine for crate::scene::EntityDesc {
    fn animator(&self) -> String {
        self.part::<AnimatorRef>().map(|a| a.0).unwrap_or_default()
    }
    fn bone(&self) -> String {
        self.part::<BoneName>().map(|b| b.0).unwrap_or_default()
    }
    fn set_animator(&mut self, animator: impl Into<String>) {
        self.set_part(&AnimatorRef(animator.into()))
    }
    fn set_bone(&mut self, bone: impl Into<String>) {
        self.set_part(&BoneName(bone.into()))
    }
}

/// Held by a joint of the parent's skeleton, by the joint's name: from a
/// line's `bone`. [`apply_hierarchy`] places it where the parent's pose
/// puts that joint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnBone(pub String);
