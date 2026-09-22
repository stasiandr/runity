//! The scene file: what the editor writes and the game reads.
//!
//! Scenes are RON, not code. That is the whole point of the split — a mouse
//! drag in the editor and a change made by an agent produce the same kind of
//! diff, and `git` can show either one. Nothing here knows about rendering or
//! physics; it is the description they are both built from.

use std::path::Path;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Position, rotation and scale, in the form a person can edit.
///
/// Rotation is stored as Euler degrees rather than a quaternion on purpose:
/// a quaternion in a text file is unreadable and uneditable, and the editor
/// converts on the way in and out. Order is Y (yaw), X (pitch), Z (roll),
/// which is what a turntable-style gizmo produces.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    #[serde(default = "zero")]
    pub position: Vec3,
    #[serde(default = "zero")]
    pub rotation_deg: Vec3,
    #[serde(default = "one")]
    pub scale: Vec3,
}

// `serde(default = "...")` names a function, and glam's consts are not one.
fn zero() -> Vec3 {
    Vec3::ZERO
}
fn one() -> Vec3 {
    Vec3::ONE
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation_deg: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn rotation(&self) -> Quat {
        let r = self.rotation_deg * std::f32::consts::PI / 180.0;
        Quat::from_euler(glam::EulerRot::YXZ, r.y, r.x, r.z)
    }

    pub fn set_rotation(&mut self, q: Quat) {
        let (y, x, z) = q.to_euler(glam::EulerRot::YXZ);
        self.rotation_deg = Vec3::new(x, y, z) * 180.0 / std::f32::consts::PI;
    }

    pub fn matrix(&self) -> glam::Mat4 {
        glam::Mat4::from_scale_rotation_translation(self.scale, self.rotation(), self.position)
    }
}

/// How an entity takes part in the physics world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Body {
    /// Drawn only; rapier never sees it.
    #[default]
    None,
    /// Never moves, but everything collides with it — terrain, trunks, walls.
    Static,
    /// Moved by the solver.
    Dynamic,
}

/// One thing in the valley.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityDesc {
    /// Shown in the editor's tree; not required to be unique.
    pub name: String,
    /// Path under the asset root, e.g. `models/pine_large.obj`.
    pub model: String,
    #[serde(default)]
    pub transform: Transform,
    /// Multiplied into the model's own color. This is how four settlers get
    /// four shirts out of one mesh (`docs/design/07-look.md`, "тинт инстанса").
    #[serde(default)]
    pub tint: Option<[f32; 3]>,
    #[serde(default)]
    pub body: Body,
}

/// The sun, which is the only light the valley has.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sun {
    /// Hour of the day in `[0, 24)`. Direction and color are derived from it
    /// by the game, so a scene stores the hour, not a vector.
    pub hour: f32,
    pub intensity: f32,
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            hour: 9.0,
            intensity: 1.15,
        }
    }
}

/// Distance fog. On by default, unlike the old renderer's — without it
/// nothing in a forest reads as far away.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fog {
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}

impl Default for Fog {
    fn default() -> Self {
        Self {
            color: [0.62, 0.68, 0.74],
            start: 30.0,
            end: 180.0,
        }
    }
}

/// A whole scene, as it sits on disk.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default)]
    pub sun: Sun,
    #[serde(default)]
    pub fog: Fog,
    #[serde(default)]
    pub entities: Vec<EntityDesc>,
}

impl Scene {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Ok(ron::from_str(&text)?)
    }

    /// Write the scene back out, pretty-printed so that a diff is readable.
    ///
    /// This is the editor's save button, and the reason the editor never has
    /// to be the only way to change a scene.
    pub fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let pretty = ron::ser::PrettyConfig::new()
            .depth_limit(4)
            .struct_names(true);
        let text = ron::ser::to_string_pretty(self, pretty)?;
        std::fs::write(path.as_ref(), text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_survives_a_round_trip_through_the_file() {
        let scene = Scene {
            sun: Sun {
                hour: 17.5,
                intensity: 0.8,
            },
            fog: Fog::default(),
            entities: vec![EntityDesc {
                name: "pine".into(),
                model: "models/pine_large.obj".into(),
                transform: Transform {
                    position: Vec3::new(1.0, 0.0, -3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(1.2),
                },
                tint: None,
                body: Body::Static,
            }],
        };
        let dir = std::env::temp_dir().join("valley-scene-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap(), scene);
    }

    #[test]
    fn euler_degrees_survive_the_trip_through_a_quaternion() {
        let t = Transform {
            rotation_deg: glam::Vec3::new(10.0, -35.0, 0.0),
            ..Transform::default()
        };
        let q = t.rotation();
        let mut back = Transform::default();
        back.set_rotation(q);
        assert!((back.rotation_deg - t.rotation_deg).length() < 0.01);
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing_to_load() {
        // An agent writing a scene by hand should not have to spell out
        // every default, and an older file should still open.
        let text = r#"(entities: [(name: "rock", model: "models/boulder.obj")])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        assert_eq!(scene.entities[0].transform, Transform::default());
        assert_eq!(scene.entities[0].body, Body::None);
        assert_eq!(scene.sun.hour, 9.0);
    }
}
