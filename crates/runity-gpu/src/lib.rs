//! The GPU (DNA, "Видеокарта — модуль `gpu`"): the device and its queue,
//! the surface a host hands the engine, and offscreen targets. The render
//! module draws the 3D scene on it and the UI module draws the interface;
//! the editor always has it, a headless server never does.

pub mod gpu;
pub mod surface;

pub use gpu::{Gpu, GpuError, OffscreenTarget};
pub use surface::{AcquiredFrame, SurfaceError};

/// This module's manifest (`module.ron`): its name, what it stands on,
/// the fields of a line it reads.
pub const MANIFEST: &str = include_str!("../module.ron");

#[cfg(test)]
mod manifest {
    #[test]
    fn the_manifest_reads_and_lists_the_fields_the_module_reads() {
        let manifest = runity_core::module::Manifest::parse(super::MANIFEST).unwrap();
        assert_eq!(manifest.name, "gpu");
        let problems = manifest.part_problems(&Vec::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
