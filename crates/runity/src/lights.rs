//! Point and spot lights, as URP's Forward+ has them: as many as a scene
//! places, each pixel shading only the ones that reach it, and the nearest
//! of them casting shadows.
//!
//! **Clusters.** The view is cut into a grid — [`TILES_X`] × [`TILES_Y`]
//! across the screen, [`SLICES`] deep, the slices thinner near the camera
//! where things are big on screen — and each light is listed in every cell
//! its sphere touches. A pixel finds its cell from where it is on screen
//! and how deep, and loops over that cell's list alone: a hundred torches
//! down a corridor cost each pixel the two or three near it, not a hundred.
//! The lists are made on the CPU each frame: a light's sphere is boxed, the
//! box projected, and the cells under it taken — conservative, so a cell
//! may list a light that turns out not to reach it, never the other way.
//!
//! **Shadows.** A light casts them unless told not to (`shadows: false`),
//! as URP's Soft Shadows are on for a light by default. They take room in
//! one array of maps, [`SHADOW_LAYERS`] of them: a spot one, a point six —
//! a face of a cube each. The nearest lights to the camera that the view
//! sees get theirs first, and when the room runs out the rest shine
//! unshadowed, rather than every lamp's shadow getting blurrier.

use glam::{Mat4, Vec3, Vec4};

use crate::render::{Camera, PointLight};

/// Cells across the screen.
pub const TILES_X: u32 = 16;
/// Cells down it.
pub const TILES_Y: u32 = 9;
/// Cells deep, from the near plane to the far one.
pub const SLICES: u32 = 24;
/// The most lights a frame shades with; the nearest the camera sees win.
pub const MAX_LIGHTS: usize = 512;
/// Shadow maps for lights: a spot takes one, a point six.
pub const SHADOW_LAYERS: usize = 24;
/// Where a light's shadow map starts, in metres: nothing nearer the lamp
/// than this shadows it.
pub const SHADOW_NEAR: f32 = 0.05;

/// One light as the shader reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuLight {
    /// Where, and how far it reaches.
    pub position_range: [f32; 4],
    /// Its colour times its intensity; `w` its first shadow map, or −1.
    pub color_shadow: [f32; 4],
    /// For a spot, which way it shines and the cosine of half its cone;
    /// −2 for every way.
    pub spot: [f32; 4],
}

/// What a frame's lights come to: the ones shaded, each cell's share of
/// them, and the shadow maps they get.
pub(crate) struct Clustered {
    pub lights: Vec<GpuLight>,
    /// Per cell: where its list starts in `indices`, and how long it is.
    pub cells: Vec<[u32; 2]>,
    pub indices: Vec<u32>,
    /// Per shadow map, the light's view of the world: world to its clip
    /// space. A point's six follow each other: +x, −x, +y, −y, +z, −z.
    pub shadow_views: Vec<Mat4>,
}

/// The camera's view: world to its own space, looking down −z.
pub(crate) fn view_of(camera: &Camera) -> Mat4 {
    Mat4::look_at_rh(camera.position, camera.target, camera.up)
}

/// A spot's views: one, through its cone.
fn spot_view(position: Vec3, direction: Vec3, cone_degrees: f32, range: f32) -> Mat4 {
    let direction = direction.normalize_or(Vec3::NEG_Y);
    let up = if direction.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    // A little wider than the cone, so the filter's taps at its edge still
    // land inside the map.
    let fov = (cone_degrees.clamp(1.0, 170.0) + 4.0).to_radians();
    Mat4::perspective_rh(fov, 1.0, SHADOW_NEAR, range.max(SHADOW_NEAR * 2.0))
        * Mat4::look_at_rh(position, position + direction, up)
}

/// A point's six views, a cube's faces, in the order the shader picks them
/// by the largest axis: +x, −x, +y, −y, +z, −z.
fn cube_views(position: Vec3, range: f32, resolution: u32) -> [Mat4; 6] {
    // A shade wider than a right angle, for the same reason as a spot's.
    let widen = 1.0 + 4.0 / resolution.max(1) as f32;
    let fov = 2.0 * widen.atan();
    let projection = Mat4::perspective_rh(fov, 1.0, SHADOW_NEAR, range.max(SHADOW_NEAR * 2.0));
    let faces = [
        (Vec3::X, Vec3::Y),
        (Vec3::NEG_X, Vec3::Y),
        (Vec3::Y, Vec3::Z),
        (Vec3::NEG_Y, Vec3::Z),
        (Vec3::Z, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y),
    ];
    faces.map(|(forward, up)| projection * Mat4::look_at_rh(position, position + forward, up))
}

/// How deep, in the camera's space, each slice starts: thin near, thick
/// far, so that a cell is about as deep as it is wide on screen.
fn slice_of(depth: f32, near: f32, far: f32) -> i64 {
    if depth <= near {
        return 0;
    }
    let t = (depth / near).ln() / (far / near).ln();
    (t * SLICES as f32).floor() as i64
}

/// A run of cells across, down and deep, each first to last.
type Cells = ([u32; 2], [u32; 2], [u32; 2]);

/// Which cells a light's sphere touches: a range across, down and deep, or
/// `None` when it is out of view.
fn cells_of(
    centre: Vec3,
    radius: f32,
    view: Mat4,
    projection: Mat4,
    near: f32,
    far: f32,
) -> Option<Cells> {
    let at = view.transform_point3(centre);
    // Depth runs down −z.
    let (front, back) = (-at.z - radius, -at.z + radius);
    if back < near || front > far {
        return None;
    }
    let first_slice = slice_of(front.max(near), near, far).clamp(0, SLICES as i64 - 1) as u32;
    let last_slice = slice_of(back.min(far), near, far).clamp(0, SLICES as i64 - 1) as u32;

    // The sphere's box, cut to what is in front of the near plane, and its
    // corners projected: the screen rectangle that holds the sphere.
    let z_near = -(front.max(near));
    let z_far = -(back.min(far));
    let mut min = glam::Vec2::splat(f32::INFINITY);
    let mut max = glam::Vec2::splat(f32::NEG_INFINITY);
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 {
                at.x - radius
            } else {
                at.x + radius
            },
            if i & 2 == 0 {
                at.y - radius
            } else {
                at.y + radius
            },
            if i & 4 == 0 { z_near } else { z_far },
        );
        let clip = projection * Vec4::new(corner.x, corner.y, corner.z, 1.0);
        let ndc = clip.truncate().truncate() / clip.w.max(1e-6);
        min = min.min(ndc);
        max = max.max(ndc);
    }
    if max.x < -1.0 || min.x > 1.0 || max.y < -1.0 || min.y > 1.0 {
        return None;
    }
    // NDC y runs up, the screen's rows down.
    let column = |x: f32| {
        (((x * 0.5 + 0.5) * TILES_X as f32).floor() as i64).clamp(0, TILES_X as i64 - 1) as u32
    };
    let row = |y: f32| {
        (((0.5 - y * 0.5) * TILES_Y as f32).floor() as i64).clamp(0, TILES_Y as i64 - 1) as u32
    };
    Some((
        [column(min.x), column(max.x)],
        [row(max.y), row(min.y)],
        [first_slice, last_slice],
    ))
}

/// The cell a pixel is in — the shader's side of [`cells_of`], here for
/// the tests.
#[cfg(test)]
fn cell_at(ndc: glam::Vec2, depth: f32, near: f32, far: f32) -> u32 {
    let x = (((ndc.x * 0.5 + 0.5) * TILES_X as f32) as u32).min(TILES_X - 1);
    let y = (((0.5 - ndc.y * 0.5) * TILES_Y as f32) as u32).min(TILES_Y - 1);
    let z = slice_of(depth, near, far).clamp(0, SLICES as i64 - 1) as u32;
    (z * TILES_Y + y) * TILES_X + x
}

/// Sort, cull and cluster a frame's lights, and give out the shadow maps.
pub(crate) fn cluster(
    lights: &[PointLight],
    camera: &Camera,
    aspect: f32,
    shadows_on: bool,
    shadow_resolution: u32,
) -> Clustered {
    let view = view_of(camera);
    let view_projection = camera.view_projection(aspect);
    let projection = view_projection * view.inverse();
    let (near, far) = (camera.near.max(1e-3), camera.far.max(camera.near + 1e-2));

    let mut seen: Vec<(&PointLight, Cells)> = lights
        .iter()
        .filter(|l| l.range > 0.0)
        .filter_map(|l| {
            cells_of(l.position, l.range, view, projection, near, far).map(|cells| (l, cells))
        })
        .collect();
    let eye = camera.position;
    seen.sort_by(|a, b| {
        (a.0.position - eye)
            .length_squared()
            .total_cmp(&(b.0.position - eye).length_squared())
    });
    seen.truncate(MAX_LIGHTS);

    let mut shadow_views = Vec::new();
    let out: Vec<GpuLight> = seen
        .iter()
        .map(|(light, _)| {
            let mut first = -1.0;
            if shadows_on && light.shadows {
                let views: Vec<Mat4> = match light.spot {
                    Some((direction, cone)) => {
                        vec![spot_view(light.position, direction, cone, light.range)]
                    }
                    None => cube_views(light.position, light.range, shadow_resolution).to_vec(),
                };
                if shadow_views.len() + views.len() <= SHADOW_LAYERS {
                    first = shadow_views.len() as f32;
                    shadow_views.extend(views);
                }
            }
            GpuLight {
                position_range: [
                    light.position.x,
                    light.position.y,
                    light.position.z,
                    light.range.max(0.01),
                ],
                color_shadow: [light.color.x, light.color.y, light.color.z, first],
                spot: match light.spot {
                    Some((direction, cone)) => {
                        let d = direction.normalize_or_zero();
                        [
                            d.x,
                            d.y,
                            d.z,
                            (cone.clamp(1.0, 179.0).to_radians() * 0.5).cos(),
                        ]
                    }
                    None => [0.0, 0.0, 0.0, -2.0],
                },
            }
        })
        .collect();

    // Count, then place: each cell's list lies in one run of `indices`.
    let cell_count = (TILES_X * TILES_Y * SLICES) as usize;
    let index = |x: u32, y: u32, z: u32| ((z * TILES_Y + y) * TILES_X + x) as usize;
    let mut counts = vec![0u32; cell_count];
    for (_, (xs, ys, zs)) in &seen {
        for z in zs[0]..=zs[1] {
            for y in ys[0]..=ys[1] {
                for x in xs[0]..=xs[1] {
                    counts[index(x, y, z)] += 1;
                }
            }
        }
    }
    let mut cells = Vec::with_capacity(cell_count);
    let mut start = 0u32;
    for &count in &counts {
        cells.push([start, 0]);
        start += count;
    }
    let mut indices = vec![0u32; start as usize];
    for (i, (_, (xs, ys, zs))) in seen.iter().enumerate() {
        for z in zs[0]..=zs[1] {
            for y in ys[0]..=ys[1] {
                for x in xs[0]..=xs[1] {
                    let cell = &mut cells[index(x, y, z)];
                    indices[(cell[0] + cell[1]) as usize] = i as u32;
                    cell[1] += 1;
                }
            }
        }
    }
    Clustered {
        lights: out,
        cells,
        indices,
        shadow_views,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(position: Vec3, range: f32) -> PointLight {
        PointLight {
            position,
            color: Vec3::ONE,
            range,
            spot: None,
            shadows: true,
        }
    }

    fn camera() -> Camera {
        Camera {
            position: Vec3::ZERO,
            target: Vec3::NEG_Z,
            ..Camera::default()
        }
    }

    fn lists(c: &Clustered, cell: u32) -> Vec<u32> {
        let [start, count] = c.cells[cell as usize];
        c.indices[start as usize..(start + count) as usize].to_vec()
    }

    #[test]
    fn a_lamp_ahead_is_listed_where_it_is_and_not_across_the_screen() {
        let c = cluster(
            &[lamp(Vec3::new(0.0, 0.0, -10.0), 1.0)],
            &camera(),
            16.0 / 9.0,
            true,
            512,
        );
        assert_eq!(c.lights.len(), 1);
        let middle = cell_at(glam::Vec2::ZERO, 10.0, 0.1, 500.0);
        assert_eq!(lists(&c, middle), vec![0]);
        let corner = cell_at(glam::Vec2::new(-0.95, 0.95), 10.0, 0.1, 500.0);
        assert!(lists(&c, corner).is_empty());
        let far_behind_it = cell_at(glam::Vec2::ZERO, 100.0, 0.1, 500.0);
        assert!(lists(&c, far_behind_it).is_empty());
    }

    #[test]
    fn a_lamp_behind_the_camera_is_not_shaded_at_all() {
        let c = cluster(
            &[lamp(Vec3::new(0.0, 0.0, 10.0), 2.0)],
            &camera(),
            1.0,
            true,
            512,
        );
        assert!(c.lights.is_empty() && c.indices.is_empty());
    }

    #[test]
    fn a_lamp_around_the_camera_reaches_every_cell_near_it() {
        let c = cluster(&[lamp(Vec3::ZERO, 3.0)], &camera(), 1.0, true, 512);
        for (x, y) in [(-0.9, -0.9), (0.9, 0.9), (0.0, 0.0)] {
            let cell = cell_at(glam::Vec2::new(x, y), 1.0, 0.1, 500.0);
            assert_eq!(lists(&c, cell), vec![0], "at {x}, {y}");
        }
    }

    #[test]
    fn shadow_maps_go_to_the_nearest_lamps_until_they_run_out() {
        let far_away: Vec<PointLight> = (0..6)
            .map(|i| lamp(Vec3::new(0.0, 0.0, -5.0 - i as f32 * 3.0), 1.0))
            .collect();
        let c = cluster(&far_away, &camera(), 1.0, true, 512);
        // Four points fill 24 maps; the two farthest go without.
        let firsts: Vec<f32> = c.lights.iter().map(|l| l.color_shadow[3]).collect();
        assert_eq!(firsts, vec![0.0, 6.0, 12.0, 18.0, -1.0, -1.0]);
        assert_eq!(c.shadow_views.len(), SHADOW_LAYERS);
        let off = cluster(&far_away, &camera(), 1.0, false, 512);
        assert!(off.shadow_views.is_empty());
    }

    #[test]
    fn a_cube_face_sees_what_lies_along_its_axis() {
        let views = cube_views(Vec3::new(1.0, 2.0, 3.0), 10.0, 512);
        let axes = [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ];
        for (view, axis) in views.iter().zip(axes) {
            let clip = *view * (Vec3::new(1.0, 2.0, 3.0) + axis * 4.0).extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4, "{axis}: {ndc}");
            assert!((0.0..1.0).contains(&ndc.z));
        }
    }
}
