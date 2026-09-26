//! Playing clips, and blending between them.
//!
//! [`Clip::sample`] answers "what pose is this animation at this time". An
//! animator is what turns that into something a game can use: a clip that
//! advances on its own, and a crossfade so that changing animation does not
//! snap.
//!
//! The crossfade is not a nicety. Cutting straight from a walk to an idle
//! moves every joint at once, which reads as a glitch rather than as a
//! change of motion, and it is the single thing that separates animation
//! that plays from animation that looks played.

use std::sync::Arc;

use glam::Mat4;
use hecs::World;

use crate::animation::{Clip, PoseTransform, Skeleton};
use crate::world::Posed;

/// One clip in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Playing {
    pub clip: usize,
    pub time: f32,
    pub speed: f32,
    pub looping: bool,
}

/// A skeleton, its clips, and what is currently playing on it.
///
/// The skeleton and clips are shared: a crowd of the same character is one
/// set of animation data and many animators, and cloning a skeleton per
/// entity is how a hundred settlers become a hundred copies of the same
/// matrices.
#[derive(Clone)]
pub struct Animator {
    pub skeleton: Arc<Skeleton>,
    pub clips: Arc<Vec<Clip>>,
    current: Option<Playing>,
    /// What is fading out, if anything.
    previous: Option<Playing>,
    fade_remaining: f32,
    fade_length: f32,
    /// Two clips mixed by a weight, in step: a blend tree's output, in
    /// place of `current` while it is set.
    blend: Option<Blend>,
    /// What is playing started from nothing, and fades in from nothing:
    /// a layer that had no clip. See [`Self::presence`].
    from_empty: bool,
    /// Layers over the pose, in order: each its own clips and crossfade,
    /// laid on the joints its mask names (see [`Layer`]).
    layers: Vec<Layer>,
}

/// How a layer lays its pose on the one below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LayerBlend {
    /// The layer's pose instead of the one below, by the weight: an arm
    /// waving over a walk.
    #[default]
    Override,
    /// The layer's offset from its clip's first frame on top of the pose
    /// below: a breath or a lean over whatever the body does.
    Additive,
}

/// A layer of an animator: an animator of its own on the same skeleton,
/// the joints it moves (a mask, 0..1 each), how and how much.
///
/// Unity's Animator Controller layer with its Avatar Mask: an upper body
/// that aims or waves while the legs below walk.
#[derive(Clone)]
pub struct Layer {
    pub animator: Animator,
    /// Per joint of the skeleton, how much of the layer it takes.
    pub mask: Vec<f32>,
    pub blend: LayerBlend,
    /// The whole layer's weight, 0..1.
    pub weight: f32,
}

/// Each joint's share of a layer from its mask: a name takes that joint
/// and everything under it (`Spine` is the upper body), matched without a
/// rig's prefix, as [`crate::animation::Clip::retarget`] matches. No names
/// is the whole skeleton. Returns the weights and the names no joint has.
pub fn mask_weights(skeleton: &Skeleton, names: &[String]) -> (Vec<f32>, Vec<String>) {
    use crate::animation::bare_joint_name;
    let n = skeleton.joints.len();
    if names.is_empty() {
        return (vec![1.0; n], Vec::new());
    }
    let mut weights = vec![0.0; n];
    let mut unknown = Vec::new();
    for name in names {
        let wanted = bare_joint_name(name);
        let Some(root) = skeleton
            .joints
            .iter()
            .position(|j| j.name == *name || bare_joint_name(&j.name) == wanted)
        else {
            unknown.push(name.clone());
            continue;
        };
        weights[root] = 1.0;
    }
    // Down the tree: a joint under a masked one is masked. Parents come
    // first in a sorted skeleton; an unsorted one goes round until nothing
    // changes.
    loop {
        let mut changed = false;
        for (i, joint) in skeleton.joints.iter().enumerate() {
            if let Some(p) = joint.parent {
                let from = weights.get(p as usize).copied().unwrap_or(0.0);
                if from > weights[i] {
                    weights[i] = from;
                    changed = true;
                }
            }
        }
        if !changed || skeleton.is_sorted() {
            break;
        }
    }
    (weights, unknown)
}

/// Two clips playing in step and mixed: idle and walk at half a metre a
/// second is a quarter of the way between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blend {
    pub a: usize,
    pub b: usize,
    /// 0 is all `a`, 1 all `b`.
    pub weight: f32,
    /// Where both are in their cycle, 0..1: a walk and a run mixed out of
    /// step put the left foot of one on the right foot of the other.
    pub phase: f32,
    pub speed: f32,
    /// A third clip over the mix of the two, and how much of it: a 2D
    /// blend's middle (standing still) under two directions.
    pub third: Option<(usize, f32)>,
}

impl Animator {
    pub fn new(skeleton: Arc<Skeleton>, clips: Arc<Vec<Clip>>) -> Self {
        Self {
            skeleton,
            clips,
            current: None,
            previous: None,
            fade_remaining: 0.0,
            fade_length: 0.0,
            blend: None,
            from_empty: false,
            layers: Vec::new(),
        }
    }

    /// Add a layer over what plays now, masked to the joints `mask` names
    /// ([`mask_weights`]), at full weight, playing nothing yet. An additive
    /// layer plays its clips as offsets from their first frames
    /// ([`crate::animation::Clip::additive`]), made once here. Returns the
    /// layer's index and the mask's names no joint has.
    pub fn add_layer(&mut self, mask: &[String], blend: LayerBlend) -> (usize, Vec<String>) {
        let (weights, unknown) = mask_weights(&self.skeleton, mask);
        let clips = match blend {
            LayerBlend::Override => self.clips.clone(),
            LayerBlend::Additive => Arc::new(
                self.clips
                    .iter()
                    .map(|c| c.additive(&self.skeleton))
                    .collect(),
            ),
        };
        self.layers.push(Layer {
            animator: Animator::new(self.skeleton.clone(), clips),
            mask: weights,
            blend,
            weight: 1.0,
        });
        (self.layers.len() - 1, unknown)
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn layer_mut(&mut self, index: usize) -> Option<&mut Layer> {
        self.layers.get_mut(index)
    }

    /// Stop playing, fading out over `fade` seconds: a layer's empty
    /// state, which gives the joints back to the layer below.
    pub fn stop(&mut self, fade: f32) {
        if self.current.is_none() && self.blend.is_none() {
            return;
        }
        self.previous = self.freeze_blend().or(self.current);
        self.current = None;
        self.blend = None;
        self.from_empty = false;
        self.fade_length = fade.max(0.0);
        self.fade_remaining = self.fade_length;
    }

    /// How much of its pose this animator has to give, 0..1: 1 while a
    /// clip plays, rising from 0 over the fade when it started from
    /// nothing, falling to 0 over the fade after [`Self::stop`]. What a
    /// layer's weight is multiplied by, so a layer going empty lets the
    /// body below come back smoothly rather than snap.
    pub fn presence(&self) -> f32 {
        let fading = self.fade_remaining > 0.0 && self.fade_length > 0.0;
        let t = if fading {
            1.0 - self.fade_remaining / self.fade_length
        } else {
            1.0
        };
        let playing = self.current.is_some() || self.blend.is_some();
        match (playing, self.previous.is_some() && fading) {
            (true, _) if self.from_empty => t,
            (true, _) => 1.0,
            (false, true) => 1.0 - t,
            (false, false) => 0.0,
        }
    }

    /// Clips made on another rig, to play on this one (see
    /// [`Clip::retarget`]): Mixamo's, one clip to a file. A clip Mixamo
    /// named as it names all of them (`mixamo.com`) takes `model`, the
    /// name of the file it came from — as the Unity import names it.
    pub fn take_clips(&mut self, model: &str, from: &crate::animation::Skeleton, clips: &[Clip]) {
        let taken: Vec<Clip> = clips
            .iter()
            .map(|clip| {
                let mut clip = clip.retarget(from, &self.skeleton);
                if clip.name == "mixamo.com" || (clips.len() == 1 && clip.name.is_empty()) {
                    clip.name = model.to_string();
                }
                clip
            })
            .collect();
        std::sync::Arc::make_mut(&mut self.clips).extend(taken);
    }

    pub fn playing(&self) -> Option<Playing> {
        self.current
    }

    /// The clip a graph names `name`: the one of that name, or else one
    /// whose name ends `|name` — Blender writes a model's takes as
    /// `<armature>|<take>`, where Unity names them by the take alone.
    pub fn clip_named(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.name == name).or_else(|| {
            self.clips.iter().position(|c| {
                c.name
                    .strip_suffix(name)
                    .is_some_and(|head| head.ends_with('|'))
            })
        })
    }

    /// Start a clip, fading from whatever was playing.
    ///
    /// Asking for the clip that is already playing does nothing: a game that
    /// calls `play(walk)` every frame while walking would otherwise restart
    /// the walk every frame and stand still with its legs twitching.
    pub fn play(&mut self, clip: usize, fade: f32) {
        if self.blend.is_none() && self.current.map(|p| p.clip) == Some(clip) {
            return;
        }
        self.from_empty = self.current.is_none() && self.blend.is_none();
        self.previous = self.freeze_blend().or(self.current);
        self.blend = None;
        self.fade_length = fade.max(0.0);
        self.fade_remaining = self.fade_length;
        self.current = Some(Playing {
            clip,
            time: 0.0,
            speed: 1.0,
            looping: true,
        });
    }

    /// Play a clip once, holding its last pose.
    pub fn play_once(&mut self, clip: usize, fade: f32) {
        self.play(clip, fade);
        if let Some(playing) = &mut self.current {
            playing.looping = false;
        }
    }

    /// Stand the clip playing at `fraction` of its length, 0..1: a clip
    /// a parameter scrubs instead of time.
    pub fn set_fraction(&mut self, fraction: f32) {
        if let Some(playing) = &mut self.current {
            let length = self.clips.get(playing.clip).map_or(0.0, |c| c.duration);
            playing.time = fraction.clamp(0.0, 1.0) * length;
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        if let Some(playing) = &mut self.current {
            playing.speed = speed;
        }
        if let Some(blend) = &mut self.blend {
            blend.speed = speed;
        }
    }

    /// Mix two clips by `weight`, in step — what a blend tree plays. Called
    /// again while blending, it only moves the clips and the weight on and
    /// keeps the cycle where it is; called from a plain clip, it fades in
    /// over `fade` seconds.
    pub fn blend(&mut self, a: usize, b: usize, weight: f32, fade: f32) {
        self.blend_three(a, b, weight, None, fade);
    }

    /// [`Self::blend`] with a third clip laid over the two by its weight:
    /// what a 2D blend tree plays.
    pub fn blend_three(
        &mut self,
        a: usize,
        b: usize,
        weight: f32,
        third: Option<(usize, f32)>,
        fade: f32,
    ) {
        let weight = weight.clamp(0.0, 1.0);
        let third = third.map(|(c, w)| (c, w.clamp(0.0, 1.0)));
        if let Some(blend) = &mut self.blend {
            (blend.a, blend.b, blend.weight, blend.third) = (a, b, weight, third);
            return;
        }
        self.from_empty = self.current.is_none();
        self.previous = self.current;
        self.fade_length = fade.max(0.0);
        self.fade_remaining = self.fade_length;
        self.current = Some(Playing {
            clip: a,
            time: 0.0,
            speed: 1.0,
            looping: true,
        });
        self.blend = Some(Blend {
            a,
            b,
            weight,
            phase: 0.0,
            speed: 1.0,
            third,
        });
    }

    /// The blend now, if one is playing.
    pub fn blending(&self) -> Option<Blend> {
        self.blend
    }

    /// The heavier clip of a blend at its time, for a fade out of it.
    fn freeze_blend(&self) -> Option<Playing> {
        let blend = self.blend?;
        let clip = if blend.weight < 0.5 { blend.a } else { blend.b };
        let duration = self.clips.get(clip)?.duration;
        Some(Playing {
            clip,
            time: blend.phase * duration,
            speed: blend.speed,
            looping: true,
        })
    }

    /// Whether the clip has played through once — a looping one too, as
    /// Unity's exit time of 1 waits for the end of the first pass: a
    /// drone's flying in that loops on landing still goes on to its next
    /// state.
    pub fn finished(&self) -> bool {
        match (
            self.current,
            self.current.and_then(|p| self.clips.get(p.clip)),
        ) {
            (Some(playing), Some(clip)) => playing.time >= clip.duration,
            _ => false,
        }
    }

    /// Advance by a timestep and return the pose: what plays, then each
    /// layer laid over it on the joints of its mask, in order.
    pub fn advance(&mut self, dt: f32) -> Vec<PoseTransform> {
        let mut pose = self.advance_own(dt);
        for layer in &mut self.layers {
            // A layer keeps its time at no weight, as Unity's does: turned
            // back up, its wave is where it would have been.
            let own = layer.animator.advance(dt);
            let weight = layer.weight.clamp(0.0, 1.0) * layer.animator.presence();
            if weight <= 0.0 {
                continue;
            }
            for (j, slot) in pose.iter_mut().enumerate() {
                let w = weight * layer.mask.get(j).copied().unwrap_or(0.0);
                let (Some(with), true) = (own.get(j), w > 0.0) else {
                    continue;
                };
                *slot = match layer.blend {
                    LayerBlend::Override => slot.lerp(with, w),
                    LayerBlend::Additive => slot.add(with, &self.skeleton.joints[j].rest, w),
                };
            }
        }
        pose
    }

    /// This animator's own clips' pose, its layers not laid on.
    fn advance_own(&mut self, dt: f32) -> Vec<PoseTransform> {
        if let Some(playing) = &mut self.current {
            playing.time += dt * playing.speed;
        }
        if let Some(blend) = &mut self.blend {
            // One cycle takes as long as the mix of the two lengths.
            let length = |c: usize| self.clips.get(c).map_or(1.0, |c| c.duration.max(1e-3));
            let mut cycle = length(blend.a) + (length(blend.b) - length(blend.a)) * blend.weight;
            if let Some((c, w)) = blend.third {
                cycle += (length(c) - cycle) * w;
            }
            blend.phase = (blend.phase + dt * blend.speed / cycle).rem_euclid(1.0);
        }
        if let Some(previous) = &mut self.previous {
            // The outgoing clip keeps running while it fades. Freezing it
            // instead makes the blend cross from a still pose, which looks
            // like a stumble.
            previous.time += dt * previous.speed;
        }
        self.fade_remaining = (self.fade_remaining - dt).max(0.0);
        if self.fade_remaining <= 0.0 {
            self.previous = None;
        }

        let sample = |playing: Playing| -> Option<Vec<PoseTransform>> {
            let clip = self.clips.get(playing.clip)?;
            Some(clip.sample(&self.skeleton, playing.time, playing.looping))
        };

        let current = match self.blend {
            Some(blend) => {
                let at = |c: usize| -> Option<Vec<PoseTransform>> {
                    let clip = self.clips.get(c)?;
                    Some(clip.sample(&self.skeleton, blend.phase * clip.duration, true))
                };
                let two: Option<Vec<PoseTransform>> = match (at(blend.a), at(blend.b)) {
                    (Some(a), Some(b)) => Some(
                        a.iter()
                            .zip(b.iter())
                            .map(|(x, y)| x.lerp(y, blend.weight))
                            .collect(),
                    ),
                    (a, b) => a.or(b),
                };
                match (two, blend.third.and_then(|(c, w)| Some((at(c)?, w)))) {
                    (Some(two), Some((c, w))) => Some(
                        two.iter()
                            .zip(c.iter())
                            .map(|(x, y)| x.lerp(y, w))
                            .collect(),
                    ),
                    (two, _) => two,
                }
            }
            None => self.current.and_then(sample),
        };
        let Some(current) = current else {
            // Stopped: what fades out is held until it is gone; how much
            // of it shows is the presence's to say.
            return self
                .previous
                .and_then(sample)
                .unwrap_or_else(|| self.skeleton.rest_pose());
        };
        let Some(previous) = self.previous.and_then(sample) else {
            return current;
        };

        // 0 just after the fade starts, 1 when it ends.
        let blend = if self.fade_length > 0.0 {
            1.0 - self.fade_remaining / self.fade_length
        } else {
            1.0
        };
        previous
            .iter()
            .zip(current.iter())
            .map(|(from, to)| from.lerp(to, blend))
            .collect()
    }

    /// The pose as skinning matrices, which is what a draw wants.
    pub fn advance_to_matrices(&mut self, dt: f32) -> Vec<Mat4> {
        let pose = self.advance(dt);
        self.skeleton.skinning_matrices(&pose)
    }
}

impl std::fmt::Debug for Animator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Animator")
            .field("joints", &self.skeleton.len())
            .field("clips", &self.clips.len())
            .field("current", &self.current)
            .field("fading", &self.previous.is_some())
            .field("layers", &self.layers.len())
            .finish()
    }
}

/// A line's model, which may be skinned to bones of the scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinOf(pub crate::AssetLink);

/// A skinned model bent by things of the scene: each joint of its skin by
/// the entity of that name near it (a Unity skinned mesh's bones), and
/// where the bone stood when it was bound — the pose the model was made in.
#[derive(Debug, Clone)]
pub struct BoundSkin {
    pub bones: Vec<Option<hecs::Entity>>,
    /// Per joint: the bone's place when bound, undone, then the model's
    /// own place then (its vertices are in its own frame).
    pub unbind: Vec<Mat4>,
}

/// Bind every skinned model not yet bound and not played by an animator
/// of its own to the bones its skin names: the entities of those names
/// under its parent — a Unity model's root, whose frame its vertices are
/// in. A model with no skin, or none of whose joints is found, is left
/// as it is drawn.
pub fn bind_skins(world: &mut World, skins: &dyn Fn(&crate::AssetLink) -> Option<crate::asset::MeshSkin>) {
    use crate::world::{LineName, Parent, WorldTransform};
    let waiting: Vec<(hecs::Entity, crate::AssetLink)> = world
        .query::<(hecs::Entity, &SkinOf)>()
        .without::<&BoundSkin>()
        .without::<&Animator>()
        .iter()
        .map(|(e, s)| (e, s.0.clone()))
        .collect();
    if waiting.is_empty() {
        return;
    }
    let mut children: std::collections::HashMap<hecs::Entity, Vec<hecs::Entity>> = Default::default();
    for (e, p) in world.query::<(hecs::Entity, &Parent)>().iter() {
        children.entry(p.0).or_default().push(e);
    }
    let world_of = |world: &World, e: hecs::Entity| world.get::<&WorldTransform>(e).map(|t| t.0).ok();
    let mut bound = Vec::new();
    let mut gone = Vec::new();
    for (entity, link) in waiting {
        let Some(skin) = skins(&link) else {
            gone.push(entity);
            continue;
        };
        let root = world.get::<&Parent>(entity).map(|p| p.0).unwrap_or(entity);
        // The things under the root, by name; the nearest of a name wins.
        let mut named: std::collections::HashMap<String, hecs::Entity> = Default::default();
        let mut queue = std::collections::VecDeque::from([root]);
        while let Some(e) = queue.pop_front() {
            if let Ok(name) = world.get::<&LineName>(e) {
                named.entry(name.0.clone()).or_insert(e);
            }
            queue.extend(children.get(&e).into_iter().flatten().copied());
        }
        let bones: Vec<Option<hecs::Entity>> = skin
            .skeleton
            .joints
            .iter()
            .map(|j| named.get(&j.name).or_else(|| named.get(crate::animation::bare_joint_name(&j.name))).copied())
            .collect();
        // Its vertices are in its own frame (a piece of a Unity model's
        // are): undone from where each bone stood when bound.
        let root_at = world_of(world, entity).unwrap_or(Mat4::IDENTITY);
        if bones.iter().all(Option::is_none) {
            gone.push(entity);
            continue;
        }
        let unbind = bones
            .iter()
            .map(|b| b.and_then(|b| world_of(world, b)).map_or(Mat4::IDENTITY, |at| at.inverse() * root_at))
            .collect();
        bound.push((entity, BoundSkin { bones, unbind }));
    }
    for entity in gone {
        scrap_core::world::take_off::<SkinOf>(world, entity);
    }
    for (entity, skin) in bound {
        let _ = world.insert_one(entity, skin);
    }
    pose_bound_skins(world);
}

/// Each bound skin posed by its bones as they stand now: a joint moves its
/// vertices by how far its bone has moved since it was bound, in the
/// frame of the entity that draws them.
pub fn pose_bound_skins(world: &mut World) {
    use crate::world::WorldTransform;
    let mut posed: Vec<(hecs::Entity, Vec<Mat4>)> = Vec::new();
    for (entity, skin, at) in world.query::<(hecs::Entity, &BoundSkin, &WorldTransform)>().iter() {
        let into_own = at.0.inverse();
        let matrices = skin
            .bones
            .iter()
            .zip(&skin.unbind)
            .map(|(bone, unbind)| match bone.and_then(|b| world.get::<&WorldTransform>(b).ok().map(|t| t.0)) {
                Some(now) => into_own * now * *unbind,
                None => into_own * at.0,
            })
            .collect();
        posed.push((entity, matrices));
    }
    for (entity, matrices) in posed {
        let _ = world.insert_one(entity, Posed(matrices));
    }
}

/// Advance every animator in the world and write the poses it produces.
///
/// Called from the fixed step, not the frame: an animation that advances by
/// the frame delta plays at a different speed on a faster machine, and two
/// machines replaying the same inputs stop agreeing about where a limb is.
pub fn advance_animations(world: &mut World, dt: f32) {
    advance_animations_on(world, dt, &crate::ik::no_ground);
}

/// [`advance_animations`], each skeleton with an [`crate::ik::Ik`] bent by
/// it between its clips and its skinning, its feet on `ground`.
pub fn advance_animations_on(world: &mut World, dt: f32, ground: crate::ik::Ground) {
    use crate::ik::{Ik, LookingAt};
    use crate::world::WorldTransform;
    // What each look is at, found once and kept.
    let unfound: Vec<(hecs::Entity, crate::id::EntityRef)> = world
        .query::<(hecs::Entity, &Ik)>()
        .without::<&LookingAt>()
        .iter()
        .filter(|(_, ik)| !ik.look_at.is_off() && ik.look_at.target.0.is_some())
        .map(|(e, ik)| (e, ik.look_at.target))
        .collect();
    for (entity, target) in unfound {
        if let Some(found) = target.get(world) {
            let _ = world.insert_one(entity, LookingAt(found));
        }
    }
    let at = |e: hecs::Entity| world.get::<&WorldTransform>(e).ok().map(|t| t.0);
    let looks: scrap_core::hash::FastMap<hecs::Entity, Option<glam::Vec3>> = world
        .query::<(hecs::Entity, &LookingAt)>()
        .iter()
        .map(|(e, l)| (e, at(l.0).map(|m| m.w_axis.truncate())))
        .collect();
    let mut posed: Vec<(hecs::Entity, Vec<Mat4>)> = Vec::new();
    for (entity, animator, ik, placed) in world
        .query::<(
            hecs::Entity,
            &mut Animator,
            Option<&Ik>,
            Option<&WorldTransform>,
        )>()
        .iter()
    {
        let mut pose = animator.advance(dt);
        if let Some(ik) = ik {
            let look = looks.get(&entity).copied().flatten();
            let placed = placed.map_or(Mat4::IDENTITY, |p| p.0);
            crate::ik::solve(&animator.skeleton, &mut pose, placed, ik, look, ground);
        }
        posed.push((entity, animator.skeleton.skinning_matrices(&pose)));
    }
    for (entity, matrices) in posed {
        // In place after the first step: an insert would look up the
        // archetype to move it to, every step, for every skeleton.
        if let Ok(was) = world.query_one_mut::<&mut Posed>(entity) {
            was.0 = matrices;
        } else {
            let _ = world.insert_one(entity, Posed(matrices));
        }
    }
    pose_bound_skins(world);
    hold_on_bones(world);
}

/// What rides on a bone of its parent's skeleton — a spade in a hand, a
/// hat on a head — gets the bone's place as it is posed now, for
/// [`crate::world::apply_hierarchy`] to put between the parent and it.
pub fn hold_on_bones(world: &mut World) {
    use crate::world::{Between, OnBone, Parent};
    let mut held: Vec<(hecs::Entity, Option<Mat4>)> = Vec::new();
    for (entity, bone, parent) in world.query::<(hecs::Entity, &OnBone, &Parent)>().iter() {
        let mut q = world.query_one::<(&Animator, &Posed)>(parent.0);
        let placed = q.get().ok().and_then(|(animator, posed)| {
            let (i, joint) = animator
                .skeleton
                .joints
                .iter()
                .enumerate()
                .find(|(_, j)| j.name == bone.0)?;
            // Skinning is the joint's place times its inverse bind: undo
            // the second to get the first.
            let skinning = posed.0.get(i)?;
            Some(*skinning * Mat4::from_cols_array_2d(&joint.inverse_bind).inverse())
        });
        held.push((entity, placed));
    }
    for (entity, placed) in held {
        match placed {
            Some(bone) => {
                let _ = world.insert_one(entity, Between(bone));
            }
            None => {
                scrap_core::world::take_off::<Between>(world, entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Channel, Joint, Path};

    fn skeleton() -> Arc<Skeleton> {
        Arc::new(Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent: None,
                inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
                rest: PoseTransform::default(),
            }],
        })
    }

    /// Two clips that hold the root at a constant height: 10 and 20.
    fn clips() -> Arc<Vec<Clip>> {
        let at = |name: &str, height: f32| Clip {
            name: name.into(),
            duration: 1.0,
            channels: vec![Channel {
                joint: 0,
                path: Path::Translation,
                times: vec![0.0, 1.0],
                values: vec![0.0, height, 0.0, 0.0, height, 0.0],
            }],
        };
        Arc::new(vec![at("low", 10.0), at("high", 20.0)])
    }

    #[test]
    fn a_looping_clip_has_finished_once_it_has_played_through() {
        let mut a = Animator::new(skeleton(), clips());
        a.play(0, 0.0);
        a.advance(0.5);
        assert!(!a.finished(), "half way");
        a.advance(0.6);
        assert!(a.finished(), "one pass done, looping or not");
    }

    fn animator() -> Animator {
        Animator::new(skeleton(), clips())
    }

    fn height(pose: &[PoseTransform]) -> f32 {
        pose[0].translation[1]
    }

    /// A graph names a model's take as Unity does, by the take; Blender
    /// wrote it with its armature before it: the same clip.
    #[test]
    fn a_clip_is_found_by_its_take_under_blenders_armature_prefix() {
        let mut named = (*clips()).clone();
        named[1].name = "Armature|Armature|Grab|BaseLayer".into();
        let a = Animator::new(skeleton(), Arc::new(named));
        assert_eq!(a.clip_named("low"), Some(0));
        assert_eq!(a.clip_named("Armature|Grab|BaseLayer"), Some(1));
        assert_eq!(a.clip_named("Grab|BaseLayer"), Some(1));
        assert_eq!(a.clip_named("rab|BaseLayer"), None, "whole names only");
        assert_eq!(a.clip_named("Point"), None);
    }

    /// Stopped — Unity's base layer going to a state with no motion — the
    /// pose is held through the fade and then is the rest pose, as Unity's
    /// hand holds its grip through the 0.25 s to its empty Idle and then
    /// opens (HandsAnimOracle); what plays next is there at once.
    #[test]
    fn stopped_the_pose_is_held_through_the_fade_then_rests() {
        let mut a = animator();
        a.play_once(1, 0.0);
        assert_eq!(height(&a.advance(0.1)), 20.0);
        a.stop(0.25);
        assert_eq!(height(&a.advance(0.2)), 20.0, "held while fading");
        assert_eq!(height(&a.advance(0.1)), 0.0, "then at rest");
        a.play(0, 0.25);
        assert_eq!(height(&a.advance(0.05)), 10.0, "from nothing, at once");
    }

    #[test]
    fn clips_from_another_file_play_under_that_files_name() {
        let mut animator = animator();
        let mut mixamo = clips()[1].clone();
        mixamo.name = "mixamo.com".into();
        animator.take_clips("A_Running", &skeleton(), &[mixamo]);
        let at = animator.clips.iter().position(|c| c.name == "A_Running");
        animator.play(at.expect("taken, under the file's name"), 0.0);
        assert_eq!(height(&animator.advance(0.0)), 20.0);
    }

    #[test]
    fn nothing_playing_gives_the_rest_pose() {
        let mut animator = animator();
        assert_eq!(height(&animator.advance(0.1)), 0.0);
    }

    #[test]
    fn a_crossfade_passes_through_the_middle() {
        let mut animator = animator();
        animator.play(0, 0.0);
        assert_eq!(height(&animator.advance(0.0)), 10.0);

        animator.play(1, 1.0);
        // Halfway through the fade, halfway between the two poses.
        assert!(
            (height(&animator.advance(0.5)) - 15.0).abs() < 0.01,
            "got {}",
            height(&animator.advance(0.0))
        );
        // And all the way once it finishes.
        assert!((height(&animator.advance(0.6)) - 20.0).abs() < 0.01);
    }

    #[test]
    fn playing_what_is_already_playing_does_not_restart_it() {
        // A game that calls play(walk) every frame while walking would
        // otherwise stand still with its legs twitching.
        let mut animator = animator();
        animator.play(0, 0.0);
        animator.advance(0.4);
        let before = animator.playing().unwrap().time;
        animator.play(0, 0.2);
        assert_eq!(animator.playing().unwrap().time, before);
    }

    #[test]
    fn a_one_shot_clip_finishes_and_says_so() {
        let mut animator = animator();
        animator.play_once(0, 0.0);
        animator.advance(0.5);
        assert!(!animator.finished());
        animator.advance(0.6);
        assert!(animator.finished(), "past its duration");
        // And holds, rather than snapping back to rest.
        assert_eq!(height(&animator.advance(5.0)), 10.0);
    }

    #[test]
    fn speed_scales_the_clock_without_touching_the_clip() {
        let mut animator = animator();
        animator.play(0, 0.0);
        animator.set_speed(2.0);
        animator.advance(0.25);
        assert!((animator.playing().unwrap().time - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_outgoing_clip_keeps_running_while_it_fades() {
        // Freezing it makes the blend cross from a still pose, which reads
        // as a stumble rather than as a change of motion.
        let mut animator = animator();
        animator.play(0, 0.0);
        animator.advance(0.2);
        animator.play(1, 0.5);
        animator.advance(0.25);
        let outgoing = animator.previous.expect("still fading");
        assert!(
            outgoing.time > 0.2,
            "the old clip should have advanced, sat at {}",
            outgoing.time
        );
    }

    /// Hips at the root, a leg under them, and a spine with an arm under
    /// it; two clips, each holding every joint at its own height: 1 for
    /// "walk", 5 for "wave".
    fn body() -> Animator {
        let joint = |name: &str, parent: Option<u16>| Joint {
            name: format!("mixamorig:{name}"),
            parent,
            inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
            rest: PoseTransform::default(),
        };
        let skeleton = Arc::new(Skeleton {
            joints: vec![
                joint("Hips", None),
                joint("LeftLeg", Some(0)),
                joint("Spine", Some(0)),
                joint("LeftArm", Some(2)),
            ],
        });
        let at = |name: &str, height: f32| Clip {
            name: name.into(),
            duration: 1.0,
            channels: (0..4)
                .map(|j| Channel {
                    joint: j,
                    path: Path::Translation,
                    times: vec![0.0, 1.0],
                    values: vec![0.0, height, 0.0, 0.0, height, 0.0],
                })
                .collect(),
        };
        Animator::new(skeleton, Arc::new(vec![at("walk", 1.0), at("wave", 5.0)]))
    }

    fn heights(pose: &[PoseTransform]) -> Vec<f32> {
        pose.iter().map(|p| p.translation[1]).collect()
    }

    #[test]
    fn a_layer_moves_only_the_joints_under_its_mask() {
        let mut a = body();
        a.play(0, 0.0);
        let (upper, unknown) = a.add_layer(&["Spine".into(), "Tail".into()], LayerBlend::Override);
        assert_eq!(unknown, ["Tail"], "a name no joint has is said");
        a.layer_mut(upper).unwrap().animator.play(1, 0.0);
        assert_eq!(
            heights(&a.advance(0.1)),
            [1.0, 1.0, 5.0, 5.0],
            "the spine and the arm under it wave; the hips and leg walk"
        );
        a.layer_mut(upper).unwrap().weight = 0.5;
        assert_eq!(heights(&a.advance(0.1)), [1.0, 1.0, 3.0, 3.0], "half way");
        a.layer_mut(upper).unwrap().weight = 0.0;
        assert_eq!(heights(&a.advance(0.1)), [1.0; 4], "no weight: all walk");
    }

    #[test]
    fn a_layer_with_nothing_playing_fades_in_and_out_over_the_body() {
        let mut a = body();
        a.play(0, 0.0);
        let (upper, _) = a.add_layer(&[], LayerBlend::Override);
        assert_eq!(
            heights(&a.advance(0.1)),
            [1.0; 4],
            "an empty layer is nothing"
        );
        a.layer_mut(upper).unwrap().animator.play(1, 1.0);
        let half = heights(&a.advance(0.5));
        assert!((half[0] - 3.0).abs() < 1e-4, "half faded in: {half:?}");
        assert_eq!(heights(&a.advance(0.6)), [5.0; 4]);
        a.layer_mut(upper).unwrap().animator.stop(1.0);
        let half = heights(&a.advance(0.5));
        assert!((half[3] - 3.0).abs() < 1e-4, "half faded out: {half:?}");
        assert_eq!(heights(&a.advance(0.6)), [1.0; 4], "the body back");
    }

    #[test]
    fn an_additive_layer_adds_its_move_since_its_first_frame() {
        let mut a = body();
        a.play(0, 0.0);
        // A clip that rises from 5 to 7: an additive layer adds the 2, not
        // the 7.
        let mut clips = (*a.clips).clone();
        for channel in &mut clips[1].channels {
            channel.values[4] = 7.0;
        }
        a.clips = Arc::new(clips);
        let (lean, _) = a.add_layer(&["Spine".into()], LayerBlend::Additive);
        a.layer_mut(lean).unwrap().animator.play(1, 0.0);
        a.layer_mut(lean).unwrap().weight = 0.5;
        let pose = heights(&a.advance(0.0));
        assert_eq!(pose, [1.0; 4], "nothing added at the clip's start");
        let pose = heights(&a.advance(0.5));
        assert!(
            (pose[2] - 1.5).abs() < 1e-4 && (pose[3] - 1.5).abs() < 1e-4,
            "{pose:?}"
        );
        assert_eq!(pose[..2], [1.0, 1.0]);
    }

    #[test]
    fn the_system_writes_a_pose_onto_every_animated_entity() {
        let mut world = World::new();
        let mut playing = animator();
        playing.play(1, 0.0);
        let animated = world.spawn((playing,));
        let still = world.spawn((7u32,));

        advance_animations(&mut world, 0.1);

        assert!(world.get::<&Posed>(animated).is_ok());
        assert!(
            world.get::<&Posed>(still).is_err(),
            "an entity with no animator gets no pose"
        );
        let pose = world.get::<&Posed>(animated).unwrap();
        assert_eq!(pose.0.len(), 1);
        assert!((pose.0[0].w_axis.y - 20.0).abs() < 0.01);
    }
}
