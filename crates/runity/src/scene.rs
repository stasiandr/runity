//! The scene file: what the editor writes and the game reads.
//!
//! Scenes are RON, not code. That is the whole point of the split — a mouse
//! drag in the editor and a change made by an agent produce the same kind of
//! diff, and `git` can show either one. Nothing here knows about rendering or
//! physics; it is the description they are both built from.

use std::path::Path;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::material::Material;

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

/// The shape physics uses for an entity, which is not the mesh.
///
/// A collider is always a primitive here. Colliding against the triangles a
/// thing is drawn from is available and almost always the wrong trade: it is
/// slower, it cannot be a dynamic body at all in most engines, and a box
/// around a crate behaves better than the crate's own bevelled corners.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Collider {
    #[default]
    None,
    /// Half the size on each axis, before the transform's scale.
    Box {
        half: Vec3,
    },
    Sphere {
        radius: f32,
    },
    /// A cylinder with hemispherical caps: what a person is, because a box
    /// catches on corners and a sphere rolls.
    Capsule {
        half_height: f32,
        radius: f32,
    },
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
    /// A material by name — a `.rmat` asset in the library, or one of the
    /// engine's builtins (`grass`, `bark`, `ember`, …) — or a material
    /// written out in full. Named first because that is what keeps a scene
    /// readable and a palette consistent: a colour spelled out in twenty
    /// places drifts in nineteen of them.
    /// Left out of the file entirely when it is the default, so a scene
    /// full of plain grey things stays readable.
    #[serde(default, skip_serializing_if = "MaterialRef::is_default")]
    pub material: MaterialRef,
    #[serde(default)]
    pub body: Body,
    /// The shape physics sees. Without one, `body` does nothing: a thing can
    /// be declared solid and still have no shape to be solid with, and
    /// saying so in one place beats guessing a box from the mesh.
    #[serde(default)]
    pub collider: Collider,
    /// Things attached to this one. A child's transform is relative to its
    /// parent, so moving the parent moves the lot — which is what makes a
    /// cart with wheels, or a settler carrying a log, one thing to place
    /// rather than several to keep in step.
    ///
    /// Nested rather than a `parent:` field pointing at a name, because a
    /// tree written as a tree cannot describe a cycle or a dangling parent,
    /// and both of those are states an editor would otherwise have to guard
    /// against every time it saves.
    #[serde(default)]
    pub children: Vec<EntityDesc>,
}

impl EntityDesc {
    /// This entity and everything under it, depth first, each with the
    /// transform that stacks its ancestors' on top of its own.
    pub fn flatten(&self) -> Vec<(&EntityDesc, glam::Mat4)> {
        let mut out = Vec::new();
        self.flatten_into(glam::Mat4::IDENTITY, &mut out);
        out
    }

    pub(crate) fn flatten_into<'a>(
        &'a self,
        parent: glam::Mat4,
        out: &mut Vec<(&'a EntityDesc, glam::Mat4)>,
    ) {
        let world = parent * self.transform.matrix();
        out.push((self, world));
        for child in &self.children {
            child.flatten_into(world, out);
        }
    }
}

/// How an entity names its surface.
///
/// Not an `Option`: RON wants `Some(...)` spelled out around an optional
/// field, and `material: Some("grass")` is noise in every line of every
/// scene. A default variant costs nothing and reads better.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaterialRef {
    /// A material asset in the library by file stem, or one of the engine's
    /// builtins. `builtin:stone` forces the builtin even when a project has
    /// an asset of that name.
    Named(String),
    /// Spelled out, for a colour that has not earned a name yet.
    Inline(Material),
}

impl Default for MaterialRef {
    fn default() -> Self {
        MaterialRef::Inline(Material::default())
    }
}

impl MaterialRef {
    fn is_default(&self) -> bool {
        *self == MaterialRef::default()
    }
}

impl EntityDesc {
    /// The material to draw with, using the engine's builtins only.
    ///
    /// What a test and the reference scene want, since neither has a library.
    /// Anything that does have one calls [`EntityDesc::material_from`].
    pub fn material(&self) -> Material {
        self.material_from(|_| None)
    }

    /// The material to draw with, asking `lookup` first.
    ///
    /// The order is the project's palette, then the engine's builtins, then
    /// plain grey. A project's own `stone` therefore shadows the engine's,
    /// which is the useful direction: the builtins exist so that an example
    /// scene can be written before a palette exists, not to reserve seven
    /// names forever. `builtin:stone` reaches past the shadow when that is
    /// what was meant.
    ///
    /// An unknown name falls back rather than failing to load — a scene with
    /// a typo should still open, showing plain grey where the mistake is,
    /// which is more useful than an error and no scene at all.
    pub fn material_from(&self, lookup: impl Fn(&str) -> Option<Material>) -> Material {
        match &self.material {
            MaterialRef::Named(name) => match name.strip_prefix("builtin:") {
                Some(builtin) => crate::material::builtin::by_name(builtin).unwrap_or_default(),
                None => lookup(name)
                    .or_else(|| crate::material::builtin::by_name(name))
                    .unwrap_or_default(),
            },
            MaterialRef::Inline(material) => *material,
        }
    }
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

/// Where the scene is looked at from.
///
/// In the file, because a picture has to be reproducible. The loop this
/// engine is built for is "change something, render it, look" — and a
/// viewpoint that lives in whichever tool happened to render means the
/// picture moves for reasons the scene never recorded, which makes two
/// renders impossible to compare. Putting it here also gives an agent a way
/// to frame a shot: it is a line of text like everything else.
///
/// Three numbers and a field of view, not a whole [`Camera`](crate::Camera).
/// Near and far planes and an up vector are renderer business; a scene says
/// where it is looked at from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub position: Vec3,
    pub target: Vec3,
    /// Vertical field of view, in degrees.
    pub fov_deg: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 3.4, 12.0),
            target: Vec3::new(0.0, 1.4, -4.0),
            fov_deg: 55.0,
        }
    }
}

/// A whole scene, as it sits on disk.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default)]
    pub view: View,
    #[serde(default)]
    pub sun: Sun,
    #[serde(default)]
    pub fog: Fog,
    #[serde(default)]
    pub entities: Vec<EntityDesc>,
}

impl Scene {
    /// Every entity in the scene, roots and descendants alike, each with the
    /// world transform its ancestors give it.
    ///
    /// Anything that asks "what is in this scene" wants this rather than
    /// `entities`, which holds only the roots. An editor's tree, a search, a
    /// count — all of them get the nesting wrong exactly once and then use
    /// this.
    pub fn flatten(&self) -> Vec<(&EntityDesc, glam::Mat4)> {
        let mut out = Vec::new();
        for entity in &self.entities {
            entity.flatten_into(glam::Mat4::IDENTITY, &mut out);
        }
        out
    }

    /// The first entity with this name, at any depth.
    pub fn find(&self, name: &str) -> Option<&EntityDesc> {
        self.flatten()
            .into_iter()
            .find(|(e, _)| e.name == name)
            .map(|(e, _)| e)
    }

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
        // Struct names deliberately off. With them on, a material is written
        // as `Material(...)`, and an untagged enum cannot match a named
        // struct — so a scene the editor saved would not open again. A
        // round trip that only fails on the way back is the worst kind.
        let pretty = ron::ser::PrettyConfig::new().depth_limit(4);
        let text = ron::ser::to_string_pretty(self, pretty)?;
        std::fs::write(path.as_ref(), text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_with_every_field_set_survives_a_round_trip() {
        // The round trip that matters is the one an editor does: write a
        // scene out, open it again, get the same thing. It failed for a
        // while because struct names were on and an untagged enum cannot
        // match a named struct — a file that saved cleanly and would not
        // reopen.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "crate".into(),
                model: "builtin:cube".into(),
                transform: Transform {
                    position: Vec3::new(1.0, 2.0, 3.0),
                    rotation_deg: Vec3::new(0.0, 45.0, 0.0),
                    scale: Vec3::splat(2.0),
                },
                material: MaterialRef::Inline(Material::new(0.3, 0.2, 0.1)),
                body: Body::Dynamic,
                collider: Collider::Box {
                    half: Vec3::splat(0.5),
                },
                children: vec![EntityDesc {
                    name: "lid".into(),
                    model: "builtin:cube".into(),
                    material: MaterialRef::Named("stone".into()),
                    body: Body::None,
                    collider: Collider::None,
                    transform: Transform::default(),
                    children: Vec::new(),
                }],
            }],
            ..Default::default()
        };
        let dir = std::env::temp_dir().join("runity-scene-full");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("full.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap(), scene);
    }

    #[test]
    fn a_scene_survives_a_round_trip_through_the_file() {
        let scene = Scene {
            view: View::default(),
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
                material: MaterialRef::Named("needle".into()),
                body: Body::Static,
                collider: Collider::None,
                children: Vec::new(),
            }],
        };
        let dir = std::env::temp_dir().join("runity-scene-test");
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
    fn flatten_reaches_every_depth_and_stacks_the_transforms() {
        let scene: Scene = ron::from_str(
            r#"(entities: [(
                name: "cart", model: "m", transform: (position: (10.0, 0.0, 0.0)),
                children: [(
                    name: "wheel", model: "m", transform: (position: (1.0, 0.0, 0.0)),
                    children: [(name: "bolt", model: "m", transform: (position: (0.5, 0.0, 0.0)))],
                )],
            )])"#,
        )
        .unwrap();

        assert_eq!(scene.entities.len(), 1, "one root");
        let all = scene.flatten();
        assert_eq!(all.len(), 3, "three entities once the nesting is walked");
        let bolt = scene.find("bolt").expect("found at depth two");
        assert_eq!(bolt.transform.position.x, 0.5, "its own transform is local");
        let bolt_world = all
            .iter()
            .find(|(e, _)| e.name == "bolt")
            .map(|(_, m)| m.w_axis.x)
            .unwrap();
        assert_eq!(bolt_world, 11.5, "10 + 1 + 0.5");
    }

    #[test]
    fn a_material_is_named_or_spelled_out_and_a_typo_still_opens() {
        let named: Scene =
            ron::from_str(r#"(entities: [(name: "a", model: "m", material: "grass")])"#).unwrap();
        assert_eq!(
            named.entities[0].material(),
            crate::material::builtin::GRASS
        );

        let inline: Scene = ron::from_str(
            r#"(entities: [(name: "a", model: "m", material: (base_color: (0.5, 0.1, 0.1)))])"#,
        )
        .unwrap();
        assert_eq!(inline.entities[0].material().base_color, [0.5, 0.1, 0.1]);

        let typo: Scene =
            ron::from_str(r#"(entities: [(name: "a", model: "m", material: "grsas")])"#).unwrap();
        assert_eq!(typo.entities[0].material(), Material::default());
    }

    #[test]
    fn a_scene_remembers_where_it_is_looked_at_from() {
        let scene = Scene {
            view: View {
                position: Vec3::new(3.0, 9.0, -2.0),
                target: Vec3::new(0.0, 1.0, 0.0),
                fov_deg: 35.0,
            },
            ..Scene::default()
        };
        let dir = std::env::temp_dir().join("runity-scene-view");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("view.ron");
        scene.save(&path).unwrap();
        assert_eq!(Scene::load(&path).unwrap().view, scene.view);

        // And a scene written before there was a camera in the format still
        // opens, framed the way everything used to be.
        let older: Scene = ron::from_str(r#"(entities: [])"#).unwrap();
        assert_eq!(older.view, View::default());
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing_to_load() {
        // An agent writing a scene by hand should not have to spell out
        // every default, and an older file should still open.
        let text = r#"(entities: [(name: "rock", model: "models/boulder.obj")])"#;
        let scene: Scene = ron::from_str(text).unwrap();
        assert_eq!(scene.entities[0].transform, Transform::default());
        assert_eq!(scene.entities[0].body, Body::None);
        assert_eq!(scene.entities[0].material(), Material::default());
        assert_eq!(scene.sun.hour, 9.0);
    }
}
