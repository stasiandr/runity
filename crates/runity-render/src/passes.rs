//! The render path's passes, switched (DNA, "Рендер — один путь,
//! проходы выключаются"): the path is one — no choosing between pipelines
//! — and each pass on it can be off, at run time here, as a game's
//! graphics settings, whatever the scene asks for. The most is every pass
//! the scene asks for ([`Passes::MAX`], the default); the least is meshes
//! with no light ([`Passes::MIN`]).
//!
//! A pass off here is not run and costs nothing but its memory. A pass
//! left out of the build is a cargo feature of this module: `ray-tracing`
//! today.

use crate::render::Frame;

/// Which passes run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Passes {
    /// The sun and the lamps light what they reach; off, everything is
    /// drawn in its own colour, unlit — the least there is.
    pub lighting: bool,
    pub shadows: bool,
    pub ambient_occlusion: bool,
    pub screen_space_reflections: bool,
    /// Probes baked and reflected.
    pub reflection_probes: bool,
    pub volumetric_fog: bool,
    pub clouds: bool,
    /// Temporal antialiasing.
    pub taa: bool,
    /// Depth of field, motion blur, heat haze.
    pub lens: bool,
    /// Bloom, grading, tonemapping, exposure, FXAA.
    pub post: bool,
    /// Hardware rays, where the device has them and the scene asks.
    pub ray_tracing: bool,
}

impl Default for Passes {
    fn default() -> Self {
        Self::MAX
    }
}

impl Passes {
    /// Every pass the scene asks for.
    pub const MAX: Passes = Passes {
        lighting: true,
        shadows: true,
        ambient_occlusion: true,
        screen_space_reflections: true,
        reflection_probes: true,
        volumetric_fog: true,
        clouds: true,
        taa: true,
        lens: true,
        post: true,
        ray_tracing: true,
    };

    /// Meshes with no light: the least the path draws.
    pub const MIN: Passes = Passes {
        lighting: false,
        shadows: false,
        ambient_occlusion: false,
        screen_space_reflections: false,
        reflection_probes: false,
        volumetric_fog: false,
        clouds: false,
        taa: false,
        lens: false,
        post: false,
        ray_tracing: false,
    };

    /// The frame as these passes draw it: what is off, taken out of it.
    pub fn apply(&self, frame: &Frame) -> Frame {
        let mut frame = frame.clone();
        if !self.lighting {
            for draw in &mut frame.draws {
                draw.material = draw.material.unlit();
            }
            frame.lights.clear();
            frame.flares.clear();
            // Behind them, the clear colour: a sky is lit by the sun too.
            frame.sky.mode = crate::render::SkyMode::Color;
        }
        if !self.shadows || !self.lighting {
            frame.shadows.enabled = false;
        }
        if !self.ambient_occlusion {
            frame.ambient_occlusion.enabled = false;
        }
        if !self.screen_space_reflections {
            frame.screen_space_reflections.enabled = false;
        }
        if !self.reflection_probes {
            frame.reflection_probes.clear();
        }
        if !self.volumetric_fog {
            frame.volumetric_fog.enabled = false;
            frame.puffs.clear();
        }
        if !self.clouds {
            frame.sky.clouds.coverage = 0.0;
        }
        if !self.taa {
            frame.post.taa = false;
        }
        if !self.lens {
            frame.post.depth_of_field.mode = crate::lens::FocusMode::Off;
            frame.post.motion_blur.intensity = 0.0;
            frame.post.heat_haze.intensity = 0.0;
            frame.post.heat_haze.mirage = 0.0;
        }
        if !self.post {
            frame.post.enabled = false;
        }
        if !self.ray_tracing {
            frame.ray_tracing = crate::ray::RayTracing::default();
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_is_meshes_with_no_light() {
        let mut frame = Frame::default();
        frame.shadows.enabled = true;
        frame.post.taa = true;
        frame.draws.push(crate::render::Draw {
            mesh: crate::render::MeshHandle::TEST,
            transform: glam::Mat4::IDENTITY,
            texture: crate::render::TextureHandle::WHITE,
            material: crate::material::Material::new(1.0, 0.0, 0.0),
            pose: None,
        });
        assert_eq!(Passes::MAX.apply(&frame), frame);
        let least = Passes::MIN.apply(&frame);
        assert!(!least.shadows.enabled && !least.post.taa && !least.post.enabled);
        assert_eq!(least.draws[0].material, frame.draws[0].material.unlit());
    }
}
