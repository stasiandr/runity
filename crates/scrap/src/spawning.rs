//! Putting a scene into a world with every module of this build dressing
//! it: the core's spawning ([`crate::world::spawn_scene_dressed`]) with the
//! build's dressers, resolving models and materials with the closures the
//! caller gives.

use hecs::World;

use crate::material::Material;
use crate::render::MeshHandle;
use crate::scene::{EntityDesc, Scene};
use crate::world::{patch_scene_dressed, spawn_owned_dressed, spawn_scene_dressed, Dress, Patched, Unresolved};

/// Which lines a spawn dresses: the builtin meshes the simulation modules
/// draw with are uploaded only when some line has a field that needs one,
/// so a scene — or a streamed region — without ropes uploads no link.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Needs {
    /// Ropes, cloth, hair and the rest of the soft module: links, spheres.
    pub soft: bool,
    /// Water, smoke and the rest of the fluid module: cubes.
    pub fluid: bool,
    /// Ragdolls and crawlers: capsules.
    pub character: bool,
}

impl Needs {
    /// What `lines` and everything under them ask for.
    pub fn of<'a>(lines: impl IntoIterator<Item = &'a EntityDesc>) -> Self {
        const SOFT: &[&str] = &["rope", "cloth", "hair", "soft_body", "jiggle", "fluid", "grains", "distance_field"];
        const FLUID: &[&str] = &["mpm", "shallow_water", "ripples", "ocean", "floats", "smoke", "snow_cover"];
        const CHARACTER: &[&str] = &["ragdoll", "crawler"];
        // Each line's own few fields looked up in the lists, not every
        // name of the lists looked for among its fields: a reload walks
        // every line of the level.
        fn walk(desc: &EntityDesc, out: &mut Needs) {
            for name in desc.parts.names() {
                out.soft |= SOFT.contains(&name);
                out.fluid |= FLUID.contains(&name);
                out.character |= CHARACTER.contains(&name);
            }
            for child in &desc.children {
                if *out == Needs::ALL {
                    return;
                }
                walk(child, out);
            }
        }
        let mut out = Needs::default();
        for desc in lines {
            if out == Needs::ALL {
                break;
            }
            walk(desc, &mut out);
        }
        out
    }

    /// Everything: when what will be dressed is not known ahead.
    pub const ALL: Needs = Needs { soft: true, fluid: true, character: true };
}

/// Every module's dresser this build has, the look resolving models and
/// materials with `resolve` and `palette`; the builtin meshes of the
/// simulation modules only as `needs` says.
// Pushed one by one: each is there only with its module's feature.
#[allow(clippy::vec_init_then_push)]
pub fn dressers<'a>(
    resolve: &'a mut dyn FnMut(&crate::AssetLink) -> Option<MeshHandle>,
    palette: &'a dyn Fn(&crate::AssetLink) -> Option<Material>,
    needs: Needs,
) -> Vec<Box<dyn Dress + 'a>> {
    let mut out: Vec<Box<dyn Dress + 'a>> = Vec::new();
    let mut builtin = |on: bool, name: &str| on.then(|| resolve(&crate::AssetLink::named(name))).flatten();
    #[cfg(feature = "fluid")]
    let cube = builtin(needs.fluid, "builtin:cube");
    #[cfg(feature = "character")]
    let capsule = builtin(needs.character, "builtin:capsule");
    #[cfg(feature = "soft")]
    let (link, sphere) = (builtin(needs.soft, "builtin:link"), builtin(needs.soft, "builtin:sphere"));
    let _ = (&mut builtin, needs);
    #[cfg(feature = "physics")]
    out.push(Box::new(crate::physics::PhysicsDress));
    #[cfg(feature = "animation")]
    out.push(Box::new(crate::motion::MotionDress));
    #[cfg(feature = "routes")]
    out.push(Box::new(crate::routes::RouteDress));
    #[cfg(feature = "wires")]
    out.push(Box::new(crate::wires::WireDress));
    out.push(Box::new(crate::appearance::LookDress { resolve, palette }));
    out.push(Box::new(crate::streaming::StreamDress));
    #[cfg(feature = "soft")]
    {
        out.push(Box::new(crate::soft::RopeDress));
        out.push(Box::new(crate::soft::ClothDress));
        out.push(Box::new(crate::soft::HairDress));
        out.push(Box::new(crate::soft::SoftBodyDress));
        out.push(Box::new(crate::soft::JiggleDress));
        out.push(Box::new(crate::soft::FluidDress));
        out.push(Box::new(crate::soft::GrainsDress));
        out.push(Box::new(crate::soft::DistanceFieldDress));
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
    let mut dressers = dressers(&mut resolve, &palette, Needs::of(&scene.entities));
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
    let mut dressers = dressers(&mut resolve, &palette, Needs::of(&after.entities));
    patch_scene_dressed(before, after, world, &mut dressers)
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
    let mut dressers = dressers(&mut resolve, &palette, Needs::of(std::iter::once(desc)));
    spawn_owned_dressed(desc, parent, world, &mut dressers)
}
