//! The running side of a line's `route`: entities travelling on their own.
//!
//! [`run_routes`] is the system: in the fixed step, before the hierarchy
//! and physics, it moves each travelling entity's local position along its
//! way at its speed — evenly, by distance along the curve, not by point —
//! so a platform does not hurry through long legs and crawl through short
//! ones. A `Kinematic` body so moved carries what stands on it.

use glam::Vec3;

use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use crate::defaults::*;
use crate::scene::Transform;

/// Pieces a smooth leg is measured and moved along.
const PIECES: usize = 16;

/// A route being travelled.
#[derive(Debug, Clone, PartialEq)]
pub struct Travelling {
    pub route: Route,
    /// Where the entity stood in its parent when it set off: the route's
    /// points are from here.
    pub origin: Vec3,
    /// How far along, in metres.
    pub along: f32,
    /// 1 going out, −1 coming back.
    heading: f32,
    /// Seconds left to wait.
    waiting: f32,
    /// The way as a line of short pieces: the points it passes, and the
    /// distance from the start to each.
    line: Vec<(Vec3, f32)>,
}

impl Travelling {
    pub fn new(route: Route, origin: Vec3) -> Self {
        let line = measure(&route);
        Self {
            route,
            origin,
            along: 0.0,
            heading: 1.0,
            waiting: 0.0,
            line,
        }
    }

    /// The way's length, in metres.
    pub fn length(&self) -> f32 {
        self.line.last().map_or(0.0, |(_, d)| *d)
    }

    /// Where the entity is now, in its parent's space.
    pub fn position(&self) -> Vec3 {
        self.origin + self.at(self.along)
    }

    /// The way itself, in its parent's space, as short straight pieces: what
    /// an editor draws.
    pub fn points(&self) -> Vec<Vec3> {
        self.line.iter().map(|(p, _)| self.origin + *p).collect()
    }

    fn at(&self, along: f32) -> Vec3 {
        let Some(i) = self.line.iter().position(|(_, d)| *d >= along) else {
            return self.line.last().map_or(Vec3::ZERO, |(p, _)| *p);
        };
        if i == 0 {
            return self.line[0].0;
        }
        let ((a, da), (b, db)) = (self.line[i - 1], self.line[i]);
        a.lerp(b, (along - da) / (db - da).max(1e-6))
    }

    /// Move on by `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        let length = self.length();
        if length <= 0.0 {
            return;
        }
        if self.waiting > 0.0 {
            self.waiting -= dt;
            return;
        }
        self.along += self.heading * self.route.speed.max(0.0) * dt;
        match self.route.ends {
            RouteEnds::Loop => {
                if self.along >= length {
                    self.along -= length;
                    self.waiting = self.route.pause;
                }
            }
            RouteEnds::Back => {
                if self.along >= length || self.along <= 0.0 {
                    self.along = self.along.clamp(0.0, length);
                    self.heading = -self.heading;
                    self.waiting = self.route.pause;
                }
            }
            RouteEnds::Stop => self.along = self.along.min(length),
        }
    }
}

/// The way as short pieces with their distances from the start.
fn measure(route: &Route) -> Vec<(Vec3, f32)> {
    let mut points = route.points.clone();
    if route.ends == RouteEnds::Loop {
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
    }
    let mut line: Vec<Vec3> = Vec::new();
    for i in 0..points.len().saturating_sub(1) {
        let (p1, p2) = (points[i], points[i + 1]);
        if !route.smooth {
            line.push(p1);
            continue;
        }
        // Catmull-Rom: through every point, its bend set by the neighbours.
        let p0 = if i == 0 { p1 } else { points[i - 1] };
        let p3 = points.get(i + 2).copied().unwrap_or(p2);
        for k in 0..PIECES {
            let t = k as f32 / PIECES as f32;
            let (t2, t3) = (t * t, t * t * t);
            line.push(
                0.5 * ((2.0 * p1)
                    + (-p0 + p2) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3),
            );
        }
    }
    if let Some(last) = points.last() {
        line.push(*last);
    }
    let mut out = Vec::with_capacity(line.len());
    let mut distance = 0.0;
    for (i, p) in line.iter().enumerate() {
        if i > 0 {
            distance += (*p - line[i - 1]).length();
        }
        out.push((*p, distance));
    }
    out
}

/// Move every travelling entity on: call it in the fixed step, before
/// [`crate::world::apply_hierarchy`] and physics.
pub fn run_routes(world: &mut hecs::World, dt: f32) {
    for (transform, travelling) in world.query_mut::<(&mut Transform, &mut Travelling)>() {
        travelling.advance(dt);
        transform.position = travelling.position();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(text: &str) -> Route {
        ron::from_str(text).unwrap()
    }

    #[test]
    fn a_lift_goes_up_waits_and_comes_back_at_its_speed() {
        let mut lift = Travelling::new(
            route("(points: [(0.0, 0.0, 0.0), (0.0, 4.0, 0.0)], speed: 2.0, pause: 1.0)"),
            Vec3::new(3.0, 0.0, 0.0),
        );
        assert!((lift.length() - 4.0).abs() < 1e-4);
        for _ in 0..60 {
            lift.advance(1.0 / 60.0);
        }
        assert!(
            (lift.position() - Vec3::new(3.0, 2.0, 0.0)).length() < 0.05,
            "a second: 2 m up"
        );
        for _ in 0..60 {
            lift.advance(1.0 / 60.0);
        }
        assert!((lift.position().y - 4.0).abs() < 0.05, "at the top");
        for _ in 0..30 {
            lift.advance(1.0 / 60.0);
        }
        assert!((lift.position().y - 4.0).abs() < 0.05, "waiting there");
        for _ in 0..90 {
            lift.advance(1.0 / 60.0);
        }
        assert!(
            lift.position().y < 3.5,
            "on its way down: {}",
            lift.position().y
        );
    }

    #[test]
    fn a_smooth_loop_passes_through_its_points_at_an_even_pace() {
        let square = "(points: [(0.0, 0.0, 0.0), (4.0, 0.0, 0.0), (4.0, 0.0, 4.0), (0.0, 0.0, 4.0)], speed: 1.0, ends: Loop)";
        let boat = Travelling::new(route(square), Vec3::ZERO);
        let way = boat.points();
        for corner in [Vec3::new(4.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 4.0)] {
            assert!(
                way.iter().any(|p| (*p - corner).length() < 1e-3),
                "passes {corner}"
            );
        }
        // Even pace: equal times cover equal distances along the curve.
        let mut boat = boat;
        let mut steps = Vec::new();
        let mut last = boat.position();
        for _ in 0..8 {
            for _ in 0..30 {
                boat.advance(1.0 / 60.0);
            }
            steps.push((boat.position() - last).length());
            last = boat.position();
        }
        let (low, high) = steps
            .iter()
            .fold((f32::MAX, 0.0f32), |(a, b), s| (a.min(*s), b.max(*s)));
        assert!(high - low < 0.08, "{steps:?}");
        let straight = Travelling::new(
            route(&square.replace("Loop)", "Loop, smooth: false)")),
            Vec3::ZERO,
        );
        assert!(
            (straight.length() - 16.0).abs() < 1e-3,
            "four straight legs round"
        );
    }
}

/// The routes module's dresser ([`crate::world::Dress`]): a line's `route`,
/// travelled from where it stands. Changed, it starts again.
pub struct RouteDress;

impl crate::world::Dress for RouteDress {
    fn parts(&self) -> &[&'static str] {
        &["route"]
    }

    fn dress(
        &mut self,
        line: &crate::scene::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: crate::world::Changed,
        _: &mut Vec<crate::world::Unresolved>,
    ) {
        match line.route() {
            Some(route) => {
                let _ = world.insert_one(entity, Travelling::new(route, line.transform.position));
            }
            None => {
                scrap_core::world::take_off::<Travelling>(world, entity);
            }
        }
    }
}

/// A way an entity travels by itself — a moving platform, a lift, a boat
/// on a loop, a cart on a track: Unity's Splines with SplineAnimate.
/// `route: (points: [(0.0, 0.0, 0.0), (0.0, 4.0, 0.0)], speed: 1.5, ends:
/// Back)` — points relative to where the entity stands in its parent,
/// passed through on a smooth curve (`smooth: false` for straight legs).
/// Give it a `Kinematic` body and what stands on it rides along.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub points: Vec<Vec3>,
    /// Metres a second along the way.
    #[serde(default = "unit")]
    pub speed: f32,
    #[serde(default)]
    pub ends: RouteEnds,
    #[serde(default = "yes_route")]
    pub smooth: bool,
    /// Seconds to wait at each end (Back) or at the start of each lap (Loop).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pause: f32,
}

/// What a route does at its last point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RouteEnds {
    /// Back the way it came: a lift, a door that slides.
    #[default]
    Back,
    /// On from the last point to the first: a loop.
    Loop,
    /// Stop there: a drawbridge lowered once.
    Stop,
}

fn yes_route() -> bool {
    true
}

crate::impl_parts! {
    Route => "route";
}

/// The way a line of a scene travels by itself, read off it.
pub trait RouteLine {
    fn route(&self) -> Option<Route>;
}

impl RouteLine for crate::scene::EntityDesc {
    fn route(&self) -> Option<Route> {
        self.part()
    }
}

impl RouteLine for crate::scene::Override {
    fn route(&self) -> Option<Route> {
        self.part()
    }
}
