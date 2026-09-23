//! A picture of an asset: what the Project window shows for a model or a
//! prefab, Unity's asset preview — and what an agent looks at before
//! placing one.

use runity::glam::Vec3;
use runity::render::{Camera, FogSettings};
use runity::{EntityDesc, OffscreenTarget, Scene};

use crate::{EditError, EditResult, Session};

impl Session {
    /// A `size`-pixel square picture, RGBA, of a prefab or a model by the
    /// name scenes use (`campfire`, `builtin:cone`, `rock`), alone on a
    /// plain background, framed whole from above and to the side. Nothing
    /// in the open scene or the view changes.
    pub fn thumbnail(&mut self, what: &str, size: u32) -> EditResult<Vec<u8>> {
        let size = size.clamp(16, 1024);
        let mut line = EntityDesc {
            name: what.to_string(),
            ..Default::default()
        };
        if self.prefabs.get(what).is_some() {
            line.prefab = what.into();
        } else if self.bounds_of(what).is_some() {
            line.model = what.into();
        } else {
            return Err(EditError::Scene(format!(
                "no prefab or model named `{what}` to picture"
            )));
        }
        let scene = Scene {
            entities: vec![line],
            ..Default::default()
        };
        let expanded = runity::instantiate(&scene, &self.prefabs).scene;

        // Everything it draws, in one box.
        let mut low = Vec3::splat(f32::MAX);
        let mut high = Vec3::splat(f32::MIN);
        for (desc, placed) in expanded.flatten() {
            let Some((a, b)) = self.bounds_of(&desc.model) else {
                continue;
            };
            for corner in 0..8u32 {
                let pick = |axis: usize| {
                    if corner & (1 << axis) == 0 {
                        a[axis]
                    } else {
                        b[axis]
                    }
                };
                let p = placed.transform_point3(Vec3::new(pick(0), pick(1), pick(2)));
                low = low.min(p);
                high = high.max(p);
            }
        }
        if low.x > high.x {
            (low, high) = (Vec3::splat(-0.5), Vec3::splat(0.5));
        }
        let centre = (low + high) * 0.5;
        let radius = ((high - low) * 0.5).length().max(0.05);
        let fov = 35.0f32;
        let distance = radius / (fov.to_radians() * 0.5).sin() * 1.05;
        let camera = Camera {
            position: centre + Vec3::new(1.0, 0.75, 1.3).normalize() * distance,
            target: centre,
            fov_y_degrees: fov,
            near: (distance - radius * 2.0).max(0.01),
            far: distance + radius * 2.0,
            ..Camera::default()
        };

        let mut world = hecs::World::new();
        let renderer = &mut self.renderer;
        let gpu = &self.gpu;
        let library = self.library.as_ref();
        let uploaded = &mut self.uploaded;
        runity::spawn_scene_with(
            &expanded,
            &mut world,
            |name| {
                if let Some(found) = uploaded.iter().find(|(n, _)| **n == **name) {
                    return Some(found.1);
                }
                let handle = if let Some(mesh) = runity::builtin::by_name(name) {
                    renderer.upload_mesh_owned(gpu, &mesh)
                } else {
                    renderer.upload_mesh(gpu, library?.mesh_by_name(name)?)
                };
                uploaded.push((name.to_string(), handle));
                Some(handle)
            },
            |name| library?.material_by_name(name),
        );
        runity::world::apply_hierarchy(&mut world);
        let backdrop = Vec3::new(0.32, 0.33, 0.36);
        let mut frame = runity::build_frame(
            &world,
            camera,
            runity::scene_lighting(&Scene::default().sun),
            FogSettings {
                color: backdrop,
                start: 1.0e6,
                end: 2.0e6,
                ..Default::default()
            },
        );
        frame.clear_color = backdrop;
        frame.sky.mode = runity::render::SkyMode::Color;
        // A preview, as Unity draws one: tonemapped like the game, but no
        // glow bleeding from the asset onto its backdrop.
        frame.post.bloom.intensity = 0.0;
        let target = OffscreenTarget::new(&self.gpu, size, size);
        self.renderer.render(&self.gpu, &target, &frame);
        Ok(target.read_rgba(&self.gpu))
    }
}
