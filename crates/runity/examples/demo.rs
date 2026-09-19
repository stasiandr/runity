//! Card #40 in four stop-frames: the valley's rim and stream from above, the
//! sparse forest and the dense grove seen straight down, and the fixed-scale
//! debug view switched on.
//!
//! ```text
//! cargo run --release --example demo
//! ```
//!
//! Headless on purpose, writing PNGs into `$STUDIO_ARTIFACTS` (or the working
//! directory when that is unset):
//!
//! * `1-overview.png` — the whole chunk from a fixed flyover camera: the
//!   mountain rim and the streambed's dip.
//! * `2-sparse-forest.png` — straight down over the ordinary part of the
//!   forest, ~1 trunk per 120 m².
//! * `3-dense-grove.png` — straight down over the one deliberately dense
//!   patch by the stream, four times as thick.
//! * `4-debug-view.png` — the fixed-scale top-down debug overlay: the static
//!   grid's cells, every trunk's footprint, and the marker's live contact
//!   with the tree it stands against.
//!
//! This recreates a smaller slice of `valley.rs`'s own terrain formula,
//! forest placement and debug overlay — that file is the tested, canonical
//! version; this one is thrown away before the card lands.

use runity::prelude::*;
use runity_core::physics::{Blocker, Body, Heightfield, StaticGrid};
use std::io;
use std::path::PathBuf;

const CHUNK_HALF: f32 = 256.0;
const INNER_RADIUS: f32 = 200.0;
const RIM_HEIGHT: f32 = 80.0;
const BASIN_K: f32 = 0.0002;
const STREAM_DEPTH: f32 = 3.0;
const STREAM_HALF_WIDTH: f32 = 15.0;

fn terrain_height(x: f32, z: f32) -> f32 {
    let r = (x * x + z * z).sqrt();
    let basin = BASIN_K * r * r;
    let rim = if r <= INNER_RADIUS {
        0.0
    } else {
        let t = ((r - INNER_RADIUS) / (CHUNK_HALF - INNER_RADIUS)).clamp(0.0, 1.0);
        RIM_HEIGHT * t * t * (3.0 - 2.0 * t)
    };
    let normalized = z / STREAM_HALF_WIDTH;
    let stream = -STREAM_DEPTH / (1.0 + normalized * normalized);
    basin + rim + stream
}

const WORLD_CELL: f32 = 2.0;

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

const TREE_RADIUS: f32 = 0.35;
const MIN_TREE_SPACING: f32 = 4.0;
const BASE_TREE_DENSITY: f32 = 1.0 / 120.0;
const FOREST_RADIUS: f32 = 190.0;
const DENSE_PATCH_CENTER: Vec2 = Vec2::new(60.0, 5.0);
const DENSE_PATCH_RADIUS: f32 = 15.0;
const DENSE_DENSITY_MULT: f32 = 4.0;
const SEED: u64 = 0xC0FF_EE15_5EED_2026;
/// Well clear of the dense grove, so the sparse screenshot shows only the
/// ordinary density.
const SPARSE_SAMPLE: Vec2 = Vec2::new(-90.0, -10.0);

struct Tree {
    position: Vec2,
    height: f32,
}

struct Rng(u64);

impl Rng {
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

fn place_forest(seed: u64) -> Vec<Tree> {
    let mut rng = Rng(seed);
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

fn draw_terrain_and_forest(engine: &mut Engine, far_mesh: &Mesh, tree_mesh: &Mesh, trees: &[Tree]) {
    let ground = engine
        .lit_shader(Mat4::IDENTITY)
        .with_base_color(Color::rgb(0.40, 0.48, 0.30));
    engine.draw(far_mesh, &ground);

    let trunk_color = Color::rgb(0.30, 0.22, 0.14);
    for tree in trees {
        let y = terrain_height(tree.position.x, tree.position.y);
        let model = Mat4::from_translation(Vec3::new(tree.position.x, y, tree.position.y))
            * Mat4::from_scale(Vec3::new(TREE_RADIUS, tree.height, TREE_RADIUS));
        let shader = engine.lit_shader(model).with_base_color(trunk_color);
        engine.draw(tree_mesh, &shader);
    }
}

fn light_the_scene(engine: &mut Engine) {
    engine.clear_color = Color::rgb(0.55, 0.72, 0.85);
    // `headless::render` clears with the *old* `clear_color` before running
    // this closure, so the sky needs a clear of its own here.
    engine.framebuffer.clear(engine.clear_color);
    engine.light = DirectionalLight {
        direction: Vec3::new(-0.4, -0.85, -0.35).normalized(),
        color: Color::rgb(1.0, 0.97, 0.90),
        intensity: 1.1,
    };
    engine.ambient = Color::rgb(0.24, 0.26, 0.30);
}

/// A camera looking straight down at `target`, from far enough up to frame
/// roughly `view_diameter` metres across with a narrow field of view — wide
/// enough would slant each tree's silhouette outward near the edges, which
/// reads as motion blur rather than a forest seen from above.
fn top_down_camera(engine: &mut Engine, target: Vec2, view_diameter: f32) {
    let fov_y = 20f32.to_radians();
    let height = view_diameter * 0.5 / (fov_y * 0.5).tan();
    engine.camera = Camera::look_at(
        Vec3::new(target.x, height, target.y),
        Vec3::new(target.x, 0.0, target.y),
    );
    engine.camera.fov_y = fov_y;
    // Looking straight down makes the default +Y up vector degenerate.
    engine.camera.up = Vec3::Z;
    engine.camera.far = (height + 50.0).max(engine.camera.far);
}

fn draw_top_down_debug(framebuffer: &mut Framebuffer, grid: &StaticGrid, marker: &Body) {
    // A closer crop than `valley.rs`'s own default, so the marker's contact
    // with the trunk it's touching is legible in a still screenshot.
    let view_radius = 6.0;
    let center = Vec2::new(marker.position.x, marker.position.z);
    let (width, height) = (framebuffer.width() as f32, framebuffer.height() as f32);
    framebuffer.clear(Color::rgb(0.08, 0.09, 0.10));
    let scale = (width.min(height) * 0.5 - 8.0) / view_radius;
    let to_screen = |p: Vec2| -> (f32, f32) {
        let rel = p - center;
        (width * 0.5 + rel.x * scale, height * 0.5 - rel.y * scale)
    };

    let cell = grid.cell_size();
    let grid_color = Color::rgb(0.28, 0.30, 0.30);
    let first = (
        ((center.x - view_radius) / cell).floor() as i32,
        ((center.y - view_radius) / cell).floor() as i32,
    );
    let last = (
        ((center.x + view_radius) / cell).ceil() as i32,
        ((center.y + view_radius) / cell).ceil() as i32,
    );
    for cx in first.0..=last.0 {
        let x = cx as f32 * cell;
        let a = to_screen(Vec2::new(x, center.y - view_radius));
        let b = to_screen(Vec2::new(x, center.y + view_radius));
        debug::draw_line(framebuffer, a, b, grid_color);
    }
    for cz in first.1..=last.1 {
        let z = cz as f32 * cell;
        let a = to_screen(Vec2::new(center.x - view_radius, z));
        let b = to_screen(Vec2::new(center.x + view_radius, z));
        debug::draw_line(framebuffer, a, b, grid_color);
    }

    let tree_color = Color::rgb(0.35, 0.85, 0.40);
    for shape in grid.shapes() {
        let position = Vec2::new(shape.base.x, shape.base.z);
        if (position - center).length() > view_radius + 5.0 {
            continue;
        }
        if let Blocker::Cylinder { radius, .. } = shape.blocker {
            debug::draw_circle(
                framebuffer,
                to_screen(position),
                radius * scale,
                14,
                tree_color,
            );
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

fn artifacts_dir() -> PathBuf {
    std::env::var_os("STUDIO_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() -> io::Result<()> {
    let out = artifacts_dir();
    std::fs::create_dir_all(&out)?;

    let trees = place_forest(SEED);
    let anchor = trees
        .last()
        .expect("the dense patch places at least one tree")
        .position;
    let marker_xz = anchor + Vec2::new(0.3, 0.0);
    let marker = Body::new(Vec3::new(
        marker_xz.x,
        terrain_height(marker_xz.x, marker_xz.y),
        marker_xz.y,
    ));

    let mut world = World::new();
    populate_world(&mut world, &trees);
    let mut grid = StaticGrid::new(4.0);
    grid.rebuild(&world);

    let far_mesh = build_far_heightfield().to_mesh_lod(2);
    let tree_mesh = Mesh::cylinder(1.0, 1.0, 8);

    // 1: the whole valley from a fixed flyover.
    let frame = headless::render(960, 540, |engine| {
        light_the_scene(engine);
        engine.camera =
            Camera::look_at(Vec3::new(-300.0, 420.0, 380.0), Vec3::new(0.0, -10.0, 0.0));
        engine.camera.far = 900.0;
        draw_terrain_and_forest(engine, &far_mesh, &tree_mesh, &trees);
    });
    save_png(out.join("1-overview.png"), &frame)?;

    // 2: straight down over the ordinary part of the forest.
    let frame = headless::render(960, 540, |engine| {
        light_the_scene(engine);
        top_down_camera(engine, SPARSE_SAMPLE, 100.0);
        draw_terrain_and_forest(engine, &far_mesh, &tree_mesh, &trees);
    });
    save_png(out.join("2-sparse-forest.png"), &frame)?;

    // 3: straight down over the dense grove.
    let frame = headless::render(960, 540, |engine| {
        light_the_scene(engine);
        top_down_camera(engine, DENSE_PATCH_CENTER, 100.0);
        draw_terrain_and_forest(engine, &far_mesh, &tree_mesh, &trees);
    });
    save_png(out.join("3-dense-grove.png"), &frame)?;

    // 4: the fixed-scale debug overlay.
    let frame = headless::render(960, 540, |engine| {
        draw_top_down_debug(&mut engine.framebuffer, &grid, &marker);
    });
    save_png(out.join("4-debug-view.png"), &frame)?;

    println!("wrote 4 screenshots into {}", out.display());
    Ok(())
}
