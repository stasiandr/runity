//! Turning the ground into triangles, in pieces.

use crate::heightmap::diagonal_is_forward;
use crate::terrain::Terrain;
use runity_math::{Vec2, Vec3, Vec4};
use runity_render::{Color, Mesh, Vertex};

/// A rectangle of cells, in grid coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub x: usize,
    pub z: usize,
    pub cells_x: usize,
    pub cells_z: usize,
}

/// One chunk's triangles, with the bound that decides whether to draw them.
#[derive(Clone, Debug)]
pub struct ChunkMesh {
    pub chunk: Chunk,
    pub mesh: Mesh,
    /// World-space bounding sphere, for the frustum test.
    pub centre: Vec3,
    pub radius: f32,
}

impl Terrain {
    /// Cut the ground into chunks of at most `cells` on a side.
    ///
    /// The last chunk in each direction is whatever is left over rather than a
    /// padded full-size one: a 100-cell map in chunks of 32 is 32, 32, 32, 4,
    /// and the 4 must not claim to cover 32 or the frustum test will keep it
    /// when it is off screen.
    pub fn chunks(&self, cells: usize) -> Vec<Chunk> {
        let cells = cells.max(1);
        let (total_x, total_z) = (self.heightmap().cells_x(), self.heightmap().cells_z());
        let mut chunks = Vec::new();
        let mut z = 0;
        while z < total_z {
            let mut x = 0;
            let depth = cells.min(total_z - z);
            while x < total_x {
                chunks.push(Chunk {
                    x,
                    z,
                    cells_x: cells.min(total_x - x),
                    cells_z: depth,
                });
                x += cells;
            }
            z += depth;
        }
        chunks
    }

    /// The whole ground as one mesh. Fine for a test or a small map; a real
    /// one wants [`chunks`](Terrain::chunks) so that most of it can be culled.
    pub fn mesh(&self) -> Mesh {
        self.chunk_mesh(Chunk {
            x: 0,
            z: 0,
            cells_x: self.heightmap().cells_x(),
            cells_z: self.heightmap().cells_z(),
        })
        .mesh
    }

    /// One chunk's triangles.
    ///
    /// Vertices are duplicated along chunk borders — two chunks each own a
    /// copy of the samples they share — but their **normals come from the
    /// heightmap**, not from the triangles in this chunk. That is what keeps
    /// the join invisible: face normals differ on either side of the border
    /// because each side sees different triangles, and the seam is a bright
    /// line across the hill that only appears once the map is big enough to
    /// need chunking.
    pub fn chunk_mesh(&self, chunk: Chunk) -> ChunkMesh {
        let heightmap = self.heightmap();
        let cells_x = chunk
            .cells_x
            .min(heightmap.cells_x().saturating_sub(chunk.x));
        let cells_z = chunk
            .cells_z
            .min(heightmap.cells_z().saturating_sub(chunk.z));
        let chunk = Chunk {
            cells_x,
            cells_z,
            ..chunk
        };

        let across = cells_x + 1;
        let mut vertices = Vec::with_capacity(across * (cells_z + 1));
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        for local_z in 0..=cells_z {
            for local_x in 0..=cells_x {
                let gx = chunk.x + local_x;
                let gz = chunk.z + local_z;
                let position = Vec3::new(
                    self.origin().x + gx as f32 * self.cell_size(),
                    self.origin().y + heightmap.height(gx, gz),
                    self.origin().z + gz as f32 * self.cell_size(),
                );
                low = low.min(position.y);
                high = high.max(position.y);
                let normal = heightmap.normal(gx as f32, gz as f32, self.cell_size());
                vertices.push(Vertex {
                    position,
                    normal,
                    // Tangent along +X, which is where u runs.
                    tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                    uv_density: self.cell_size(),
                    // One unit of uv per cell: a texture tiles with the grid,
                    // and the game scales it by editing the material.
                    uv: Vec2::new(gx as f32, gz as f32),
                    color: Color::WHITE,
                });
            }
        }

        let mut indices = Vec::with_capacity(cells_x * cells_z * 6);
        for local_z in 0..cells_z {
            for local_x in 0..cells_x {
                let a = (local_z * across + local_x) as u32;
                let b = a + 1;
                let c = a + across as u32;
                let d = c + 1;
                // The same split the height query and the ray cast use, keyed
                // on the *grid* position so that it does not change when the
                // chunk boundaries do.
                if diagonal_is_forward(chunk.x + local_x, chunk.z + local_z) {
                    indices.extend_from_slice(&[a, c, d, a, d, b]);
                } else {
                    indices.extend_from_slice(&[a, c, b, b, c, d]);
                }
            }
        }

        let mesh = Mesh::new(vertices, indices);
        let min = Vec3::new(
            self.origin().x + chunk.x as f32 * self.cell_size(),
            low.min(0.0),
            self.origin().z + chunk.z as f32 * self.cell_size(),
        );
        let max = Vec3::new(
            min.x + cells_x as f32 * self.cell_size(),
            high.max(0.0),
            min.z + cells_z as f32 * self.cell_size(),
        );
        let centre = (min + max) * 0.5;
        ChunkMesh {
            chunk,
            mesh,
            centre,
            radius: (max - centre).length(),
        }
    }
}
