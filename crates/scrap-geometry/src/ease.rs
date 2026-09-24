//! Easing: how a change goes from nothing to all of it — the curves of
//! easings.net and DOTween's `Ease`, by the same names. A camera's blend,
//! a tween and a motion clip's track all take one (docs/feel.md).
//!
//! Every curve starts at 0 and ends at 1; `Back` and `Elastic` go past
//! either end on the way, `Bounce` touches the end before it settles.

use serde::{Deserialize, Serialize};

/// A curve from 0 to 1. `Linear` by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Ease {
    #[default]
    Linear,
    /// Slow at both ends, `t²(3 − 2t)` — Unity's `Mathf.SmoothStep`.
    Smooth,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    InSine,
    OutSine,
    InOutSine,
    InExpo,
    OutExpo,
    InOutExpo,
    /// Pulls back before it goes.
    InBack,
    /// Overshoots, then comes back.
    OutBack,
    InOutBack,
    InElastic,
    /// Springs past the end and wobbles into it.
    OutElastic,
    InOutElastic,
    InBounce,
    /// Drops onto the end and bounces on it.
    OutBounce,
    InOutBounce,
}

impl Ease {
    /// Every curve, in the order above: for a picker and for tests.
    pub const ALL: [Ease; 23] = [
        Ease::Linear,
        Ease::Smooth,
        Ease::InQuad,
        Ease::OutQuad,
        Ease::InOutQuad,
        Ease::InCubic,
        Ease::OutCubic,
        Ease::InOutCubic,
        Ease::InSine,
        Ease::OutSine,
        Ease::InOutSine,
        Ease::InExpo,
        Ease::OutExpo,
        Ease::InOutExpo,
        Ease::InBack,
        Ease::OutBack,
        Ease::InOutBack,
        Ease::InElastic,
        Ease::OutElastic,
        Ease::InOutElastic,
        Ease::InBounce,
        Ease::OutBounce,
        Ease::InOutBounce,
    ];

    /// How far along the change is at `t` of the way (clamped to 0..1).
    pub fn at(self, t: f32) -> f32 {
        use std::f32::consts::PI;
        let t = t.clamp(0.0, 1.0);
        // Back's overshoot, easings.net's: about 10%.
        const C1: f32 = 1.70158;
        const C2: f32 = C1 * 1.525;
        const C3: f32 = C1 + 1.0;
        const C4: f32 = 2.0 * PI / 3.0;
        const C5: f32 = 2.0 * PI / 4.5;
        match self {
            Ease::Linear => t,
            Ease::Smooth => t * t * (3.0 - 2.0 * t),
            Ease::InQuad => t * t,
            Ease::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::InOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Ease::InCubic => t * t * t,
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Ease::InSine => 1.0 - (t * PI / 2.0).cos(),
            Ease::OutSine => (t * PI / 2.0).sin(),
            Ease::InOutSine => -((PI * t).cos() - 1.0) / 2.0,
            Ease::InExpo => {
                if t == 0.0 {
                    0.0
                } else {
                    2f32.powf(10.0 * t - 10.0)
                }
            }
            Ease::OutExpo => {
                if t == 1.0 {
                    1.0
                } else {
                    1.0 - 2f32.powf(-10.0 * t)
                }
            }
            Ease::InOutExpo => {
                if t == 0.0 || t == 1.0 {
                    t
                } else if t < 0.5 {
                    2f32.powf(20.0 * t - 10.0) / 2.0
                } else {
                    (2.0 - 2f32.powf(-20.0 * t + 10.0)) / 2.0
                }
            }
            Ease::InBack => C3 * t * t * t - C1 * t * t,
            Ease::OutBack => 1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2),
            Ease::InOutBack => {
                if t < 0.5 {
                    (2.0 * t).powi(2) * ((C2 + 1.0) * 2.0 * t - C2) / 2.0
                } else {
                    ((2.0 * t - 2.0).powi(2) * ((C2 + 1.0) * (t * 2.0 - 2.0) + C2) + 2.0) / 2.0
                }
            }
            Ease::InElastic => {
                if t == 0.0 || t == 1.0 {
                    t
                } else {
                    -(2f32.powf(10.0 * t - 10.0)) * ((t * 10.0 - 10.75) * C4).sin()
                }
            }
            Ease::OutElastic => {
                if t == 0.0 || t == 1.0 {
                    t
                } else {
                    2f32.powf(-10.0 * t) * ((t * 10.0 - 0.75) * C4).sin() + 1.0
                }
            }
            Ease::InOutElastic => {
                if t == 0.0 || t == 1.0 {
                    t
                } else if t < 0.5 {
                    -(2f32.powf(20.0 * t - 10.0) * ((20.0 * t - 11.125) * C5).sin()) / 2.0
                } else {
                    2f32.powf(-20.0 * t + 10.0) * ((20.0 * t - 11.125) * C5).sin() / 2.0 + 1.0
                }
            }
            Ease::InBounce => 1.0 - out_bounce(1.0 - t),
            Ease::OutBounce => out_bounce(t),
            Ease::InOutBounce => {
                if t < 0.5 {
                    (1.0 - out_bounce(1.0 - 2.0 * t)) / 2.0
                } else {
                    (1.0 + out_bounce(2.0 * t - 1.0)) / 2.0
                }
            }
        }
    }

    /// From `a` to `b`, `t` of the way along this curve.
    pub fn lerp(self, a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * self.at(t)
    }
}

fn out_bounce(t: f32) -> f32 {
    const N: f32 = 7.5625;
    const D: f32 = 2.75;
    if t < 1.0 / D {
        N * t * t
    } else if t < 2.0 / D {
        let t = t - 1.5 / D;
        N * t * t + 0.75
    } else if t < 2.5 / D {
        let t = t - 2.25 / D;
        N * t * t + 0.9375
    } else {
        let t = t - 2.625 / D;
        N * t * t + 0.984375
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_nothing_and_ends_at_all_of_it() {
        for ease in Ease::ALL {
            assert!(ease.at(0.0).abs() < 1e-5, "{ease:?} at 0: {}", ease.at(0.0));
            assert!(
                (ease.at(1.0) - 1.0).abs() < 1e-5,
                "{ease:?} at 1: {}",
                ease.at(1.0)
            );
            // And past either end it holds, as a finished tween does.
            assert_eq!(ease.at(-1.0), ease.at(0.0), "{ease:?}");
            assert_eq!(ease.at(2.0), ease.at(1.0), "{ease:?}");
        }
    }

    #[test]
    fn the_curves_have_their_shapes() {
        assert_eq!(Ease::Linear.at(0.25), 0.25);
        assert!(Ease::InQuad.at(0.5) < 0.5, "slow to start");
        assert!(Ease::OutQuad.at(0.5) > 0.5, "slow to finish");
        for ease in [
            Ease::Smooth,
            Ease::InOutQuad,
            Ease::InOutCubic,
            Ease::InOutSine,
            Ease::InOutExpo,
        ] {
            assert!(
                (ease.at(0.5) - 0.5).abs() < 1e-5,
                "{ease:?} is half way at half time"
            );
        }
        assert!(Ease::InBack.at(0.2) < 0.0, "pulls back first");
        assert!(Ease::OutBack.at(0.7) > 1.0, "overshoots");
        let peak = (1..100)
            .map(|i| Ease::OutElastic.at(i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(peak > 1.2, "springs past: {peak}");
        let bounced = (1..100).map(|i| Ease::OutBounce.at(i as f32 / 100.0));
        assert!(
            bounced.clone().all(|v| v <= 1.0 + 1e-5),
            "a bounce never goes through the floor"
        );
        assert!(
            (Ease::OutBounce.at(1.0 / 2.75) - 1.0).abs() < 1e-5,
            "touches the end early"
        );
        assert_eq!(Ease::OutCubic.lerp(10.0, 20.0, 1.0), 20.0);
    }

    #[test]
    fn a_curve_is_written_by_its_name() {
        let ease: Ease = scrap_core::ron::from_str("OutBack").unwrap();
        assert_eq!(ease, Ease::OutBack);
        assert_eq!(
            scrap_core::ron::to_string(&Ease::InOutCubic).unwrap(),
            "InOutCubic"
        );
    }
}
