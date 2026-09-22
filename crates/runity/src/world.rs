//! From a scene file to things in a world, and from a world to a draw list.
//!
//! The entity store is `hecs`. Nothing here wraps it or hides it: a game
//! wanting to add a component adds one, and the renderer only asks for the
//! three it needs to draw something.

use hecs::World;

use crate::material::Material;
use crate::render::{Camera, Draw, FogSettings, Frame, Lighting, MeshHandle, TextureHandle};
use crate::scene::{Body, Scene, Transform};

/// Which mesh an entity draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model(pub MeshHandle);

/// What the entity's surface is made of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface(pub Material);

/// The image on an entity's surface, already uploaded.
///
/// Separate from [`Surface`] because a material is data a scene can hold and
/// a handle is not: one survives a save, the other is valid only for the
/// renderer that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Textured(pub crate::render::TextureHandle);

/// Where an entity ends up in the world, with its parents already applied.
///
/// Stored rather than recomputed per frame: a draw list is built every frame
/// and a hierarchy is not, so the work belongs where the change happens.
/// Moving a parent means re-running [`apply_hierarchy`] over its subtree, and
/// nothing else has to know.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldTransform(pub glam::Mat4);

/// What an entity is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parent(pub hecs::Entity);

/// Kept from the scene so that physics can pick entities up later without the
/// scene having to be re-read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Physics(pub Body);

/// What could not be spawned, and why. Returned rather than logged: a missing
/// model is a fact the editor wants to show next to the entity, not a line in
/// a terminal nobody is reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Unresolved {
    pub entity_name: String,
    pub model: String,
}

/// Put a scene into a world.
///
/// `resolve` turns the scene's model name into an uploaded mesh. It is a
/// closure rather than a `&Library` so that the caller decides what a name
/// means — a test can answer with one mesh for everything, and the editor can
/// answer with a placeholder for an asset that failed to import.
pub fn spawn_scene(
    scene: &Scene,
    world: &mut World,
    mut resolve: impl FnMut(&str) -> Option<MeshHandle>,
) -> Vec<Unresolved> {
    let mut missing = Vec::new();
    for desc in &scene.entities {
        spawn_subtree(
            desc,
            None,
            glam::Mat4::IDENTITY,
            world,
            &mut resolve,
            &mut missing,
        );
    }
    missing
}

/// Spawn one entity and everything hanging off it.
///
/// A child whose model is missing still spawns, because its own children may
/// be fine and dropping the branch would move them. It just gets nothing to
/// draw.
fn spawn_subtree(
    desc: &crate::scene::EntityDesc,
    parent: Option<hecs::Entity>,
    parent_matrix: glam::Mat4,
    world: &mut World,
    resolve: &mut impl FnMut(&str) -> Option<MeshHandle>,
    missing: &mut Vec<Unresolved>,
) {
    let world_matrix = parent_matrix * desc.transform.matrix();
    let entity = world.spawn((
        desc.transform,
        WorldTransform(world_matrix),
        Physics(desc.body),
    ));
    if let Some(parent) = parent {
        let _ = world.insert_one(entity, Parent(parent));
    }
    match resolve(&desc.model) {
        Some(mesh) => {
            let _ = world.insert(entity, (Model(mesh), Surface(desc.material())));
        }
        None => missing.push(Unresolved {
            entity_name: desc.name.clone(),
            model: desc.model.clone(),
        }),
    }
    for child in &desc.children {
        spawn_subtree(child, Some(entity), world_matrix, world, resolve, missing);
    }
}

/// Recompute every [`WorldTransform`] from the local transforms and parents.
///
/// Call it after moving something that has children. It resolves each entity
/// by walking up to its root, and it stops at a depth limit rather than
/// looping: nothing built from a scene file can contain a cycle, but an
/// editor that sets a parent by hand can make one, and hanging is a worse
/// answer than a wrong transform.
pub fn apply_hierarchy(world: &mut World) {
    const MAX_DEPTH: usize = 64;

    // Snapshotted first, then resolved, then written back. Walking up the
    // tree while holding a query borrow would mean reading the world through
    // the same handle that is iterating it; taking the local transforms out
    // once is both simpler and cheaper than resolving inside the loop.
    let mut locals: std::collections::HashMap<hecs::Entity, (glam::Mat4, Option<hecs::Entity>)> =
        std::collections::HashMap::new();
    for (entity, local, parent) in world
        .query::<(hecs::Entity, &Transform, Option<&Parent>)>()
        .iter()
    {
        locals.insert(entity, (local.matrix(), parent.map(|p| p.0)));
    }

    let mut resolved: Vec<(hecs::Entity, glam::Mat4)> = Vec::with_capacity(locals.len());
    for (entity, (local, parent)) in &locals {
        let mut matrix = *local;
        let mut current = *parent;
        let mut depth = 0;
        while let Some(ancestor) = current {
            let Some((ancestor_local, ancestor_parent)) = locals.get(&ancestor) else {
                // A parent that no longer exists leaves the child where it
                // is rather than dropping it: a despawn should not teleport
                // whatever was attached.
                break;
            };
            matrix = *ancestor_local * matrix;
            current = *ancestor_parent;
            depth += 1;
            if depth >= MAX_DEPTH {
                // Nothing built from a scene file can contain a cycle — a
                // tree written as a tree cannot describe one — but an editor
                // setting a parent by hand can. A wrong transform is a better
                // answer than a hang.
                break;
            }
        }
        resolved.push((*entity, matrix));
    }

    for (entity, matrix) in resolved {
        let _ = world.insert_one(entity, WorldTransform(matrix));
    }
}

/// Collect everything drawable in the world into a frame.
pub fn build_frame(world: &World, camera: Camera, lighting: Lighting, fog: FogSettings) -> Frame {
    let mut draws = Vec::new();
    for (placed, model, surface, textured) in world
        .query::<(&WorldTransform, &Model, &Surface, Option<&Textured>)>()
        .iter()
    {
        draws.push(Draw {
            mesh: model.0,
            transform: placed.0,
            texture: textured.map(|t| t.0).unwrap_or(TextureHandle::WHITE),
            material: surface.0,
        });
    }
    Frame {
        camera,
        lighting,
        clear_color: fog.color,
        fog,
        shadows: crate::render::ShadowSettings::default(),
        draws,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::EntityDesc;
    use glam::Vec3;

    /// An entity with nothing set, for `..blank()` in the tests below.
    fn blank() -> EntityDesc {
        EntityDesc {
            name: String::new(),
            model: "m".into(),
            transform: Transform::default(),
            material: Default::default(),
            body: Body::None,
            children: Vec::new(),
        }
    }

    fn scene_with(models: &[&str]) -> Scene {
        Scene {
            entities: models
                .iter()
                .enumerate()
                .map(|(i, model)| EntityDesc {
                    name: format!("thing {i}"),
                    model: (*model).into(),
                    transform: Transform {
                        position: Vec3::new(i as f32, 0.0, 0.0),
                        ..Default::default()
                    },
                    material: Default::default(),
                    body: Body::Static,
                    children: Vec::new(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_scene_becomes_entities_and_then_a_draw_list() {
        let scene = scene_with(&["pine", "pine", "rock"]);
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        assert!(missing.is_empty());

        let frame = build_frame(
            &world,
            Camera::default(),
            Lighting::default(),
            FogSettings::default(),
        );
        assert_eq!(frame.draws.len(), 3);
        // The clear colour is the fog colour, so the horizon and the far
        // distance meet instead of showing a seam.
        assert_eq!(frame.clear_color, frame.fog.color);
    }

    #[test]
    fn a_child_is_placed_relative_to_its_parent() {
        // A cart at x = 10 with a wheel at x = 1 puts the wheel at 11, and
        // that is the whole point of a hierarchy: place the cart, and the
        // wheel comes along.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "cart".into(),
                model: "m".into(),
                transform: Transform {
                    position: Vec3::new(10.0, 0.0, 0.0),
                    ..Default::default()
                },
                children: vec![EntityDesc {
                    name: "wheel".into(),
                    model: "m".into(),
                    transform: Transform {
                        position: Vec3::new(1.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        let mut positions: Vec<f32> = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| placed.0.w_axis.x)
            .collect();
        positions.sort_by(f32::total_cmp);
        assert_eq!(positions, vec![10.0, 11.0]);
    }

    #[test]
    fn a_parents_rotation_carries_its_children_around_with_it() {
        // Translation alone would pass even if the child's matrix were being
        // added rather than multiplied. A quarter turn tells them apart.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "turntable".into(),
                model: "m".into(),
                transform: Transform {
                    rotation_deg: Vec3::new(0.0, 90.0, 0.0),
                    ..Default::default()
                },
                children: vec![EntityDesc {
                    name: "arm".into(),
                    model: "m".into(),
                    transform: Transform {
                        position: Vec3::new(2.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        // +X rotated 90° about Y lands on -Z in a right-handed system.
        let arm = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| placed.0.w_axis.truncate())
            .find(|p| p.length() > 0.5)
            .expect("the arm is offset from its parent");
        assert!(arm.x.abs() < 1e-4, "expected the arm off the X axis: {arm}");
        assert!((arm.z + 2.0).abs() < 1e-4, "expected z = -2, got {arm}");
    }

    #[test]
    fn moving_a_parent_moves_everything_under_it() {
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "root".into(),
                model: "m".into(),
                children: vec![EntityDesc {
                    name: "child".into(),
                    model: "m".into(),
                    transform: Transform {
                        position: Vec3::new(0.0, 3.0, 0.0),
                        ..Default::default()
                    },
                    children: vec![EntityDesc {
                        name: "grandchild".into(),
                        model: "m".into(),
                        transform: Transform {
                            position: Vec3::new(0.0, 3.0, 0.0),
                            ..Default::default()
                        },
                        ..blank()
                    }],
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));

        // Shove the root sideways and re-resolve.
        let root = world
            .query::<(hecs::Entity, &Transform)>()
            .iter()
            .find(|(_, t)| t.position == Vec3::ZERO)
            .map(|(e, _)| e)
            .expect("the root sits at the origin");
        world.get::<&mut Transform>(root).unwrap().position = Vec3::new(5.0, 0.0, 0.0);
        apply_hierarchy(&mut world);

        let mut heights: Vec<(f32, f32)> = world
            .query::<&WorldTransform>()
            .iter()
            .map(|placed| (placed.0.w_axis.x, placed.0.w_axis.y))
            .collect();
        heights.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(heights, vec![(5.0, 0.0), (5.0, 3.0), (5.0, 6.0)]);
    }

    #[test]
    fn a_child_whose_model_is_missing_keeps_its_own_children_in_place() {
        // Dropping the branch would move the grandchild, which is a worse
        // failure than a hole where one model should be.
        let scene = Scene {
            entities: vec![EntityDesc {
                name: "root".into(),
                model: "m".into(),
                children: vec![EntityDesc {
                    name: "broken".into(),
                    model: "missing".into(),
                    transform: Transform {
                        position: Vec3::new(4.0, 0.0, 0.0),
                        ..Default::default()
                    },
                    children: vec![EntityDesc {
                        name: "fine".into(),
                        model: "m".into(),
                        transform: Transform {
                            position: Vec3::new(1.0, 0.0, 0.0),
                            ..Default::default()
                        },
                        ..blank()
                    }],
                    ..blank()
                }],
                ..blank()
            }],
            ..Default::default()
        };
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |name| {
            (name != "missing").then_some(MeshHandle::TEST)
        });
        assert_eq!(missing.len(), 1);

        let drawn: Vec<f32> = world
            .query::<(&WorldTransform, &Model)>()
            .iter()
            .map(|(placed, _)| placed.0.w_axis.x)
            .collect();
        assert!(
            drawn.contains(&5.0),
            "the grandchild should still stand at 4 + 1, got {drawn:?}"
        );
    }

    #[test]
    fn a_model_that_cannot_be_resolved_is_reported_not_skipped_in_silence() {
        let scene = scene_with(&["pine", "missing"]);
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |name| {
            (name != "missing").then_some(MeshHandle::TEST)
        });
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].model, "missing");
        assert_eq!(missing[0].entity_name, "thing 1");

        // The entity still exists — it just has nothing to draw. Dropping it
        // would move anything parented to it, and a hole where one model
        // should be is a smaller failure than a subtree that silently moved.
        assert_eq!(world.len(), 2);
        assert_eq!(world.query::<&Model>().iter().count(), 1);
    }
}
