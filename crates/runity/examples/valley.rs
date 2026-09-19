//! The Valley: terrain, forest and a debug top-down view — a world with no
//! player yet.
//!
//! ```text
//! cargo run --release --example valley
//! RUNITY_HEADLESS=1 cargo run --release --example valley   # writes valley.png
//! ```
//!
//! There is no controllable character in this card — that arrives with the
//! next one. What is here is the ground it will stand on: a 512x512 m chunk
//! shaped by a closed-form formula (a basin with a streambed, ringed by a
//! mountain rim steep enough to hit the physics' own 40 degree slope limit),
//! a sparse forest placed by rejection sampling with a guaranteed 4 m gap
//! between trunks, one deliberately dense grove by the stream, and a
//! stationary marker standing in for the body a player will one day move.
//!
//! Terrain renders in two levels of detail sharing one formula: a 64x64 m
//! patch at full resolution around the marker ([`runity_core::physics::Heightfield::to_mesh`])
//! and a coarse ring covering the whole chunk
//! ([`runity_core::physics::Heightfield::to_mesh_lod`]). Both grids sample
//! the same [`terrain_height`] at aligned world coordinates, so the patch's
//! edge lands exactly on a node the coarse ring already has — no crack at
//! the seam.
//!
//! Press `M` to switch to a fixed-scale top-down debug view, locked directly
//! above the marker: it draws every tree's footprint, the static grid's
//! cells, and — since the marker stands close enough to one trunk in the
//! dense grove to be touching it — the resulting contact.

use runity::prelude::*;
use runity_core::physics::{Blocker, Body, Heightfield, StaticGrid};
use std::io;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;

// --- terrain: a 512x512 m chunk, bowl + streambed + rim -------------------

/// Half the side of the chunk: the field spans `[-CHUNK_HALF, CHUNK_HALF]`.
const CHUNK_HALF: f32 = 256.0;
/// Radius the basin proper reaches before the rim starts climbing.
const INNER_RADIUS: f32 = 200.0;
/// How high the rim rises above the basin at the edge of the chunk.
const RIM_HEIGHT: f32 = 80.0;
/// Curvature of the basin floor: gentle enough to stay walkable everywhere.
const BASIN_K: f32 = 0.0002;
/// How far the streambed dips below the basin floor, at its centre line.
const STREAM_DEPTH: f32 = 3.0;
/// Half-width of the streambed's dip.
const STREAM_HALF_WIDTH: f32 = 15.0;

/// Height of the valley's surface at world `(x, z)`.
///
/// A bowl (`BASIN_K * r^2`) for the basin floor, a smoothstep ramp added on
/// top for the rim beyond [`INNER_RADIUS`], and a witch-of-Agnesi dip along
/// `z = 0` for the streambed — no water is drawn, the stream is simply lower
/// ground. This is example code, not the physics core, so it is free to use
/// `sin`/`cos`/`sqrt` however it likes; only `runity_core::physics` itself is
/// held to the no-transcendentals rule that keeps a tick reproducible.
fn terrain_height(x: f32, z: f32) -> f32 {
    let r = (x * x + z * z).sqrt();
    let basin = BASIN_K * r * r;
    let rim = rim_extra(r);
    let normalized = z / STREAM_HALF_WIDTH;
    let stream = -STREAM_DEPTH / (1.0 + normalized * normalized);
    basin + rim + stream
}

/// Extra height the rim adds beyond [`INNER_RADIUS`], ramping smoothly (zero
/// slope at both ends) up to [`RIM_HEIGHT`] at `r = CHUNK_HALF` and staying
/// there beyond it — the chunk's square corners reach further than the rim's
/// own radius, and a flat plateau out there is harmless.
fn rim_extra(r: f32) -> f32 {
    if r <= INNER_RADIUS {
        return 0.0;
    }
    let t = ((r - INNER_RADIUS) / (CHUNK_HALF - INNER_RADIUS)).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    RIM_HEIGHT * smooth
}

/// Base resolution the whole chunk is sampled at — fine enough to be a
/// believable physics ground later, and the source the coarse ring's
/// [`Heightfield::to_mesh_lod`] strides over.
const WORLD_CELL: f32 = 2.0;
/// Stride applied to [`WORLD_CELL`] to get the coarse ring's 4 m step.
const FAR_LOD_STRIDE: usize = 2;
/// Half the side of the full-resolution patch (64x64 m in total).
const PATCH_HALF: f32 = 32.0;
const PATCH_CELL: f32 = 1.0;

/// The whole 512x512 m chunk at [`WORLD_CELL`] resolution — the source for
/// the coarse distant ring, and the one place `terrain_height` is sampled
/// with a fixed, world-aligned lattice.
fn build_far_heightfield() -> Heightfield {
    let cols = (2.0 * CHUNK_HALF / WORLD_CELL) as usize + 1;
    let origin = Vec2::splat(-CHUNK_HALF);
    let mut heights = Vec::with_capacity(cols * cols);
    for row in 0..cols {
        let z = origin.y + row as f32 * WORLD_CELL;
        for col in 0..cols {
            let x = origin.x + col as f32 * WORLD_CELL;
            heights.push(terrain_height(x, z));
        }
    }
    Heightfield::new(origin, WORLD_CELL, cols, cols, heights)
}

/// A full-resolution 64x64 m patch around `center`, snapped onto the same
/// `4 m` lattice the coarse ring's stride keeps — so the patch's own edge
/// nodes land exactly on nodes the ring already drew, at heights computed by
/// the very same [`terrain_height`] call.
fn build_patch_heightfield(center: Vec2) -> Heightfield {
    let snap = WORLD_CELL * FAR_LOD_STRIDE as f32;
    let snapped = Vec2::new(
        (center.x / snap).round() * snap,
        (center.y / snap).round() * snap,
    );
    let origin = snapped - Vec2::splat(PATCH_HALF);
    let cols = (2.0 * PATCH_HALF / PATCH_CELL) as usize + 1;
    let mut heights = Vec::with_capacity(cols * cols);
    for row in 0..cols {
        let z = origin.y + row as f32 * PATCH_CELL;
        for col in 0..cols {
            let x = origin.x + col as f32 * PATCH_CELL;
            heights.push(terrain_height(x, z));
        }
    }
    Heightfield::new(origin, PATCH_CELL, cols, cols, heights)
}

// --- forest: rejection sampling, plus one dense grove ----------------------

const TREE_RADIUS: f32 = 0.35;
const MIN_TREE_SPACING: f32 = 4.0;
/// Roughly one trunk per 120 square metres — visible clean through at a
/// hundred metres.
const BASE_TREE_DENSITY: f32 = 1.0 / 120.0;
/// The sparse forest's extent: inside the basin, clear of the rim entirely.
const FOREST_RADIUS: f32 = 190.0;
/// Centre of the one deliberately dense patch, close to the stream.
const DENSE_PATCH_CENTER: Vec2 = Vec2::new(60.0, 5.0);
/// About 30 m across.
const DENSE_PATCH_RADIUS: f32 = 15.0;
const DENSE_DENSITY_MULT: f32 = 4.0;
/// Fixed so the forest — and the marker anchored to it — are the same every
/// run.
const FOREST_SEED: u64 = 0xC0FF_EE15_5EED_2026;

struct Tree {
    position: Vec2,
    height: f32,
}

/// A small deterministic PRNG (SplitMix64) — the engine has no shared one
/// (see `docs/design/10-mvp.md`), and this example only needs a seeded,
/// repeatable stream of floats.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// Scatter trees over a disk by dart-throwing: propose a uniformly random
/// point, keep it only if it clears [`MIN_TREE_SPACING`] from every tree
/// already placed (in this region or any other) and `allow` accepts it.
///
/// Rejection sampling rather than plain noise is the whole point: noise alone
/// can put two trunks a hand's width apart, which is a gap nothing can walk
/// through.
fn place_region(
    rng: &mut Rng,
    trees: &mut Vec<Tree>,
    center: Vec2,
    radius: f32,
    density: f32,
    allow: impl Fn(Vec2) -> bool,
) {
    let area = std::f32::consts::PI * radius * radius;
    let target = (area * density).round() as usize;
    let max_attempts = (target.max(50)) * 200;
    let mut placed = 0;
    let mut attempts = 0;
    while placed < target && attempts < max_attempts {
        attempts += 1;
        // sqrt corrects for the disk's radial density, so points don't
        // bunch up near the centre.
        let r = radius * rng.unit().sqrt();
        let theta = rng.range(0.0, std::f32::consts::TAU);
        let point = center + Vec2::new(r * theta.cos(), r * theta.sin());
        if !allow(point) {
            continue;
        }
        if trees
            .iter()
            .any(|t| (t.position - point).length() < MIN_TREE_SPACING)
        {
            continue;
        }
        trees.push(Tree {
            position: point,
            height: rng.range(5.0, 9.0),
        });
        placed += 1;
    }
}

/// The whole forest: the sparse basin, then the dense grove by the stream —
/// appended last, so [`Vec::last`] always names a tree from it.
fn place_forest(seed: u64) -> Vec<Tree> {
    let mut rng = Rng::new(seed);
    let mut trees = Vec::new();
    place_region(
        &mut rng,
        &mut trees,
        Vec2::ZERO,
        FOREST_RADIUS,
        BASE_TREE_DENSITY,
        |p| (p - DENSE_PATCH_CENTER).length() > DENSE_PATCH_RADIUS,
    );
    place_region(
        &mut rng,
        &mut trees,
        DENSE_PATCH_CENTER,
        DENSE_PATCH_RADIUS,
        BASE_TREE_DENSITY * DENSE_DENSITY_MULT,
        |_| true,
    );
    trees
}

/// Spawn one entity per tree, each carrying the [`Transform`] and
/// [`Blocker`] the static grid reads.
fn populate_world(world: &mut World, trees: &[Tree]) {
    for tree in trees {
        let entity = world.spawn();
        let y = terrain_height(tree.position.x, tree.position.y);
        world.insert(
            entity,
            Transform::from_position(Vec3::new(tree.position.x, y, tree.position.y)),
        );
        world.insert(
            entity,
            Blocker::Cylinder {
                radius: TREE_RADIUS,
                top: tree.height,
            },
        );
    }
}

// --- the debug top-down view -------------------------------------------

/// Metres shown from the centre of the screen to its shorter edge.
const DEBUG_VIEW_RADIUS: f32 = 20.0;

/// A fixed-scale, straight-down view centred on `marker` — not a free
/// camera, so it never tempts anyone into playing from above. Draws the
/// static grid's cells, every tree's footprint, the marker itself, and a
/// segment for each contact the marker currently has.
fn draw_top_down_debug(framebuffer: &mut Framebuffer, grid: &StaticGrid, marker: &Body) {
    let center = Vec2::new(marker.position.x, marker.position.z);
    let (width, height) = (framebuffer.width() as f32, framebuffer.height() as f32);
    framebuffer.clear(Color::rgb(0.08, 0.09, 0.10));
    let scale = (width.min(height) * 0.5 - 8.0) / DEBUG_VIEW_RADIUS;
    let to_screen = |p: Vec2| -> (f32, f32) {
        let rel = p - center;
        // North (+z) is drawn up the screen.
        (width * 0.5 + rel.x * scale, height * 0.5 - rel.y * scale)
    };

    let cell = grid.cell_size();
    let grid_color = Color::rgb(0.28, 0.30, 0.30);
    let first = (
        ((center.x - DEBUG_VIEW_RADIUS) / cell).floor() as i32,
        ((center.y - DEBUG_VIEW_RADIUS) / cell).floor() as i32,
    );
    let last = (
        ((center.x + DEBUG_VIEW_RADIUS) / cell).ceil() as i32,
        ((center.y + DEBUG_VIEW_RADIUS) / cell).ceil() as i32,
    );
    for cx in first.0..=last.0 {
        let x = cx as f32 * cell;
        let a = to_screen(Vec2::new(x, center.y - DEBUG_VIEW_RADIUS));
        let b = to_screen(Vec2::new(x, center.y + DEBUG_VIEW_RADIUS));
        debug::draw_line(framebuffer, a, b, grid_color);
    }
    for cz in first.1..=last.1 {
        let z = cz as f32 * cell;
        let a = to_screen(Vec2::new(center.x - DEBUG_VIEW_RADIUS, z));
        let b = to_screen(Vec2::new(center.x + DEBUG_VIEW_RADIUS, z));
        debug::draw_line(framebuffer, a, b, grid_color);
    }

    let tree_color = Color::rgb(0.35, 0.85, 0.40);
    let building_color = Color::rgb(0.55, 0.65, 0.95);
    for shape in grid.shapes() {
        let position = Vec2::new(shape.base.x, shape.base.z);
        if (position - center).length() > DEBUG_VIEW_RADIUS + 5.0 {
            continue;
        }
        match shape.blocker {
            Blocker::Cylinder { radius, .. } => {
                debug::draw_circle(
                    framebuffer,
                    to_screen(position),
                    radius * scale,
                    14,
                    tree_color,
                );
            }
            Blocker::Box { half, facing, .. } => {
                let corners = box_footprint(position, half, facing).map(to_screen);
                debug::draw_quad(framebuffer, corners, building_color);
            }
        }
    }

    let marker_xz = Vec2::new(marker.position.x, marker.position.z);
    debug::draw_circle(
        framebuffer,
        to_screen(marker_xz),
        marker.radius * scale,
        20,
        Color::rgb(1.0, 0.9, 0.25),
    );

    let contact_color = Color::rgb(1.0, 0.25, 0.25);
    let mut nearby = Vec::new();
    grid.query(marker_xz.x, marker_xz.y, &mut nearby);
    for index in nearby {
        let shape = grid.shape(index);
        if let Some((normal, depth)) = shape.resolve(marker_xz, marker.radius) {
            let from = to_screen(marker_xz);
            let to = to_screen(marker_xz + normal * depth);
            debug::draw_line(framebuffer, from, to, contact_color);
        }
    }
}

/// The four corners of a [`Blocker::Box`] footprint, in order around the
/// rectangle.
fn box_footprint(position: Vec2, half: Vec2, facing: Vec2) -> [Vec2; 4] {
    let f = facing.normalized();
    let r = Vec2::new(-f.y, f.x);
    [
        position + f * half.x + r * half.y,
        position - f * half.x + r * half.y,
        position - f * half.x - r * half.y,
        position + f * half.x - r * half.y,
    ]
}

// --- the game ---------------------------------------------------------

/// The eye and target of the fixed flyover camera — a vantage chosen to show
/// the rim, the stream and the forest at once, not tied to the marker.
const FLIGHT_EYE: Vec3 = Vec3::new(-300.0, 420.0, 380.0);
const FLIGHT_TARGET: Vec3 = Vec3::new(0.0, -10.0, 0.0);

struct Valley {
    far_mesh: Mesh,
    patch_mesh: Mesh,
    tree_mesh: Mesh,
    marker_mesh: Mesh,
    trees: Vec<Tree>,
    grid: StaticGrid,
    marker: Body,
    top_down: bool,
}

impl Valley {
    fn new() -> Self {
        let trees = place_forest(FOREST_SEED);
        // The marker sits close enough to a trunk in the dense grove to be
        // touching it — with no player yet, this is the only way to show the
        // debug view's contact segment drawing something real.
        let anchor = trees
            .last()
            .expect("the dense patch places at least one tree")
            .position;
        let marker_xz = anchor + Vec2::new(0.3, 0.0);
        let marker_y = terrain_height(marker_xz.x, marker_xz.y);
        let marker = Body::new(Vec3::new(marker_xz.x, marker_y, marker_xz.y));

        let mut world = World::new();
        populate_world(&mut world, &trees);
        let mut grid = StaticGrid::new(4.0);
        grid.rebuild(&world);

        let far = build_far_heightfield();
        let patch = build_patch_heightfield(marker_xz);

        Self {
            far_mesh: far.to_mesh_lod(FAR_LOD_STRIDE),
            patch_mesh: patch.to_mesh(),
            tree_mesh: Mesh::cylinder(1.0, 1.0, 8),
            marker_mesh: Mesh::cylinder(marker.radius, marker.height, 12),
            trees,
            grid,
            marker,
            top_down: false,
        }
    }

    fn draw_scene(&self, engine: &mut Engine) {
        let ground_color = Color::rgb(0.40, 0.48, 0.30);
        let ground = engine
            .lit_shader(Mat4::IDENTITY)
            .with_base_color(ground_color);
        engine.draw(&self.far_mesh, &ground);
        engine.draw(&self.patch_mesh, &ground);

        let trunk_color = Color::rgb(0.30, 0.22, 0.14);
        for tree in &self.trees {
            let y = terrain_height(tree.position.x, tree.position.y);
            let model = Mat4::from_translation(Vec3::new(tree.position.x, y, tree.position.y))
                * Mat4::from_scale(Vec3::new(TREE_RADIUS, tree.height, TREE_RADIUS));
            let shader = engine.lit_shader(model).with_base_color(trunk_color);
            engine.draw(&self.tree_mesh, &shader);
        }

        let model = Mat4::from_translation(self.marker.position);
        let shader = engine
            .lit_shader(model)
            .with_base_color(Color::rgb(1.0, 0.85, 0.2));
        engine.draw(&self.marker_mesh, &shader);
    }
}

impl Game for Valley {
    fn start(&mut self, engine: &mut Engine) -> io::Result<()> {
        engine.clear_color = Color::rgb(0.55, 0.72, 0.85);
        engine.light = DirectionalLight {
            direction: Vec3::new(-0.4, -0.85, -0.35).normalized(),
            color: Color::rgb(1.0, 0.97, 0.90),
            intensity: 1.1,
        };
        engine.ambient = Color::rgb(0.24, 0.26, 0.30);
        engine.camera = Camera::look_at(FLIGHT_EYE, FLIGHT_TARGET);
        // The chunk is 512 m across; the default far plane (500 m) would
        // clip the far rim from this vantage.
        engine.camera.far = 900.0;
        Ok(())
    }

    fn update(&mut self, engine: &mut Engine) {
        if engine.input.key_pressed(Key::Escape) {
            engine.quit();
        }
        if engine.input.key_pressed(Key::M) {
            self.top_down = !self.top_down;
        }
    }

    fn render(&mut self, engine: &mut Engine) {
        if self.top_down {
            draw_top_down_debug(&mut engine.framebuffer, &self.grid, &self.marker);
        } else {
            self.draw_scene(engine);
        }
    }
}

fn main() -> io::Result<()> {
    let config = WindowConfig::new("runity — valley", WIDTH, HEIGHT);
    let headless = std::env::var("RUNITY_HEADLESS").is_ok();

    let mut app = App::new(config);
    if headless {
        app = app
            .with_max_frames(1)
            .with_frame_delta(1.0 / 60.0)
            .with_target_fps(None);
    }

    let engine = app.run(Valley::new())?;

    if headless {
        let path = std::env::var("RUNITY_SCREENSHOT").unwrap_or_else(|_| "valley.png".to_string());
        save_png(&path, &engine.framebuffer)?;
        println!(
            "wrote {path} ({}x{})",
            engine.framebuffer.width(),
            engine.framebuffer.height()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_core::physics::SLOPE_40_DEGREES_COS;

    // --- terrain -----------------------------------------------------

    #[test]
    fn the_rim_reaches_the_forty_degree_limit() {
        let far = build_far_heightfield();
        let steep = (0..200)
            .map(|i| INNER_RADIUS + 1.0 + i as f32 * 0.3)
            .any(|r| far.normal_at(r, 0.0).y < SLOPE_40_DEGREES_COS);
        assert!(
            steep,
            "no radius along the rim is steep enough to block a 40 degree climb"
        );
    }

    #[test]
    fn the_forest_area_stays_within_the_walkable_slope() {
        let far = build_far_heightfield();
        for &(x, z) in &[
            (0.0, 0.0),
            (100.0, 0.0),
            (-150.0, 80.0),
            (0.0, 180.0),
            (60.0, 5.0),
        ] {
            let normal = far.normal_at(x, z);
            assert!(
                normal.y >= SLOPE_40_DEGREES_COS,
                "({x}, {z}) inside the forest is too steep to stand on: {normal:?}"
            );
        }
    }

    #[test]
    fn the_patch_edge_matches_nodes_the_far_ring_already_kept() {
        let far = build_far_heightfield();
        let patch = build_patch_heightfield(Vec2::ZERO);
        let far_mesh = far.to_mesh_lod(FAR_LOD_STRIDE);
        let patch_mesh = patch.to_mesh();

        let height_at = |mesh: &Mesh, x: f32, z: f32| -> f32 {
            mesh.vertices
                .iter()
                .find(|v| (v.position.x - x).abs() < 1e-3 && (v.position.z - z).abs() < 1e-3)
                .unwrap_or_else(|| panic!("no vertex at ({x}, {z})"))
                .position
                .y
        };

        for &fixed in &[-32.0f32, 32.0] {
            for &varying in &[-32.0f32, -16.0, 0.0, 16.0, 32.0] {
                assert_eq!(
                    height_at(&far_mesh, fixed, varying),
                    height_at(&patch_mesh, fixed, varying),
                    "seam at x={fixed}, z={varying}"
                );
                assert_eq!(
                    height_at(&far_mesh, varying, fixed),
                    height_at(&patch_mesh, varying, fixed),
                    "seam at x={varying}, z={fixed}"
                );
            }
        }
    }

    // --- forest --------------------------------------------------------

    #[test]
    fn no_two_trees_stand_closer_than_the_minimum_clearance() {
        let trees = place_forest(FOREST_SEED);
        assert!(
            trees.len() > 100,
            "sanity: the forest actually placed trees"
        );
        for i in 0..trees.len() {
            for j in (i + 1)..trees.len() {
                let distance = (trees[i].position - trees[j].position).length();
                assert!(
                    distance >= MIN_TREE_SPACING - 1e-3,
                    "trees {i} and {j} are only {distance} m apart"
                );
            }
        }
    }

    #[test]
    fn the_sparse_forest_is_close_to_its_target_density() {
        let trees = place_forest(FOREST_SEED);
        let sparse_area = std::f32::consts::PI * FOREST_RADIUS * FOREST_RADIUS
            - std::f32::consts::PI * DENSE_PATCH_RADIUS * DENSE_PATCH_RADIUS;
        let expected = sparse_area * BASE_TREE_DENSITY;
        let sparse_count = trees
            .iter()
            .filter(|t| (t.position - DENSE_PATCH_CENTER).length() > DENSE_PATCH_RADIUS)
            .count() as f32;
        assert!(
            (sparse_count - expected).abs() < expected * 0.2,
            "sparse count {sparse_count} vs target {expected}"
        );
    }

    #[test]
    fn the_dense_patch_is_markedly_denser_than_the_rest_of_the_forest() {
        let trees = place_forest(FOREST_SEED);
        let dense_area = std::f32::consts::PI * DENSE_PATCH_RADIUS * DENSE_PATCH_RADIUS;
        let dense_count = trees
            .iter()
            .filter(|t| (t.position - DENSE_PATCH_CENTER).length() <= DENSE_PATCH_RADIUS)
            .count();
        let dense_density = dense_count as f32 / dense_area;
        assert!(
            dense_density > BASE_TREE_DENSITY * (DENSE_DENSITY_MULT - 1.0),
            "the grove isn't noticeably denser than the rest: {dense_density} vs base {BASE_TREE_DENSITY}"
        );
    }

    #[test]
    fn every_tree_is_registered_in_the_static_grid_as_a_cylinder() {
        let trees = place_forest(FOREST_SEED);
        let mut world = World::new();
        populate_world(&mut world, &trees);
        let mut grid = StaticGrid::new(4.0);
        grid.rebuild(&world);

        assert_eq!(grid.len(), trees.len());
        for shape in grid.shapes() {
            match shape.blocker {
                Blocker::Cylinder { radius, top } => {
                    assert_eq!(radius, TREE_RADIUS);
                    assert!(top > 0.0);
                }
                other => panic!("a tree should be a cylinder blocker, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_marker_touches_a_tree_in_the_dense_grove() {
        let valley = Valley::new();
        let marker_xz = Vec2::new(valley.marker.position.x, valley.marker.position.z);
        let mut nearby = Vec::new();
        valley.grid.query(marker_xz.x, marker_xz.y, &mut nearby);
        let touching = nearby.iter().any(|&i| {
            valley
                .grid
                .shape(i)
                .resolve(marker_xz, valley.marker.radius)
                .is_some()
        });
        assert!(touching, "the debug view needs a real contact to draw");
    }
}
