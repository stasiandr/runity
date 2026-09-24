//! A scene drawn offscreen, as a tool draws it: its project's prefabs,
//! library and materials' shaders found, its world spawned, its frame
//! built — what `scene_shot` takes a picture of and `runity perf` times.

use std::path::Path;

use crate::prelude::*;
use crate::render::{FogSettings, Frame, MeshHandle};
use crate::{Gpu, Library, OffscreenTarget, Renderer, Scene};

/// A scene ready to draw.
pub struct Shot {
    pub gpu: Gpu,
    pub target: OffscreenTarget,
    pub renderer: Renderer,
    pub frame: Frame,
    /// What did not resolve, in words: prefabs, models, shaders, maps.
    pub problems: Vec<String>,
    /// Entities in the scene, once its prefabs are expanded.
    pub entities: usize,
    /// The world it was spawned into, and what its frame is built with:
    /// for timing a step of it and the frame's making ([`Shot::step`],
    /// [`Shot::build`]).
    pub world: hecs::World,
    scene: Scene,
    camera: crate::render::Camera,
    lighting: crate::render::Lighting,
    fog: FogSettings,
    #[cfg(feature = "physics")]
    physics: Option<crate::physics::PhysicsWorld>,
}

impl Shot {
    /// Open `scene_path` at `width` × `height`, the library at `library`
    /// or else the project's own if built.
    pub fn open(
        scene_path: &Path,
        width: u32,
        height: u32,
        library: Option<&Path>,
    ) -> Result<Shot, Box<dyn std::error::Error>> {
        let gpu = Gpu::headless_blocking(false)?;
        Self::open_on(gpu, scene_path, width, height, library)
    }

    /// The same, on a device already made.
    pub fn open_on(
        gpu: Gpu,
        scene_path: &Path,
        width: u32,
        height: u32,
        library: Option<&Path>,
    ) -> Result<Shot, Box<dyn std::error::Error>> {
        let document = Scene::load(scene_path)?;
        let mut problems = Vec::new();
        // The project the scene is in says where its prefabs and its
        // library are. Every instance is replaced by what it stands for
        // before anything else looks at the scene.
        let project = crate::Project::find(scene_path).ok();
        let (prefabs, prefab_problems) = project.as_ref().map(crate::Prefabs::of).unwrap_or_default();
        for (path, e) in &prefab_problems {
            problems.push(format!("skipped {}: {e}", path.display()));
        }
        let instanced = crate::instantiate(&document, &prefabs);
        for problem in &instanced.problems {
            problems.push(format!(
                "{}: prefab {} — {}",
                problem.entity_name, problem.prefab, problem.reason
            ));
        }
        let scene = instanced.scene;

        let target = OffscreenTarget::new(&gpu, width, height);
        let mut renderer = Renderer::new(&gpu, &target);
        // The project's materials' own shaders.
        if let Some(project) = &project {
            let mut shaders = crate::render::MaterialShaders::new(project.root().join(crate::project::SHADERS));
            for (name, result) in shaders.poll(&mut renderer, &gpu) {
                if let Err(problem) = result {
                    problems.push(format!("shader {name}: {problem}"));
                }
            }
        }
        // Models resolve from the builtins first, then from a library: the
        // one given, or else the project's own if it has been built.
        let library_dir = library
            .map(Path::to_path_buf)
            .or_else(|| project.as_ref().map(|p| p.library()).filter(|dir| dir.is_dir()));
        let library = match &library_dir {
            Some(dir) => {
                let (library, skipped) = Library::open(dir)?;
                for (path, e) in &skipped {
                    problems.push(format!("skipped {}: {e}", path.display()));
                }
                Some(library)
            }
            None => None,
        };

        let mut world = hecs::World::new();
        let mut uploaded: Vec<(String, MeshHandle)> = Vec::new();
        let missing = crate::spawn_scene_with(
            &scene,
            &mut world,
            |name| {
                let name: &str = name;
                if let Some(found) = uploaded.iter().find(|(n, _)| n == name) {
                    return Some(found.1);
                }
                let handle = if let Some(mesh) = crate::builtin::by_name(name) {
                    renderer.upload_mesh_owned(&gpu, &mesh)
                } else {
                    let mesh = library.as_ref()?.mesh_by_name(name)?;
                    renderer.upload_mesh(&gpu, mesh)
                };
                uploaded.push((name.to_string(), handle));
                Some(handle)
            },
            |name| library.as_ref()?.material_by_name(name),
        );
        for m in &missing {
            problems.push(format!("{}: no model named {}", m.entity_name, m.model));
        }
        // Mesh colliders get their triangles, as a live scene's do: the
        // distance field is baked from them.
        #[cfg(feature = "physics")]
        crate::physics::attach_scene_collision_meshes(&mut world, &scene, library.as_ref());

        let lighting = crate::scene_lighting(&scene.sun());
        let fog = FogSettings {
            color: glam::Vec3::from_array(scene.fog().color),
            start: scene.fog().start,
            end: scene.fog().end,
            ..Default::default()
        };
        // The scene says where it is looked at from, so two renders of the
        // same file are the same picture.
        let camera = crate::scene_camera(&scene.view());
        // Ropes hang a few seconds before the picture, to rest where they
        // would: strung as a parabola, they sway into a catenary and settle
        // on what is under them.
        #[cfg(feature = "soft")]
        {
            crate::world::apply_hierarchy(&mut world);
            for _ in 0..180 {
                crate::soft::step(&mut world, 1.0 / 60.0);
            }
            crate::soft::show(&mut world, 0.0);
        }
        // Water, snow and smoke run the same few seconds.
        #[cfg(feature = "fluid")]
        {
            for _ in 0..180 {
                crate::fluid::step(&mut world, 1.0 / 60.0);
            }
            crate::fluid::show(&mut world, 0.0);
        }
        crate::terrain::upload_terrains(&mut world, &gpu, &mut renderer);
        problems.extend(crate::world::upload_material_maps(&world, library.as_ref(), &gpu, &mut renderer));
        let mut frame = crate::build_frame(&world, camera, lighting, fog);
        crate::world::scene_look(&mut frame, &scene);
        Ok(Shot {
            gpu,
            target,
            renderer,
            frame,
            problems,
            entities: scene.entities.len(),
            world,
            scene,
            camera,
            lighting,
            fog,
            #[cfg(feature = "physics")]
            physics: None,
        })
    }

    /// One fixed step of the scene's simulation, as a game's: soft bodies,
    /// water and smoke, then physics. What a render thread would run
    /// beside the drawing.
    pub fn step(&mut self, seconds: f32) {
        Self::step_world(
            &mut self.world,
            #[cfg(feature = "physics")]
            &mut self.physics,
            seconds,
        );
    }

    /// A game's frame with a render thread (`runity::shell`): the frame
    /// built, then drawn on a thread of its own while this one runs the
    /// next step.
    pub fn frame_pipelined(&mut self, seconds: f32) {
        self.build();
        let (renderer, gpu, target, frame) = (&mut self.renderer, &self.gpu, &self.target, &self.frame);
        std::thread::scope(|scope| {
            let drawing = scope.spawn(move || renderer.render(gpu, target, frame));
            Self::step_world(
                &mut self.world,
                #[cfg(feature = "physics")]
                &mut self.physics,
                seconds,
            );
            drawing.join().expect("the drawing thread panicked");
        });
    }

    fn step_world(
        world: &mut hecs::World,
        #[cfg(feature = "physics")] physics: &mut Option<crate::physics::PhysicsWorld>,
        seconds: f32,
    ) {
        #[cfg(feature = "soft")]
        crate::soft::step(world, seconds);
        #[cfg(feature = "fluid")]
        crate::fluid::step(world, seconds);
        #[cfg(feature = "physics")]
        physics
            .get_or_insert_with(|| crate::physics::PhysicsWorld::new(seconds))
            .run(world);
    }

    /// The frame built again from the world, as a game builds it each
    /// frame.
    pub fn build(&mut self) {
        #[cfg(feature = "soft")]
        crate::soft::show(&mut self.world, 0.0);
        #[cfg(feature = "fluid")]
        crate::fluid::show(&mut self.world, 0.0);
        let mut frame = crate::build_frame(&self.world, self.camera, self.lighting, self.fog);
        crate::world::scene_look(&mut frame, &self.scene);
        self.frame = frame;
    }

    /// Frames to draw before the picture settles: what builds a history
    /// over frames — probes, pages, reservoirs, an upscaler's — needs them.
    pub fn warm_frames(&self) -> u32 {
        let f = &self.frame;
        if let Some(clock) = f.smoke.iter().filter_map(|s| s.gpu.as_ref()).map(|g| g.clock).reduce(f32::max) {
            // Smokes stepped where they are drawn, a few steps a frame:
            // as many frames as their settling owes.
            ((clock / crate::volume::SMOKE_STEP) as u32).div_ceil(8).max(2)
        } else if !f.irradiance_volumes.is_empty() {
            90
        } else if f.post.upscaling.enabled || f.shadows.virtual_maps || f.ray_tracing.restir {
            16
        } else {
            2
        }
    }

    /// Draw the frame `n` times.
    pub fn draw(&mut self, n: u32) {
        for _ in 0..n {
            self.renderer.render(&self.gpu, &self.target, &self.frame);
        }
    }

    /// Whether the device draws in software (a CI runner's): its times
    /// say nothing about a GPU's.
    pub fn software(&self) -> bool {
        self.gpu.software()
    }

    /// The picture, waiting for the GPU.
    pub fn pixels(&self) -> Vec<u8> {
        self.target.read_rgba(&self.gpu)
    }
}
