//! A picture of an asset: what the Project window shows for a model or a
//! prefab, Unity's asset preview — and what an agent looks at before
//! placing one.

use scrap::glam::Vec3;
#[allow(unused_imports)]
use scrap::prelude::*;
use scrap::render::{Camera, FogSettings};
use scrap::{EntityDesc, OffscreenTarget, Scene};

use crate::{EditError, EditResult, Session};

/// What [`Session::thumbnail`] takes before a material's name, for its
/// picture on a ball: `material:bark`.
pub const MATERIAL_PICTURE: &str = "material:";

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
        if let Some(material) = what.strip_prefix(MATERIAL_PICTURE) {
            // A material on a ball, as Unity previews one.
            if !self.palette().iter().any(|(n, _)| n == material) {
                return Err(EditError::Scene(format!(
                    "no material named `{material}` to picture"
                )));
            }
            line.set_part(&scrap::scene::ModelRef("builtin:sphere".into()));
            line.set_part(&scrap::scene::MaterialRef::Named(material.into()));
        } else if self.prefabs.get(what).is_some() {
            line.prefab = what.into();
        } else if self.bounds_of(what).is_some() {
            line.set_part(&scrap::scene::ModelRef(what.into()));
        } else {
            return Err(EditError::Scene(format!(
                "no prefab or model named `{what}` to picture"
            )));
        }
        let scene = Scene {
            entities: vec![line],
            ..Default::default()
        };
        let expanded = scrap::instantiate(&scene, &self.prefabs).scene;

        // Everything it draws, in one box.
        let mut low = Vec3::splat(f32::MAX);
        let mut high = Vec3::splat(f32::MIN);
        for (desc, placed) in expanded.flatten() {
            let Some((a, b)) = self.bounds_of(&desc.model()) else {
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
        scrap::spawn_scene_with(
            &expanded,
            &mut world,
            |name| {
                if let Some(found) = uploaded.iter().find(|(n, _)| **n == **name) {
                    return Some(found.1);
                }
                let handle = if let Some(mesh) = scrap::builtin::by_name(name) {
                    renderer.upload_mesh_owned(gpu, &mesh)
                } else {
                    renderer.upload_mesh(gpu, library?.mesh_by_name(name)?)
                };
                uploaded.push((name.to_string(), handle));
                Some(handle)
            },
            |link| library?.material_link(link),
        );
        scrap::world::apply_hierarchy(&mut world);
        let _ = scrap::world::upload_material_maps(&world, library, gpu, renderer);
        let backdrop = Vec3::new(0.32, 0.33, 0.36);
        let mut frame = scrap::build_frame(
            &world,
            camera,
            scrap::scene_lighting(&Scene::default().sun()),
            FogSettings {
                color: backdrop,
                start: 1.0e6,
                end: 2.0e6,
                ..Default::default()
            },
        );
        frame.clear_color = backdrop;
        frame.sky.mode = scrap::render::SkyMode::Color;
        // A preview, as Unity draws one: tonemapped like the game, but no
        // glow bleeding from the asset onto its backdrop.
        frame.post.bloom.intensity = 0.0;
        let target = OffscreenTarget::new(&self.gpu, size, size);
        self.renderer.render(&self.gpu, &target, &frame);
        Ok(target.read_rgba(&self.gpu))
    }
}

impl Session {
    /// A `width` × `height` picture, RGBA, of the scene in `path` as its
    /// own view looks at it, lit and fogged as it asks: the Project's
    /// picture of a scene. The open document and the view do not change.
    pub fn scene_thumbnail(
        &mut self,
        path: &std::path::Path,
        width: u32,
        height: u32,
    ) -> EditResult<Vec<u8>> {
        let (width, height) = (width.clamp(16, 1024), height.clamp(16, 1024));
        let mut scene = crate::load_document(path)?;
        scrap::refs::settle(&mut scene.entities, self.library.as_ref(), &self.prefabs);
        let expanded = scrap::instantiate(&scene, &self.prefabs).scene;
        let mut world = hecs::World::new();
        let renderer = &mut self.renderer;
        let gpu = &self.gpu;
        let library = self.library.as_ref();
        let uploaded = &mut self.uploaded;
        scrap::spawn_scene_with(
            &expanded,
            &mut world,
            |name| {
                if let Some(found) = uploaded.iter().find(|(n, _)| **n == **name) {
                    return Some(found.1);
                }
                let handle = if let Some(mesh) = scrap::builtin::by_name(name) {
                    renderer.upload_mesh_owned(gpu, &mesh)
                } else {
                    renderer.upload_mesh(gpu, library?.mesh_by_name(name)?)
                };
                uploaded.push((name.to_string(), handle));
                Some(handle)
            },
            |link| library?.material_link(link),
        );
        scrap::world::apply_hierarchy(&mut world);
        let _ = scrap::world::upload_material_maps(&world, library, gpu, renderer);
        let camera = scrap::scene_camera(&scene.view());
        let mut frame = scrap::render::Frame {
            lighting: scrap::scene_lighting(&scene.sun()),
            fog: scrap::scene_fog(&scene.fog()),
            clear_color: Vec3::from_array(scene.fog().color),
            sky: scrap::render::Sky {
                horizon: scene.fog().color,
                ..Default::default()
            },
            ..scrap::build_frame(
                &world,
                camera,
                scrap::render::Lighting::default(),
                FogSettings::default(),
            )
        };
        scrap::world::scene_look(&mut frame, &scene);
        // A still picture: nothing to smooth over frames, nothing to adapt.
        frame.post.taa = false;
        frame.post.fxaa = true;
        frame.post.auto_exposure.enabled = false;
        let target = OffscreenTarget::new(&self.gpu, width, height);
        self.renderer.render(&self.gpu, &target, &frame);
        Ok(target.read_rgba(&self.gpu))
    }
}

/// The shader [`Session::graph_preview`] draws with, its own: never a
/// material's.
const PREVIEW_SHADER: &str = "__graph_preview";

impl Session {
    /// A `size`-pixel square picture, RGBA, of the shader graph at `path`
    /// on a ball: the material it makes, lit, or with `node` the value of
    /// that node glowing on black — Shader Graph's main preview and a
    /// node's. Its numbers are those of the first material that uses the
    /// graph, or zeros. Refused in words when the graph does not build.
    pub fn graph_preview(
        &mut self,
        path: &std::path::Path,
        node: Option<&str>,
        size: u32,
    ) -> EditResult<Vec<u8>> {
        let size = size.clamp(16, 1024);
        let text = std::fs::read_to_string(path).map_err(|e| EditError::Io(e.to_string()))?;
        let name = path
            .file_name()
            .and_then(|f| f.to_str())
            .and_then(scrap::shader_graph::shader_name)
            .ok_or_else(|| {
                EditError::Scene(format!(
                    "{} is not a material's shader graph",
                    path.display()
                ))
            })?
            .to_string();
        let graph = scrap::shader_graph::surface::parse(&text).map_err(EditError::Scene)?;
        let library = scrap::render::subgraphs_beside(path);
        let shown = match node {
            Some(node) => scrap::shader_graph::surface::preview_of(&graph, node, &library)
                .map_err(EditError::Scene)?,
            None => graph,
        };
        let wgsl = scrap::shader_graph::surface::to_wgsl_with(
            &shown,
            &format!("shaders/{name}.graph.ron"),
            &library,
        )
        .map_err(EditError::Scene)?;
        let id = scrap::asset::shader_id(PREVIEW_SHADER);
        self.renderer
            .set_material_shader(&self.gpu, id, &wgsl)
            .map_err(EditError::Scene)?;
        let own = scrap::asset::shader_id(&name);
        let params = self
            .palette()
            .into_iter()
            .find(|(_, m)| m.shader == Some(own))
            .map(|(_, m)| m.params)
            .unwrap_or_default();
        let ball = match self.uploaded.iter().find(|(n, _)| n == "builtin:sphere") {
            Some(found) => found.1,
            None => {
                let handle = self
                    .renderer
                    .upload_mesh_owned(&self.gpu, &scrap::builtin::sphere(0.5, 48, 32));
                self.uploaded.push(("builtin:sphere".to_string(), handle));
                handle
            }
        };
        let backdrop = Vec3::new(0.32, 0.33, 0.36);
        let mut frame = scrap::render::Frame {
            camera: Camera {
                position: Vec3::new(0.0, 0.35, 1.9),
                target: Vec3::ZERO,
                fov_y_degrees: 35.0,
                ..Camera::default()
            },
            clear_color: backdrop,
            draws: vec![scrap::render::Draw {
                mesh: ball,
                transform: scrap::glam::Mat4::IDENTITY,
                texture: scrap::render::TextureHandle::WHITE,
                material: scrap::Material {
                    shading: if node.is_some() {
                        scrap::material::Shading::Unlit
                    } else {
                        scrap::material::Shading::Lit
                    },
                    shader: Some(id),
                    params,
                    ..scrap::Material::new(1.0, 1.0, 1.0)
                },
                pose: None,
            }],
            ..Default::default()
        };
        frame.sky.mode = scrap::render::SkyMode::Color;
        frame.post.bloom.intensity = 0.0;
        frame.post.taa = false;
        frame.post.auto_exposure.enabled = false;
        let target = OffscreenTarget::new(&self.gpu, size, size);
        self.renderer.render(&self.gpu, &target, &frame);
        Ok(target.read_rgba(&self.gpu))
    }
}
