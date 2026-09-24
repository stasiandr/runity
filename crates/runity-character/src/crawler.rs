//! Crawlers: a line's `crawler`. `crawler: (legs: 6)` on an entity grows
//! it legs that walk it wherever it is moved — by a route, by the game —
//! a spider, a crab, a walking machine: procedural animation. Each foot
//! stays planted on the ground where it stepped; when the body has moved
//! so far that a foot lags past `step` from where it would stand, it lifts
//! and swings in an arc to a new place a little ahead, the legs in two
//! groups so that half always stand. Each leg is two bones from its hip
//! to its foot, the knee found by IK ([`crate::ik::two_bone`]) and bent up
//! and out. Feet go down onto what is solid under them: slopes, steps,
//! crates.

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::ik::two_bone;

/// A crawler, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crawler {
    /// Legs: an even number, in pairs down its sides.
    pub legs: u32,
    /// Half its body across (x) and long (z), where the hips are, metres.
    pub span: Vec2,
    /// A leg's length, hip to foot.
    pub leg: f32,
    /// How far a foot lags before it steps.
    pub step: f32,
    /// How high a foot lifts, stepping.
    pub lift: f32,
    /// Seconds a step takes.
    pub stride: f32,
    /// How thick a leg is.
    pub thickness: f32,
}

impl Default for Crawler {
    fn default() -> Self {
        Self { legs: 6, span: Vec2::new(0.25, 0.35), leg: 1.0, step: 0.35, lift: 0.18, stride: 0.22, thickness: 0.05 }
    }
}

runity_core::impl_parts! {
    Crawler => "crawler";
}

/// The crawler of a line, read off it.
pub trait CrawlerLine {
    fn crawler(&self) -> Option<Crawler>;
}

impl CrawlerLine for runity_core::EntityDesc {
    fn crawler(&self) -> Option<Crawler> {
        self.part()
    }
}

impl CrawlerLine for runity_core::scene::Override {
    fn crawler(&self) -> Option<Crawler> {
        self.part()
    }
}

/// A foot's step under way: from, to, and how far along.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Step {
    from: Vec3,
    to: Vec3,
    t: f32,
}

/// A crawler as it walks: the component the facade steps.
#[derive(Debug, Clone)]
pub struct CrawlerState {
    pub crawler: Crawler,
    pub feet: Vec<Vec3>,
    steps: Vec<Option<Step>>,
    /// Each leg now: hip, knee, foot.
    pub legs: Vec<[Vec3; 3]>,
    was: Option<Vec3>,
    velocity: Vec3,
    /// Steps taken, all legs.
    pub stepped: usize,
}

impl CrawlerState {
    pub fn new(crawler: Crawler) -> Self {
        Self { crawler, feet: Vec::new(), steps: Vec::new(), legs: Vec::new(), was: None, velocity: Vec3::ZERO, stepped: 0 }
    }

    /// Leg `i`'s hip on the body, and which way is out from it, in the
    /// body's space.
    fn hip(&self, i: usize) -> (Vec3, Vec3) {
        let pairs = (self.crawler.legs.max(2) / 2) as usize;
        let row = i / 2;
        let side = if i % 2 == 0 { -1.0 } else { 1.0 };
        let z = if pairs > 1 { -self.crawler.span.y + 2.0 * self.crawler.span.y * row as f32 / (pairs - 1) as f32 } else { 0.0 };
        let hip = Vec3::new(side * self.crawler.span.x, 0.0, z);
        // Splayed: the front legs forward, the back legs back.
        let out = Vec3::new(side, 0.0, z / self.crawler.span.y.max(1e-3) * 0.6).normalize();
        (hip, out)
    }

    /// Where leg `i`'s foot would stand, the body at `body`, on the ground
    /// that `ground` gives the height of.
    fn home(&self, i: usize, body: Mat4, ground: &impl Fn(Vec3) -> f32) -> Vec3 {
        let (hip, out) = self.hip(i);
        let turn = body.to_scale_rotation_translation().1;
        let at = body.transform_point3(hip) + turn * out * (self.crawler.leg * 0.6);
        Vec3::new(at.x, ground(at), at.z)
    }

    /// Two groups, as a tripod: legs of one step while the other stands.
    fn group(i: usize) -> usize {
        (i / 2 + i % 2) % 2
    }

    /// On by `seconds`, the body now at `body`, over ground `ground`.
    pub fn advance(&mut self, body: Mat4, ground: impl Fn(Vec3) -> f32, seconds: f32) {
        let n = (self.crawler.legs.max(2) & !1) as usize;
        let centre = body.w_axis.truncate();
        if self.feet.len() != n {
            self.feet = (0..n).map(|i| self.home(i, body, &ground)).collect();
            self.steps = vec![None; n];
        }
        if let Some(was) = self.was {
            if seconds > 0.0 {
                let v = (centre - was) / seconds;
                self.velocity = self.velocity.lerp(v, 0.3);
            }
        }
        self.was = Some(centre);
        let duration = self.crawler.stride.max(0.02);
        // Steps under way go on.
        for i in 0..n {
            if let Some(step) = self.steps[i].as_mut() {
                step.t = (step.t + seconds / duration).min(1.0);
                let t = step.t;
                let arc = (t * std::f32::consts::PI).sin() * self.crawler.lift;
                self.feet[i] = step.from.lerp(step.to, t * t * (3.0 - 2.0 * t)) + Vec3::Y * arc;
                if t >= 1.0 {
                    self.feet[i] = step.to;
                    self.steps[i] = None;
                    self.stepped += 1;
                }
            }
        }
        // Feet lagging too far step, a group at a time, the one that lags
        // most first.
        let stepping_group = (0..n).find(|i| self.steps[*i].is_some()).map(Self::group);
        let mut worst: Vec<(f32, usize)> = (0..n)
            .filter(|i| self.steps[*i].is_none())
            .map(|i| {
                let home = self.home(i, body, &ground);
                (Vec2::new(home.x - self.feet[i].x, home.z - self.feet[i].z).length(), i)
            })
            .filter(|(d, _)| *d > self.crawler.step)
            .collect();
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        if let Some(&(_, first)) = worst.first() {
            let group = stepping_group.unwrap_or(Self::group(first));
            for &(_, i) in &worst {
                if Self::group(i) != group {
                    continue;
                }
                // A little ahead of where it would stand: it will be
                // passed as the body goes on.
                let lead = Vec3::new(self.velocity.x, 0.0, self.velocity.z) * duration * 1.5;
                let ahead = self.home(i, body, &ground) + lead;
                let to = Vec3::new(ahead.x, ground(ahead), ahead.z);
                self.steps[i] = Some(Step { from: self.feet[i], to, t: 0.0 });
            }
        }
        // Each leg by IK: hip to foot, the knee up and out.
        let half = self.crawler.leg * 0.5;
        self.legs = (0..n)
            .map(|i| {
                let (hip, out) = self.hip(i);
                let turn = body.to_scale_rotation_translation().1;
                let hip = body.transform_point3(hip);
                let pole = hip + turn * out * half + Vec3::Y * self.crawler.leg;
                let (knee, foot) = two_bone(hip, half, half, self.feet[i], pole);
                [hip, knee, foot]
            })
            .collect();
    }

    /// Its legs' bones, each from and to, for drawing.
    pub fn bones(&self) -> Vec<(Vec3, Vec3)> {
        self.legs.iter().flat_map(|l| [(l[0], l[1]), (l[1], l[2])]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spider_walked_along_steps_its_feet_down_the_way_and_over_a_step() {
        let mut state = CrawlerState::new(Crawler::default());
        // Flat ground, then a step up 0.2 m past x = 1.
        let ground = |p: Vec3| if p.x > 1.0 { 0.2 } else { 0.0 };
        for frame in 0..180 {
            let x = frame as f32 * 0.02;
            let body = Mat4::from_rotation_translation(glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::new(x, 0.6, 0.0));
            state.advance(body, ground, 1.0 / 60.0);
            // The legs keep their length.
            for leg in &state.legs {
                assert!((leg[0].distance(leg[1]) - 0.5).abs() < 1e-3 && (leg[1].distance(leg[2]) - 0.5).abs() < 1e-3);
            }
            // Never more than one group in the air.
            let lifted: Vec<usize> = (0..6).filter(|i| state.steps[*i].is_some()).collect();
            assert!(lifted.iter().all(|i| CrawlerState::group(*i) == CrawlerState::group(lifted[0])));
        }
        assert!(state.stepped >= 12, "it walked: {} steps", state.stepped);
        // Standing still now: every foot planted, on the ground under it.
        for _ in 0..30 {
            let body = Mat4::from_rotation_translation(glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::new(3.58, 0.6, 0.0));
            state.advance(body, ground, 1.0 / 60.0);
        }
        for f in &state.feet {
            assert!((f.y - ground(*f)).abs() < 1e-4, "on the ground: {f}");
            assert!(f.x > 2.4, "come along: {f}");
        }
    }
}
