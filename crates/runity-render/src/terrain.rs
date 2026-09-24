//! A terrain drawn: the ground near the camera as a grid raised on the
//! GPU, and its mesh elsewhere. The shape itself — dunes, heights, the
//! mesh — is the geometry module's ([`runity_geometry::terrain`]).

pub use runity_geometry::terrain::*;

use glam::Vec3;

use crate::asset::{MeshAsset, Vertex};

/// A terrain as a frame carries it: drawn near the camera as a finely
/// divided grid raised on the GPU — its dunes, and the ripples on them
/// in the geometry itself — and elsewhere, and in shadows, by its mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainSurface {
    /// The mesh it is drawn with elsewhere: the draw to stand in for.
    pub mesh: crate::render::MeshHandle,
    pub placed: glam::Mat4,
    pub terrain: Terrain,
}

/// Rings of the grid round the camera: each twice the last's spacing.
pub(crate) const CLIPMAP_LEVELS: u32 = 9;
/// Cells across each ring.
pub(crate) const CLIPMAP_CELLS: i32 = 128;
/// The finest spacing, metres: a sand ripple is fourteen centimetres.
pub(crate) const CLIPMAP_FINEST: f32 = 0.03;
/// With mesh shaders, the patches of a ring: 18 by 18, each 8 by 8 cells
/// (terrain_mesh.wgsl), reaching as far as the grid's rings do, the middle
/// ones of every outer ring left to the ring inside.
pub(crate) const PATCHES_PER_RING: u32 = 18 * 18;

/// The grid, in its own terms: each vertex its cell `(x, ring, z)`; the
/// vertex shader places and raises it. The innermost ring is whole, each
/// outer one a square with a hole where the finer one lies.
pub(crate) fn clipmap_mesh() -> MeshAsset {
    let n = CLIPMAP_CELLS;
    // Each ring reaches a cell of the next ring's spacing past its own
    // half-width: the next ring's hole, snapped to its coarser grid, may
    // sit a cell off, and this covers it wherever it lands.
    let half = n / 2;
    let reach = half + 2;
    let stride = (2 * reach + 1) as u32;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for level in 0..CLIPMAP_LEVELS {
        let base = vertices.len() as u32;
        for z in -reach..=reach {
            for x in -reach..=reach {
                vertices.push(Vertex {
                    position: [x as f32, level as f32, z as f32],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                });
            }
        }
        let inner = half / 2;
        for z in -reach..reach {
            for x in -reach..reach {
                if level > 0 && (-inner..inner).contains(&x) && (-inner..inner).contains(&z) {
                    continue;
                }
                let a = base + ((z + reach) as u32) * stride + (x + reach) as u32;
                let (b, c, d) = (a + 1, a + stride, a + stride + 1);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    crate::builtin::finish("clipmap", vertices, indices)
}

/// A terrain on an entity: its settings, and the mesh drawn once made.
#[derive(Debug, Clone)]
pub struct Relief {
    pub terrain: Terrain,
    /// Its crests, in its own space: found once.
    pub crests: Vec<Vec3>,
    /// The uploaded mesh, and the settings it was made from.
    made: Option<(crate::render::MeshHandle, Terrain)>,
}

impl Relief {
    /// The mesh it is drawn with, once made.
    pub fn mesh(&self) -> Option<crate::render::MeshHandle> {
        self.made.map(|(handle, _)| handle)
    }

    pub fn new(terrain: Terrain) -> Self {
        Self {
            crests: terrain.crests(),
            terrain,
            made: None,
        }
    }
}

/// Make and upload the mesh of every terrain that has none, or whose
/// settings changed since: call it before drawing, as the material maps
/// are. Its entity then draws it like any model.
pub fn upload_terrains(
    world: &mut hecs::World,
    gpu: &crate::gpu::Gpu,
    renderer: &mut crate::render::Renderer,
) {
    let mut drawn = Vec::new();
    for (entity, relief) in world.query_mut::<(hecs::Entity, &mut Relief)>() {
        let current = relief.made.filter(|(_, t)| *t == relief.terrain);
        let handle = match current {
            Some((handle, _)) => handle,
            None => {
                let handle = renderer.upload_mesh_owned(gpu, &relief.terrain.mesh());
                relief.made = Some((handle, relief.terrain));
                handle
            }
        };
        drawn.push((entity, handle));
    }
    for (entity, handle) in drawn {
        let has = world
            .get::<&crate::world::Model>(entity)
            .is_ok_and(|m| m.0 == handle);
        if !has {
            let _ = world.insert_one(entity, crate::world::Model(handle));
        }
    }
}
