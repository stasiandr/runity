//! What a line looks like in the world: the render module's dresser
//! ([`crate::world::Dress`]). A model and its material, a decal pressed
//! from it, ground shaped from numbers, particles, a light, a camera, a
//! reflection probe, a place that looks different, a picture a camera
//! draws, prints left as it walks and grass it bends.

#[allow(unused_imports)]
use crate::prelude::*;
use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::EntityDesc;
use crate::world::{
    BendsGrass, CameraLens, Changed, Dress, LightSource, Model, PostVolumeBox, Pressing, ProbeBox, Surface, ToTexture,
    Unresolved,
};

/// The render module's dresser: models and materials found by `resolve`
/// and `palette`.
pub struct LookDress<'a> {
    pub resolve: &'a mut dyn FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    pub palette: &'a dyn Fn(&crate::AssetLink) -> Option<Material>,
}

/// Put a component on for `Some`, take it off for `None`.
fn put<T: hecs::Component>(world: &mut World, entity: hecs::Entity, value: Option<T>) {
    match value {
        Some(value) => {
            let _ = world.insert_one(entity, value);
        }
        None => {
            scrap_core::world::take_off::<T>(world, entity);
        }
    }
}

impl Dress for LookDress<'_> {
    fn parts(&self) -> &[&'static str] {
        &[
            "model",
            "material",
            "decal",
            "terrain",
            "particles",
            "light",
            "reflection_probe",
            "irradiance_volume",
            "post_volume",
            "camera",
            "render_texture",
            "footprints",
            "cloth",
            "rope",
            "heap",
            "bends_grass",
        ]
    }

    fn dress(
        &mut self,
        line: &EntityDesc,
        entity: hecs::Entity,
        world: &mut World,
        changed: Changed,
        missing: &mut Vec<Unresolved>,
    ) {
        if changed.any(&["model", "material", "decal", "terrain", "cloth", "rope", "heap"]) {
            dress_look(line, entity, world, &mut *self.resolve, self.palette, missing);
        }
        if changed.has("particles") {
            particles(line, entity, world, &mut *self.resolve, self.palette);
        }
        if changed.has("light") {
            put(world, entity, line.light().map(LightSource));
        }
        if changed.has("reflection_probe") {
            put(world, entity, line.reflection_probe().map(ProbeBox));
        }
        if changed.has("irradiance_volume") {
            put(world, entity, line.part::<crate::ddgi::IrradianceVolume>().map(crate::world::VolumeBox));
        }
        if changed.has("post_volume") {
            put(world, entity, line.post_volume().map(PostVolumeBox));
        }
        if changed.has("camera") {
            put(world, entity, line.camera().map(CameraLens));
        }
        if changed.has("render_texture") {
            put(world, entity, line.render_texture().map(ToTexture));
        }
        if changed.has("footprints") {
            // Retuned, it starts a fresh trail: the old prints were made by
            // the old settings.
            put(world, entity, line.footprints().map(crate::footprints::Trail::new));
        }
        if changed.has("bends_grass") {
            let metres = line.bends_grass();
            put(world, entity, (metres > 0.0).then_some(BendsGrass(metres)));
        }
    }
}

/// An emitter ready to run: what its particles are drawn as (its model, or
/// a small cube) and with (its material, or the plain colour). A retuned
/// emitter keeps what is already in the air.
fn particles(
    line: &EntityDesc,
    entity: hecs::Entity,
    world: &mut World,
    resolve: &mut dyn FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &dyn Fn(&crate::AssetLink) -> Option<Material>,
) {
    let Some(emitter) = line.particles() else {
        scrap_core::world::take_off::<crate::particles::Emitting>(world, entity);
        return;
    };
    let fresh = emitting(&emitter, resolve, palette);
    // Spawned where there is no GPU (a game's fixed step): its particles'
    // mesh comes with the next frame, as a model's does.
    match fresh.as_ref().filter(|f| f.mesh == MeshHandle::TEST) {
        Some(_) => {
            let _ = world.insert_one(entity, EmitterMeshPending(particle_model(&emitter)));
        }
        None => {
            let _ = world.remove_one::<EmitterMeshPending>(entity);
        }
    }
    let running = world.get::<&mut crate::particles::Emitting>(entity).ok().map(|mut e| {
        // The knobs change; what is in the air stays.
        if let Some(fresh) = &fresh {
            e.mesh = fresh.mesh;
            e.material = fresh.material;
        }
        e.emitter = emitter.clone();
    });
    if running.is_none() {
        if let Some(emitting) = fresh {
            let _ = world.insert_one(entity, emitting);
        }
    }
}

/// What an emitter's particles are drawn as when it names no model: a
/// quad that faces the eye, or a small cube.
fn particle_stand_in(emitter: &crate::scene::Emitter) -> crate::AssetLink {
    crate::AssetLink::named(if emitter.facing {
        "builtin:plane"
    } else {
        "builtin:cube"
    })
}

/// The model an emitter's particles are drawn as.
pub fn particle_model(emitter: &crate::scene::Emitter) -> crate::AssetLink {
    if emitter.model.is_empty() {
        particle_stand_in(emitter)
    } else {
        emitter.model.clone()
    }
}

fn emitting(
    emitter: &crate::scene::Emitter,
    resolve: &mut dyn FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &dyn Fn(&crate::AssetLink) -> Option<Material>,
) -> Option<crate::particles::Emitting> {
    let cube = particle_stand_in(emitter);
    let model = particle_model(emitter);
    let mesh = resolve(&model).or_else(|| resolve(&cube))?;
    let mut out = crate::particles::Emitting::new(emitter.clone(), mesh);
    out.material = emitter.material.as_ref().and_then(palette);
    Some(out)
}

/// A model spawned without a GPU, drawn with a stand-in until its mesh is
/// uploaded: the link it names.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshPending(pub crate::AssetLink);

/// An emitter spawned without a GPU, its particles drawn with nothing until
/// their mesh is uploaded: the link they are drawn as.
#[derive(Debug, Clone, PartialEq)]
pub struct EmitterMeshPending(pub crate::AssetLink);

/// Give an entity the mesh and surface its line names — or its decal, or
/// its shaped ground — or take them away when the mesh cannot be found.
pub fn dress_look(
    desc: &EntityDesc,
    entity: hecs::Entity,
    world: &mut World,
    resolve: &mut dyn FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &dyn Fn(&crate::AssetLink) -> Option<Material>,
    missing: &mut Vec<Unresolved>,
) {
    match desc.decal() {
        Some(decal) => {
            let _ = world.insert_one(entity, Pressing(decal, desc.material_from(palette)));
        }
        None => {
            scrap_core::world::take_off::<Pressing>(world, entity);
        }
    }
    // Ground made from its numbers: its mesh comes when it is first drawn
    // (`terrain::upload_terrains`), and again only if they changed.
    if let Some(terrain) = desc.terrain() {
        let same = world
            .get::<&crate::terrain::Relief>(entity)
            .is_ok_and(|r| r.terrain == terrain);
        if !same {
            scrap_core::world::take_off::<Model>(world, entity);
            let _ = world.insert_one(entity, crate::terrain::Relief::new(terrain));
        }
        let _ = world.insert_one(entity, Surface(desc.material_from(palette)));
        return;
    }
    scrap_core::world::take_off::<crate::terrain::Relief>(world, entity);
    // No model is nothing to draw — a probe, a decal, a light, an empty to
    // hang children on — not a model that could not be found.
    let model = desc.model();
    if model.is_empty() {
        scrap_core::world::take_off::<Model>(world, entity);
        scrap_core::world::take_off::<Surface>(world, entity);
        return;
    }
    match resolve(&model) {
        Some(mesh) => {
            let surface = Surface(desc.material_from(palette));
            let _ = world.insert(entity, (Model(mesh), surface));
            // Spawned where there is no GPU (a game's fixed step, with the
            // renderer on its own thread): its mesh comes with the next
            // frame (`LiveScene::upload_pending`).
            if mesh == crate::render::MeshHandle::TEST {
                let _ = world.insert_one(entity, MeshPending(model.clone()));
            } else {
                let _ = world.remove_one::<MeshPending>(entity);
            }
        }
        None => {
            let _ = world.remove::<(Model, Surface)>(entity);
            missing.push(Unresolved {
                entity_name: desc.name.clone(),
                model: model.to_string(),
            });
        }
    }
}
