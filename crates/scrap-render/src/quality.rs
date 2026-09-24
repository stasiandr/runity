//! Quality presets: what a game's graphics menu offers — Low, Medium,
//! High, Ultra — as the passes that run ([`Passes`]), the sun's shadow
//! map, and the share of the screen drawn with the rest made up
//! ([`crate::upscale`]), and which one a machine starts on.
//!
//! A preset only takes away and lowers: it never turns on what the scene
//! did not ask for, so Ultra is the scene as authored, with rays where the
//! device traces. Below it:
//!
//! | | Low | Medium | High | Ultra |
//! |---|---|---|---|---|
//! | shadows | 1024², 2 cascades | 2048², 3 | 2048², 4 | as asked |
//! | AO, SSR, fog, clouds | off | AO, fog, clouds | all | all |
//! | TAA, lens | FXAA only | TAA | TAA, lens | all |
//! | rays | off | off | off | where the device traces |
//! | drawn at | 0.5, dynamic to 16.6 ms | 0.75, dynamic | full, dynamic under 16.6 ms | as asked |
//!
//! [`Quality::for_device`] picks the start: Low on a software renderer,
//! Medium on an integrated GPU without rays, High on one with them or a
//! discrete one, Ultra on a discrete one that traces.

use crate::passes::Passes;
use crate::render::Frame;

/// A graphics menu's settings, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Quality {
    Low,
    Medium,
    High,
    Ultra,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::Low, Quality::Medium, Quality::High, Quality::Ultra];

    /// The passes that run.
    pub fn passes(self) -> Passes {
        let all = Passes::MAX;
        match self {
            Quality::Low => Passes {
                ambient_occlusion: false,
                screen_space_reflections: false,
                reflection_probes: false,
                volumetric_fog: false,
                clouds: false,
                taa: false,
                lens: false,
                ray_tracing: false,
                ..all
            },
            Quality::Medium => Passes {
                screen_space_reflections: false,
                lens: false,
                ray_tracing: false,
                ..all
            },
            Quality::High => Passes {
                ray_tracing: false,
                ..all
            },
            Quality::Ultra => all,
        }
    }

    /// The frame as this preset draws it: its passes, then the shadow map
    /// and drawn share lowered, never raised.
    pub fn apply(self, frame: &Frame) -> Frame {
        let mut frame = self.passes().apply(frame);
        let (resolution, cascades) = match self {
            Quality::Low => (1024, 2),
            Quality::Medium => (2048, 3),
            Quality::High => (2048, 4),
            Quality::Ultra => return frame,
        };
        frame.shadows.resolution = frame.shadows.resolution.min(resolution);
        frame.shadows.cascades = frame.shadows.cascades.min(cascades);
        if self == Quality::Low {
            // What TAA would have smoothed, FXAA at least softens.
            frame.post.fxaa = true;
            frame.shadows.virtual_maps = false;
        }
        let up = &mut frame.post.upscaling;
        let (scale, target) = match self {
            Quality::Low => (0.5, 16.6),
            Quality::Medium => (0.75, 16.6),
            _ => (1.0, 16.6),
        };
        if !up.enabled || up.scale > scale {
            up.enabled = true;
            up.scale = up.scale.min(scale);
            up.dynamic.enabled = true;
            up.dynamic.target_ms = target;
            up.dynamic.min_scale = (scale * 0.67).max(0.33);
        }
        frame
    }

    /// What a machine starts on, by its GPU.
    pub fn for_device(gpu: &crate::gpu::Gpu) -> Quality {
        let info = gpu.adapter.get_info();
        match info.device_type {
            wgpu::DeviceType::Cpu => Quality::Low,
            wgpu::DeviceType::DiscreteGpu if gpu.ray_tracing => Quality::Ultra,
            wgpu::DeviceType::DiscreteGpu => Quality::High,
            _ if gpu.ray_tracing => Quality::High,
            _ => Quality::Medium,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_preset_takes_away_more_than_the_one_above_and_ultra_nothing() {
        let mut frame = Frame::default();
        frame.shadows.enabled = true;
        frame.shadows.resolution = 4096;
        frame.ambient_occlusion.enabled = true;
        frame.screen_space_reflections.enabled = true;
        frame.post.taa = true;
        assert_eq!(Quality::Ultra.apply(&frame), frame, "Ultra is the scene as asked");
        let low = Quality::Low.apply(&frame);
        assert!(!low.ambient_occlusion.enabled && !low.post.taa && low.post.fxaa);
        assert_eq!(low.shadows.resolution, 1024);
        assert!(low.post.upscaling.enabled && low.post.upscaling.scale <= 0.5 && low.post.upscaling.dynamic.enabled);
        let medium = Quality::Medium.apply(&frame);
        assert!(medium.ambient_occlusion.enabled && !medium.screen_space_reflections.enabled);
        let high = Quality::High.apply(&frame);
        assert!(high.screen_space_reflections.enabled && high.shadows.cascades <= 4);
        // Never raised: a scene that asked for less keeps less.
        frame.shadows.resolution = 512;
        assert_eq!(Quality::High.apply(&frame).shadows.resolution, 512);
        assert!(Quality::Low < Quality::Ultra);
    }
}
