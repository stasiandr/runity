//! What soft things bump into: simple solid shapes in the world — the
//! ground, a post, a crate, a person's capsule. The module does not know
//! the physics' colliders; the facade hands them over as these
//! (docs/simulation.md), and a test or a server hands its own.

use glam::{Mat4, Quat, Vec3};

/// A solid shape, in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Obstacle {
    /// Everything below the plane through `point` facing `normal` is
    /// solid: the ground.
    Plane { point: Vec3, normal: Vec3 },
    Sphere { center: Vec3, radius: f32 },
    /// A segment from `a` to `b` fattened by `radius`: a person, a post.
    Capsule { a: Vec3, b: Vec3, radius: f32 },
    /// A box turned by `rotation`, `half` its size on each of its axes.
    Box { center: Vec3, rotation: Quat, half: Vec3 },
}

impl Obstacle {
    /// Level ground at height `y`.
    pub fn ground(y: f32) -> Self {
        Obstacle::Plane {
            point: Vec3::new(0.0, y, 0.0),
            normal: Vec3::Y,
        }
    }

    /// A box of half size `half` around `center` in the space of `placed`
    /// (scale, turn and place), in the world.
    pub fn placed_box(placed: Mat4, center: Vec3, half: Vec3) -> Self {
        let (scale, rotation, translation) = placed.to_scale_rotation_translation();
        Obstacle::Box {
            center: translation + rotation * (center * scale),
            rotation,
            half: (half * scale).abs(),
        }
    }

    /// Whether it may touch anything within the box from `low` to `high`:
    /// a cheap test that leaves most of a scene out before each point is
    /// tried against what is left.
    pub fn near(&self, low: Vec3, high: Vec3) -> bool {
        match (*self, self.bounds()) {
            // A plane reaches everywhere below it.
            (Obstacle::Plane { point, normal }, _) => {
                let n = normal.normalize_or(Vec3::Y);
                let lowest = Vec3::select(n.cmpge(Vec3::ZERO), low, high);
                (lowest - point).dot(n) <= 0.0
            }
            (_, Some((a, b))) => a.cmple(high).all() && b.cmpge(low).all(),
            (_, None) => true,
        }
    }

    /// A wedge — `builtin:ramp` in the space of `placed`, high at its back
    /// (−z) — as a box lying under its slope: the slope is the box's top,
    /// and it reaches into the wedge as deep as the wedge is thick there.
    pub fn placed_ramp(placed: Mat4, half: Vec3) -> Self {
        let at = |x: f32, y: f32, z: f32| placed.transform_point3(Vec3::new(x, y, z) * half * 2.0);
        let (low, high) = ((at(-0.5, -0.5, 0.5) + at(0.5, -0.5, 0.5)) * 0.5, (at(-0.5, 0.5, -0.5) + at(0.5, 0.5, -0.5)) * 0.5);
        let across = at(0.5, -0.5, 0.5) - at(-0.5, -0.5, 0.5);
        let along = high - low;
        let up = across.cross(along).normalize_or(Vec3::Y);
        let up = if up.y < 0.0 { -up } else { up };
        let x = across.normalize_or(Vec3::X);
        let z = x.cross(up);
        // As thick as the wedge's height over its slope, where it is thickest.
        let (height, depth) = ((at(0.0, 0.5, -0.5) - at(0.0, -0.5, -0.5)).length(), (at(0.0, -0.5, 0.5) - at(0.0, -0.5, -0.5)).length());
        let thick = height * depth / along.length().max(1e-6);
        Obstacle::Box {
            center: (low + high) * 0.5 - up * (thick * 0.5),
            rotation: Quat::from_mat3(&glam::Mat3::from_cols(x, up, z)),
            half: Vec3::new(across.length() * 0.5, thick * 0.5, along.length() * 0.5),
        }
    }

    /// Where a ball of `radius` at `p` pokes into it: the way out, and how
    /// deep. `None` when it does not touch.
    pub fn contact(&self, p: Vec3, radius: f32) -> Option<(Vec3, f32)> {
        match *self {
            Obstacle::Plane { point, normal } => {
                let n = normal.normalize_or(Vec3::Y);
                let depth = radius - (p - point).dot(n);
                (depth > 0.0).then_some((n, depth))
            }
            Obstacle::Sphere { center, radius: r } => ball(p - center, r + radius),
            Obstacle::Capsule { a, b, radius: r } => {
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                ball(p - (a + ab * t), r + radius)
            }
            Obstacle::Box { center, rotation, half } => {
                let local = rotation.inverse() * (p - center);
                let outside = local.abs() - half;
                if outside.max_element() > radius {
                    return None;
                }
                if outside.max_element() > 0.0 {
                    // Outside the box: out from its nearest point.
                    let nearest = local.clamp(-half, half);
                    let (n, depth) = ball(local - nearest, radius)?;
                    return Some((rotation * n, depth));
                }
                // Inside: out through the nearest face.
                let axis = if outside.x > outside.y && outside.x > outside.z {
                    Vec3::X * local.x.signum()
                } else if outside.y > outside.z {
                    Vec3::Y * local.y.signum()
                } else {
                    Vec3::Z * local.z.signum()
                };
                Some((rotation * axis, radius - outside.max_element()))
            }
        }
    }
}

/// A scene's obstacles, sorted into a grid once a step so that each soft
/// thing finds those near it without looking at the rest: a thousand ropes
/// among a thousand crates is a thousand short lists, not a million tests
/// (DNA, postulate 6).
#[derive(Debug, Clone, Default)]
pub struct Obstacles {
    all: Vec<Obstacle>,
    /// Those too big for the grid — the ground — tried by everything.
    everywhere: Vec<u32>,
    cells: std::collections::HashMap<[i32; 3], Vec<u32>>,
}

/// Metres a side of a grid cell.
const CELL: f32 = 4.0;
/// Cells an obstacle may cover and still be put in each; bigger ones are
/// tried by everything.
const MOST_CELLS: i64 = 64;

fn cell(p: Vec3) -> [i32; 3] {
    let c = (p / CELL).floor();
    [c.x as i32, c.y as i32, c.z as i32]
}

impl Obstacles {
    pub fn new(all: Vec<Obstacle>) -> Self {
        let mut out = Obstacles {
            all,
            ..Default::default()
        };
        for (i, obstacle) in out.all.iter().enumerate() {
            let Some((low, high)) = obstacle.bounds() else {
                out.everywhere.push(i as u32);
                continue;
            };
            let (a, b) = (cell(low), cell(high));
            let count = (0..3).map(|k| (b[k] - a[k] + 1) as i64).product::<i64>();
            if count > MOST_CELLS {
                out.everywhere.push(i as u32);
                continue;
            }
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        out.cells.entry([x, y, z]).or_default().push(i as u32);
                    }
                }
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// Those that may touch anything from `low` to `high`, into `out`.
    pub fn near(&self, low: Vec3, high: Vec3, out: &mut Vec<Obstacle>) {
        out.clear();
        let (a, b) = (cell(low), cell(high));
        let mut found: Vec<u32> = self.everywhere.clone();
        let count = (0..3).map(|k| (b[k] - a[k] + 1) as i64).product::<i64>();
        if count > MOST_CELLS * 8 {
            // Bigger than the grid is worth: everything, tested by bounds.
            found.extend(0..self.all.len() as u32);
        } else {
            for x in a[0]..=b[0] {
                for y in a[1]..=b[1] {
                    for z in a[2]..=b[2] {
                        if let Some(here) = self.cells.get(&[x, y, z]) {
                            found.extend_from_slice(here);
                        }
                    }
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        out.extend(
            found
                .into_iter()
                .map(|i| self.all[i as usize])
                .filter(|o| o.near(low, high)),
        );
    }
}

impl Obstacle {
    /// The box it lies in; `None` for a plane, which has no end.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        match *self {
            Obstacle::Plane { .. } => None,
            Obstacle::Sphere { center, radius } => Some((center - Vec3::splat(radius), center + Vec3::splat(radius))),
            Obstacle::Capsule { a, b, radius } => Some((a.min(b) - Vec3::splat(radius), a.max(b) + Vec3::splat(radius))),
            Obstacle::Box { center, rotation, half } => {
                let m = glam::Mat3::from_quat(rotation);
                let reach = m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
                Some((center - reach, center + reach))
            }
        }
    }
}

/// A ball of `reach` round the origin, and a point `d` from it.
fn ball(d: Vec3, reach: f32) -> Option<(Vec3, f32)> {
    let far = d.length();
    if far >= reach {
        return None;
    }
    let n = if far > 1e-6 { d / far } else { Vec3::Y };
    Some((n, reach - far))
}

/// Push a point of `radius` at `x`, which was at `was` a step ago, out of
/// every obstacle, with friction: along the surface it keeps only what
/// `friction` lets it slide.
pub fn collide(x: &mut Vec3, was: Vec3, radius: f32, friction: f32, obstacles: &[Obstacle]) {
    for obstacle in obstacles {
        let Some((n, depth)) = obstacle.contact(*x, radius) else {
            continue;
        };
        *x += n * depth;
        // Friction: of the way it moved along the surface this step, as
        // much is taken back as the push out allows (Coulomb, as a
        // position: static below the cone, sliding above it).
        let moved = *x - was;
        let along = moved - n * moved.dot(n);
        let slide = along.length();
        if slide > 1e-9 {
            let hold = (friction * depth / slide).min(1.0);
            *x -= along * hold;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_shape_pushes_a_ball_out_the_nearest_way() {
        let ground = Obstacle::ground(0.0);
        let (n, d) = ground.contact(Vec3::new(3.0, 0.05, 1.0), 0.1).unwrap();
        assert_eq!(n, Vec3::Y);
        assert!((d - 0.05).abs() < 1e-6);
        assert!(ground.contact(Vec3::new(0.0, 0.2, 0.0), 0.1).is_none());

        let post = Obstacle::Capsule {
            a: Vec3::ZERO,
            b: Vec3::new(0.0, 2.0, 0.0),
            radius: 0.1,
        };
        let (n, d) = post.contact(Vec3::new(0.15, 1.0, 0.0), 0.1).unwrap();
        assert!(n.distance(Vec3::X) < 1e-5 && (d - 0.05).abs() < 1e-5);

        let crate_ = Obstacle::placed_box(
            Mat4::from_scale_rotation_translation(
                Vec3::splat(2.0),
                Quat::from_rotation_y(0.3),
                Vec3::new(0.0, 1.0, 0.0),
            ),
            Vec3::ZERO,
            Vec3::splat(0.5),
        );
        // Just inside its top: out through the top.
        let (n, d) = crate_.contact(Vec3::new(0.1, 1.95, 0.0), 0.0).unwrap();
        assert!(n.distance(Vec3::Y) < 1e-5, "{n}");
        assert!((d - 0.05).abs() < 1e-4, "{d}");
        // Off its corner, within reach: out from the corner.
        assert!(crate_.contact(Vec3::new(0.0, 2.05, 0.0), 0.1).is_some());
        assert!(crate_.contact(Vec3::new(0.0, 2.3, 0.0), 0.1).is_none());
    }

    #[test]
    fn a_ramp_is_solid_under_its_slope_and_not_over_it() {
        // builtin:ramp two metres deep and high: its slope at 45°, rising to
        // the back (−z).
        let ramp = Obstacle::placed_ramp(Mat4::from_scale(Vec3::splat(2.0)), Vec3::splat(0.5));
        // Over the middle of the slope, a little above: clear.
        assert!(ramp.contact(Vec3::new(0.0, 0.1, 0.0), 0.05).is_none());
        // Just under it: pushed out up the slope's normal.
        let (n, _) = ramp.contact(Vec3::new(0.0, -0.05, 0.0), 0.05).unwrap();
        assert!(n.distance(Vec3::new(0.0, 1.0, 1.0).normalize()) < 1e-3, "{n}");
        // Where a box standing in for it would be solid, over the front: clear.
        assert!(ramp.contact(Vec3::new(0.0, 0.5, 0.8), 0.05).is_none());
    }

    #[test]
    fn only_what_is_near_a_box_is_kept() {
        let (low, high) = (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0));
        assert!(Obstacle::ground(0.0).near(low, high));
        assert!(!Obstacle::ground(-0.5).near(low, high));
        let ball = |x: f32| Obstacle::Sphere { center: Vec3::new(x, 1.0, 0.0), radius: 0.5 };
        assert!(ball(1.4).near(low, high));
        assert!(!ball(1.6).near(low, high));
        let turned = Obstacle::Box { center: Vec3::new(2.3, 1.0, 0.0), rotation: Quat::from_rotation_y(0.785), half: Vec3::splat(1.0) };
        assert!(turned.near(low, high), "its corner reaches in");
    }

    #[test]
    fn the_grid_finds_what_is_near_and_the_ground_everywhere() {
        let crates: Vec<Obstacle> = (0..100)
            .map(|i| Obstacle::Box {
                center: Vec3::new(i as f32 * 3.0, 0.5, 0.0),
                rotation: Quat::IDENTITY,
                half: Vec3::splat(0.5),
            })
            .chain([Obstacle::ground(0.0)])
            .collect();
        let grid = Obstacles::new(crates);
        let mut near = Vec::new();
        grid.near(Vec3::new(29.0, 0.0, -1.0), Vec3::new(31.0, 2.0, 1.0), &mut near);
        // The crate at 30 m, and the ground.
        assert_eq!(near.len(), 2, "{near:?}");
        grid.near(Vec3::new(-50.0, 5.0, -1.0), Vec3::new(-40.0, 6.0, 1.0), &mut near);
        assert!(near.is_empty(), "{near:?}");
    }

    #[test]
    fn friction_holds_a_point_that_presses_and_lets_one_that_skims_slide() {
        let ground = [Obstacle::ground(0.0)];
        // Pressed hard into the ground, moving a little along it: held.
        let mut x = Vec3::new(0.01, -0.2, 0.0);
        collide(&mut x, Vec3::new(0.0, 0.0, 0.0), 0.0, 0.5, &ground);
        assert!(x.distance(Vec3::ZERO) < 1e-6, "{x}");
        // Barely touching, moving fast along it: slides nearly all the way.
        let mut x = Vec3::new(1.0, -0.001, 0.0);
        collide(&mut x, Vec3::new(0.0, 0.0, 0.0), 0.0, 0.5, &ground);
        assert!(x.x > 0.99 && x.y.abs() < 1e-6, "{x}");
    }
}
