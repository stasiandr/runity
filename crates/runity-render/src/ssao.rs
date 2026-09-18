//! Screen-space ambient occlusion.
//!
//! Ambient light in this renderer arrives from the whole sky, which is right
//! for an open field and wrong for a corner: the sky cannot reach into a crease
//! between two walls, but the irradiance harmonics do not know the walls are
//! there. SSAO puts that back by asking, per pixel, how much of the hemisphere
//! above the surface is blocked by whatever else the depth buffer contains.
//!
//! The sampling follows the usual normal-oriented hemisphere approach: take
//! points in the hemisphere around the surface normal, project each back to the
//! screen, and compare its depth against what the G-buffer has there. Points
//! that turn out to be *behind* real geometry were occluded.
//!
//! The kernel and the rotation pattern are both fixed rather than random.
//! Random noise plus a temporal filter is what a real-time renderer does; here
//! determinism matters more, because the golden images depend on it.

use crate::framebuffer::Framebuffer;
use crate::gbuffer::GBuffer;
use crate::view::CameraView;
use runity_math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SsaoSettings {
    pub enabled: bool,
    /// World-space radius of the hemisphere. Roughly "how big is a crease".
    pub radius: f32,
    /// Number of samples per pixel. Cost is linear in this.
    pub samples: usize,
    /// How dark full occlusion gets: 1.0 removes all ambient light.
    pub intensity: f32,
    /// Ignore differences smaller than this, so a flat surface's own depth
    /// gradient does not read as occlusion.
    pub bias: f32,
    /// Radius, in pixels, of the box blur that removes the sampling pattern.
    pub blur_radius: usize,
}

impl Default for SsaoSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 0.5,
            samples: 24,
            intensity: 0.9,
            bias: 0.02,
            blur_radius: 2,
        }
    }
}

/// Per-pixel ambient visibility, 1 = open, 0 = fully enclosed.
#[derive(Debug, Default, Clone)]
pub struct OcclusionBuffer {
    width: usize,
    height: usize,
    values: Vec<f32>,
    scratch: Vec<f32>,
}

impl OcclusionBuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            values: vec![1.0; width * height],
            scratch: vec![1.0; width * height],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> f32 {
        self.values[y * self.width + x]
    }

    #[inline]
    pub fn at(&self, index: usize) -> f32 {
        self.values.get(index).copied().unwrap_or(1.0)
    }

    fn resize(&mut self, width: usize, height: usize) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        self.values = vec![1.0; width * height];
        self.scratch = vec![1.0; width * height];
    }

    pub fn fill_open(&mut self) {
        self.values.fill(1.0);
    }
}

/// A fixed set of directions in the +Z hemisphere, packed towards the center so
/// nearby geometry counts for more.
fn kernel(samples: usize) -> Vec<Vec3> {
    let golden = core::f32::consts::PI * (3.0 - 5f32.sqrt());
    (0..samples)
        .map(|i| {
            let t = (i as f32 + 0.5) / samples as f32;
            let z = t.sqrt(); // cosine-ish distribution towards the normal
            let radius = (1.0 - z * z).sqrt();
            let theta = golden * i as f32;
            let direction = Vec3::new(theta.cos() * radius, theta.sin() * radius, z);
            // Bias the lengths towards the origin: occluders close to the
            // surface matter more than ones at the edge of the radius.
            let scale = 0.3 + 0.7 * t * t;
            direction * scale
        })
        .collect()
}

/// A deterministic per-pixel rotation, so the kernel does not leave a visible
/// grid. Interleaved gradient noise: cheap, and a pure function of the pixel.
#[inline]
fn rotation_angle(x: usize, y: usize) -> f32 {
    let value = 52.982_92 * (0.067_110_56 * x as f32 + 0.005_837_15 * y as f32).fract();
    value.fract() * core::f32::consts::TAU
}

/// Compute ambient visibility for every pixel with geometry.
pub fn compute(
    gbuffer: &GBuffer,
    camera: &CameraView,
    settings: &SsaoSettings,
    out: &mut OcclusionBuffer,
) {
    let (width, height) = (gbuffer.width(), gbuffer.height());
    out.resize(width, height);
    if !settings.enabled || settings.samples == 0 || settings.radius <= 0.0 {
        out.fill_open();
        return;
    }

    let kernel = kernel(settings.samples);
    let view_projection = camera.view_projection();

    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let surface = gbuffer.at(index);
            if !surface.is_geometry() {
                out.values[index] = 1.0;
                continue;
            }

            let normal = surface.normal;
            // Build a frame around the normal, rotated per pixel.
            let angle = rotation_angle(x, y);
            let (sin, cos) = angle.sin_cos();
            let helper = if normal.y.abs() < 0.9 {
                Vec3::Y
            } else {
                Vec3::X
            };
            let tangent = helper.cross(normal).normalized();
            let bitangent = normal.cross(tangent);
            let tangent = tangent * cos + bitangent * sin;
            let bitangent = normal.cross(tangent);

            let origin_depth = camera.view_depth(surface.position);
            let mut occlusion = 0.0;
            for offset in &kernel {
                let world = surface.position
                    + (tangent * offset.x + bitangent * offset.y + normal * offset.z)
                        * settings.radius;

                let Some((sx, sy, _)) = camera.project(world, width, height, &view_projection)
                else {
                    continue;
                };
                if sx < 0.0 || sy < 0.0 || sx >= width as f32 || sy >= height as f32 {
                    continue;
                }
                let sample_index = sy as usize * width + sx as usize;
                let occluder = gbuffer.at(sample_index);
                if !occluder.is_geometry() {
                    continue;
                }

                // Is the real surface at that pixel closer to the eye than the
                // sample point we just placed?
                let sample_depth = camera.view_depth(world);
                let scene_depth = camera.view_depth(occluder.position);
                if scene_depth < sample_depth - settings.bias {
                    // Ignore occluders far outside the radius: a distant wall
                    // must not darken the floor in front of it.
                    let range = settings.radius / (origin_depth - scene_depth).abs().max(1e-4);
                    occlusion += range.clamp(0.0, 1.0);
                }
            }

            let visibility = 1.0 - (occlusion / kernel.len() as f32) * settings.intensity;
            out.values[index] = visibility.clamp(0.0, 1.0);
        }
    }

    blur(out, settings.blur_radius);
}

/// Separable box blur, skipping pixels with no geometry so edges stay put.
fn blur(buffer: &mut OcclusionBuffer, radius: usize) {
    if radius == 0 {
        return;
    }
    let (width, height) = (buffer.width, buffer.height);
    let radius = radius as isize;

    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            let mut count = 0.0;
            for dx in -radius..=radius {
                let sx = x as isize + dx;
                if sx < 0 || sx >= width as isize {
                    continue;
                }
                sum += buffer.values[y * width + sx as usize];
                count += 1.0;
            }
            buffer.scratch[y * width + x] = sum / count;
        }
    }
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            let mut count = 0.0;
            for dy in -radius..=radius {
                let sy = y as isize + dy;
                if sy < 0 || sy >= height as isize {
                    continue;
                }
                sum += buffer.scratch[sy as usize * width + x];
                count += 1.0;
            }
            buffer.values[y * width + x] = sum / count;
        }
    }
}

/// Render the occlusion buffer as greyscale, for a debug view.
pub fn occlusion_view(occlusion: &OcclusionBuffer) -> Framebuffer {
    use crate::color::Color;
    let mut out = Framebuffer::new_raw(occlusion.width.max(1), occlusion.height.max(1));
    for y in 0..occlusion.height {
        for x in 0..occlusion.width {
            let v = occlusion.get(x, y);
            out.set_pixel(x, y, Color::rgb(v, v, v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::gbuffer::Surface;
    use runity_math::Mat4;

    fn camera(width: usize, height: usize) -> CameraView {
        let position = Vec3::new(0.0, 0.0, 4.0);
        CameraView::new(
            Mat4::look_at(position, Vec3::ZERO, Vec3::Y),
            Mat4::perspective(60f32.to_radians(), width as f32 / height as f32, 0.1, 100.0),
            position,
            0.1,
            100.0,
        )
    }

    /// A flat wall facing the camera, filling the frame.
    fn flat_wall(width: usize, height: usize, camera: &CameraView) -> GBuffer {
        let mut gbuffer = GBuffer::new(width, height);
        let inverse = camera.inverse_view_projection();
        for y in 0..height {
            for x in 0..width {
                // Put every pixel on the z = 0 plane, along its own view ray.
                let direction = camera.ray_direction(&inverse, x, y, width, height);
                let t = -camera.position.z / direction.z;
                gbuffer.set(
                    y * width + x,
                    Surface {
                        albedo: Color::WHITE,
                        normal: Vec3::Z,
                        position: camera.position + direction * t,
                        roughness: 1.0,
                        ..Surface::default()
                    },
                );
            }
        }
        gbuffer
    }

    #[test]
    fn a_flat_surface_is_not_occluded() {
        let (width, height) = (48, 48);
        let camera = camera(width, height);
        let gbuffer = flat_wall(width, height, &camera);
        let mut occlusion = OcclusionBuffer::new(width, height);
        compute(&gbuffer, &camera, &SsaoSettings::default(), &mut occlusion);

        // Away from the border, where samples fall off screen, a plane must
        // come back fully open.
        for y in 8..height - 8 {
            for x in 8..width - 8 {
                assert!(
                    occlusion.get(x, y) > 0.9,
                    "false occlusion at ({x},{y}): {}",
                    occlusion.get(x, y)
                );
            }
        }
    }

    #[test]
    fn a_step_in_depth_darkens_the_pixels_behind_it() {
        let (width, height) = (48, 48);
        let camera = camera(width, height);
        let mut gbuffer = flat_wall(width, height, &camera);

        // Push the right half of the wall a long way back: the pixels just
        // behind the step are now in a corner.
        for y in 0..height {
            for x in width / 2..width {
                let index = y * width + x;
                let mut surface = *gbuffer.at(index);
                surface.position.z -= 0.6;
                gbuffer.set(index, surface);
            }
        }

        // A wider radius and no blur, so the effect is measured where it
        // happens rather than smeared across the step.
        let settings = SsaoSettings {
            radius: 1.0,
            blur_radius: 0,
            ..SsaoSettings::default()
        };
        let mut occlusion = OcclusionBuffer::new(width, height);
        compute(&gbuffer, &camera, &settings, &mut occlusion);

        let in_the_corner = occlusion.get(width / 2 + 1, height / 2);
        let out_in_the_open = occlusion.get(width - 6, height / 2);
        assert!(
            in_the_corner < 0.85,
            "the step should darken its own side: {in_the_corner}"
        );
        assert!(
            out_in_the_open > in_the_corner + 0.1,
            "and the far side should stay open: {out_in_the_open} vs {in_the_corner}"
        );

        // The darkening must fade with distance from the step.
        let near = occlusion.get(width / 2 + 2, height / 2);
        let far = occlusion.get(width / 2 + 8, height / 2);
        assert!(far > near, "occlusion should fall off: {far} vs {near}");
    }

    #[test]
    fn pixels_with_no_geometry_stay_open() {
        let (width, height) = (16, 16);
        let camera = camera(width, height);
        let gbuffer = GBuffer::new(width, height);
        let mut occlusion = OcclusionBuffer::new(width, height);
        compute(&gbuffer, &camera, &SsaoSettings::default(), &mut occlusion);
        assert!(occlusion.values().iter().all(|v| *v == 1.0));
    }

    #[test]
    fn disabling_it_leaves_everything_open() {
        let (width, height) = (16, 16);
        let camera = camera(width, height);
        let gbuffer = flat_wall(width, height, &camera);
        let settings = SsaoSettings {
            enabled: false,
            ..SsaoSettings::default()
        };
        let mut occlusion = OcclusionBuffer::new(width, height);
        compute(&gbuffer, &camera, &settings, &mut occlusion);
        assert!(occlusion.values().iter().all(|v| *v == 1.0));
    }

    #[test]
    fn the_result_is_the_same_every_run() {
        let (width, height) = (32, 32);
        let camera = camera(width, height);
        let gbuffer = flat_wall(width, height, &camera);
        let mut first = OcclusionBuffer::new(width, height);
        let mut second = OcclusionBuffer::new(width, height);
        compute(&gbuffer, &camera, &SsaoSettings::default(), &mut first);
        compute(&gbuffer, &camera, &SsaoSettings::default(), &mut second);
        assert_eq!(first.values(), second.values());
    }

    #[test]
    fn the_rotation_pattern_varies_between_neighbours_but_repeats_exactly() {
        assert_ne!(rotation_angle(10, 10), rotation_angle(11, 10));
        assert_eq!(rotation_angle(7, 3), rotation_angle(7, 3));
    }
}
