//! Particles: what every soft thing is made of (docs/simulation.md, "один
//! решатель"). Points with a mass, held by constraints that a solver
//! satisfies one after another — XPBD (Macklin, Müller, Chentanez 2016) in
//! small substeps, one pass each (Macklin et al. 2019, "Small Steps").
//!
//! Stored as columns, as the solver walks them (DNA, postulate 6).

use glam::Vec3;

use crate::obstacle::{collide, Obstacle};

/// Points with a mass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Particles {
    /// Where each is.
    pub x: Vec<Vec3>,
    /// Where each was at the start of the substep.
    pub was: Vec<Vec3>,
    /// Metres a second.
    pub v: Vec<Vec3>,
    /// One over its mass; 0 is held where it is put — pinned.
    pub w: Vec<f32>,
}

impl Particles {
    /// Points at `x`, each of `mass` kilograms, at rest.
    pub fn new(x: Vec<Vec3>, mass: f32) -> Self {
        let n = x.len();
        Self {
            was: x.clone(),
            x,
            v: vec![Vec3::ZERO; n],
            w: vec![1.0 / mass.max(1e-6); n],
        }
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// Put particle `i` where it is at rest: nothing it did before counts.
    pub fn place(&mut self, i: usize, at: Vec3) {
        self.x[i] = at;
        self.was[i] = at;
        self.v[i] = Vec3::ZERO;
    }

    /// The start of a substep of `h` seconds: each free particle goes on
    /// where it was going, pulled by `accelerate(i)`.
    pub fn predict(&mut self, h: f32, accelerate: impl Fn(usize) -> Vec3) {
        for i in 0..self.x.len() {
            self.was[i] = self.x[i];
            if self.w[i] > 0.0 {
                self.v[i] += accelerate(i) * h;
                self.x[i] += self.v[i] * h;
            }
        }
    }

    /// Out of the obstacles, the free ones, each a ball of `radius`.
    pub fn collide(&mut self, radius: f32, friction: f32, obstacles: &[Obstacle]) {
        if obstacles.is_empty() {
            return;
        }
        for i in 0..self.x.len() {
            if self.w[i] > 0.0 {
                collide(&mut self.x[i], self.was[i], radius, friction, obstacles);
            }
        }
    }

    /// The end of a substep: speeds from how far each went, less `damping`
    /// (a share a second).
    pub fn finish(&mut self, h: f32, damping: f32) {
        let keep = 1.0 / (1.0 + damping.max(0.0) * h);
        for i in 0..self.x.len() {
            self.v[i] = (self.x[i] - self.was[i]) / h * keep;
        }
    }
}

/// Hold `a` and `b` `rest` apart, softly by `compliance` (metres a newton;
/// 0 is rigid) over a substep of `h`: one XPBD projection. `held` is the
/// constraint's multiplier so far this substep (0 on its first pass);
/// returned, it is what to pass next time.
pub fn distance(p: &mut Particles, a: usize, b: usize, rest: f32, compliance: f32, h: f32, held: f32) -> f32 {
    let (wa, wb) = (p.w[a], p.w[b]);
    let sum = wa + wb;
    if sum <= 0.0 {
        return held;
    }
    let d = p.x[b] - p.x[a];
    let apart = d.length();
    if apart < 1e-9 {
        return held;
    }
    let n = d / apart;
    let c = apart - rest;
    let soft = compliance / (h * h);
    let lambda = (-c - soft * held) / (sum + soft);
    p.x[a] -= n * (lambda * wa);
    p.x[b] += n * (lambda * wb);
    held + lambda
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compliance_makes_a_spring_as_stiff_whatever_the_substeps() {
        // A weight on a soft spring, hung from a pin: it settles where the
        // spring's stretch carries its weight — k = 1 / compliance — at 10
        // substeps a step as at 40.
        let settle = |substeps: usize| {
            let mut p = Particles::new(vec![Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0)], 1.0);
            p.w[0] = 0.0;
            let h = 1.0 / 60.0 / substeps as f32;
            for _ in 0..600 * substeps {
                p.predict(h, |_| Vec3::new(0.0, -9.81, 0.0));
                distance(&mut p, 0, 1, 1.0, 0.01, h, 0.0);
                p.finish(h, 2.0);
            }
            -p.x[1].y - 1.0
        };
        let (coarse, fine) = (settle(10), settle(40));
        // 9.81 N on 100 N/m: 9.8 cm.
        assert!((coarse - 0.0981).abs() < 0.005, "{coarse}");
        assert!((fine - coarse).abs() < 0.003, "{coarse} vs {fine}");
    }
}
