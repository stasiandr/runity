//! Curves through points on a line of a scene, and copies of a model set
//! along them: the spline module's fields, `spline` and `along`, as types,
//! and the reading of them off a line ([`SplineLine`]).

#[allow(unused_imports)]
use crate::prelude::*;
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use crate::defaults::*;
use crate::id::EntityId;
use crate::scene::{EntityDesc, Transform};

/// A curve through points, in its entity's space. Straight between the
/// points for now; the points are what is edited.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Spline {
    pub points: Vec<Vec3>,
    /// Back from the last point to the first: a fence round a field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
}

impl Spline {
    /// Where things go along it, `spacing` apart from the first point, and
    /// which way the curve runs there: `(position, direction)`.
    pub fn stations(&self, spacing: f32) -> Vec<(Vec3, Vec3)> {
        let spacing = spacing.max(0.05);
        let mut points = self.points.clone();
        if self.closed && points.len() > 2 {
            points.push(points[0]);
        }
        let mut out = Vec::new();
        // How far into the next segment the next station is.
        let mut carry = 0.0;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let length = (b - a).length();
            if length < 1e-4 {
                continue;
            }
            let direction = (b - a) / length;
            let mut at = carry;
            while at <= length + 1e-4 {
                out.push((a + direction * at, direction));
                at += spacing;
                if out.len() >= 10_000 {
                    return out;
                }
            }
            carry = at - length;
        }
        out
    }
}

/// What a spline carries: copies of a model, `spacing` apart, each turned
/// to face along the curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Along {
    pub model: crate::AssetLink,
    #[serde(default = "one_metre")]
    pub spacing: f32,
    /// The copies' scale.
    #[serde(default = "unit_scale", skip_serializing_if = "is_unit_scale")]
    pub scale: Vec3,
}

fn one_metre() -> f32 {
    1.0
}

fn unit_scale() -> Vec3 {
    Vec3::ONE
}

fn is_unit_scale(v: &Vec3) -> bool {
    *v == Vec3::ONE
}

impl Along {
    /// The copies for a spline, as children of the entity carrying both:
    /// named after the model and numbered, with IDs derived from the
    /// carrier's and their number, so the same fence gives the same IDs.
    pub fn grow(&self, carrier: EntityId, spline: &Spline) -> Vec<EntityDesc> {
        let name = self.model.trim_start_matches("builtin:").to_string();
        spline
            .stations(self.spacing)
            .into_iter()
            .enumerate()
            .map(|(i, (position, direction))| {
                let mut transform = Transform {
                    position,
                    scale: self.scale,
                    ..Default::default()
                };
                let yaw = (-direction.z).atan2(direction.x);
                transform.set_rotation(glam::Quat::from_rotation_y(yaw));
                let mut copy = EntityDesc {
                    id: carrier.within(EntityId::from_raw(i as u64 + 1)),
                    name: format!("{name} {}", i + 1),
                    transform,
                    ..Default::default()
                };
                copy.set_model(self.model.clone());
                copy
            })
            .collect()
    }
}

crate::impl_parts! {
    Spline => "spline";
    Along => "along";
}

/// A line's curve and what is set along it, read off it.
pub trait SplineLine {
    fn spline(&self) -> Option<Spline>;
    fn along(&self) -> Option<Along>;
}

impl SplineLine for EntityDesc {
    fn spline(&self) -> Option<Spline> {
        self.part()
    }
    fn along(&self) -> Option<Along> {
        self.part()
    }
}

/// What every spline in a scene carries, grown: copies of its model set
/// along it, as children of the line that has both. A pass over a scene
/// its prefabs are expanded in; the file keeps the spline and the spacing,
/// everything downstream sees copies.
pub fn grow_all(entities: &mut [EntityDesc]) {
    for desc in entities {
        if let (Some(spline), Some(along)) = (desc.spline(), desc.along()) {
            let grown = along.grow(desc.id, &spline);
            desc.children.extend(grown);
        }
        grow_all(&mut desc.children);
    }
}
