//! Which camera the game looks through, as Cinemachine's brain decides
//! it: the highest `priority` wins, and when another wins the view
//! blends to it over the newcomer's `blend` rather than cutting. Over
//! whatever the view is, a shake — trauma that noise turns into a small
//! turn and nudge and that dies away on its own (docs/feel.md).
//!
//! The brain is one entity's [`CameraBrain`]; [`run_brain`] (LateUpdate,
//! after the follow) moves it on, [`seen`] — what `camera_of` returns —
//! reads it. Blends and shake run on real seconds
//! (`scrap_core::time::unscaled_delta`): slow motion does not stretch a
//! cut, and a hit-stop shakes while the world is still.

use glam::{Quat, Vec3};
use hecs::{Entity, World};

use crate::ease::Ease;
use crate::render::Camera;
use crate::world::{Shown, WorldTransform};
use crate::world_look::{CameraLens, ToTexture};

/// The world's camera brain: which camera is live, the blend into it, the
/// shake. One entity carries it, made by the first [`run_brain`] or
/// [`shake`]. Not saved and not sent: what one player's screen does.
#[derive(Debug, Clone, Default)]
pub struct CameraBrain {
    live: Option<Entity>,
    blend: Option<Blending>,
    /// The view as the last run left it, unshaken: what a blend starts
    /// from when the camera that was live is gone.
    last: Option<Camera>,
    /// The shake over the view: [`shake`] adds to it, and its settings
    /// are here to change.
    pub shake: Shake,
}

#[derive(Debug, Clone, Copy)]
struct Blending {
    from: From,
    elapsed: f32,
    seconds: f32,
    ease: Ease,
}

/// What a blend comes from: the camera that was live, as it moves on —
/// or, when a blend was cut short by another, the view as it stood.
#[derive(Debug, Clone, Copy)]
enum From {
    Camera(Entity),
    Still(Camera),
}

/// Trauma-driven shake ("Juicing Your Cameras With Math", Eiserloh,
/// GDC 2016): each knock adds trauma, the shake is trauma squared — small
/// knocks barely show, big ones rattle — and trauma drains at `recovery`
/// a second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shake {
    /// 0 is still, 1 the most.
    pub trauma: f32,
    /// Trauma lost per real second.
    pub recovery: f32,
    /// Degrees of turn (each way, round each axis) at full trauma.
    pub angle_deg: f32,
    /// Metres of nudge at full trauma.
    pub offset: f32,
    /// How fast it wobbles: noise samples a second.
    pub frequency: f32,
    time: f32,
}

impl Default for Shake {
    fn default() -> Self {
        Self {
            trauma: 0.0,
            recovery: 1.0,
            angle_deg: 4.0,
            offset: 0.15,
            frequency: 15.0,
            time: 0.0,
        }
    }
}

impl Shake {
    /// Knock the camera: more trauma, up to 1.
    pub fn add(&mut self, trauma: f32) {
        self.trauma = (self.trauma + trauma.max(0.0)).min(1.0);
    }

    fn advance(&mut self, seconds: f32) {
        self.time += seconds;
        self.trauma = (self.trauma - self.recovery * seconds).max(0.0);
    }

    /// `camera` shaken as much as the trauma says: the same camera when
    /// there is none.
    pub fn apply(&self, camera: Camera) -> Camera {
        let amount = self.trauma * self.trauma;
        if amount <= 0.0 {
            return camera;
        }
        let t = self.time * self.frequency;
        let wobble = |seed: u32| noise(seed, t) * amount;
        let (right, up, ahead, distance) = basis(&camera);
        let angle = self.angle_deg.to_radians();
        let turn = Quat::from_axis_angle(up, angle * wobble(1))
            * Quat::from_axis_angle(right, angle * wobble(2))
            * Quat::from_axis_angle(ahead, angle * wobble(3));
        let position = camera.position
            + (right * wobble(4) + up * wobble(5) + ahead * wobble(6)) * self.offset;
        Camera {
            position,
            target: position + turn * ahead * distance,
            up: turn * up,
            ..camera
        }
    }
}

/// Smooth noise in -1..1, the same for the same seed and time.
fn noise(seed: u32, t: f32) -> f32 {
    let hash = |i: i32| -> f32 {
        let mut x = (i as u32).wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA6B);
        x ^= x >> 15;
        x = x.wrapping_mul(0x2C1B_3C6D);
        x ^= x >> 12;
        x = x.wrapping_mul(0x297A_2D39);
        x ^= x >> 15;
        x as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let i = t.floor();
    let f = t - i;
    let (a, b) = (hash(i as i32), hash(i as i32 + 1));
    a + (b - a) * f * f * (3.0 - 2.0 * f)
}

/// A camera's right, up and ahead, and how far its target is.
fn basis(camera: &Camera) -> (Vec3, Vec3, Vec3, f32) {
    let to = camera.target - camera.position;
    let distance = to.length().max(1e-3);
    let ahead = (to / distance).normalize_or(Vec3::Z);
    let right = camera.up.cross(ahead).normalize_or(Vec3::X);
    (right, ahead.cross(right), ahead, distance)
}

fn rotation(camera: &Camera) -> Quat {
    let (right, up, ahead, _) = basis(camera);
    Quat::from_mat3(&glam::Mat3::from_cols(right, up, ahead))
}

/// `w` of the way from one camera to another: along a line between where
/// they are, turning the shortest way, the lens and the distance to what
/// they look at in between. Orthographic to perspective has no in
/// between: it is `b`.
pub fn blend(a: &Camera, b: &Camera, w: f32) -> Camera {
    if w >= 1.0 || a.ortho.is_some() != b.ortho.is_some() {
        return *b;
    }
    if w <= 0.0 {
        return *a;
    }
    let lerp = |x: f32, y: f32| x + (y - x) * w;
    let turn = rotation(a).slerp(rotation(b), w);
    let position = a.position.lerp(b.position, w);
    let distance = lerp(a.target.distance(a.position), b.target.distance(b.position));
    Camera {
        position,
        target: position + turn * Vec3::Z * distance.max(1e-3),
        up: turn * Vec3::Y,
        fov_y_degrees: lerp(a.fov_y_degrees, b.fov_y_degrees),
        near: lerp(a.near, b.near),
        far: lerp(a.far, b.far),
        ortho: a.ortho.zip(b.ortho).map(|(x, y)| lerp(x, y)),
        clip: None,
    }
}

/// What the camera on an entity sees: from where it is, along its +z.
pub(crate) fn lens_camera(lens: crate::scene::Lens, placed: glam::Mat4) -> Camera {
    let (_, rotation, position) = placed.to_scale_rotation_translation();
    Camera {
        position,
        target: position + rotation * Vec3::Z,
        up: rotation * Vec3::Y,
        fov_y_degrees: lens.fov_deg,
        ortho: lens.ortho,
        ..Camera::default()
    }
}

/// The camera an entity carries, as it stands.
fn camera_at(world: &World, entity: Entity) -> Option<Camera> {
    let lens = world.get::<&CameraLens>(entity).ok()?.0;
    let placed = world.get::<&WorldTransform>(entity).ok()?;
    let shown = world.get::<&Shown>(entity).ok();
    Some(lens_camera(
        lens,
        crate::world::drawn_at(&placed, shown.as_deref()),
    ))
}

/// The camera that should be live: the highest priority (the lowest id
/// among equals, so the answer does not change between runs), not one
/// that draws into a picture.
pub fn chosen(world: &World) -> Option<(Entity, Camera, crate::scene::Lens)> {
    /// Higher wins: the priority, then the lower id.
    type Rank = (i32, std::cmp::Reverse<crate::id::EntityId>);
    let mut best: Option<(Rank, Entity, Camera, crate::scene::Lens)> = None;
    for (entity, lens, placed, shown, id) in world
        .query::<(
            Entity,
            &CameraLens,
            &WorldTransform,
            Option<&Shown>,
            Option<&crate::world::SceneId>,
        )>()
        .without::<&ToTexture>()
        .iter()
    {
        let key = (
            lens.0.priority,
            std::cmp::Reverse(id.map(|i| i.0).unwrap_or_default()),
        );
        if best.as_ref().is_none_or(|(k, ..)| key > *k) {
            let camera = lens_camera(lens.0, crate::world::drawn_at(placed, shown));
            best = Some((key, entity, camera, lens.0));
        }
    }
    best.map(|(_, entity, camera, lens)| (entity, camera, lens))
}

fn brain_of(world: &World) -> Option<CameraBrain> {
    world.query::<&CameraBrain>().iter().next().cloned()
}

/// The world's brain, made if there is none.
pub fn brain(world: &mut World) -> hecs::RefMut<'_, CameraBrain> {
    let found = world
        .query::<(Entity, &CameraBrain)>()
        .iter()
        .next()
        .map(|(e, _)| e);
    let entity = found.unwrap_or_else(|| world.spawn((CameraBrain::default(),)));
    world
        .get::<&mut CameraBrain>(entity)
        .expect("just found or made")
}

/// The view through `live` as the brain has it, before the shake: part
/// way from what came before while it blends.
fn unshaken(world: &World, brain: &CameraBrain, live: Camera) -> Camera {
    let Some(blending) = brain.blend else {
        return live;
    };
    let from = match blending.from {
        From::Camera(entity) => camera_at(world, entity),
        From::Still(camera) => Some(camera),
    };
    match from {
        Some(from) => blend(
            &from,
            &live,
            blending
                .ease
                .at(blending.elapsed / blending.seconds.max(1e-6)),
        ),
        None => live,
    }
}

/// What the world's camera sees: the live camera, blended and shaken as
/// the brain says. `None` when no entity has a camera.
pub fn seen(world: &World) -> Option<Camera> {
    let (entity, live, _) = chosen(world)?;
    let Some(brain) = brain_of(world) else {
        return Some(live);
    };
    let view = if brain.live == Some(entity) {
        unshaken(world, &brain, live)
    } else {
        // Chosen since the brain last ran: the blend starts when it does.
        live
    };
    Some(brain.shake.apply(view))
}

/// How far into a blend the view is, 0 to 1; `None` when it is not
/// blending.
pub fn blending(world: &World) -> Option<f32> {
    let blend = brain_of(world)?.blend?;
    Some((blend.elapsed / blend.seconds.max(1e-6)).min(1.0))
}

/// The brain one frame on: a camera newly on top starts a blend to it
/// (or a cut, with no `blend`, from nothing, or between orthographic and
/// perspective), a blend moves on, the shake settles. Real seconds when
/// the loop leaves them in the world, `dt` otherwise.
pub fn run_brain(world: &mut World, dt: f32) {
    let real = scrap_core::time::unscaled_delta(world).unwrap_or(dt);
    let before = brain_of(world);
    if before.is_none() && chosen(world).is_none() {
        return;
    }
    let Some((entity, live, lens)) = chosen(world) else {
        brain(world).shake.advance(real);
        return;
    };
    // The view as it stood with the camera that was live — or as the last
    // run left it, when that camera is gone: where a blend starts from.
    let prev_there = before
        .as_ref()
        .and_then(|b| b.live)
        .is_some_and(|prev| camera_at(world, prev).is_some());
    let was = before.as_ref().and_then(|b| {
        let prev = b.live?;
        if prev == entity {
            return None;
        }
        camera_at(world, prev)
            .map(|c| unshaken(world, b, c))
            .or(b.last)
    });
    let mut brain = brain(world);
    brain.shake.advance(real);
    match brain.live {
        Some(prev) if prev != entity => {
            brain.blend = match (lens.blend, was) {
                (Some(b), Some(was))
                    if b.seconds > 0.0 && was.ortho.is_some() == live.ortho.is_some() =>
                {
                    Some(Blending {
                        from: if brain.blend.is_some() || !prev_there {
                            From::Still(was)
                        } else {
                            From::Camera(prev)
                        },
                        elapsed: 0.0,
                        seconds: b.seconds,
                        ease: b.ease,
                    })
                }
                _ => None,
            };
        }
        Some(_) => {
            if let Some(blend) = brain.blend.as_mut() {
                blend.elapsed += real;
                if blend.elapsed >= blend.seconds {
                    brain.blend = None;
                }
            }
        }
        None => brain.blend = None,
    }
    brain.live = Some(entity);
    let settled = brain.clone();
    drop(brain);
    let last = unshaken(world, &settled, live);
    self::brain(world).last = Some(last);
}

/// Knock the game's camera: `trauma` from 0.1 (a footstep of something
/// big) to 1 (an explosion in the face), added to what is there.
pub fn shake(world: &mut World, trauma: f32) {
    brain(world).shake.add(trauma);
}

/// Knock the camera from something at `at`: all of `trauma` at it,
/// fading to nothing `radius` metres away — Cinemachine's Impulse Source.
pub fn shake_at(world: &mut World, at: Vec3, trauma: f32, radius: f32) {
    let Some((_, camera, _)) = chosen(world) else {
        return;
    };
    let near = 1.0 - camera.position.distance(at) / radius.max(1e-3);
    if near > 0.0 {
        shake(world, trauma * near);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Blend, Lens};

    fn lens(priority: i32, blend: Option<Blend>) -> Lens {
        Lens {
            fov_deg: 60.0,
            priority,
            ortho: None,
            follow: None,
            blend,
        }
    }

    fn at(x: f32) -> WorldTransform {
        WorldTransform(glam::Mat4::from_translation(Vec3::new(x, 0.0, 0.0)))
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn a_camera_taking_over_blends_in_over_its_seconds_on_its_curve() {
        let mut world = World::new();
        let blend = Some(Blend {
            seconds: 1.0,
            ease: Ease::Linear,
        });
        world.spawn((CameraLens(lens(0, None)), at(0.0)));
        let second = world.spawn((CameraLens(lens(-1, blend)), at(10.0)));
        run_brain(&mut world, 0.1);
        assert!(
            close(seen(&world).unwrap().position, Vec3::ZERO),
            "the first is live"
        );
        assert_eq!(blending(&world), None, "nothing to blend from: a cut");

        world.get::<&mut CameraLens>(second).unwrap().0.priority = 1;
        run_brain(&mut world, 0.1);
        assert_eq!(blending(&world), Some(0.0));
        assert!(
            close(seen(&world).unwrap().position, Vec3::ZERO),
            "starts where it was"
        );
        for _ in 0..4 {
            run_brain(&mut world, 0.1);
        }
        let half = seen(&world).unwrap().position.x;
        assert!((half - 4.0).abs() < 1e-3, "four tenths of the way: {half}");
        for _ in 0..6 {
            run_brain(&mut world, 0.1);
        }
        assert_eq!(blending(&world), None, "done");
        assert!(close(
            seen(&world).unwrap().position,
            Vec3::new(10.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn a_blend_runs_on_real_time_not_the_worlds() {
        let mut world = World::new();
        let blend = Some(Blend {
            seconds: 0.5,
            ease: Ease::InOutCubic,
        });
        world.spawn((CameraLens(lens(0, None)), at(0.0)));
        run_brain(&mut world, 0.1);
        world.spawn((CameraLens(lens(1, blend)), at(10.0)));
        // A hit-stop: the world's delta is 0, the real one is not.
        let mut time = scrap_core::Time::default();
        time.hit_stop(1.0);
        for _ in 0..6 {
            time.advance(0.1);
            scrap_core::time::sync(&mut world, &time);
            run_brain(&mut world, time.delta());
        }
        assert_eq!(time.delta(), 0.0);
        assert_eq!(blending(&world), None, "blended in half a real second");
        assert!(close(
            seen(&world).unwrap().position,
            Vec3::new(10.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn a_blend_cut_short_starts_again_from_where_the_view_was() {
        let mut world = World::new();
        let blend = Some(Blend {
            seconds: 1.0,
            ease: Ease::Linear,
        });
        world.spawn((CameraLens(lens(0, None)), at(0.0)));
        run_brain(&mut world, 0.1);
        let b = world.spawn((CameraLens(lens(1, blend)), at(10.0)));
        run_brain(&mut world, 0.1);
        for _ in 0..5 {
            run_brain(&mut world, 0.1);
        }
        let midway = seen(&world).unwrap().position;
        let c = world.spawn((CameraLens(lens(2, blend)), at(-10.0)));
        run_brain(&mut world, 0.1);
        let after = seen(&world).unwrap().position;
        assert!(close(after, midway), "no jump: {after} after {midway}");
        run_brain(&mut world, 0.5);
        let on_the_way = seen(&world).unwrap().position;
        assert!(
            on_the_way.x < midway.x - 1.0,
            "and on to the third: {on_the_way}"
        );

        // The live camera gone mid-blend: from the view as it was.
        let _ = world.despawn(c);
        let _ = world.despawn(b);
        world.spawn((CameraLens(lens(3, blend)), at(20.0)));
        run_brain(&mut world, 0.1);
        let after = seen(&world).unwrap().position;
        assert!(
            close(after, on_the_way),
            "no jump: {after} after {on_the_way}"
        );
    }

    #[test]
    fn orthographic_to_perspective_cuts() {
        let mut world = World::new();
        world.spawn((
            CameraLens(Lens {
                ortho: Some(5.0),
                ..lens(0, None)
            }),
            at(0.0),
        ));
        run_brain(&mut world, 0.1);
        world.spawn((
            CameraLens(lens(
                1,
                Some(Blend {
                    seconds: 1.0,
                    ease: Ease::Linear,
                }),
            )),
            at(10.0),
        ));
        run_brain(&mut world, 0.1);
        assert_eq!(blending(&world), None);
        assert!(close(
            seen(&world).unwrap().position,
            Vec3::new(10.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn a_blend_turns_the_shortest_way_and_eases_the_lens() {
        let a = Camera {
            position: Vec3::ZERO,
            target: Vec3::Z,
            fov_y_degrees: 40.0,
            ..Camera::default()
        };
        let b = Camera {
            position: Vec3::ZERO,
            target: Vec3::X * 4.0,
            fov_y_degrees: 80.0,
            ..Camera::default()
        };
        let mid = blend(&a, &b, 0.5);
        let ahead = (mid.target - mid.position).normalize();
        assert!(
            close(ahead, Vec3::new(1.0, 0.0, 1.0).normalize()),
            "{ahead}"
        );
        assert!((mid.target.length() - 2.5).abs() < 1e-4);
        assert_eq!(mid.fov_y_degrees, 60.0);
        assert!(close(mid.up, Vec3::Y));
        assert_eq!(blend(&a, &b, 1.0), b);
        assert_eq!(blend(&a, &b, 0.0), a);
    }

    #[test]
    fn a_shake_rattles_the_view_and_dies_away_to_nothing() {
        let mut world = World::new();
        world.spawn((CameraLens(lens(0, None)), at(0.0)));
        run_brain(&mut world, 0.1);
        let still = seen(&world).unwrap();
        shake(&mut world, 0.8);
        run_brain(&mut world, 1.0 / 60.0);
        let shaken = seen(&world).unwrap();
        assert!(
            shaken.position != still.position || shaken.target != still.target,
            "it moves"
        );
        let angle = (shaken.target - shaken.position)
            .normalize()
            .angle_between(Vec3::Z)
            .to_degrees();
        assert!(
            angle < 4.0 * 3f32.sqrt(),
            "within the shake's reach: {angle}°"
        );
        let mut last = f32::MAX;
        for _ in 0..60 {
            run_brain(&mut world, 1.0 / 60.0);
            let trauma = brain(&mut world).shake.trauma;
            assert!(trauma <= last, "only ever settles");
            last = trauma;
        }
        assert_eq!(last, 0.0, "a second at recovery 1 drains 0.8");
        assert_eq!(
            seen(&world).unwrap(),
            still,
            "and leaves the view as it was"
        );

        shake(&mut world, 5.0);
        assert_eq!(brain(&mut world).shake.trauma, 1.0, "never more than all");
    }

    #[test]
    fn a_knock_far_away_shakes_less_and_beyond_its_radius_not_at_all() {
        let mut world = World::new();
        world.spawn((CameraLens(lens(0, None)), at(0.0)));
        shake_at(&mut world, Vec3::new(5.0, 0.0, 0.0), 1.0, 10.0);
        assert!((brain(&mut world).shake.trauma - 0.5).abs() < 1e-5);
        shake_at(&mut world, Vec3::new(50.0, 0.0, 0.0), 1.0, 10.0);
        assert!((brain(&mut world).shake.trauma - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_noise_is_smooth_and_the_same_every_run() {
        assert_eq!(noise(3, 1.25), noise(3, 1.25));
        assert_ne!(noise(3, 1.25), noise(4, 1.25));
        for i in 0..1000 {
            let t = i as f32 * 0.01;
            assert!(
                (noise(1, t) - noise(1, t + 0.01)).abs() < 0.1,
                "no jumps at {t}"
            );
            assert!(noise(1, t).abs() <= 1.0);
        }
    }
}
