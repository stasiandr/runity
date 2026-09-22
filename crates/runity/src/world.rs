//! From a scene file to things in a world, and from a world to a draw list.
//!
//! The entity store is `hecs`. Nothing here wraps it or hides it: a game
//! wanting to add a component adds one, and the renderer only asks for the
//! three it needs to draw something.

use hecs::World;

use crate::material::Material;
use crate::render::{Camera, Draw, FogSettings, Frame, Lighting, MeshHandle};
use crate::scene::{Body, Scene, Transform};

/// Which mesh an entity draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model(pub MeshHandle);

/// What the entity's surface is made of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface(pub Material);

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
        let Some(mesh) = resolve(&desc.model) else {
            missing.push(Unresolved {
                entity_name: desc.name.clone(),
                model: desc.model.clone(),
            });
            continue;
        };
        world.spawn((
            desc.transform,
            Model(mesh),
            Surface(desc.material()),
            Physics(desc.body),
        ));
    }
    missing
}

/// Collect everything drawable in the world into a frame.
pub fn build_frame(world: &World, camera: Camera, lighting: Lighting, fog: FogSettings) -> Frame {
    let mut draws = Vec::new();
    for (transform, model, surface) in world.query::<(&Transform, &Model, &Surface)>().iter() {
        draws.push(Draw {
            mesh: model.0,
            transform: transform.matrix(),
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
    fn a_model_that_cannot_be_resolved_is_reported_not_skipped_in_silence() {
        let scene = scene_with(&["pine", "missing"]);
        let mut world = World::new();
        let missing = spawn_scene(&scene, &mut world, |name| {
            (name != "missing").then_some(MeshHandle::TEST)
        });
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].model, "missing");
        assert_eq!(missing[0].entity_name, "thing 1");
        // The rest of the scene still stands up.
        assert_eq!(world.len(), 1);
    }
}
