//! Hair and fur: a line's `hair`. `hair: (length: 0.3)` grows hair on the
//! entity — a head, a creature, a tuft of grass that is not grass — from
//! a sphere round it (`radius`, 0.5 for `builtin:sphere`), over the cap of
//! it that `cover` says.
//!
//! A few guide strands are simulated (`guides`), each a Cosserat rod
//! ([`crate::rod`]) rooted in the skin — its first particle pinned there,
//! its first link clamped out along the skin's normal — bent by gravity
//! and the wind, kept off the head it grows from and the colliders about
//! it. They are combed as they fall: hung for a moment, the shape they
//! settle into is the shape they keep, as stiffly as `stiffness` says.
//! Many more are drawn (`strands`): each follows the guides nearest its
//! root, weighted by how near, with its root's own offset carried along
//! them and drawn in toward the guides by `clump` — guide hairs and
//! interpolation, as in TressFX.
//!
//! Only a look: every machine combs its own (DNA, postulate 4).

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use scrap_core::wind::Wind;
use scrap_core::world::WorldTransform;
use scrap_geometry::mesh_asset::Vertex;

use crate::obstacle::{Obstacle, Obstacles};
use crate::rod::{Give, Rod};

/// Hair, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hair {
    /// Metres, root to tip.
    pub length: f32,
    /// Strands drawn.
    pub strands: u32,
    /// Strands simulated, which the drawn ones follow.
    pub guides: u32,
    /// Links along each guide.
    pub segments: u32,
    /// The radius of the sphere it grows from, in the entity's space.
    pub radius: f32,
    /// How much of the sphere grows it, from the top: 0.5 its upper half,
    /// 1 all of it (fur).
    pub cover: f32,
    /// How wide a strand is at its root, metres; it tapers to its tip.
    pub thickness: f32,
    /// How much it keeps its combed shape, 0 (loose) to 1 (stiff fur).
    pub stiffness: f32,
    /// How much the drawn strands are drawn in to their guides toward
    /// the tip: 0 loose, 1 in locks.
    pub clump: f32,
    /// Waves along a strand, as many as this in its length.
    pub curl: f32,
    /// How much the wind takes it.
    pub catch: f32,
    /// How much it leaves the skin lying along it, down and away from the
    /// crown, rather than straight out: 0 fur standing up, 1 hair lying
    /// flat.
    pub lie: f32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "scrap_core::netsim::NetMode::is_local")]
    pub net: scrap_core::netsim::NetMode,
}

impl Default for Hair {
    fn default() -> Self {
        Self {
            length: 0.3,
            strands: 1200,
            guides: 48,
            segments: 8,
            radius: 0.5,
            cover: 0.55,
            thickness: 0.004,
            stiffness: 0.3,
            clump: 0.3,
            curl: 0.0,
            catch: 0.6,
            lie: 0.7,
            net: scrap_core::netsim::NetMode::Local,
        }
    }
}

scrap_core::impl_parts! {
    Hair => "hair", fractions ["cover", "stiffness", "clump", "lie"];
}

/// The hair of a line, read off it.
pub trait HairLine {
    fn hair(&self) -> Option<Hair>;
}

impl HairLine for scrap_core::EntityDesc {
    fn hair(&self) -> Option<Hair> {
        self.part()
    }
}

impl HairLine for scrap_core::scene::Override {
    fn hair(&self) -> Option<Hair> {
        self.part()
    }
}

pub const STEP: f32 = 1.0 / 60.0;
pub const SUBSTEPS: usize = 8;
const WIND_SPEED: f32 = 4.0;
/// Seconds a head of hair hangs, when it is first grown, to be combed.
const COMBING: f32 = 1.0;

/// A drawn strand: the guides it follows and how much, and where its root
/// is from theirs, in the entity's space.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Follow {
    guides: [u32; 3],
    weights: [f32; 3],
    offset: Vec3,
    /// A little of its own, so no two strands are the same length.
    length: f32,
}

/// Hair as it moves: the component [`run_hair`] steps.
#[derive(Debug, Clone)]
pub struct HairState {
    pub hair: Hair,
    pub wind: Wind,
    /// Each guide's root and the way it grows, in the entity's space.
    roots: Vec<(Vec3, Vec3)>,
    rods: Vec<Rod>,
    /// Each guide's first link turned from the entity, as it grows.
    clamps: Vec<Quat>,
    follows: Vec<Follow>,
    /// Where the entity was at the last step.
    placed: Option<Mat4>,
    near: Vec<Obstacle>,
    owed: f32,
    time: f32,
}

/// `n` points spread evenly over the unit sphere's cap above `low` (its
/// y), by the golden angle.
fn spread(n: usize, low: f32) -> Vec<Vec3> {
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    (0..n)
        .map(|i| {
            let y = 1.0 - (1.0 - low) * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = golden * i as f32;
            Vec3::new(r * a.cos(), y, r * a.sin())
        })
        .collect()
}

impl HairState {
    pub fn new(hair: Hair) -> Self {
        let low = 1.0 - 2.0 * hair.cover.clamp(0.01, 1.0);
        let guides = spread(hair.guides.clamp(1, 512) as usize, low);
        // Each root grows out of the skin, leaning down the head as much
        // as `lie` says.
        let lie = hair.lie.clamp(0.0, 1.0);
        let roots = guides
            .iter()
            .map(|n| {
                let down = (-Vec3::Y - *n * -n.y).normalize_or(Vec3::ZERO);
                let way = (*n * (1.0 - lie) + down * lie * 2.0).normalize_or(*n);
                // Never into the skin.
                let way = if way.dot(*n) < 0.1 { (way + *n * 0.2).normalize() } else { way };
                (*n * hair.radius, way)
            })
            .collect();
        let follows = spread(hair.strands.clamp(1, 100_000) as usize, low)
            .into_iter()
            .enumerate()
            .map(|(k, n)| {
                // The three guides nearest, weighted by how near.
                let mut near: Vec<(f32, u32)> =
                    guides.iter().enumerate().map(|(g, m)| (n.distance_squared(*m), g as u32)).collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                let take = near.len().min(3);
                let mut guides_of = [near[0].1; 3];
                let mut weights = [0.0; 3];
                for i in 0..take {
                    guides_of[i] = near[i].1;
                    weights[i] = 1.0 / (near[i].0.sqrt() + 1e-3);
                }
                let sum: f32 = weights.iter().sum();
                let weights = weights.map(|w| w / sum);
                let from: Vec3 = (0..3).map(|i| guides[guides_of[i] as usize] * weights[i]).sum();
                // A hash of the strand: its own length, a little shorter
                // or longer.
                let jitter = ((k as f32 * 12.9898).sin() * 43758.547).fract().abs();
                Follow {
                    guides: guides_of,
                    weights,
                    offset: (n - from) * hair.radius,
                    length: 0.85 + 0.3 * jitter,
                }
            })
            .collect();
        Self {
            hair,
            wind: Wind::default(),
            roots,
            rods: Vec::new(),
            clamps: Vec::new(),
            follows,
            placed: None,
            near: Vec::new(),
            owed: 0.0,
            time: 0.0,
        }
    }

    /// The simulated guides, once grown.
    pub fn guides(&self) -> &[Rod] {
        &self.rods
    }

    /// Grown out along the skin where the entity is, and combed: hung a
    /// moment in still air, the shape it falls into kept.
    fn grow(&mut self, placed: Mat4, obstacles: &[Obstacle]) {
        let h = &self.hair;
        let n = h.segments.clamp(1, 64) as usize;
        let link = h.length.max(0.01) / n as f32;
        let turn = placed.to_scale_rotation_translation().1;
        // A lock of hair weighs next to nothing, so a compliance in
        // absolute terms would be rigid at any setting: it is set against
        // the lock's own resistance to turning instead — at stiffness ½ a
        // bend is half undone each substep.
        let mass = 1e-4;
        let s = h.stiffness.clamp(0.0, 1.0);
        let turn_w = 1.0 / (mass * link * link);
        let sub = STEP / SUBSTEPS as f32;
        let bend = if s >= 1.0 { 0.0 } else { 2.0 * turn_w * sub * sub * (1.0 / s.max(1e-4) - 1.0) };
        let give = Give { stretch: 0.0, bend, twist: bend };
        self.rods.clear();
        self.clamps.clear();
        for (root, out) in &self.roots {
            let from = placed.transform_point3(*root);
            let way = (turn * *out).normalize_or(Vec3::Y);
            let points = (0..=n).map(|i| from + way * (link * i as f32)).collect();
            let mut rod = Rod::new(points, vec![link; n], mass, give);
            rod.radius = (h.thickness * 0.5).max(0.001);
            rod.damping = 3.0;
            rod.friction = 0.3;
            rod.pin(0, true, mass);
            rod.clamp(0, true, mass);
            self.clamps.push(turn.inverse() * rod.turn[0]);
            self.rods.push(rod);
        }
        self.placed = Some(placed);
        // Combed by gravity: hung a moment, and that shape is the rest.
        let still = Wind { direction: Vec3::X, strength: 0.0 };
        for _ in 0..(COMBING / STEP) as usize {
            self.step(placed, &still, obstacles);
        }
        for rod in &mut self.rods {
            rod.keep_shape();
        }
    }

    /// The sphere it grows from, in the world: kept out of.
    fn head(&self, placed: Mat4) -> Obstacle {
        let (scale, _, at) = placed.to_scale_rotation_translation();
        Obstacle::Sphere {
            center: at,
            radius: self.hair.radius * scale.abs().min_element(),
        }
    }

    /// Along by `seconds`, where the entity now is, in `wind`, lying on
    /// `obstacles` and off its own head.
    pub fn advance(&mut self, placed: Mat4, wind: &Wind, obstacles: &[Obstacle], seconds: f32) {
        if self.rods.is_empty() {
            let mut all = obstacles.to_vec();
            all.push(self.head(placed));
            self.grow(placed, &all);
        }
        self.owed = (self.owed + seconds.max(0.0)).min(0.25);
        if self.owed < STEP {
            return;
        }
        self.near.clear();
        self.near.extend_from_slice(obstacles);
        self.near.push(self.head(placed));
        let near = std::mem::take(&mut self.near);
        while self.owed >= STEP {
            self.owed -= STEP;
            self.step(placed, wind, &near);
        }
        self.near = near;
    }

    fn step(&mut self, placed: Mat4, wind: &Wind, obstacles: &[Obstacle]) {
        let was = self.placed.unwrap_or(placed);
        self.placed = Some(placed);
        let h = STEP / SUBSTEPS as f32;
        let turn = placed.to_scale_rotation_translation().1;
        let level = Vec3::new(wind.direction.x, 0.0, wind.direction.z).normalize_or_zero();
        let speed = WIND_SPEED * wind.strength.max(0.0);
        let catch = self.hair.catch.max(0.0) * 20.0;
        for (g, rod) in self.rods.iter_mut().enumerate() {
            let root = self.roots[g].0;
            let (a, b) = (was.transform_point3(root), placed.transform_point3(root));
            let clamp = turn * self.clamps[g];
            for s in 0..SUBSTEPS {
                let t = (s + 1) as f32 / SUBSTEPS as f32;
                let time = self.time + h * (s + 1) as f32;
                rod.particles.x[0] = a.lerp(b, t);
                rod.set_turn(0, clamp);
                let x = &rod.particles.x;
                let v = &rod.particles.v;
                let gusts: Vec<Vec3> = (0..x.len())
                    .map(|i| {
                        let along = x[i].dot(level);
                        let gust = 0.6 + 0.4 * (time * 1.7 - along * 0.8 + g as f32 * 0.37).sin();
                        (level * speed * gust - v[i]) * catch
                    })
                    .collect();
                rod.substep(h, |i| Vec3::new(0.0, -9.81, 0.0) + gusts[i], obstacles);
            }
        }
        self.time += STEP;
    }

    /// The drawn strands as thin tapering tubes of three sides, in the
    /// space of `placed` (what they are drawn at).
    pub fn mesh(&self, placed: Mat4) -> (Vec<Vertex>, Vec<u32>) {
        if self.rods.is_empty() {
            return (Vec::new(), Vec::new());
        }
        const SIDES: usize = 3;
        let back = placed.inverse();
        let turn_now = placed.to_scale_rotation_translation().1;
        let n = self.rods[0].particles.len();
        let width = self.hair.thickness.max(0.0005) * 0.5;
        let clump = self.hair.clump.clamp(0.0, 1.0);
        let curl = self.hair.curl.max(0.0);
        // How each guide has turned at each of its points since it grew:
        // the same for every strand that follows it, so found once here
        // rather than once a strand (a guide leads dozens).
        let turned_since: Vec<glam::Quat> = self
            .rods
            .iter()
            .zip(&self.clamps)
            .flat_map(|(rod, clamp)| {
                let then = turn_now * *clamp;
                (0..n).map(move |i| rod.turn_at(i) * then.inverse())
            })
            .collect();
        // The sides' directions round a strand, the same at every point.
        let round: [(f32, f32); SIDES] = std::array::from_fn(|s| {
            let a = s as f32 / SIDES as f32 * std::f32::consts::TAU;
            (a.cos(), a.sin())
        });
        // Each strand is its own `n × SIDES` vertices at a place known
        // ahead: strands are made across the cores, straight into it.
        let each = n * SIDES;
        let blank = Vertex { position: [0.0; 3], normal: [0.0; 3], uv: [0.0; 2] };
        let mut vertices = vec![blank; self.follows.len() * each];
        scrap_core::jobs::for_each_chunk_mut(&mut vertices, each, |start, out| {
            let k = start / each;
            let f = &self.follows[k];
            let mut line = vec![Vec3::ZERO; n];
            let offset = turn_now * f.offset;
            // Along the guides it follows, its root's offset carried as
            // their links are turned, and drawn in toward the tip.
            for (i, point) in line.iter_mut().enumerate() {
                let t = i as f32 / (n - 1) as f32;
                let mut at = Vec3::ZERO;
                let mut turned = Vec3::ZERO;
                for j in 0..3 {
                    let guide = f.guides[j] as usize;
                    let rod = &self.rods[guide];
                    let w = f.weights[j];
                    at += rod.particles.x[i] * w;
                    turned += turned_since[guide * n + i] * offset * w;
                }
                let wave = if curl > 0.0 {
                    let a = t * curl * std::f32::consts::TAU + k as f32;
                    Vec3::new(a.cos(), 0.0, a.sin()) * (width * 6.0 * t)
                } else {
                    Vec3::ZERO
                };
                *point = at + turned * (1.0 - clump * t) + wave;
            }
            // Its own length: the last of it left off.
            let reach = ((n - 1) as f32 * f.length).clamp(1.0, (n - 1) as f32);
            let mut out = out.iter_mut();
            for i in 0..n {
                let t = (i as f32 / (n - 1) as f32 * reach).min(reach);
                let (lo, frac) = (t.floor() as usize, t.fract());
                let p = if lo + 1 < n { line[lo].lerp(line[lo + 1], frac) } else { line[n - 1] };
                let next = if lo + 1 < n { line[lo + 1] } else { line[n - 1] };
                let prev = line[lo.saturating_sub(1).min(n - 2)];
                let tangent = (next - prev).normalize_or(Vec3::Y);
                let side = tangent.any_orthonormal_vector();
                let up = tangent.cross(side);
                let r = width * (1.0 - 0.85 * i as f32 / (n - 1) as f32);
                let local = back.transform_point3(p);
                for (s, &(cos, sin)) in round.iter().enumerate() {
                    let normal = side * cos + up * sin;
                    *out.next().expect("n × SIDES a strand") = Vertex {
                        position: (local + back.transform_vector3(normal * r)).to_array(),
                        normal: back.transform_vector3(normal).normalize_or(normal).to_array(),
                        uv: [s as f32 / SIDES as f32, i as f32 / (n - 1) as f32],
                    };
                }
            }
        });
        let mut indices = Vec::with_capacity(self.follows.len() * (n - 1) * SIDES * 6);
        for k in 0..self.follows.len() {
            let first = (k * each) as u32;
            for i in 0..n as u32 - 1 {
                for s in 0..SIDES as u32 {
                    let a = first + i * SIDES as u32 + s;
                    let b = first + i * SIDES as u32 + (s + 1) % SIDES as u32;
                    let (c, d) = (a + SIDES as u32, b + SIDES as u32);
                    indices.extend_from_slice(&[a, b, c, b, d, c]);
                }
            }
        }
        (vertices, indices)
    }
}

/// Every head of hair swayed by this wind.
pub fn set_wind(world: &mut hecs::World, wind: Wind) {
    for state in world.query_mut::<&mut HairState>() {
        state.wind = wind;
    }
}

/// Step all hair by `seconds` in its wind, off the colliders near it.
pub fn run_hair(world: &mut hecs::World, seconds: f32, obstacles: &Obstacles) {
    let clock = scrap_core::netsim::session_time(world);
    let mut near = Vec::new();
    for (state, placed) in world.query_mut::<(&mut HairState, &WorldTransform)>() {
        let (scale, _, at) = placed.0.to_scale_rotation_translation();
        let reach = Vec3::splat((state.hair.radius * scale.abs().max_element() + state.hair.length) * 1.2 + 0.5);
        obstacles.near(at - reach, at + reach, &mut near);
        let wind = state.wind;
        crate::net::keep_time(&mut state.time, clock);
        state.advance(placed.0, &wind, &near, seconds);
    }
}

/// The soft module's dresser for hair.
pub struct HairDress;

impl scrap_core::world::Dress for HairDress {
    fn parts(&self) -> &[&'static str] {
        &["hair"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        match line.hair() {
            Some(hair) => {
                let _ = world.insert_one(entity, HairState::new(hair));
            }
            None => {
                let _ = world.remove_one::<HairState>(entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still() -> Wind {
        Wind { direction: Vec3::X, strength: 0.0 }
    }

    fn head() -> Mat4 {
        Mat4::from_translation(Vec3::new(0.0, 1.7, 0.0)) * Mat4::from_scale(Vec3::splat(0.4))
    }

    #[test]
    fn hair_grows_from_the_crown_falls_about_the_head_and_stays_off_it() {
        let hair = Hair { length: 0.3, guides: 32, strands: 400, ..Hair::default() };
        let mut state = HairState::new(hair);
        for _ in 0..60 {
            state.advance(head(), &still(), &[], 1.0 / 30.0);
        }
        let centre = Vec3::new(0.0, 1.7, 0.0);
        let skin = 0.5 * 0.4;
        for rod in state.guides() {
            let root = rod.particles.x[0];
            assert!((root.distance(centre) - skin).abs() < 1e-3, "rooted in the skin: {root}");
            assert!(root.y >= 1.7 - skin * 0.2, "on the upper cap: {root}");
            for p in &rod.particles.x[1..] {
                assert!(p.distance(centre) >= skin + 0.001, "not in the head: {p}");
            }
            // The tips fall below the roots, but not through the head.
            let tip = *rod.particles.x.last().unwrap();
            assert!(tip.y < root.y + 0.05, "falls: {tip} from {root}");
        }
        let (vertices, indices) = state.mesh(head());
        assert_eq!(vertices.len(), 400 * 9 * 3);
        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
        assert!(vertices.iter().all(|v| v.position.iter().all(|c| c.is_finite())));
    }

    #[test]
    fn stiff_fur_stands_out_and_loose_hair_hangs() {
        let tips_out = |stiffness: f32| {
            let fur = Hair { length: 0.2, cover: 1.0, guides: 24, strands: 24, stiffness, lie: 0.0, ..Hair::default() };
            let mut state = HairState::new(fur);
            for _ in 0..30 {
                state.advance(head(), &still(), &[], 1.0 / 30.0);
            }
            // How far each guide's tip is from the head's centre, on the
            // mean: fur standing out reaches the skin plus its length.
            let centre = Vec3::new(0.0, 1.7, 0.0);
            let rods = state.guides();
            rods.iter().map(|r| r.particles.x.last().unwrap().distance(centre)).sum::<f32>() / rods.len() as f32
        };
        let (stiff, loose) = (tips_out(1.0), tips_out(0.0));
        assert!(stiff > 0.2 + 0.17, "stands out: {stiff}");
        assert!(loose < stiff - 0.03, "hangs closer: {loose} vs {stiff}");
    }

    #[test]
    fn the_wind_blows_it_downwind_and_the_head_turning_takes_it_along() {
        let hair = Hair { length: 0.4, guides: 16, strands: 16, stiffness: 0.1, ..Hair::default() };
        let mean_tip = |state: &HairState| {
            let rods = state.guides();
            rods.iter().map(|r| *r.particles.x.last().unwrap()).sum::<Vec3>() / rods.len() as f32
        };
        let mut calm = HairState::new(hair);
        let mut blown = HairState::new(hair);
        for _ in 0..60 {
            calm.advance(head(), &still(), &[], 1.0 / 30.0);
            blown.advance(head(), &Wind { direction: Vec3::Z, strength: 3.0 }, &[], 1.0 / 30.0);
        }
        assert!(mean_tip(&blown).z > mean_tip(&calm).z + 0.05, "{} vs {}", mean_tip(&blown), mean_tip(&calm));
        // The head turned half round: the hair's roots go with it.
        let turned = head() * Mat4::from_rotation_y(std::f32::consts::PI);
        for _ in 0..30 {
            calm.advance(turned, &still(), &[], 1.0 / 30.0);
        }
        let root = calm.guides()[5].particles.x[0];
        let expected = turned.transform_point3(calm.roots[5].0);
        assert!(root.distance(expected) < 1e-4, "{root} vs {expected}");
    }
}
