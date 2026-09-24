//! `.scrbrush`: a blockout solid made of brushes, each added or cut out
//! in order — Hammer's and TrenchBroom's brushes, Godot's CSG.
//!
//! ```text
//! (
//!     brushes: [
//!         (shape: Box, position: (0.0, 1.5, 0.0), scale: (8.0, 3.0, 0.25)),
//!         (op: Subtract, shape: Box, position: (-2.0, 1.0, 0.0), scale: (1.0, 2.0, 1.0)),
//!         (op: Subtract, shape: Cylinder, position: (2.0, 1.8, 0.0), rotation_deg: (90.0, 0.0, 0.0), scale: (0.8, 1.0, 0.8)),
//!     ],
//! )
//! ```
//!
//! A brush is a convex shape — `Box`, `Ramp` (high at the back, −z),
//! `Cylinder` (`sides`, 24 when left out) or `Stairs` (a box a step) —
//! a unit in size and centred, like the `builtin:` shapes, placed by
//! `position`, `rotation_deg` and `scale` as a scene places an entity.
//! `op: Subtract` cuts it out of everything added before it; a brush added
//! after a cut fills it again. One brush is one line: a diff says "a
//! window was cut here", and two people adding brushes merge.
//!
//! Imported into an ordinary mesh named after the file, where the brushes
//! say, and placed like any model — `collider: Model, body: Static`. The
//! faces two brushes press together are dropped, so only the outside is
//! drawn and collided with. Texture coordinates are metres laid on by the
//! axis each face looks along most: a material tiles the same on every
//! wall, and a grid lines up across them.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use scrap::asset::{Bounds, MeshAsset, Submesh, Vertex};
use scrap::glam::{Mat4, Quat, Vec3};
use scrap::solid::{self, Solid};
use serde::{Deserialize, Serialize};

use crate::ImportSettings;

/// What a `.scrbrush` says: its brushes, in order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BrushSource {
    pub brushes: Vec<Brush>,
}

/// Whether a brush adds to the solid or cuts from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Op {
    #[default]
    Add,
    Subtract,
}

/// A brush's shape, a unit in size and centred on its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    Box,
    /// High at the back (−z), like `builtin:ramp`.
    Ramp,
    /// Standing on y, `sides` sided.
    Cylinder,
    /// `builtin:stairs`: four steps rising toward the back.
    Stairs,
}

impl Shape {
    /// The shape a `builtin:` model is, when a brush can be one.
    pub fn of_builtin(model: &str) -> Option<Shape> {
        Some(match model {
            "builtin:cube" => Shape::Box,
            "builtin:ramp" => Shape::Ramp,
            "builtin:cylinder" => Shape::Cylinder,
            "builtin:stairs" => Shape::Stairs,
            _ => return None,
        })
    }
}

/// `builtin:cylinder`'s sides, so a carved cylinder looks as it did.
pub const CYLINDER_SIDES: u32 = 24;

/// One brush.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    #[serde(default, skip_serializing_if = "is_add")]
    pub op: Op,
    pub shape: Shape,
    #[serde(default = "zero")]
    pub position: Vec3,
    #[serde(default = "zero")]
    pub rotation_deg: Vec3,
    #[serde(default = "one")]
    pub scale: Vec3,
    /// A cylinder's sides; 24, like `builtin:cylinder`, when not written.
    #[serde(default = "cylinder_sides", skip_serializing_if = "is_cylinder_sides")]
    pub sides: u32,
}

fn is_add(op: &Op) -> bool {
    *op == Op::Add
}

fn cylinder_sides() -> u32 {
    CYLINDER_SIDES
}

fn is_cylinder_sides(sides: &u32) -> bool {
    *sides == CYLINDER_SIDES
}

fn zero() -> Vec3 {
    Vec3::ZERO
}

fn one() -> Vec3 {
    Vec3::ONE
}

impl Brush {
    /// A box from `low` to `high`.
    pub fn cuboid(op: Op, low: Vec3, high: Vec3) -> Self {
        Brush {
            op,
            shape: Shape::Box,
            position: (low + high) * 0.5,
            rotation_deg: Vec3::ZERO,
            scale: high - low,
            sides: CYLINDER_SIDES,
        }
    }

    /// Where it is: from its unit shape's space to the solid's.
    pub fn matrix(&self) -> Mat4 {
        let r = self.rotation_deg * (std::f32::consts::PI / 180.0);
        Mat4::from_scale_rotation_translation(
            self.scale,
            Quat::from_euler(scrap::glam::EulerRot::YXZ, r.y, r.x, r.z),
            self.position,
        )
    }

    /// The convex solids it is, in the solid's space: one, or a step each.
    pub fn solids(&self) -> Vec<Solid> {
        let h = Vec3::splat(0.5);
        let unit = match self.shape {
            Shape::Box => vec![Solid::cuboid(-h, h)],
            Shape::Ramp => vec![Solid::wedge(-h, h)],
            Shape::Cylinder => vec![Solid::prism(self.sides, 0.5, 0.5)],
            Shape::Stairs => {
                let steps = scrap::builtin::STAIRS;
                let depth = 1.0 / steps as f32;
                (0..steps)
                    .map(|i| {
                        let front = 0.5 - i as f32 * depth;
                        let top = -0.5 + (i + 1) as f32 * depth;
                        Solid::cuboid(
                            Vec3::new(-0.5, -0.5, front - depth),
                            Vec3::new(0.5, top, front),
                        )
                    })
                    .collect()
            }
        };
        let m = self.matrix();
        unit.iter().map(|s| s.transformed(m)).collect()
    }

    /// Its line in the file: only what differs from a unit box added at
    /// the origin.
    pub fn to_line(&self) -> String {
        let v = |v: Vec3| format!("({:?}, {:?}, {:?})", v.x, v.y, v.z);
        let mut parts = Vec::new();
        if self.op == Op::Subtract {
            parts.push("op: Subtract".to_string());
        }
        parts.push(format!("shape: {:?}", self.shape));
        if self.position != Vec3::ZERO {
            parts.push(format!("position: {}", v(self.position)));
        }
        if self.rotation_deg != Vec3::ZERO {
            parts.push(format!("rotation_deg: {}", v(self.rotation_deg)));
        }
        if self.scale != Vec3::ONE {
            parts.push(format!("scale: {}", v(self.scale)));
        }
        if self.sides != CYLINDER_SIDES {
            parts.push(format!("sides: {}", self.sides));
        }
        format!("({})", parts.join(", "))
    }

    fn check(&self, k: usize) -> Result<()> {
        let finite = |v: Vec3| v.is_finite();
        ensure!(
            finite(self.position) && finite(self.rotation_deg),
            "brush {k}: its position and rotation are numbers"
        );
        ensure!(
            self.scale.is_finite() && self.scale.min_element() > 0.0,
            "brush {k}: its scale is how big it is along each axis, each more than zero, not {:?}",
            self.scale.to_array()
        );
        ensure!(
            (3..=64).contains(&self.sides),
            "brush {k}: a cylinder has 3 to 64 sides, not {}",
            self.sides
        );
        Ok(())
    }
}

impl BrushSource {
    /// The file's text: a line a brush, as a person would write it.
    pub fn to_text(&self) -> String {
        let lines: String = self
            .brushes
            .iter()
            .map(|b| format!("        {},\n", b.to_line()))
            .collect();
        format!("(\n    brushes: [\n{lines}    ],\n)\n")
    }

    /// The convex pieces the brushes leave, not overlapping.
    pub fn pieces(&self) -> Result<Vec<Solid>> {
        ensure!(
            !self.brushes.is_empty(),
            "there are no brushes: add a Box to start from"
        );
        let mut pieces = Vec::new();
        for (k, brush) in self.brushes.iter().enumerate() {
            brush.check(k)?;
            for s in brush.solids() {
                pieces = match brush.op {
                    Op::Add => solid::add(pieces, s),
                    Op::Subtract => solid::subtract(pieces, &s),
                };
            }
        }
        if pieces.is_empty() {
            bail!("the brushes leave nothing: everything added is cut away");
        }
        Ok(pieces)
    }
}

/// The vertices and triangles of the solid the brushes make.
pub fn build(source: &BrushSource) -> Result<(Vec<Vertex>, Vec<u32>)> {
    let pieces = source.pieces()?;
    Ok(solid::mesh_of(&solid::boundary(&pieces)))
}

pub fn mesh_from_brushes(path: &Path, settings: &ImportSettings) -> Result<MeshAsset> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let source: BrushSource =
        ron::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "brush".into());
    let (vertices, indices) = build(&source).with_context(|| format!("{}", path.display()))?;
    Ok(MeshAsset {
        id: settings.asset_id(),
        name,
        bounds: Bounds::of(&vertices),
        submeshes: vec![Submesh {
            first_index: 0,
            index_count: indices.len() as u32,
            material: None,
        }],
        vertices,
        indices,
        skin: None,
        look: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(vertices: &[Vertex], indices: &[u32]) -> f32 {
        indices
            .chunks(3)
            .map(|t| {
                let [p, q, r] =
                    [t[0], t[1], t[2]].map(|i| Vec3::from(vertices[i as usize].position));
                p.dot(q.cross(r)) / 6.0
            })
            .sum()
    }

    fn wall_with_door() -> BrushSource {
        BrushSource {
            brushes: vec![
                Brush::cuboid(
                    Op::Add,
                    Vec3::new(-2.0, 0.0, -0.125),
                    Vec3::new(2.0, 3.0, 0.125),
                ),
                Brush::cuboid(
                    Op::Subtract,
                    Vec3::new(-0.5, -0.5, -1.0),
                    Vec3::new(0.5, 2.0, 1.0),
                ),
            ],
        }
    }

    #[test]
    fn the_text_reads_back_and_says_only_what_differs() {
        let source = wall_with_door();
        let text = source.to_text();
        assert!(
            text.contains(
                "(op: Subtract, shape: Box, position: (0.0, 0.75, 0.0), scale: (1.0, 2.5, 2.0))"
            ),
            "{text}"
        );
        assert!(!text.contains("rotation_deg"), "{text}");
        let back: BrushSource = ron::from_str(&text).unwrap();
        assert_eq!(back, source);
        // Written by hand, with only a shape.
        let hand: BrushSource = ron::from_str("(brushes: [(shape: Cylinder, sides: 8)])").unwrap();
        assert_eq!(hand.brushes[0].scale, Vec3::ONE);
        assert_eq!(hand.brushes[0].op, Op::Add);
        assert_eq!(hand.brushes[0].sides, 8);
        assert_eq!(hand.brushes[0].to_line(), "(shape: Cylinder, sides: 8)");
    }

    #[test]
    fn a_wall_with_a_doorway_is_a_closed_solid_with_a_hole() {
        let (vertices, indices) = build(&wall_with_door()).unwrap();
        assert!((volume(&vertices, &indices) - 2.5).abs() < 1e-3);
        // Nothing of it in the doorway: a ray along z through the middle
        // of the door meets no triangle.
        let o = Vec3::new(0.0, 1.0, -1.0);
        let hit = indices.chunks(3).any(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(vertices[i as usize].position));
            // Where the line x = 0, y = 1 crosses the triangle's plane.
            let n = (b - a).cross(c - a);
            if n.z.abs() < 1e-6 {
                return false;
            }
            let t_ = n.dot(a - o) / n.z;
            let p = o + Vec3::Z * t_;
            let inside = |u: Vec3, v: Vec3| (v - u).cross(p - u).dot(n) >= 0.0;
            inside(a, b) && inside(b, c) && inside(c, a)
        });
        assert!(!hit, "the doorway is open");
        // Pictures in metres: the wall's front spans 4 by 3 of them.
        let front: Vec<[f32; 2]> = vertices
            .iter()
            .filter(|v| v.normal[2] > 0.9)
            .map(|v| v.uv)
            .collect();
        let span = |i: usize| {
            front.iter().map(|u| u[i]).fold(f32::MIN, f32::max)
                - front.iter().map(|u| u[i]).fold(f32::MAX, f32::min)
        };
        assert!((span(0) - 4.0).abs() < 1e-4 && (span(1) - 3.0).abs() < 1e-4);
    }

    #[test]
    fn every_shape_builds_and_turns() {
        for shape in [Shape::Box, Shape::Ramp, Shape::Cylinder, Shape::Stairs] {
            let source = BrushSource {
                brushes: vec![Brush {
                    op: Op::Add,
                    shape,
                    position: Vec3::new(1.0, 2.0, 3.0),
                    rotation_deg: Vec3::new(0.0, 90.0, 0.0),
                    scale: Vec3::new(2.0, 1.0, 1.0),
                    sides: CYLINDER_SIDES,
                }],
            };
            let (vertices, indices) = build(&source).unwrap();
            let v = volume(&vertices, &indices);
            let want = match shape {
                Shape::Box => 2.0,
                Shape::Ramp => 1.0,
                Shape::Cylinder => 2.0 * 0.5 * 24.0 * 0.25 * (std::f32::consts::TAU / 24.0).sin(),
                Shape::Stairs => 2.0 * (1.0 + 2.0 + 3.0 + 4.0) / 16.0,
            };
            assert!((v - want).abs() < 1e-3, "{shape:?}: {v} ≠ {want}");
        }
    }

    #[test]
    fn what_cannot_be_built_says_why() {
        let e = build(&BrushSource::default()).unwrap_err().to_string();
        assert!(e.contains("no brushes"), "{e}");
        let mut all_cut = wall_with_door();
        all_cut.brushes.push(Brush::cuboid(
            Op::Subtract,
            Vec3::splat(-10.0),
            Vec3::splat(10.0),
        ));
        let e = build(&all_cut).unwrap_err().to_string();
        assert!(e.contains("leave nothing"), "{e}");
        let mut flat = wall_with_door();
        flat.brushes[1].scale.y = 0.0;
        let e = build(&flat).unwrap_err().to_string();
        assert!(e.contains("brush 1") && e.contains("scale"), "{e}");
    }
}
