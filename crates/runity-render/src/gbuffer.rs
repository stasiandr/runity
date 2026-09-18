//! The geometry buffer: what the deferred passes read instead of re-shading.
//!
//! Screen-space effects need to know, for every pixel, *what surface is there*.
//! SSAO compares neighbouring positions against a normal; SSR walks a ray
//! through the depth buffer and needs the surface's roughness to know how wide
//! the lobe is. None of that is answerable from a color buffer, so the geometry
//! pass writes it down.

use crate::color::Color;
use runity_math::Vec3;

/// Per-pixel surface parameters, written by the geometry pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    /// Base color in linear light. Metals put their reflectance here.
    pub albedo: Color,
    /// World-space normal, unit length. Zero means "no geometry here".
    pub normal: Vec3,
    /// World-space position of the fragment.
    pub position: Vec3,
    pub roughness: f32,
    pub metallic: f32,
    /// Baked or painted occlusion, multiplied into ambient light.
    pub occlusion: f32,
    /// Light the surface emits regardless of what reaches it.
    pub emissive: Color,
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            albedo: Color::BLACK,
            normal: Vec3::ZERO,
            position: Vec3::ZERO,
            roughness: 1.0,
            metallic: 0.0,
            occlusion: 1.0,
            emissive: Color::BLACK,
        }
    }
}

impl Surface {
    /// Whether the geometry pass wrote anything here.
    #[inline]
    pub fn is_geometry(&self) -> bool {
        self.normal.length_squared() > 0.0
    }
}

/// Screen-sized arrays of [`Surface`], one entry per pixel.
#[derive(Debug, Clone)]
pub struct GBuffer {
    width: usize,
    height: usize,
    surfaces: Vec<Surface>,
}

impl GBuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            surfaces: vec![Surface::default(); width * height],
        }
    }

    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    #[inline]
    pub fn surfaces(&self) -> &[Surface] {
        &self.surfaces
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> &Surface {
        &self.surfaces[y * self.width + x]
    }

    #[inline]
    pub fn at(&self, index: usize) -> &Surface {
        &self.surfaces[index]
    }

    #[inline]
    pub fn set(&mut self, index: usize, surface: Surface) {
        self.surfaces[index] = surface;
    }

    pub fn clear(&mut self) {
        self.surfaces.fill(Surface::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleared_gbuffer_reports_no_geometry() {
        let gbuffer = GBuffer::new(3, 2);
        assert_eq!(gbuffer.len(), 6);
        assert!(gbuffer.surfaces().iter().all(|s| !s.is_geometry()));
    }

    #[test]
    fn a_written_surface_is_geometry_and_survives_until_cleared() {
        let mut gbuffer = GBuffer::new(2, 2);
        let surface = Surface {
            albedo: Color::RED,
            normal: Vec3::Y,
            position: Vec3::new(1.0, 2.0, 3.0),
            roughness: 0.3,
            ..Surface::default()
        };
        gbuffer.set(2, surface);
        assert!(gbuffer.get(0, 1).is_geometry());
        assert_eq!(gbuffer.get(0, 1).roughness, 0.3);
        gbuffer.clear();
        assert!(!gbuffer.get(0, 1).is_geometry());
    }
}
