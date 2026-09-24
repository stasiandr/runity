//! Virtual shadow maps: the sun's shadow as a clipmap of small pages, each
//! drawn once and kept — Unreal's Virtual Shadow Maps, planned on the CPU.
//!
//! `shadows: (virtual_maps: true)`. The sun's view is cut into levels:
//! level 0's texels are 1.5 cm across, each level's twice the last's, and
//! each is a window of 32 × 32 pages of 128² texels round the camera. A
//! point is shadowed from the level whose texel is about the size of the
//! screen pixel it is in — near things by the finest, the distance by
//! coarse ones — so the shadow is as sharp as the screen can show at every
//! distance, with no cascades to fit.
//!
//! Which pages a frame needs is worked out from the camera: for each level,
//! the slice of the view at the distances that level serves, laid along
//! the light, and the pages it covers. Pages live in a pool: one more
//! layer of the cascades' shadow map, `resolution` square — 2048 holds 256
//! pages, 4096 a thousand. A page is drawn when it is first needed and kept
//! until something in it changes — a caster moved, came or went (its box,
//! then and now), or the sun turned — so a still scene draws almost no
//! shadow at all after its first frames, however sharp. What was not
//! needed for longest goes when the pool is full. A frame draws at most
//! 192 pages, coarsest first; until a page is drawn, the level above
//! stands in for it.
//!
//! A drawn page's texels are looked up through a page table the lit shader
//! reads: level, page, atlas tile; nine taps of the comparison sampler,
//! held inside the tile. Sun shadows by rays, where asked, take precedence;
//! the lamps keep their own maps.

use std::collections::HashMap;

use glam::{Mat4, Vec2, Vec3};

use crate::gpu::Gpu;

/// Texels across a page.
pub const PAGE: u32 = 128;
/// Levels of the clipmap.
pub const LEVELS: u32 = 8;
/// Pages across a level's window.
pub const WINDOW: i32 = 32;
/// Metres a texel of level 0.
pub const FINEST: f32 = 0.015;
/// Pages drawn a frame, at most.
pub const BUDGET: usize = 192;
/// How far along the light the pages see either way of the camera.
const DEPTH_REACH: f32 = 500.0;

/// Metres a page of `level` covers.
pub fn page_size(level: u32) -> f32 {
    FINEST * PAGE as f32 * (1u32 << level) as f32
}

/// The level a point `distance` metres off is shadowed from, when a screen
/// pixel there is `footprint` metres a metre away (or, negative, that many
/// metres anywhere — an orthographic camera).
pub fn level_at(distance: f32, footprint: f32) -> u32 {
    let pixel = if footprint < 0.0 { -footprint } else { distance * footprint };
    let level = (pixel.max(1e-9) / FINEST).log2().ceil();
    level.clamp(0.0, (LEVELS - 1) as f32) as u32
}

/// The light's frame: across, up, and along it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Basis {
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
}

impl Basis {
    fn of(sun: Vec3) -> Self {
        let forward = sun.normalize_or(Vec3::NEG_Y);
        let hint = if forward.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let right = forward.cross(hint).normalize();
        let up = right.cross(forward);
        Self { right, up, forward }
    }

    fn uv(&self, p: Vec3) -> Vec2 {
        Vec2::new(p.dot(self.right), p.dot(self.up))
    }
}

/// A page's key: level and where in its level's grid.
type Key = (u32, i32, i32);

struct Resident {
    physical: u32,
    drawn: bool,
    needed_at: u64,
}

/// A page to draw this frame.
pub(crate) struct Job {
    pub physical: u32,
    /// Its rectangle in the light's across and up, metres.
    pub rect: [f32; 4],
}

/// A caster as the pages see it: its rectangle across the light, and what
/// it is, to tell whether it changed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Caster {
    pub rect: [f32; 4],
    pub key: u64,
}

pub(crate) struct VirtualShadows {
    /// Pages across the pool.
    pub(crate) pool_side: u32,
    pub(crate) table: wgpu::Buffer,
    pages: wgpu::Buffer,
    stride: u64,
    pub(crate) page_group: wgpu::BindGroup,
    pub(crate) clear: wgpu::RenderPipeline,
    resident: HashMap<Key, Resident>,
    free: Vec<u32>,
    frame: u64,
    basis: Option<Basis>,
    depth_origin: f32,
    casters: Vec<Caster>,
    /// This frame's: the frame uniform's part, and how many were drawn.
    pub(crate) uniform: [[f32; 4]; 8],
    pub(crate) drawn_last: usize,
}

/// The page table, made before the rest: the frame's bind group holds it.
pub(crate) fn table_buffer(gpu: &Gpu) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("virtual shadow pages"),
        size: (LEVELS as i32 * WINDOW * WINDOW) as u64 * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The frame uniform's part for no virtual shadow maps.
pub(crate) const OFF: [[f32; 4]; 8] = [[0.0; 4]; 8];

/// Two convex outlines overlap: no axis of either separates them.
fn overlaps(rect: [f32; 4], hull: &[Vec2]) -> bool {
    if hull.is_empty() {
        return false;
    }
    let corners = [
        Vec2::new(rect[0], rect[2]),
        Vec2::new(rect[1], rect[2]),
        Vec2::new(rect[1], rect[3]),
        Vec2::new(rect[0], rect[3]),
    ];
    let lo = hull.iter().fold(Vec2::splat(f32::MAX), |a, &b| a.min(b));
    let hi = hull.iter().fold(Vec2::splat(f32::MIN), |a, &b| a.max(b));
    if hi.x < rect[0] || lo.x > rect[1] || hi.y < rect[2] || lo.y > rect[3] {
        return false;
    }
    for i in 0..hull.len() {
        let a = hull[i];
        let b = hull[(i + 1) % hull.len()];
        let normal = Vec2::new(b.y - a.y, a.x - b.x);
        let limit = normal.dot(a);
        // The hull is wound counter-clockwise: outside is where the normal
        // points.
        if corners.iter().all(|c| normal.dot(*c) > limit) {
            return false;
        }
    }
    true
}

/// The convex hull of points, counter-clockwise (Andrew's monotone chain).
fn hull(mut points: Vec<Vec2>) -> Vec<Vec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<Vec2> = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// What the camera shows, for planning: where it is, and the corners of its
/// view between two distances.
pub(crate) struct View {
    pub eye: Vec3,
    pub inverse_view_projection: Mat4,
    pub near: f32,
    pub far: f32,
    /// Metres a screen pixel is, a metre away; negative for an orthographic
    /// camera's metres anywhere.
    pub footprint: f32,
    pub max_distance: f32,
}

impl View {
    /// The eight corners of the view between `from` and `to` metres along
    /// it (by the perspective's depth).
    fn slice(&self, from: f32, to: f32) -> Vec<Vec3> {
        let depth = |d: f32| {
            if self.footprint < 0.0 {
                // Orthographic: depth is linear.
                ((d - self.near) / (self.far - self.near)).clamp(0.0, 1.0)
            } else {
                // wgpu's 0..1 perspective depth of a view distance.
                let (n, f) = (self.near, self.far);
                (f * (d - n) / (d * (f - n))).clamp(0.0, 1.0)
            }
        };
        let mut out = Vec::with_capacity(8);
        for z in [depth(from.max(self.near)), depth(to.min(self.far))] {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = self.inverse_view_projection * glam::Vec4::new(x, y, z, 1.0);
                out.push(p.truncate() / p.w);
            }
        }
        out
    }
}

impl VirtualShadows {
    /// Pages in a pool the size of the shadow map's `resolution`.
    pub(crate) fn new(
        gpu: &Gpu,
        shadow_layout: &wgpu::BindGroupLayout,
        caster_stride: u64,
        depth_format: wgpu::TextureFormat,
        resolution: u32,
        table: wgpu::Buffer,
    ) -> Self {
        let pool_side = (resolution / PAGE).max(2);
        let pages = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("virtual shadow views"),
            size: caster_stride * BUDGET as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let page_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("virtual shadow view"),
            layout: shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &pages,
                    offset: 0,
                    size: wgpu::BufferSize::new(caster_stride),
                }),
            }],
        });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("runity::virtual shadow clear"),
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    let x = f32((i << 1u) & 2u);
                    let y = f32(i & 2u);
                    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 1.0, 1.0);
                }"
                .into(),
            ),
        });
        let clear = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("runity::virtual shadow clear"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pool_side,
            table,
            pages,
            stride: caster_stride,
            page_group,
            clear,
            resident: HashMap::new(),
            free: (0..pool_side * pool_side).rev().collect(),
            frame: 0,
            basis: None,
            depth_origin: 0.0,
            casters: Vec::new(),
            uniform: OFF,
            drawn_last: 0,
        }
    }

    /// A pool of a new size: every page forgotten.
    pub(crate) fn resize(&mut self, resolution: u32) {
        let side = (resolution / PAGE).max(2);
        if side == self.pool_side {
            return;
        }
        self.pool_side = side;
        self.resident.clear();
        self.free = (0..side * side).rev().collect();
    }

    /// Forget every page drawn: the next frames draw them again.
    fn undraw_all(&mut self) {
        for page in self.resident.values_mut() {
            page.drawn = false;
        }
    }

    /// A caster's rectangle across the light, from its world box.
    pub(crate) fn rect_of(&self, sun: Vec3, min: Vec3, max: Vec3) -> [f32; 4] {
        let basis = Basis::of(sun);
        let mut r = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
        for c in 0..8 {
            let p = Vec3::new(
                if c & 1 == 0 { min.x } else { max.x },
                if c & 2 == 0 { min.y } else { max.y },
                if c & 4 == 0 { min.z } else { max.z },
            );
            let uv = basis.uv(p);
            r = [r[0].min(uv.x), r[1].max(uv.x), r[2].min(uv.y), r[3].max(uv.y)];
        }
        r
    }

    /// Plan this frame's pages: what is needed, what is kept, what is drawn.
    /// Returns the pages to draw; fills the page table and the uniform.
    pub(crate) fn plan(&mut self, gpu: &Gpu, sun: Vec3, view: &View, casters: Vec<Caster>, foliage: &[u8]) -> Vec<Job> {
        self.frame += 1;
        let basis = Basis::of(sun);
        // The depth the pages see, snapped so it does not move with every
        // step the camera takes: a move of it draws everything again.
        let along = view.eye.dot(basis.forward);
        let origin = (along / 100.0).round() * 100.0 - DEPTH_REACH;
        let turned = self.basis.is_none_or(|b| b.forward.dot(basis.forward) < 0.999_999);
        if turned || origin != self.depth_origin {
            self.undraw_all();
        }
        self.basis = Some(basis);
        self.depth_origin = origin;

        // What changed since last frame: its box then and now.
        let mut changed: Vec<[f32; 4]> = Vec::new();
        for i in 0..casters.len().max(self.casters.len()) {
            match (self.casters.get(i), casters.get(i)) {
                (Some(a), Some(b)) if a == b => {}
                (a, b) => {
                    changed.extend(a.map(|c| c.rect));
                    changed.extend(b.map(|c| c.rect));
                }
            }
        }
        if !changed.is_empty() {
            for (&(level, x, y), page) in self.resident.iter_mut() {
                let size = page_size(level);
                let rect = [x as f32 * size, (x + 1) as f32 * size, y as f32 * size, (y + 1) as f32 * size];
                if changed.iter().any(|c| c[0] <= rect[1] && c[1] >= rect[0] && c[2] <= rect[3] && c[3] >= rect[2]) {
                    page.drawn = false;
                }
            }
        }
        self.casters = casters;

        // What the view needs, level by level.
        let eye_uv = basis.uv(view.eye);
        let mut windows = [[0i32; 2]; LEVELS as usize];
        let mut needed: Vec<Key> = Vec::new();
        let mut lower = 0.0f32;
        for level in 0..LEVELS {
            let size = page_size(level);
            let window = [
                (eye_uv.x / size).floor() as i32 - WINDOW / 2,
                (eye_uv.y / size).floor() as i32 - WINDOW / 2,
            ];
            windows[level as usize] = window;
            // The farthest this level serves.
            let upper = if view.footprint < 0.0 {
                if level_at(0.0, view.footprint) == level { view.max_distance } else { 0.0 }
            } else {
                FINEST * (1u32 << level) as f32 / view.footprint.max(1e-9)
            };
            let upper = if level == LEVELS - 1 { view.max_distance } else { upper.min(view.max_distance) };
            if upper <= 0.0 || lower >= view.max_distance {
                lower = lower.max(upper);
                continue;
            }
            // From the eye out, not only its own distances: a level is
            // also what stands in for the finer ones until they are drawn.
            let outline = hull(
                view.slice(0.0, upper * 1.05)
                    .into_iter()
                    .map(|p| basis.uv(p))
                    .collect(),
            );
            for y in window[1]..window[1] + WINDOW {
                for x in window[0]..window[0] + WINDOW {
                    let rect = [x as f32 * size, (x + 1) as f32 * size, y as f32 * size, (y + 1) as f32 * size];
                    if overlaps(rect, &outline) {
                        needed.push((level, x, y));
                    }
                }
            }
            lower = upper;
        }
        // More than the pool: the finest go first.
        let pool = (self.pool_side * self.pool_side) as usize;
        if needed.len() > pool {
            needed.sort_by_key(|k| std::cmp::Reverse(k.0));
            needed.truncate(pool);
        }

        // Keep what is needed, find room for what is new.
        let frame = self.frame;
        let mut fresh: Vec<Key> = Vec::new();
        for key in &needed {
            match self.resident.get_mut(key) {
                Some(page) => page.needed_at = frame,
                None => fresh.push(*key),
            }
        }
        if self.free.len() < fresh.len() {
            let mut old: Vec<(u64, Key)> = self
                .resident
                .iter()
                .filter(|(_, p)| p.needed_at != frame)
                .map(|(k, p)| (p.needed_at, *k))
                .collect();
            old.sort_unstable();
            for (_, key) in old.into_iter().take(fresh.len() - self.free.len()) {
                if let Some(page) = self.resident.remove(&key) {
                    self.free.push(page.physical);
                }
            }
        }
        for key in fresh {
            let Some(physical) = self.free.pop() else { break };
            self.resident.insert(
                key,
                Resident {
                    physical,
                    drawn: false,
                    needed_at: frame,
                },
            );
        }

        // Draw what is needed and not drawn, coarsest first, up to the
        // budget.
        let mut undrawn: Vec<Key> = needed
            .iter()
            .filter(|k| self.resident.get(k).is_some_and(|p| !p.drawn))
            .copied()
            .collect();
        undrawn.sort_by_key(|k| std::cmp::Reverse(k.0));
        undrawn.truncate(BUDGET);
        let view_of = |size: f32, x: i32, y: i32| {
            let eye = basis.forward * origin;
            let look = Mat4::look_to_rh(eye, basis.forward, basis.up);
            // The light's view looks down its own −z; across is +x, up +y.
            let projection = Mat4::orthographic_rh(
                x as f32 * size,
                (x + 1) as f32 * size,
                y as f32 * size,
                (y + 1) as f32 * size,
                0.0,
                DEPTH_REACH * 2.0,
            );
            projection * look
        };
        let mut jobs = Vec::with_capacity(undrawn.len());
        let mut matrices = vec![0u8; self.stride as usize * undrawn.len().max(1)];
        for (i, key) in undrawn.iter().enumerate() {
            let page = self.resident.get_mut(key).expect("kept above");
            page.drawn = true;
            let (level, x, y) = *key;
            let size = page_size(level);
            let view_projection = view_of(size, x, y);
            let at = i * self.stride as usize;
            matrices[at..at + 64].copy_from_slice(bytemuck::bytes_of(&view_projection.to_cols_array_2d()));
            let end = (at + 64 + foliage.len()).min(at + self.stride as usize);
            matrices[at + 64..end].copy_from_slice(&foliage[..end - at - 64]);
            jobs.push(Job {
                physical: page.physical,
                rect: [x as f32 * size, (x + 1) as f32 * size, y as f32 * size, (y + 1) as f32 * size],
            });
        }
        if !jobs.is_empty() {
            gpu.queue.write_buffer(&self.pages, 0, &matrices);
        }
        self.drawn_last = jobs.len();

        // The page table: each level's window, drawn pages marked.
        let mut table = vec![0u32; (LEVELS as i32 * WINDOW * WINDOW) as usize];
        for (&(level, x, y), page) in &self.resident {
            if !page.drawn {
                continue;
            }
            let [wx, wy] = windows[level as usize];
            let (sx, sy) = (x - wx, y - wy);
            if (0..WINDOW).contains(&sx) && (0..WINDOW).contains(&sy) {
                table[(level as i32 * WINDOW * WINDOW + sy * WINDOW + sx) as usize] = page.physical + 1;
            }
        }
        gpu.queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        let mut uniform = [[0.0f32; 4]; 8];
        uniform[0] = [basis.right.x, basis.right.y, basis.right.z, self.pool_side as f32];
        uniform[1] = [basis.up.x, basis.up.y, basis.up.z, origin];
        uniform[2] = [basis.forward.x, basis.forward.y, basis.forward.z, DEPTH_REACH * 2.0];
        uniform[3] = [1.0, LEVELS as f32, view.footprint, view.max_distance];
        for (level, w) in windows.iter().enumerate() {
            uniform[4 + level / 2][(level % 2) * 2] = w[0] as f32;
            uniform[4 + level / 2][(level % 2) * 2 + 1] = w[1] as f32;
        }
        self.uniform = uniform;
        jobs
    }

    /// The offset into the page views for the `i`-th job.
    pub(crate) fn offset(&self, i: usize) -> u32 {
        (i as u64 * self.stride) as u32
    }

    /// Pages kept, and drawn this frame.
    pub(crate) fn stats(&self) -> (usize, usize) {
        (self.resident.values().filter(|p| p.drawn).count(), self.drawn_last)
    }
}

impl VirtualShadows {
    /// Where a page's tile is in the pool, texels: x, y, side.
    pub(crate) fn tile(&self, physical: u32) -> (f32, f32, f32) {
        (
            (physical % self.pool_side * PAGE) as f32,
            (physical / self.pool_side * PAGE) as f32,
            PAGE as f32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_double_with_distance_and_a_rectangle_meets_a_hull_only_where_it_does() {
        // A pixel a thousandth of a metre a metre: 1.5 cm at 15 m.
        assert_eq!(level_at(10.0, 0.001), 0);
        assert_eq!(level_at(20.0, 0.001), 1);
        assert_eq!(level_at(50.0, 0.001), 2);
        assert_eq!(level_at(1.0e6, 0.001), LEVELS - 1);
        assert_eq!(page_size(1), page_size(0) * 2.0);
        let triangle = hull(vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(0.0, 4.0), Vec2::new(1.0, 1.0)]);
        assert_eq!(triangle.len(), 3, "the inner point is not on the hull");
        assert!(overlaps([0.5, 1.0, 0.5, 1.0], &triangle));
        assert!(!overlaps([3.0, 4.0, 3.0, 4.0], &triangle), "past the long side");
        assert!(!overlaps([5.0, 6.0, 0.0, 1.0], &triangle));
    }
}
