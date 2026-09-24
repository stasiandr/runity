//! Putting a scene into a world with every module of this build dressing
//! it: the core's spawning ([`crate::world::spawn_scene_dressed`]) with the
//! build's dressers, resolving models and materials with the closures the
//! caller gives.

use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::{EntityDesc, Scene};
use crate::world::{patch_scene_dressed, spawn_owned_dressed, spawn_scene_dressed, Dress, Patched, Unresolved};

/// Every module's dresser this build has, the look resolving models and
/// materials with `resolve` and `palette`.
pub fn dressers<'a>(
    resolve: &'a mut dyn FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &'a dyn Fn(&crate::AssetLink) -> Option<Material>,
) -> Vec<Box<dyn Dress + 'a>> {
    let mut out: Vec<Box<dyn Dress + 'a>> = Vec::new();
    #[cfg(feature = "fluid")]
    let cube = resolve(&crate::AssetLink::named("builtin:cube"));
    #[cfg(feature = "character")]
    let capsule = resolve(&crate::AssetLink::named("builtin:capsule"));
    #[cfg(feature = "soft")]
    let (link, sphere) = (
        resolve(&crate::AssetLink::named("builtin:link")),
        resolve(&crate::AssetLink::named("builtin:sphere")),
    );
    #[cfg(feature = "physics")]
    out.push(Box::new(crate::physics::PhysicsDress));
    #[cfg(feature = "animation")]
    out.push(Box::new(crate::motion::MotionDress));
    #[cfg(feature = "routes")]
    out.push(Box::new(crate::routes::RouteDress));
    out.push(Box::new(crate::appearance::LookDress { resolve, palette }));
    #[cfg(feature = "soft")]
    {
        out.push(Box::new(crate::soft::RopeDress));
        out.push(Box::new(crate::soft::ClothDress));
        out.push(Box::new(crate::soft::HairDress));
        out.push(Box::new(crate::soft::SoftBodyDress));
        out.push(Box::new(crate::soft::JiggleDress));
        out.push(Box::new(crate::soft::FluidDress));
        out.push(Box::new(crate::soft::GrainsDress));
        out.push(Box::new(crate::soft::SoftLookDress { link, sphere, palette }));
    }
    #[cfg(feature = "fluid")]
    {
        out.push(Box::new(crate::fluid::MpmDress));
        out.push(Box::new(crate::fluid::HeightfieldDress));
        out.push(Box::new(crate::fluid::OceanDress));
        out.push(Box::new(crate::fluid::FloatsDress));
        out.push(Box::new(crate::fluid::SmokeDress));
        out.push(Box::new(crate::fluid::FluidLookDress { cube, palette }));
    }
    #[cfg(feature = "character")]
    {
        out.push(Box::new(crate::character::RagdollDress));
        out.push(Box::new(crate::character::CrawlerDress));
        out.push(Box::new(crate::character::CharacterLookDress { capsule, palette }));
    }
    #[cfg(feature = "destruction")]
    {
        out.push(Box::new(crate::destruction::FractureDress));
        out.push(Box::new(crate::destruction::DentsDress));
        out.push(Box::new(crate::destruction::DestructionLookDress));
    }
    #[cfg(feature = "audio")]
    out.push(Box::new(crate::audio::SoundDress));
    out
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
    resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
) -> Vec<Unresolved> {
    // No palette: named materials fall through to the engine's builtins.
    // That is what the reference scene and every test want, neither of which
    // has a library.
    spawn_scene_with(scene, world, resolve, |_| None)
}

/// Put a scene into a world, resolving named materials through a palette.
///
/// The second closure is what makes a material asset worth having: pass
/// `|name| library.material_by_name(name)` and every scene that says
/// `material: "mossy_stone"` picks up the one asset, so changing it changes
/// every scene at once.
pub fn spawn_scene_with(
    scene: &Scene,
    world: &mut World,
    mut resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: impl Fn(&crate::AssetLink) -> Option<Material>,
) -> Vec<Unresolved> {
    let mut dressers = dressers(&mut resolve, &palette);
    let missing = spawn_scene_dressed(scene, world, &mut dressers);
    drop(dressers);
    blow(scene, world);
    missing
}

/// The scene's wind into what it swings that keeps its own: ropes.
fn blow(scene: &Scene, world: &mut World) {
    #[cfg(feature = "soft")]
    {
        use crate::prelude::SceneLook;
        crate::soft::set_wind(world, scene.wind().unwrap_or_default());
        #[cfg(feature = "fluid")]
        crate::fluid::set_wind(world, scene.wind().unwrap_or_default());
    }
    #[cfg(not(feature = "soft"))]
    let _ = (scene, world);
}

/// instances already expanded — so a changed prefab is a changed scene.
pub fn patch_scene(
    before: &Scene,
    after: &Scene,
    world: &mut World,
    mut resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: impl Fn(&crate::AssetLink) -> Option<Material>,
) -> Patched {
    let mut dressers = dressers(&mut resolve, &palette);
    let patched = patch_scene_dressed(before, after, world, &mut dressers);
    drop(dressers);
    blow(after, world);
    patched
}

/// Spawn an entity and everything under it as the game's own rather than
/// the file's: [`spawn_owned_dressed`] with every module's dresser.
pub fn spawn_owned<'a>(
    desc: &'a EntityDesc,
    parent: Option<hecs::Entity>,
    world: &mut World,
    mut resolve: impl FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: impl Fn(&crate::AssetLink) -> Option<Material>,
) -> (Vec<(hecs::Entity, &'a EntityDesc)>, Vec<Unresolved>) {
    let mut dressers = dressers(&mut resolve, &palette);
    spawn_owned_dressed(desc, parent, world, &mut dressers)
}
