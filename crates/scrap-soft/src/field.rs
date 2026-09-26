//! The scene's distance field: a line's `distance_field`.
//! `distance_field: (size: (16, 6, 16))` on an entity bakes, once, how far
//! the nearest solid thing is at every point of a box round it — the
//! physics' primitives by their exact distance and its mesh colliders by
//! their triangles (docs/simulation.md, item 12). One field for everyone:
//! soft things, water, smoke and snow meet the meshes through it as one
//! more [`Obstacle::Field`], and the render darkens creases and softens the
//! sun's shadows by it.
//!
//! Baked from what stands still: what moves is left to its own collider.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use scrap_geometry::sdf::Sdf;
use serde::{Deserialize, Serialize};

use crate::obstacle::Obstacle;

/// A distance field, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistanceField {
    /// The box it covers, metres, round the entity.
    pub size: Vec3,
    /// Metres between its points.
    pub cell: f32,
    /// Whether the render draws occlusion and soft shadows by it.
    pub draw: bool,
}

impl Default for DistanceField {
    fn default() -> Self {
        Self { size: Vec3::new(16.0, 6.0, 16.0), cell: 0.1, draw: true }
    }
}

scrap_core::impl_parts! {
    DistanceField => "distance_field";
}

/// The distance field of a line, read off it.
pub trait DistanceFieldLine {
    fn distance_field(&self) -> Option<DistanceField>;
}

impl DistanceFieldLine for scrap_core::EntityDesc {
    fn distance_field(&self) -> Option<DistanceField> {
        self.part()
    }
}

impl DistanceFieldLine for scrap_core::scene::Override {
    fn distance_field(&self) -> Option<DistanceField> {
        self.part()
    }
}

/// The most points a field has along each way: the render's 3D texture
/// takes 256.
pub const MOST: usize = 256;
/// Metres either side of a surface a mesh is baked into the field: past
/// that the field says only "outside, at least this far".
pub const BAND: f32 = 1.0;

/// A field as it is baked: the component the facade fills.
#[derive(Debug, Clone)]
pub struct DistanceFieldState {
    pub field: DistanceField,
    pub baked: Option<Arc<Sdf>>,
}

impl DistanceFieldState {
    pub fn new(field: DistanceField) -> Self {
        Self { field, baked: None }
    }
}

/// A mesh in the world: its points and triangles.
pub struct WorldMesh<'a> {
    pub vertices: &'a [Vec3],
    pub triangles: &'a [[u32; 3]],
    pub placed: Mat4,
}

/// Bake `field`, round the entity at `placed`, from `obstacles` and
/// `meshes`.
pub fn bake(field: &DistanceField, placed: Mat4, obstacles: &[Obstacle], meshes: &[WorldMesh]) -> Sdf {
    let centre = placed.w_axis.truncate();
    let half = field.size.abs() * 0.5;
    // No finer than the texture takes.
    let cell = field.cell.max(half.max_element() * 2.0 / (MOST - 1) as f32);
    let (low, high) = (centre - half, centre + half);
    let mut sdf = Sdf::new(low, high, cell, BAND);
    for o in obstacles {
        if matches!(o, Obstacle::Field(_) | Obstacle::Sheet(_)) || !o.near(low - Vec3::splat(BAND), high + Vec3::splat(BAND)) {
            continue;
        }
        sdf.add(|p| o.distance(p));
    }
    for m in meshes {
        let world: Vec<Vec3> = m.vertices.iter().map(|v| m.placed.transform_point3(*v)).collect();
        // A mirrored mesh's triangles face in: turned back.
        let flip = m.placed.determinant() < 0.0;
        let triangles: Vec<[u32; 3]> = m.triangles.iter().map(|t| if flip { [t[0], t[2], t[1]] } else { *t }).collect();
        sdf.add_triangles(&world, &triangles, BAND.min(cell * 6.0));
    }
    sdf
}

/// The soft module's dresser for distance fields.
pub struct DistanceFieldDress;

impl scrap_core::world::Dress for DistanceFieldDress {
    fn parts(&self) -> &[&'static str] {
        &["distance_field"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        match line.distance_field() {
            Some(f) => {
                let _ = world.insert_one(entity, DistanceFieldState::new(f));
            }
            None => {
                scrap_core::world::take_off::<DistanceFieldState>(world, entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_baked_from_a_box_and_a_mesh_holds_a_ball_off_both() {
        let crate_box = Obstacle::Box { center: Vec3::new(-1.0, 0.5, 0.0), rotation: glam::Quat::IDENTITY, half: Vec3::splat(0.5) };
        let ball = scrap_geometry::builtin::sphere(0.5, 24, 12);
        let vertices: Vec<Vec3> = ball.vertices.iter().map(|v| Vec3::from_array(v.position)).collect();
        let triangles: Vec<[u32; 3]> = ball.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        let field = DistanceField { size: Vec3::new(4.0, 2.0, 2.0), cell: 0.05, draw: false };
        let meshes = [WorldMesh { vertices: &vertices, triangles: &triangles, placed: Mat4::from_translation(Vec3::new(1.0, 0.5, 0.0)) }];
        let sdf = Arc::new(bake(&field, Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)), &[crate_box, Obstacle::ground(0.0)], &meshes));
        let solid = Obstacle::Field(sdf.clone());
        assert!((solid.distance(Vec3::new(-1.0, 1.2, 0.0)) - 0.2).abs() < 0.03, "over the box");
        assert!((solid.distance(Vec3::new(1.0, 1.2, 0.0)) - 0.2).abs() < 0.03, "over the ball");
        assert!(solid.distance(Vec3::new(1.0, 0.5, 0.0)) < -0.25, "in the ball");
        // A drop let fall on the ball stops on it.
        let mut x = Vec3::new(1.02, 1.5, 0.0);
        for _ in 0..120 {
            let was = x;
            x.y -= 0.02;
            crate::obstacle::collide(&mut x, was, 0.02, 0.5, std::slice::from_ref(&solid));
        }
        assert!((x.y - 1.02).abs() < 0.04, "rests on top: {x}");
    }
}
