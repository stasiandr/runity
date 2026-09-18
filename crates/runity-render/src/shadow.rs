//! Shadow mapping.
//!
//! The trick is old and still the best one available: render the scene from the
//! light's point of view, keep only the depth, and when shading ask "is this
//! point the closest thing the light can see in that direction?" If not, it is
//! in shadow.
//!
//! Two details decide whether it looks right. **Bias**, because a depth map is
//! a quantized sample of a continuous surface, and without an offset every
//! surface shadows itself in stripes. And **filtering**, because a single
//! comparison gives a binary answer, and binary answers at texel resolution
//! look like staircases — so we take several taps and average them (PCF).

use crate::color::Color;
use crate::framebuffer::Framebuffer;
use crate::mesh::Mesh;
use crate::pbr::{Light, LightKind};
use crate::raster::{CullMode, Rasterizer};
use crate::shader::{Shader, Vertex, VertexOutput};
use crate::view::CameraView;
use runity_math::{Mat4, Vec3, Vec4};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSettings {
    pub enabled: bool,
    /// Side of the (square) depth map. More texels, crisper edges, more memory.
    pub resolution: usize,
    /// Half-width of the world-space box the map covers, centered ahead of the
    /// camera. Directional lights have no position, so this is what decides
    /// how much of the world gets shadows.
    pub extent: f32,
    /// Constant offset, in light-space depth units.
    pub depth_bias: f32,
    /// Offset along the surface normal before looking up, in world units. It
    /// handles the slope cases a constant bias cannot.
    pub normal_bias: f32,
    /// Filter radius in texels. 0 is a single tap and hard, aliased edges.
    pub softness: f32,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            resolution: 1024,
            extent: 12.0,
            depth_bias: 0.0015,
            normal_bias: 0.05,
            softness: 1.5,
        }
    }
}

/// Depth of the scene as the light sees it.
pub struct ShadowMap {
    target: Framebuffer,
    light_view_projection: Mat4,
    /// World units per depth unit, for turning a bias in metres into one in
    /// the map's units.
    depth_range: f32,
    active: bool,
}

/// Writes nothing but depth. The rasterizer handles that on its own; this
/// shader exists only to place the vertex.
struct DepthOnly {
    model_view_projection: Mat4,
}

impl Shader for DepthOnly {
    type Varying = f32;

    fn vertex(&self, vertex: &Vertex) -> VertexOutput<f32> {
        VertexOutput {
            clip_position: self.model_view_projection.transform_point(vertex.position),
            varying: 0.0,
        }
    }

    fn fragment(&self, _varying: &f32) -> Option<Color> {
        Some(Color::BLACK)
    }
}

impl ShadowMap {
    pub fn new(resolution: usize) -> Self {
        Self {
            target: Framebuffer::new(resolution.max(1), resolution.max(1)),
            light_view_projection: Mat4::IDENTITY,
            depth_range: 1.0,
            active: false,
        }
    }

    pub fn resolution(&self) -> usize {
        self.target.width()
    }

    pub fn light_view_projection(&self) -> Mat4 {
        self.light_view_projection
    }

    /// The depth buffer itself, for debug views.
    pub fn depth(&self) -> &[f32] {
        self.target.depth()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Aim the map at the part of the world the camera is looking at, and clear
    /// it. Returns false for lights that cannot cast (point lights, for now).
    pub fn begin(&mut self, light: &Light, camera: &CameraView, settings: &ShadowSettings) -> bool {
        self.active = false;
        if !settings.enabled || !light.casts_shadow {
            return false;
        }
        let LightKind::Directional { direction } = light.kind else {
            return false;
        };
        if settings.resolution != self.target.width() {
            self.target
                .resize(settings.resolution.max(1), settings.resolution.max(1));
        }

        // Center the box a little ahead of the camera: shadows matter where the
        // viewer is looking, and a directional light has nowhere else to anchor.
        let center = camera.position + camera.forward() * (settings.extent * 0.6);
        let extent = settings.extent;
        let distance = extent * 2.0;
        let eye = center - direction * distance;
        // Any up vector works as long as it is not parallel to the light.
        let up = if direction.y.abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        };

        let view = Mat4::look_at(eye, center, up);
        let near = 0.05;
        let far = distance + extent * 2.0;
        let projection = Mat4::orthographic(-extent, extent, -extent, extent, near, far);
        self.light_view_projection = projection * view;
        self.depth_range = far - near;

        self.target.clear(Color::BLACK);
        self.active = true;
        true
    }

    /// Render one mesh into the map.
    pub fn draw(&mut self, mesh: &Mesh, model: Mat4) {
        if !self.active {
            return;
        }
        let mut rasterizer = Rasterizer::new();
        // Front-face culling pushes the recorded depth to the *back* of each
        // object, which moves self-shadowing acne out of sight.
        rasterizer.cull = CullMode::Front;
        let shader = DepthOnly {
            model_view_projection: self.light_view_projection * model,
        };
        rasterizer.draw_mesh(&mut self.target, mesh, &shader);
    }

    /// How much of the light reaches this point: 1 lit, 0 fully shadowed.
    pub fn visibility(&self, position: Vec3, normal: Vec3, settings: &ShadowSettings) -> f32 {
        if !self.active {
            return 1.0;
        }
        // Stepping along the normal first is what keeps curved surfaces from
        // shadowing themselves at grazing angles.
        let biased = position + normal * settings.normal_bias;
        let clip = self.light_view_projection * Vec4::new(biased.x, biased.y, biased.z, 1.0);
        if clip.w <= 0.0 {
            return 1.0;
        }
        let ndc = clip.perspective_divide();
        if !(-1.0..=1.0).contains(&ndc.x) || !(-1.0..=1.0).contains(&ndc.y) {
            return 1.0; // outside the map: assume lit rather than invent shadow
        }
        if !(0.0..=1.0).contains(&ndc.z) {
            return 1.0;
        }

        let resolution = self.target.width() as f32;
        let x = (ndc.x * 0.5 + 0.5) * resolution;
        let y = (0.5 - ndc.y * 0.5) * resolution;
        let reference = ndc.z - settings.depth_bias;

        // A fixed tap pattern, not a random one: the same pixel must resolve the
        // same way every frame or the golden images are worthless.
        const TAPS: [(f32, f32); 9] = [
            (0.0, 0.0),
            (-1.0, -1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (1.0, 1.0),
            (0.0, -1.4),
            (0.0, 1.4),
            (-1.4, 0.0),
            (1.4, 0.0),
        ];
        let radius = settings.softness.max(0.0);
        let mut lit = 0.0;
        for (dx, dy) in TAPS {
            let sx = (x + dx * radius).floor();
            let sy = (y + dy * radius).floor();
            if sx < 0.0 || sy < 0.0 || sx >= resolution || sy >= resolution {
                lit += 1.0;
                continue;
            }
            let stored = self.target.depth_at(sx as usize, sy as usize);
            if reference <= stored {
                lit += 1.0;
            }
        }
        lit / TAPS.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use runity_math::Vec3;

    fn camera() -> CameraView {
        let position = Vec3::new(0.0, 3.0, 6.0);
        CameraView::new(
            Mat4::look_at(position, Vec3::ZERO, Vec3::Y),
            Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0),
            position,
            0.1,
            100.0,
        )
    }

    /// A light straight overhead and a small slab floating above the origin.
    fn overhead_scene() -> (ShadowMap, ShadowSettings) {
        let settings = ShadowSettings {
            resolution: 256,
            extent: 8.0,
            ..ShadowSettings::default()
        };
        let mut map = ShadowMap::new(settings.resolution);
        let light = Light::directional(-Vec3::Y, Color::WHITE, 3.0);
        assert!(map.begin(&light, &camera(), &settings));
        map.draw(
            &Mesh::cube(2.0),
            Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        );
        (map, settings)
    }

    #[test]
    fn a_point_under_the_caster_is_shadowed_and_one_beside_it_is_not() {
        let (map, settings) = overhead_scene();
        let under = map.visibility(Vec3::new(0.0, 0.0, 0.0), Vec3::Y, &settings);
        assert!(under < 0.1, "directly under the cube: {under}");

        let beside = map.visibility(Vec3::new(4.0, 0.0, 0.0), Vec3::Y, &settings);
        assert!(beside > 0.9, "four units away: {beside}");
    }

    #[test]
    fn the_edge_of_the_shadow_is_filtered_rather_than_binary() {
        let (map, settings) = overhead_scene();
        // Somewhere along the penumbra the taps must disagree.
        let partial = (0..40)
            .map(|i| 0.9 + i as f32 * 0.01)
            .map(|x| map.visibility(Vec3::new(x, 0.0, 0.0), Vec3::Y, &settings))
            .any(|v| v > 0.05 && v < 0.95);
        assert!(
            partial,
            "PCF should produce intermediate values at the edge"
        );
    }

    #[test]
    fn a_lit_surface_does_not_shadow_itself() {
        // The acne test: a flat plane, lit from above, must come out fully lit.
        let settings = ShadowSettings {
            resolution: 256,
            extent: 8.0,
            ..ShadowSettings::default()
        };
        let mut map = ShadowMap::new(settings.resolution);
        let light = Light::directional(Vec3::new(-0.4, -1.0, -0.3).normalized(), Color::WHITE, 3.0);
        map.begin(&light, &camera(), &settings);
        map.draw(&Mesh::plane(12.0, 4), Mat4::IDENTITY);

        for (x, z) in [(0.0f32, 0.0f32), (1.5, -2.0), (-3.0, 2.5), (4.0, 4.0)] {
            let v = map.visibility(Vec3::new(x, 0.0, z), Vec3::Y, &settings);
            assert!(v > 0.95, "acne at ({x}, {z}): {v}");
        }
    }

    #[test]
    fn anything_outside_the_map_is_treated_as_lit() {
        let (map, settings) = overhead_scene();
        let far_away = map.visibility(Vec3::new(500.0, 0.0, 0.0), Vec3::Y, &settings);
        assert_eq!(far_away, 1.0);
    }

    #[test]
    fn lights_that_cannot_cast_are_skipped() {
        let settings = ShadowSettings::default();
        let mut map = ShadowMap::new(64);
        let point = Light::point(Vec3::Y, Color::WHITE, 5.0, 10.0);
        assert!(
            !map.begin(&point, &camera(), &settings),
            "point lights have no map yet"
        );
        assert_eq!(
            map.visibility(Vec3::ZERO, Vec3::Y, &settings),
            1.0,
            "and cast no shadow"
        );

        let disabled = ShadowSettings {
            enabled: false,
            ..ShadowSettings::default()
        };
        let sun = Light::directional(-Vec3::Y, Color::WHITE, 3.0);
        assert!(!map.begin(&sun, &camera(), &disabled));
    }

    #[test]
    fn the_map_follows_the_camera() {
        let settings = ShadowSettings {
            resolution: 128,
            extent: 6.0,
            ..ShadowSettings::default()
        };
        let mut map = ShadowMap::new(settings.resolution);
        let light = Light::directional(-Vec3::Y, Color::WHITE, 3.0);

        map.begin(&light, &camera(), &settings);
        let near_origin = map.light_view_projection();

        let moved = CameraView::new(
            Mat4::look_at(
                Vec3::new(50.0, 3.0, 56.0),
                Vec3::new(50.0, 0.0, 50.0),
                Vec3::Y,
            ),
            Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0),
            Vec3::new(50.0, 3.0, 56.0),
            0.1,
            100.0,
        );
        map.begin(&light, &moved, &settings);
        assert_ne!(
            near_origin,
            map.light_view_projection(),
            "the box must move with the camera"
        );
    }
}
