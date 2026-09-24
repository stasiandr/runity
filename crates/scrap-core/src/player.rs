//! The player, as far as the engine can know it (docs/player.md).
//!
//! The engine has no character controller: how a person walks is the
//! game's design (DNA, postulate 8). What it can know is written down by
//! the game in two places:
//!
//! * **How big the player is and how far it jumps** — [`PlayerMetrics`],
//!   `game: (player: (…))` in `scrap.ron`. The editor draws it as a
//!   reference in the Scene view, navigation bakes for it, and the game's
//!   controller reads the same numbers, so a door and a jump are measured
//!   against one player.
//! * **Which entity starts as the player** — `player_start: true` on a
//!   line ([`PlayerStart`]): the player's own line, or an empty the game
//!   spawns its player at. Unreal's Player Start. Several lines marked are
//!   one group: several players' places, in the order the file has them.
//!
//! Play from Here is the two together: the editor names a place
//! ([`Start`], in `SCRAP_START`), and the game, after spawning its scene,
//! moves what is marked there ([`place_at_start`]).

use glam::{Mat4, Quat, Vec3};
use hecs::World;
use serde::{Deserialize, Serialize};

use crate::id::EntityId;
use crate::scene::{Scene, Transform};
use crate::world::{apply_hierarchy, Parent, SceneId, WorldTransform};

/// How big the player is and how it moves, in metres, degrees and seconds:
/// what a level is built against. Level designers call these metrics —
/// door heights, gaps and ledges are sized from them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerMetrics {
    /// From the feet to the top of the head.
    pub height: f32,
    /// Half the player's width: how far from walls its middle keeps.
    pub radius: f32,
    /// The highest step it walks up without jumping.
    pub step: f32,
    /// Steeper than this, in degrees, is a wall.
    pub slope: f32,
    /// How high its feet go at the top of a jump.
    pub jump_height: f32,
    /// Running speed, metres a second: how far a jump carries.
    pub speed: f32,
    /// What pulls it down in a jump, metres a second squared. The physics
    /// module's default; a controller with a heavier fall says so here.
    pub gravity: f32,
}

impl Default for PlayerMetrics {
    fn default() -> Self {
        Self {
            height: 1.8,
            radius: 0.35,
            step: 0.3,
            slope: 40.0,
            jump_height: 1.2,
            speed: 5.0,
            gravity: 9.81,
        }
    }
}

impl PlayerMetrics {
    /// How fast it leaves the ground to reach [`Self::jump_height`].
    pub fn jump_speed(&self) -> f32 {
        (2.0 * self.gravity.max(0.01) * self.jump_height.max(0.0)).sqrt()
    }

    /// Seconds from leaving the ground to landing on the same height.
    pub fn airtime(&self) -> f32 {
        2.0 * self.jump_speed() / self.gravity.max(0.01)
    }

    /// How far a running jump carries, landing at the height it left.
    pub fn jump_reach(&self) -> f32 {
        self.speed.max(0.0) * self.airtime()
    }

    /// Where the feet go in a running jump from `feet` towards `facing`
    /// (level): `segments + 1` points, the first `feet`, the last back at
    /// its height [`Self::jump_reach`] ahead.
    pub fn jump_arc(&self, feet: Vec3, facing: Vec3, segments: usize) -> Vec<Vec3> {
        let facing = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::NEG_Z);
        let (up, g, time) = (self.jump_speed(), self.gravity.max(0.01), self.airtime());
        let segments = segments.max(1);
        (0..=segments)
            .map(|i| {
                let t = time * i as f32 / segments as f32;
                feet + facing * self.speed.max(0.0) * t + Vec3::Y * (up * t - 0.5 * g * t * t)
            })
            .collect()
    }
}

/// `player_start: true` — where the player begins: on the player's own
/// line, or on an empty the game spawns its player at. What Play from Here
/// moves ([`place_at_start`]).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerStart(pub bool);

crate::impl_parts! {
    PlayerStart => "player_start", default if |p| !p.0;
}

/// The variable the editor names a start in, for a game Play from Here
/// starts: [`Start`] as RON.
pub const START_VAR: &str = "SCRAP_START";

/// A place to start from: where the feet are, and which way the player
/// looks — degrees about the vertical, 0 along −z, as a camera looks and
/// as `rotation_deg.y` turns a line.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Start {
    pub position: Vec3,
    pub yaw_deg: f32,
}

impl Start {
    /// Standing at `position`, looking along `facing` (its level part).
    pub fn looking(position: Vec3, facing: Vec3) -> Self {
        Self {
            position,
            yaw_deg: yaw_of(facing).to_degrees(),
        }
    }

    /// The level direction it looks in.
    pub fn facing(&self) -> Vec3 {
        Quat::from_rotation_y(self.yaw_deg.to_radians()) * Vec3::NEG_Z
    }

    /// As `SCRAP_START` carries it.
    pub fn to_env(&self) -> String {
        ron::to_string(self).expect("a start serializes")
    }

    /// The start the editor gave this game, if it gave one. A value that
    /// does not read is an error in words, not a start at the origin.
    pub fn from_env() -> Result<Option<Self>, String> {
        match std::env::var(START_VAR) {
            Ok(text) => ron::from_str(&text)
                .map(Some)
                .map_err(|e| format!("{START_VAR}={text}: {e}")),
            Err(_) => Ok(None),
        }
    }
}

/// Radians about the vertical that turn −z to look along `facing`.
fn yaw_of(facing: Vec3) -> f32 {
    if facing.x.abs() + facing.z.abs() < 1e-6 {
        return 0.0;
    }
    (-facing.x).atan2(-facing.z)
}

/// The lines of `scene` marked `player_start`, children and prefab parts
/// included, in the file's order.
pub fn player_starts(scene: &Scene) -> Vec<EntityId> {
    scene
        .flatten()
        .into_iter()
        .filter(|(desc, _)| desc.part::<PlayerStart>().is_some_and(|p| p.0))
        .map(|(desc, _)| desc.id)
        .collect()
}

/// Move what `scene` marks `player_start` to `start`, in a world spawned
/// from it: the first marked line stands at the start looking its way,
/// and the others keep their places around it, turned with it. Returns the
/// lines moved; none when nothing is marked, and the world is as it was.
///
/// Called after the scene is spawned and before the game reads where its
/// player is: an empty the game spawns its player at has moved by then,
/// and so has a player that is a line of the scene.
pub fn place_at_start(world: &mut World, scene: &Scene, start: Start) -> Vec<EntityId> {
    let marked = player_starts(scene);
    let entities: std::collections::HashMap<EntityId, hecs::Entity> = world
        .query::<(hecs::Entity, &SceneId)>()
        .iter()
        .map(|(entity, id)| (id.0, entity))
        .collect();
    let placed: Vec<(EntityId, hecs::Entity, Mat4)> = marked
        .into_iter()
        .filter_map(|id| {
            let entity = *entities.get(&id)?;
            let at = world.get::<&WorldTransform>(entity).ok()?.0;
            Some((id, entity, at))
        })
        .collect();
    let Some((_, _, first)) = placed.first() else {
        return Vec::new();
    };
    let (_, first_turn, first_at) = first.to_scale_rotation_translation();
    let turn = Quat::from_rotation_y(start.yaw_deg.to_radians() - yaw_of(first_turn * Vec3::NEG_Z));
    let mut moved = Vec::new();
    for (id, entity, at) in placed {
        let (scale, rotation, position) = at.to_scale_rotation_translation();
        let now = Mat4::from_scale_rotation_translation(
            scale,
            turn * rotation,
            start.position + turn * (position - first_at),
        );
        let parent = world
            .get::<&Parent>(entity)
            .ok()
            .and_then(|p| world.get::<&WorldTransform>(p.0).ok().map(|w| w.0))
            .unwrap_or(Mat4::IDENTITY);
        let (scale, rotation, position) = (parent.inverse() * now).to_scale_rotation_translation();
        if let Ok(mut local) = world.get::<&mut Transform>(entity) {
            local.position = position;
            local.set_rotation(rotation);
            local.scale = scale;
            moved.push(id);
        }
    }
    apply_hierarchy(world);
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::EntityDesc;

    #[test]
    fn a_jump_goes_as_high_as_the_metrics_say_and_lands_as_far_as_the_speed_carries() {
        let m = PlayerMetrics::default();
        let arc = m.jump_arc(Vec3::ZERO, Vec3::X, 32);
        let top = arc.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!((top - m.jump_height).abs() < 0.01, "{top}");
        let last = *arc.last().unwrap();
        assert!(
            last.y.abs() < 1e-4 && (last.x - m.jump_reach()).abs() < 1e-4,
            "{last}"
        );
        // 1.2 m at 9.81 is about a second in the air: five metres at 5 m/s.
        assert!((m.jump_reach() - 4.95).abs() < 0.05, "{}", m.jump_reach());
    }

    #[test]
    fn metrics_left_out_of_scrap_ron_are_the_defaults() {
        let m: PlayerMetrics = ron::from_str("(height: 2.0)").unwrap();
        assert_eq!(m.height, 2.0);
        assert_eq!(m.radius, PlayerMetrics::default().radius);
    }

    #[test]
    fn a_start_goes_through_its_variable_and_looks_where_it_was_told() {
        let start = Start::looking(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, -0.5, 0.0));
        let back: Start = ron::from_str(&start.to_env()).unwrap();
        assert_eq!(back, start);
        assert!(back.facing().distance(Vec3::X) < 1e-5, "{}", back.facing());
    }

    #[test]
    fn the_marked_lines_move_to_the_start_together_and_nothing_else_does() {
        let line = |name: &str, x: f32, marked: bool| {
            let mut desc = EntityDesc {
                id: crate::id::EntityId::fresh(),
                name: name.into(),
                ..Default::default()
            };
            desc.transform.position = Vec3::new(x, 0.0, 0.0);
            if marked {
                desc.set_part(&PlayerStart(true));
            }
            desc
        };
        let scene = Scene {
            entities: vec![
                line("tree", 9.0, false),
                line("one", 0.0, true),
                line("two", 2.0, true),
            ],
            ..Default::default()
        };
        let mut world = World::new();
        crate::world::spawn_scene_dressed(&scene, &mut world, &mut []);
        // Turned a quarter to the left: −z becomes −x, and +x (where two
        // stood from one) becomes −z.
        let start = Start::looking(Vec3::new(10.0, 1.0, 10.0), Vec3::NEG_X);
        let moved = place_at_start(&mut world, &scene, start);
        assert_eq!(moved.len(), 2);
        let at = |name: &str| {
            let id = scene.find(name).unwrap().id;
            let (_, placed) = world
                .query::<(&SceneId, &WorldTransform)>()
                .iter()
                .find(|(s, _)| s.0 == id)
                .map(|(s, w)| (s.0, w.0))
                .unwrap();
            placed.w_axis.truncate()
        };
        assert!(
            at("one").distance(Vec3::new(10.0, 1.0, 10.0)) < 1e-4,
            "{}",
            at("one")
        );
        assert!(
            at("two").distance(Vec3::new(10.0, 1.0, 8.0)) < 1e-4,
            "{}",
            at("two")
        );
        assert!(at("tree").distance(Vec3::new(9.0, 0.0, 0.0)) < 1e-4);
        // Nothing marked: nothing moves.
        let bare = Scene {
            entities: vec![line("tree", 9.0, false)],
            ..Default::default()
        };
        let mut world = World::new();
        crate::world::spawn_scene_dressed(&bare, &mut world, &mut []);
        assert!(place_at_start(&mut world, &bare, start).is_empty());
    }
}
