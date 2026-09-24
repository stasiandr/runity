//! Animation: a skeleton posed from its clips ([`animator`]), graphs of
//! states and transitions that pick the clip ([`animgraph`], written as
//! text by [`graph_text`]), and motion clips that move, switch and turn a
//! scene's things rather than bones ([`motion`]) — Unity's Animator and
//! Animation Clips (docs/modules.md).
//!
//! It plays what geometry holds — skeletons and clips as data — and
//! leaves the pose on the entity ([`Posed`]) for whoever draws. A clip's
//! track of another module's property (a sound's volume, particles' rate)
//! is handed to that module through [`motion::run_with`].

pub mod animator;
pub mod animgraph;
pub mod graph_text;
pub mod ik;
pub mod motion;

pub use animator::{advance_animations, Animator, Playing};
pub use scrap_geometry::animation::Posed;

/// This module's systems in the loop: characters' graphs pick their clips
/// and skeletons take the pose, in the fixed step. Motion clips are run by
/// the engine, which hands their sound and particle tracks to the modules
/// that own them ([`motion::run_with`]).
pub fn systems(player_loop: &mut scrap_core::player_loop::PlayerLoop) {
    systems_on(player_loop, |_| Box::new(ik::no_ground));
}

/// A probe of the ground under a point, made for one step from the world.
pub type GroundOf = fn(&hecs::World) -> ik::Probe;

/// [`systems`], the feet of skeletons with IK put on the ground `ground`
/// finds: the engine hands in the scene's colliders
/// ([`animator::advance_animations_on`]).
pub fn systems_on(player_loop: &mut scrap_core::player_loop::PlayerLoop, ground: GroundOf) {
    player_loop.add(
        scrap_core::player_loop::Phase::FixedUpdate,
        "animation",
        move |world, seconds| {
            animgraph::run_controllers(world);
            let feet = world.query::<&ik::Ik>().iter().any(|ik| !ik.feet.is_off());
            if feet {
                let probe = ground(world);
                animator::advance_animations_on(world, seconds, &*probe);
            } else {
                advance_animations(world, seconds);
            }
        },
    );
}

// The core and geometry, under the names this module's code knows them by.
#[allow(unused_imports)]
use scrap_core::{id, impl_parts, project, ron_edit, spelling, AssetLink, Transform};
#[allow(unused_imports)]
use scrap_geometry::animation;
use scrap_geometry::ik as geometry_ik;

/// The scene's lines, with this module's fields and geometry's beside the
/// core's.
#[allow(unused_imports)]
mod scene {
    pub use crate::motion::{AnimatorRef, BoneName};
    pub use scrap_core::scene::*;
    pub use scrap_geometry::line::*;
}

/// The world, with this module's components beside the core's.
#[allow(unused_imports)]
mod world {
    pub use crate::motion::OnBone;
    pub use scrap_core::world::*;
    pub use scrap_geometry::animation::Posed;
}

/// The core's archive with geometry's formats.
#[allow(unused_imports)]
mod asset {
    pub use scrap_core::asset::*;
    pub use scrap_geometry::mesh_asset::*;
}

/// The traits that read a line's fields.
#[allow(unused_imports)]
mod prelude {
    pub use crate::motion::AnimationLine;
    pub use scrap_geometry::line::{GeometryLine, GeometryOverride};
}

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = scrap_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "animation");
        let problems = manifest.part_problems(&crate::motion::part_kinds());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
