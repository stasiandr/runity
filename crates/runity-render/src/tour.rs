//! A camera's tour: from shot to shot on a Catmull-Rom curve, pausing at
//! each, round and round — a lobby's fly-through while players gather, a
//! title screen's, an attract mode. [`Flyby`] eases from the view a game
//! plays in to the tour and back, so play always starts from its own view,
//! and puts the tour's lens on: focused where it looks, the rest soft.
//!
//! A tour is data, RON, usually a [`crate::Tuned`] file — saved while the
//! game runs, the camera goes the new way at once:
//!
//! ```ron
//! (travel: 3.5, hold: 2.0, focal_length: 100.0, aperture: 1.4, shots: [
//!     (at: (0.0, 3.0, 7.0), look: (0.0, 0.8, 0.0)),
//!     (at: (-1.0, 2.5, 0.0), look: (-4.5, 0.8, -1.0)),
//! ])
//! ```
//!
//! Local to each peer: nothing of it is sent.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::render::Camera;

/// Where the camera stops, and what it looks at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub at: Vec3,
    pub look: Vec3,
}

/// The tour, from its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tour {
    /// Seconds from one shot to the next.
    pub travel: f32,
    /// Seconds at each.
    pub hold: f32,
    /// The lens on the tour: millimetres and the f-number. Focused on what
    /// it looks at, a long lens wide open leaves the rest soft.
    pub focal_length: f32,
    pub aperture: f32,
    pub shots: Vec<Shot>,
}

/// Seconds to ease into the tour, and out of it.
const EASE: f32 = 1.5;

/// How far round the tour the camera is, and how much of it is the tour's
/// rather than the room's view.
#[derive(Debug, Default)]
pub struct Flyby {
    time: f32,
    weight: f32,
}

impl Flyby {
    /// The camera this frame: `rest` — the room's view — while `touring` is
    /// false and the ease out is done, the tour's while it is true.
    pub fn camera(&mut self, tour: &Tour, touring: bool, seconds: f32, rest: Camera) -> Camera {
        let step = seconds / EASE;
        self.weight = (self.weight + if touring { step } else { -step }).clamp(0.0, 1.0);
        if self.weight == 0.0 {
            // Next time from the first shot.
            self.time = 0.0;
            return rest;
        }
        self.time += seconds;
        let Some((at, look)) = tour.at(self.time) else {
            return rest;
        };
        let w = smooth(self.weight);
        Camera {
            position: rest.position.lerp(at, w),
            target: rest.target.lerp(look, w),
            ..rest
        }
    }

    /// The tour's lens over the scene's, as much as the tour is the camera:
    /// in focus where it looks, the rest soft.
    pub fn lens(&self, tour: &Tour, camera: &Camera, dof: &mut crate::lens::DepthOfField) {
        if self.weight == 0.0 {
            return;
        }
        let w = smooth(self.weight);
        let lerp = |a: f32, b: f32| a + (b - a) * w;
        dof.focus_distance = lerp(dof.focus_distance, camera.target.distance(camera.position));
        dof.focal_length = lerp(dof.focal_length, tour.focal_length);
        dof.aperture = lerp(dof.aperture, tour.aperture);
    }

    /// Whether the tour has any part in the camera.
    pub fn flying(&self) -> bool {
        self.weight > 0.0
    }
}

impl Tour {
    /// Where the camera is and looks `time` seconds into the tour.
    pub fn at(&self, time: f32) -> Option<(Vec3, Vec3)> {
        let n = self.shots.len();
        if n == 0 {
            return None;
        }
        let leg = (self.hold + self.travel).max(1e-3);
        let lap = time.rem_euclid(leg * n as f32);
        let i = (lap / leg) as usize % n;
        let into = lap - i as f32 * leg;
        if into < self.hold || n == 1 {
            return Some((self.shots[i].at, self.shots[i].look));
        }
        let t = smooth((into - self.hold) / self.travel.max(1e-3));
        let shot = |k: usize| self.shots[(i + k) % n];
        let (p0, p1, p2, p3) = (shot(n - 1), shot(0), shot(1), shot(2));
        Some((
            catmull(p0.at, p1.at, p2.at, p3.at, t),
            catmull(p0.look, p1.look, p2.look, p3.look, t),
        ))
    }
}

/// Slow at both ends.
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The curve through `b` and `c`, bent by their neighbours.
fn catmull(a: Vec3, b: Vec3, c: Vec3, d: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tour() -> Tour {
        let shot = |x: f32| Shot {
            at: Vec3::new(x, 3.0, 0.0),
            look: Vec3::new(x, 0.0, -1.0),
        };
        Tour {
            travel: 2.0,
            hold: 1.0,
            focal_length: 85.0,
            aperture: 2.0,
            shots: vec![shot(0.0), shot(10.0), shot(20.0)],
        }
    }

    #[test]
    fn the_tour_holds_at_each_shot_and_travels_between_them_round_and_round() {
        let tour = tour();
        assert_eq!(tour.at(0.5).unwrap().0.x, 0.0, "held at the first");
        let halfway = tour.at(2.0).unwrap().0.x;
        assert!(halfway > 2.0 && halfway < 8.0, "on the way: {halfway}");
        let arriving = tour.at(2.98).unwrap().0.x;
        assert!((arriving - 10.0).abs() < 0.1, "all but there: {arriving}");
        assert_eq!(tour.at(3.5).unwrap().0.x, 10.0, "held at the second");
        assert_eq!(tour.at(9.5).unwrap().0.x, 0.0, "round again");
    }

    #[test]
    fn the_camera_eases_into_the_tour_and_back_to_the_room() {
        let tour = tour();
        let rest = Camera {
            position: Vec3::new(0.0, 9.0, 7.0),
            ..Camera::default()
        };
        let mut flyby = Flyby::default();
        assert_eq!(flyby.camera(&tour, false, 0.1, rest).position, rest.position);
        let first = flyby.camera(&tour, true, 0.1, rest).position;
        assert!(first.y < 9.0 && first.y > 3.0, "on its way in: {first}");
        for _ in 0..30 {
            flyby.camera(&tour, true, 0.1, rest);
        }
        assert!(flyby.camera(&tour, true, 0.0, rest).position.y - 3.0 < 1e-3, "on the tour");
        for _ in 0..30 {
            flyby.camera(&tour, false, 0.1, rest);
        }
        assert!(!flyby.flying());
        assert_eq!(flyby.camera(&tour, false, 0.1, rest).position, rest.position, "back");
    }
}
