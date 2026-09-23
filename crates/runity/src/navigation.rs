//! Where something can walk, and how to get there.
//!
//! Unity's NavMesh, as a grid: the level is sampled from above, every cell
//! a ray down onto the solid, static world. A cell is walkable if the
//! surface there is not too steep; two cells connect if the step between
//! them is not too high; and the walkable area shrinks by the walker's
//! radius, so a path does not scrape along walls. [`NavGrid::path`] is A*
//! over that grid, pulled straight wherever the straight line is walkable.
//!
//! A grid rather than polygons: it bakes from the same colliders physics
//! uses, in a few milliseconds for a greybox level, with no second
//! representation of the world to keep in step — and "can the player get
//! from the spawn to the exit" is a question it answers the same way every
//! time. Rebake after the level changes; it is cheap.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use glam::{Vec2, Vec3};

use crate::physics::PhysicsWorld;

/// What kind of walker, and how finely to look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavSettings {
    /// The side of a cell, in metres.
    pub cell: f32,
    /// How far from walls the walker's centre keeps.
    pub radius: f32,
    /// Steeper than this, in degrees, is a wall.
    pub max_slope: f32,
    /// Higher than this, in metres, is a ledge, not a step.
    pub max_step: f32,
    /// Where the rays start: above the highest ground.
    pub ceiling: f32,
}

impl Default for NavSettings {
    fn default() -> Self {
        Self {
            cell: 0.25,
            radius: 0.35,
            max_slope: 40.0,
            max_step: 0.3,
            ceiling: 100.0,
        }
    }
}

/// A baked walkable grid.
#[derive(Debug, Clone)]
pub struct NavGrid {
    origin: Vec2,
    cell: f32,
    width: usize,
    depth: usize,
    /// Ground height per cell; `None` where there is no walkable ground.
    ground: Vec<Option<f32>>,
    max_step: f32,
}

#[derive(Copy, Clone, PartialEq)]
struct Open {
    cost: f32,
    cell: usize,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.cell.cmp(&other.cell))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl NavGrid {
    /// Sample the static world between `min` and `max` (x and z) from above.
    pub fn bake(physics: &PhysicsWorld, min: Vec2, max: Vec2, settings: NavSettings) -> Self {
        let cell = settings.cell.max(0.01);
        let width = (((max.x - min.x) / cell).ceil() as usize).max(1);
        let depth = (((max.y - min.y) / cell).ceil() as usize).max(1);
        let cos_limit = settings.max_slope.to_radians().cos();
        let mut ground: Vec<Option<f32>> = (0..width * depth)
            .map(|k| {
                let centre = min + (Vec2::new((k % width) as f32, (k / width) as f32) + 0.5) * cell;
                let from = Vec3::new(centre.x, settings.ceiling, centre.y);
                let (point, normal, _) = physics.cast_ray_with_normal(
                    from,
                    Vec3::NEG_Y,
                    settings.ceiling * 2.0 + 1000.0,
                    true,
                )?;
                (normal.y >= cos_limit).then_some(point.y)
            })
            .collect();

        // A cell beside a drop or a wall is an edge; a walker's centre keeps
        // `radius` away from every edge.
        let reach = (settings.radius / cell).ceil() as isize;
        if reach > 0 {
            let edge: Vec<bool> = (0..width * depth)
                .map(|k| {
                    let Some(h) = ground[k] else { return true };
                    let (x, z) = ((k % width) as isize, (k / width) as isize);
                    [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dz)| {
                        let (nx, nz) = (x + dx, z + dz);
                        if nx < 0 || nz < 0 || nx >= width as isize || nz >= depth as isize {
                            return false;
                        }
                        ground[nz as usize * width + nx as usize]
                            .is_none_or(|n| (n - h).abs() > settings.max_step)
                    })
                })
                .collect();
            let near_edge: Vec<bool> = (0..width * depth)
                .map(|k| {
                    let (x, z) = ((k % width) as isize, (k / width) as isize);
                    (-reach..=reach).any(|dz| {
                        (-reach..=reach).any(|dx| {
                            let (nx, nz) = (x + dx, z + dz);
                            nx >= 0
                                && nz >= 0
                                && nx < width as isize
                                && nz < depth as isize
                                && (dx * dx + dz * dz) as f32 * cell * cell
                                    <= settings.radius * settings.radius
                                && edge[nz as usize * width + nx as usize]
                                && ground[nz as usize * width + nx as usize].is_some()
                        })
                    })
                })
                .collect();
            for (k, near) in near_edge.into_iter().enumerate() {
                if near {
                    ground[k] = None;
                }
            }
        }
        Self {
            origin: min,
            cell,
            width,
            depth,
            ground,
            max_step: settings.max_step,
        }
    }

    /// How many cells a walker can stand on.
    pub fn walkable_cells(&self) -> usize {
        self.ground.iter().filter(|g| g.is_some()).count()
    }

    fn cell_of(&self, at: Vec3) -> Option<usize> {
        let local = (Vec2::new(at.x, at.z) - self.origin) / self.cell;
        let (x, z) = (local.x.floor(), local.y.floor());
        (x >= 0.0 && z >= 0.0 && (x as usize) < self.width && (z as usize) < self.depth)
            .then(|| z as usize * self.width + x as usize)
    }

    fn centre(&self, k: usize) -> Vec3 {
        let c = self.origin
            + (Vec2::new((k % self.width) as f32, (k / self.width) as f32) + 0.5) * self.cell;
        Vec3::new(c.x, self.ground[k].unwrap_or(0.0), c.y)
    }

    /// The walkable ground as strips along x — runs of cells side by side
    /// at about one height — each as (its middle, its length); a strip is
    /// one cell deep. What an editor draws to show where a walker can go,
    /// in hundreds of pieces rather than a piece per cell.
    pub fn strips(&self) -> Vec<(Vec3, f32)> {
        let mut out = Vec::new();
        for z in 0..self.depth {
            let mut x = 0;
            while x < self.width {
                let k = z * self.width + x;
                let Some(h) = self.ground[k] else {
                    x += 1;
                    continue;
                };
                let mut end = x + 1;
                while end < self.width
                    && self.ground[z * self.width + end].is_some_and(|g| (g - h).abs() < 0.05)
                {
                    end += 1;
                }
                let (first, last) = (self.centre(k), self.centre(z * self.width + end - 1));
                out.push((
                    Vec3::new((first.x + last.x) * 0.5, h, first.z),
                    (end - x) as f32 * self.cell,
                ));
                x = end;
            }
        }
        out
    }

    /// The side of a cell, in metres.
    pub fn cell(&self) -> f32 {
        self.cell
    }

    /// Whether something can stand here.
    pub fn is_walkable(&self, at: Vec3) -> bool {
        self.cell_of(at).is_some_and(|k| self.ground[k].is_some())
    }

    /// The nearest walkable cell to a point, within a few cells: a spawn
    /// point placed a hair inside a wall should still start a path.
    fn nearest(&self, at: Vec3) -> Option<usize> {
        let start = self.cell_of(at)?;
        if self.ground[start].is_some() {
            return Some(start);
        }
        let (x, z) = ((start % self.width) as isize, (start / self.width) as isize);
        (1..=4isize).find_map(|r| {
            (-r..=r)
                .flat_map(|dz| (-r..=r).map(move |dx| (dx, dz)))
                .filter_map(|(dx, dz)| {
                    let (nx, nz) = (x + dx, z + dz);
                    (nx >= 0 && nz >= 0 && nx < self.width as isize && nz < self.depth as isize)
                        .then(|| nz as usize * self.width + nx as usize)
                })
                .filter(|k| self.ground[*k].is_some())
                .min_by(|a, b| {
                    self.centre(*a)
                        .distance_squared(at)
                        .total_cmp(&self.centre(*b).distance_squared(at))
                })
        })
    }

    fn neighbours(&self, k: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
        let (x, z) = ((k % self.width) as isize, (k / self.width) as isize);
        let h = self.ground[k];
        [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ]
        .into_iter()
        .filter_map(move |(dx, dz)| {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= self.width as isize || nz >= self.depth as isize {
                return None;
            }
            let n = nz as usize * self.width + nx as usize;
            let (h, nh) = (h?, self.ground[n]?);
            if (nh - h).abs() > self.max_step {
                return None;
            }
            // A diagonal only where both sides are open: no cutting corners.
            if dx != 0 && dz != 0 {
                let a = z as usize * self.width + nx as usize;
                let b = nz as usize * self.width + x as usize;
                self.ground[a]?;
                self.ground[b]?;
            }
            let step = if dx != 0 && dz != 0 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
            Some((n, step * self.cell))
        })
    }

    /// A walkable path from `from` to `to`, as points on the ground, or
    /// `None` when there is none. Straightened: a corner only where the
    /// straight line would leave walkable ground.
    pub fn path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let (start, goal) = (self.nearest(from)?, self.nearest(to)?);
        let n = self.ground.len();
        let mut cost = vec![f32::INFINITY; n];
        let mut came = vec![usize::MAX; n];
        let mut open = BinaryHeap::new();
        let target = self.centre(goal);
        cost[start] = 0.0;
        open.push(Open {
            cost: self.centre(start).distance(target),
            cell: start,
        });
        while let Some(Open { cell, .. }) = open.pop() {
            if cell == goal {
                break;
            }
            for (next, step) in self.neighbours(cell) {
                let through = cost[cell] + step;
                if through < cost[next] {
                    cost[next] = through;
                    came[next] = cell;
                    open.push(Open {
                        cost: through + self.centre(next).distance(target),
                        cell: next,
                    });
                }
            }
        }
        if !cost[goal].is_finite() {
            return None;
        }
        let mut cells = vec![goal];
        while *cells.last()? != start {
            cells.push(came[*cells.last()?]);
        }
        cells.reverse();

        // String-pulling: from each kept point, skip ahead to the farthest
        // cell still in a straight, walkable line.
        let mut points = vec![start];
        let mut at = 0;
        while at + 1 < cells.len() {
            let mut far = at + 1;
            for candidate in (at + 2..cells.len()).rev() {
                if self.clear(cells[at], cells[candidate]) {
                    far = candidate;
                    break;
                }
            }
            points.push(cells[far]);
            at = far;
        }
        let mut path: Vec<Vec3> = points.into_iter().map(|k| self.centre(k)).collect();
        if let Some(last) = path.last_mut() {
            *last = Vec3::new(to.x, last.y, to.z);
        }
        if let Some(first) = path.first_mut() {
            *first = Vec3::new(from.x, first.y, from.z);
        }
        Some(path)
    }

    /// Whether the straight line between two cells stays on walkable ground
    /// with no ledge along it.
    fn clear(&self, a: usize, b: usize) -> bool {
        let (pa, pb) = (self.centre(a), self.centre(b));
        let samples = ((pa.distance(pb) / (self.cell * 0.5)).ceil() as usize).max(1);
        let mut last = self.ground[a];
        for i in 1..=samples {
            let p = pa.lerp(pb, i as f32 / samples as f32);
            let Some(k) = self.cell_of(p) else {
                return false;
            };
            let Some(h) = self.ground[k] else {
                return false;
            };
            if last.is_some_and(|l| (h - l).abs() > self.max_step) {
                return false;
            }
            last = Some(h);
        }
        true
    }

    /// The length of a path, in metres along the ground.
    pub fn length(path: &[Vec3]) -> f32 {
        path.windows(2).map(|w| w[0].distance(w[1])).sum()
    }
}

/// Where an agent is in getting where it was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavStatus {
    /// Nowhere to go.
    #[default]
    Idle,
    Moving,
    Arrived,
    /// No walkable way from where it stands to where it was sent.
    Unreachable,
}

/// Something that walks itself to where it is sent: Unity's NavMeshAgent,
/// as a component. The game says `go_to`; [`move_agents`] finds the way on
/// a [`NavGrid`] and walks it at `speed`, turning to face where it goes,
/// on the ground the grid found. For entities at the top of the tree —
/// their transform is the world's.
#[derive(Debug, Clone, PartialEq)]
pub struct NavAgent {
    /// Metres per second.
    pub speed: f32,
    destination: Option<Vec3>,
    path: Vec<Vec3>,
    next: usize,
    status: NavStatus,
    needs_path: bool,
}

impl NavAgent {
    pub fn new(speed: f32) -> Self {
        Self {
            speed,
            destination: None,
            path: Vec::new(),
            next: 0,
            status: NavStatus::Idle,
            needs_path: false,
        }
    }

    /// Walk there. The way is found on the next [`move_agents`].
    pub fn go_to(&mut self, destination: Vec3) {
        self.destination = Some(destination);
        self.needs_path = true;
        self.status = NavStatus::Moving;
    }

    /// Stop where it is.
    pub fn stop(&mut self) {
        self.destination = None;
        self.path.clear();
        self.status = NavStatus::Idle;
    }

    pub fn status(&self) -> NavStatus {
        self.status
    }

    /// Ask for the way again from where it stands — after the level was
    /// rebaked, say.
    pub fn repath(&mut self) {
        if self.destination.is_some() {
            self.needs_path = true;
        }
    }

    /// The corners still ahead of it.
    pub fn remaining(&self) -> &[Vec3] {
        &self.path[self.next.min(self.path.len())..]
    }
}

/// Move every [`NavAgent`] one step of `dt` seconds along its way: the
/// system. Call it in the fixed step.
pub fn move_agents(world: &mut hecs::World, grid: &NavGrid, dt: f32) {
    for (transform, agent) in world.query_mut::<(&mut crate::scene::Transform, &mut NavAgent)>() {
        if agent.needs_path {
            agent.needs_path = false;
            let Some(to) = agent.destination else {
                continue;
            };
            match grid.path(transform.position, to) {
                Some(path) => {
                    agent.path = path;
                    agent.next = 1.min(agent.path.len());
                    agent.status = NavStatus::Moving;
                }
                None => {
                    agent.path.clear();
                    agent.status = NavStatus::Unreachable;
                    continue;
                }
            }
        }
        if agent.status != NavStatus::Moving {
            continue;
        }
        let mut budget = agent.speed.max(0.0) * dt;
        while budget > 0.0 {
            let Some(&corner) = agent.path.get(agent.next) else {
                agent.status = NavStatus::Arrived;
                break;
            };
            let to = corner - transform.position;
            let flat = Vec2::new(to.x, to.z);
            if flat.length() > 1e-4 {
                transform.rotation_deg.y = flat.x.atan2(flat.y).to_degrees();
            }
            let distance = to.length();
            if distance <= budget {
                transform.position = corner;
                budget -= distance;
                agent.next += 1;
                if agent.next >= agent.path.len() {
                    agent.status = NavStatus::Arrived;
                    break;
                }
            } else {
                transform.position += to / distance * budget;
                budget = 0.0;
            }
        }
    }
    crate::world::apply_hierarchy(world);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Body, Collider, EntityDesc, Scene, Transform};
    use crate::MeshHandle;

    fn solid(name: &str, position: Vec3, half: Vec3, collider: Option<Collider>) -> EntityDesc {
        EntityDesc {
            name: name.into(),
            model: "m".into(),
            transform: Transform {
                position,
                ..Transform::default()
            },
            body: Body::Static,
            collider: collider.unwrap_or(Collider::Box { half }),
            ..EntityDesc::default()
        }
    }

    fn baked(entities: Vec<EntityDesc>) -> NavGrid {
        let mut scene = Scene {
            entities,
            ..Scene::default()
        };
        scene.assign_ids();
        let mut world = hecs::World::new();
        crate::spawn_scene(&scene, &mut world, |_| Some(MeshHandle::TEST));
        let mut physics = PhysicsWorld::new(1.0 / 60.0);
        physics.sync_from_world(&mut world);
        physics.refresh_queries();
        NavGrid::bake(
            &physics,
            Vec2::new(-10.0, -10.0),
            Vec2::new(10.0, 10.0),
            NavSettings::default(),
        )
    }

    fn floor() -> EntityDesc {
        solid(
            "floor",
            Vec3::new(0.0, -0.1, 0.0),
            Vec3::new(10.0, 0.1, 10.0),
            None,
        )
    }

    #[test]
    fn walkable_ground_comes_out_as_strips_of_one_height() {
        let grid = NavGrid {
            origin: Vec2::ZERO,
            cell: 0.5,
            width: 4,
            depth: 2,
            ground: vec![
                Some(0.0),
                Some(0.0),
                None,
                Some(1.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
            ],
            max_step: 0.3,
        };
        let strips = grid.strips();
        assert_eq!(strips.len(), 3, "{strips:?}");
        assert_eq!(strips[0], (Vec3::new(0.5, 0.0, 0.25), 1.0));
        assert_eq!(strips[1], (Vec3::new(1.75, 1.0, 0.25), 0.5));
        assert_eq!(strips[2], (Vec3::new(1.0, 0.0, 0.75), 2.0));
    }

    #[test]
    fn a_straight_path_on_open_ground_is_a_straight_line() {
        let grid = baked(vec![floor()]);
        let path = grid
            .path(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(path.len(), 2, "no corners on open ground: {path:?}");
        assert!((NavGrid::length(&path) - 10.0).abs() < 0.1);
    }

    #[test]
    fn a_wall_is_walked_around_through_its_gap() {
        // A wall across the level with one gap at z = 6..8.
        let grid = baked(vec![
            floor(),
            solid(
                "wall",
                Vec3::new(0.0, 1.0, -2.0),
                Vec3::new(0.25, 1.0, 8.0),
                None,
            ),
        ]);
        let from = Vec3::new(-5.0, 0.0, 0.0);
        let to = Vec3::new(5.0, 0.0, 0.0);
        let path = grid.path(from, to).expect("through the gap");
        assert!(path.iter().any(|p| p.z > 6.0), "via the gap: {path:?}");
        assert!(NavGrid::length(&path) > 14.0);
        assert!(
            path.iter().all(|p| p.y < 0.5),
            "on the floor, not over the wall: {path:?}"
        );

        // Close the gap, and there is no way.
        let sealed = baked(vec![
            floor(),
            solid(
                "wall",
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.25, 1.0, 10.0),
                None,
            ),
        ]);
        assert!(sealed.path(from, to).is_none());
    }

    #[test]
    fn stairs_climb_and_a_ledge_of_the_same_height_does_not() {
        let stairs = solid(
            "stairs",
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::ZERO,
            Some(Collider::Stairs {
                half: Vec3::new(2.0, 1.0, 2.0),
                steps: 8,
            }),
        );
        let top = Vec3::new(0.0, 2.0, -1.8);
        let grid = baked(vec![floor(), stairs]);
        let path = grid
            .path(Vec3::new(0.0, 0.0, 6.0), top)
            .expect("up the stairs");
        assert!(path.last().unwrap().y > 1.7, "{path:?}");

        let block = solid(
            "block",
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(2.0, 1.0, 2.0),
            None,
        );
        let grid = baked(vec![floor(), block]);
        assert!(
            grid.path(Vec3::new(0.0, 0.0, 6.0), top).is_none(),
            "two metres is a wall"
        );
    }

    #[test]
    fn an_agent_walks_round_the_wall_to_where_it_was_sent_and_says_when_it_cannot() {
        let grid = baked(vec![
            floor(),
            solid(
                "wall",
                Vec3::new(0.0, 1.0, -2.0),
                Vec3::new(0.25, 1.0, 8.0),
                None,
            ),
            // A platform a metre up: walkable on top, no step onto it.
            solid(
                "platform",
                Vec3::new(7.0, 0.5, 7.0),
                Vec3::new(1.5, 0.5, 1.5),
                None,
            ),
        ]);
        let mut world = hecs::World::new();
        let walker = world.spawn((
            Transform {
                position: Vec3::new(-5.0, 0.0, 0.0),
                ..Transform::default()
            },
            crate::world::WorldTransform(glam::Mat4::IDENTITY),
            NavAgent::new(3.0),
        ));
        world
            .get::<&mut NavAgent>(walker)
            .unwrap()
            .go_to(Vec3::new(5.0, 0.0, 0.0));
        let mut went_past_the_gap = false;
        for _ in 0..600 {
            move_agents(&mut world, &grid, 1.0 / 60.0);
            let at = world.get::<&Transform>(walker).unwrap().position;
            went_past_the_gap |= at.z > 6.0;
            assert!(
                !(at.x.abs() < 0.25 && at.z < 6.0),
                "never through the wall: {at:?}"
            );
            if world.get::<&NavAgent>(walker).unwrap().status() == NavStatus::Arrived {
                break;
            }
        }
        let agent = (*world.get::<&NavAgent>(walker).unwrap()).clone();
        assert_eq!(agent.status(), NavStatus::Arrived);
        let at = world.get::<&Transform>(walker).unwrap().position;
        assert!((at - Vec3::new(5.0, 0.0, 0.0)).length() < 0.3, "{at:?}");
        assert!(went_past_the_gap);

        // Up onto the platform: no way, and it says so rather than walking.
        world
            .get::<&mut NavAgent>(walker)
            .unwrap()
            .go_to(Vec3::new(7.0, 1.0, 7.0));
        move_agents(&mut world, &grid, 1.0 / 60.0);
        assert_eq!(
            world.get::<&NavAgent>(walker).unwrap().status(),
            NavStatus::Unreachable
        );
    }
}
