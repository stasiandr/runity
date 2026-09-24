//! A Cosserat rod: a chain of particles with a turn on each link between
//! them — what bends and twists, not just stretches. Ropes, cables and
//! chains are rods (and hair will be); a limp rope is one whose bending
//! gives, a cable one whose bending holds.
//!
//! Position and orientation based (Kugelstadt, Schömer 2016): each link's
//! turn carries its third axis along the link — the stretch-shear
//! constraint holds the two together — and neighbouring turns are held at
//! their rest difference, the Darboux vector — the bend-twist constraint.
//! Softness is XPBD's compliance, so a cable is as stiff at eight substeps
//! as at twenty.

use glam::{Quat, Vec3};

use crate::obstacle::Obstacle;
use crate::particles::Particles;

/// How a rod gives, each a compliance: 0 does not, more gives more.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Give {
    /// Along itself: a rope stretches a little, a chain not at all.
    pub stretch: f32,
    /// Bending: a rope bends at a touch, a cable holds its line.
    pub bend: f32,
    /// Twisting about itself.
    pub twist: f32,
}

/// A rod: its particles, and a turn on each link.
#[derive(Debug, Clone, PartialEq)]
pub struct Rod {
    pub particles: Particles,
    /// Each link's turn: its z axis along the link.
    pub turn: Vec<Quat>,
    turn_was: Vec<Quat>,
    /// Radians a second, in the world.
    spin: Vec<Vec3>,
    /// One over each link's resistance to turning; 0 is held.
    pub turn_w: Vec<f32>,
    /// Each link's length at rest.
    pub rest: Vec<f32>,
    /// How each pair of neighbouring links is turned from each other at
    /// rest: straight for a rope, curled for a coil.
    rest_bend: Vec<Quat>,
    pub give: Give,
    /// How thick, as a ball round each particle, metres.
    pub radius: f32,
    pub friction: f32,
    /// Of its speed, the share a second it loses to the air.
    pub damping: f32,
    /// Passes over its constraints a substep. The stretch-shear pass
    /// shares each correction between the particles and the turn, so a
    /// long heavy rod wants a second pass, and each pass also holds the
    /// links their length apart outright.
    pub passes: usize,
    /// Each constraint's multiplier this substep (XPBD's λ): what lets a
    /// second pass keep the compliance the first one had.
    held_stretch: Vec<Vec3>,
    held_bend: Vec<Vec3>,
    held_length: Vec<f32>,
    /// Ends held by bodies, which do not meet obstacles.
    pub held_ends: [bool; 2],
    /// The long-range attachments' pull on a pinned start, last substep,
    /// newtons.
    held_reach: Vec3,
}

impl Rod {
    /// A rod through `points`, each link as long at rest as `rest` says
    /// (so it may start stretched or slack), with no twist along it, each
    /// particle `mass` kilograms.
    pub fn new(points: Vec<Vec3>, rest: Vec<f32>, mass: f32, give: Give) -> Self {
        assert!(points.len() >= 2 && rest.len() + 1 == points.len());
        let links = rest.len();
        // Turns carried link to link by the least rotation, so a rod laid
        // along a curve starts untwisted.
        let mut turn = Vec::with_capacity(links);
        let mut last = Vec3::Z;
        let mut carried = Quat::IDENTITY;
        for k in 0..links {
            let along = (points[k + 1] - points[k]).normalize_or(last);
            carried = (Quat::from_rotation_arc(last, along) * carried).normalize();
            turn.push(carried);
            last = along;
        }
        let particles = Particles::new(points, mass);
        let turn_w = rest
            .iter()
            .map(|l| 1.0 / (mass.max(1e-6) * l.max(1e-3) * l.max(1e-3)))
            .collect();
        Self {
            particles,
            turn_was: turn.clone(),
            turn,
            spin: vec![Vec3::ZERO; links],
            turn_w,
            rest,
            rest_bend: vec![Quat::IDENTITY; links.saturating_sub(1)],
            give,
            radius: 0.01,
            friction: 0.4,
            damping: 0.3,
            passes: 2,
            held_stretch: vec![Vec3::ZERO; links],
            held_bend: vec![Vec3::ZERO; links.saturating_sub(1)],
            held_length: vec![0.0; links],
            held_ends: [false; 2],
            held_reach: Vec3::ZERO,
        }
    }

    /// How hard the rod pulls on each of its ends along its first and last
    /// links, newtons, from the last substep of `h` seconds: the tension a
    /// hand holding it feels. Nothing when a link is slack.
    pub fn end_pulls(&self, h: f32) -> [Vec3; 2] {
        let n = self.particles.len();
        let links = self.links();
        if links == 0 || h <= 0.0 {
            return [Vec3::ZERO; 2];
        }
        let pull = |end: usize, next: usize, k: usize| {
            let d = self.particles.x[next] - self.particles.x[end];
            let stretched = d.length() >= self.rest[k] * 0.999;
            if !stretched {
                return Vec3::ZERO;
            }
            d.normalize_or_zero() * (self.held_length[k].abs() / (h * h))
        };
        [pull(0, 1, 0) + self.held_reach, pull(n - 1, n - 2, links - 1)]
    }

    /// Every link put back to its length along the rod, from the start,
    /// and — when the end is held too — back from the end, the start and
    /// the end staying where they are: a shape made by hand (blended,
    /// carried forward) made into one the rod could have.
    pub fn restore_lengths(&mut self, end_held: bool) {
        let last = self.particles.len() - 1;
        let target = self.particles.x[last];
        for _ in 0..3 {
            for k in 0..last {
                let (a, b) = (self.particles.x[k], self.particles.x[k + 1]);
                let d = b - a;
                let len = d.length();
                if len > 1e-6 {
                    self.particles.x[k + 1] = a + d * (self.rest[k] / len);
                }
            }
            if !end_held {
                break;
            }
            self.particles.x[last] = target;
            for k in (1..last).rev() {
                let (a, b) = (self.particles.x[k + 1], self.particles.x[k]);
                let d = b - a;
                let len = d.length();
                if len > 1e-6 {
                    self.particles.x[k] = a + d * (self.rest[k] / len);
                }
            }
        }
    }

    /// Links: one fewer than the particles.
    pub fn links(&self) -> usize {
        self.rest.len()
    }

    /// Its length at rest.
    pub fn length(&self) -> f32 {
        self.rest.iter().sum()
    }

    /// Hold particle `i` where it is put ([`Particles::place`]), or let it
    /// go.
    pub fn pin(&mut self, i: usize, held: bool, mass: f32) {
        self.particles.w[i] = if held { 0.0 } else { 1.0 / mass.max(1e-6) };
    }

    /// Hold link `k`'s turn where it is put, or let it go: a cable clamped
    /// in a wall leaves it straight out.
    pub fn clamp(&mut self, k: usize, held: bool, mass: f32) {
        let l = self.rest[k].max(1e-3);
        self.turn_w[k] = if held { 0.0 } else { 1.0 / (mass.max(1e-6) * l * l) };
    }

    /// Put link `k` turned so, at rest.
    pub fn set_turn(&mut self, k: usize, turn: Quat) {
        self.turn[k] = turn;
        self.turn_was[k] = turn;
        self.spin[k] = Vec3::ZERO;
    }

    /// One substep of `h` seconds: each particle pulled by `accelerate(i)`,
    /// held to its links, and out of `obstacles`. Pinned particles and
    /// clamped links are where the caller put them before.
    pub fn substep(&mut self, h: f32, accelerate: impl Fn(usize) -> Vec3, obstacles: &[Obstacle]) {
        self.particles.predict(h, accelerate);
        for k in 0..self.links() {
            self.turn_was[k] = self.turn[k];
            if self.turn_w[k] > 0.0 {
                let w = self.spin[k];
                let turn = self.turn[k];
                self.turn[k] = (turn + Quat::from_xyzw(w.x, w.y, w.z, 0.0) * turn * (0.5 * h)).normalize();
            }
        }
        self.held_stretch.fill(Vec3::ZERO);
        self.held_bend.fill(Vec3::ZERO);
        self.held_length.fill(0.0);
        for _ in 0..self.passes.max(1) {
            for k in 0..self.links() {
                self.stretch_shear(k, h);
            }
            for k in 0..self.links().saturating_sub(1) {
                self.bend_twist(k, h);
            }
            for k in 0..self.links() {
                self.held_length[k] = crate::particles::distance(
                    &mut self.particles,
                    k,
                    k + 1,
                    self.rest[k],
                    self.give.stretch,
                    h,
                    self.held_length[k],
                );
            }
        }
        // Long-range attachments (Kim, Chentanez, Müller 2012): from a
        // pinned start no particle is further than the rod's length to it.
        // A heavy load on a light chain otherwise stretches it — the links
        // alone cannot hold ten kilograms in a few passes.
        self.held_reach = Vec3::ZERO;
        if self.particles.w[0] <= 0.0 {
            let origin = self.particles.x[0];
            let mut reach = 0.0;
            for i in 1..self.particles.len() {
                reach += self.rest[i - 1];
                if self.particles.w[i] <= 0.0 {
                    continue;
                }
                let d = self.particles.x[i] - origin;
                let far = d.length();
                if far > reach {
                    self.particles.x[i] = origin + d * (reach / far);
                    // What holding it back took, as a pull on the start.
                    let moved = far - reach;
                    self.held_reach += (d / far) * (moved / (self.particles.w[i] * h * h));
                }
            }
        }
        // The same from a pinned end, when the start is free.
        let last = self.particles.len() - 1;
        if self.particles.w[0] > 0.0 && self.particles.w[last] <= 0.0 {
            let origin = self.particles.x[last];
            let mut reach = 0.0;
            for i in (0..last).rev() {
                reach += self.rest[i];
                if self.particles.w[i] <= 0.0 {
                    continue;
                }
                let d = self.particles.x[i] - origin;
                let far = d.length();
                if far > reach {
                    self.particles.x[i] = origin + d * (reach / far);
                }
            }
        }
        // And the ends never further apart than the rod is long, each moved
        // by its weight: two bodies pulling a rope between them stretch it
        // no more than a rope stretches — with nothing pinned, the links'
        // passes alone let two heavy ends pull it half as long again.
        let (w0, w1) = (self.particles.w[0], self.particles.w[last]);
        if w0 + w1 > 0.0 {
            let total = self.length();
            let d = self.particles.x[last] - self.particles.x[0];
            let far = d.length();
            if far > total && far > 1e-9 {
                let n = d / far;
                let excess = far - total;
                self.particles.x[0] += n * (excess * w0 / (w0 + w1));
                self.particles.x[last] -= n * (excess * w1 / (w0 + w1));
            }
        }
        // An end held by a body is inside that body: it is the body that
        // meets things, not the end.
        let held = self.held_ends;
        let last = self.particles.len() - 1;
        if held == [false; 2] {
            self.particles.collide(self.radius, self.friction, obstacles);
        } else if !obstacles.is_empty() {
            for i in 0..=last {
                if (i == 0 && held[0]) || (i == last && held[1]) || self.particles.w[i] <= 0.0 {
                    continue;
                }
                let was = self.particles.was[i];
                crate::obstacle::collide(&mut self.particles.x[i], was, self.radius, self.friction, obstacles);
            }
        }
        self.particles.finish(h, self.damping);
        let keep = 1.0 / (1.0 + self.damping.max(0.0) * 4.0 * h);
        for k in 0..self.links() {
            let mut d = self.turn[k] * self.turn_was[k].conjugate();
            if d.w < 0.0 {
                d = -d;
            }
            self.spin[k] = Vec3::new(d.x, d.y, d.z) * (2.0 / h) * keep;
        }
    }

    /// Link `k`'s particles held its length apart along its turn's z axis.
    fn stretch_shear(&mut self, k: usize, h: f32) {
        let p = &mut self.particles;
        let (w0, w1, wq) = (p.w[k], p.w[k + 1], self.turn_w[k]);
        let l = self.rest[k].max(1e-6);
        let sum = (w0 + w1) / l + 4.0 * wq * l;
        if sum <= 0.0 {
            return;
        }
        let q = self.turn[k];
        let along = q * Vec3::Z;
        let off = (p.x[k + 1] - p.x[k]) / l - along;
        let soft = self.give.stretch / (h * h);
        let gamma = (off + self.held_stretch[k] * soft) / (sum + soft);
        self.held_stretch[k] -= gamma;
        p.x[k] += gamma * w0;
        p.x[k + 1] -= gamma * w1;
        // q ē₃: the turn times the z axis, taken back.
        let q_e3 = q * Quat::from_xyzw(0.0, 0.0, -1.0, 0.0);
        let fix = Quat::from_xyzw(gamma.x, gamma.y, gamma.z, 0.0) * q_e3 * (2.0 * wq * l);
        self.turn[k] = (q + fix).normalize();
    }

    /// Links `k` and `k + 1` held at their rest bend and twist.
    fn bend_twist(&mut self, k: usize, h: f32) {
        let (w0, w1) = (self.turn_w[k], self.turn_w[k + 1]);
        let sum = w0 + w1;
        if sum <= 0.0 {
            return;
        }
        let (q0, q1) = (self.turn[k], self.turn[k + 1]);
        let rest = self.rest_bend[k];
        let darboux = q0.conjugate() * q1;
        // A turn and its negation are the same turn: the nearer of the two.
        let minus = darboux - rest;
        let plus = darboux + rest;
        let off = if minus.length_squared() > plus.length_squared() { plus } else { minus };
        let hh = h * h;
        let soft = Vec3::new(self.give.bend, self.give.bend, self.give.twist) / hh;
        let held = self.held_bend[k];
        let fix = (Vec3::new(off.x, off.y, off.z) + held * soft) / (Vec3::splat(sum) + soft);
        self.held_bend[k] -= fix;
        let off = Quat::from_xyzw(fix.x, fix.y, fix.z, 0.0);
        let fix0 = q1 * off * w0;
        let fix1 = q0 * off * (-w1);
        self.turn[k] = (q0 + fix0).normalize();
        self.turn[k + 1] = (q1 + fix1).normalize();
    }

    /// How each pair of neighbours is turned now, kept as their rest: a rod
    /// laid in a curve keeps it.
    pub fn keep_shape(&mut self) {
        for k in 0..self.links().saturating_sub(1) {
            self.rest_bend[k] = self.turn[k].conjugate() * self.turn[k + 1];
        }
    }

    /// Particle `i`'s turn: the mean of the links either side.
    pub fn turn_at(&self, i: usize) -> Quat {
        let n = self.links();
        match i {
            0 => self.turn[0],
            _ if i >= n => self.turn[n - 1],
            _ => {
                let (a, mut b) = (self.turn[i - 1], self.turn[i]);
                if a.dot(b) < 0.0 {
                    b = -b;
                }
                a.slerp(b, 0.5)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f32 = 1.0 / 480.0;

    fn line(n: usize, length: f32) -> (Vec<Vec3>, Vec<f32>) {
        let l = length / (n - 1) as f32;
        let points = (0..n).map(|i| Vec3::new(i as f32 * l, 0.0, 0.0)).collect();
        (points, vec![l; n - 1])
    }

    fn run(rod: &mut Rod, seconds: f32) {
        for _ in 0..(seconds / H) as usize {
            rod.substep(H, |_| Vec3::new(0.0, -9.81, 0.0), &[]);
        }
    }

    #[test]
    fn a_limp_rod_hangs_straight_down_its_length() {
        let (points, rest) = line(16, 3.0);
        let give = Give { stretch: 0.0, bend: 1.0, twist: 1.0 };
        let mut rod = Rod::new(points, rest, 0.05, give);
        rod.pin(0, true, 0.05);
        rod.damping = 2.0;
        run(&mut rod, 4.0);
        let end = *rod.particles.x.last().unwrap();
        assert!(end.x.abs() < 0.1 && (end.y + 3.0).abs() < 0.05, "{end}");
        // Each link's turn points along it.
        for k in 0..rod.links() {
            let along = (rod.particles.x[k + 1] - rod.particles.x[k]).normalize();
            assert!((rod.turn[k] * Vec3::Z).dot(along) > 0.99);
        }
    }

    #[test]
    fn a_clamped_cable_holds_out_where_a_rope_would_droop() {
        // Held by one end, clamped level: a stiff cable sticks out, a limp
        // rope falls.
        let tip = |bend: f32| {
            let (points, rest) = line(12, 1.0);
            let mut rod = Rod::new(points, rest, 0.02, Give { stretch: 0.0, bend, twist: 0.0 });
            rod.pin(0, true, 0.02);
            rod.clamp(0, true, 0.02);
            run(&mut rod, 3.0);
            rod.particles.x.last().unwrap().y
        };
        let (cable, rope) = (tip(0.0), tip(1.0));
        assert!(cable > -0.35, "a cable droops a little: {cable}");
        assert!(rope < -0.8, "a rope falls: {rope}");
    }

    #[test]
    fn a_twist_at_one_end_winds_through_to_the_middle() {
        // Both ends pinned and clamped, no gravity; one end turned a
        // quarter about the rod: the twist spreads evenly along it, as
        // the least twisted it can be. (Were twisting not to give at all,
        // there would be no such rest to find.)
        let (points, rest) = line(11, 2.0);
        let mut rod = Rod::new(points, rest, 0.05, Give { stretch: 0.0, bend: 1e-3, twist: 1e-3 });
        let last = rod.links() - 1;
        for (i, k) in [(0, 0), (rod.particles.len() - 1, last)] {
            rod.pin(i, true, 0.05);
            rod.clamp(k, true, 0.05);
        }
        let turned = rod.turn[last] * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        rod.set_turn(last, turned);
        for _ in 0..4000 {
            rod.substep(H, |_| Vec3::ZERO, &[]);
        }
        let angles: Vec<f32> = rod
            .turn
            .iter()
            .map(|q| {
                let side = *q * Vec3::Y;
                side.z.atan2(side.y).abs()
            })
            .collect();
        // Evenly, link by link: the middle link by as much of the quarter
        // as it is along.
        let k = rod.links() / 2;
        let even = std::f32::consts::FRAC_PI_2 * k as f32 / last as f32;
        assert!((angles[last] - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert!((angles[k] - even).abs() < 0.1, "{angles:?}");
        assert!(angles.windows(2).all(|w| w[1] >= w[0] - 1e-3), "{angles:?}");
    }
}
