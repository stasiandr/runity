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
        }
    }

    pub fn playing(&self) -> Option<Playing> {
        self.current
    }

    /// Start a clip, fading from whatever was playing.
    ///
    /// Asking for the clip that is already playing does nothing: a game that
    /// calls `play(walk)` every frame while walking would otherwise restart
    /// the walk every frame and stand still with its legs twitching.
    pub fn play(&mut self, clip: usize, fade: f32) {
        if self.current.map(|p| p.clip) == Some(clip) {
            return;
        }
        self.previous = self.current;
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

    pub fn set_speed(&mut self, speed: f32) {
        if let Some(playing) = &mut self.current {
            playing.speed = speed;
        }
    }

    /// Whether a non-looping clip has run out.
    pub fn finished(&self) -> bool {
        match (
            self.current,
            self.current.and_then(|p| self.clips.get(p.clip)),
        ) {
            (Some(playing), Some(clip)) => !playing.looping && playing.time >= clip.duration,
            _ => false,
        }
    }

    /// Advance by a timestep and return the pose.
    pub fn advance(&mut self, dt: f32) -> Vec<PoseTransform> {
        if let Some(playing) = &mut self.current {
            playing.time += dt * playing.speed;
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

        let Some(current) = self.current.and_then(sample) else {
            return self.skeleton.rest_pose();
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
            .finish()
    }
}

/// Advance every animator in the world and write the poses it produces.
///
/// Called from the fixed step, not the frame: an animation that advances by
/// the frame delta plays at a different speed on a faster machine, and two
/// machines replaying the same inputs stop agreeing about where a limb is.
pub fn advance_animations(world: &mut World, dt: f32) {
    let mut posed: Vec<(hecs::Entity, Vec<Mat4>)> = Vec::new();
    for (entity, animator) in world.query::<(hecs::Entity, &mut Animator)>().iter() {
        posed.push((entity, animator.advance_to_matrices(dt)));
    }
    for (entity, matrices) in posed {
        let _ = world.insert_one(entity, Posed(matrices));
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

    fn animator() -> Animator {
        Animator::new(skeleton(), clips())
    }

    fn height(pose: &[PoseTransform]) -> f32 {
        pose[0].translation[1]
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
