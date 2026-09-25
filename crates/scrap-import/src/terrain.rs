//! `.scrterrain`: ground, described rather than modelled.
//!
//! ```text
//! (
//!     size: (200.0, 200.0),   // metres along x and z
//!     resolution: 129,        // vertices along each side
//!     height: 18.0,           // metres from the lowest point to the highest
//!     noise: (seed: 7, scale: 60.0, octaves: 5, persistence: 0.5),
//!     edits: [
//!         Raise(at: (20.0, -10.0), radius: 8.0, by: 3.0),
//!         Flatten(at: (0.0, 0.0), radius: 12.0, to: 2.0),
//!     ],
//! )
//! ```
//!
//! `edits` are what a brush leaves: applied in order over the noise, each a
//! line — so sculpting is a diff someone can read, and undoing one stroke
//! is deleting its line.
//!
//! Imported into an ordinary mesh named after the file, centred on x and z
//! with its lowest point at zero, so a scene places it like any model —
//! `model: "hills", collider: Model, body: Static` — and it reloads like
//! any model: change the seed, save, and the running game has new hills.
//!
//! Or painted: `heightmap: "hills.png"` — a greyscale image beside the
//! file, black at the bottom and white at `height` — for ground someone
//! drew. With both, the noise adds a tenth of the height as detail on top
//! of the painting. The terrain's content hash covers the image too, so
//! repainting it rebuilds the terrain like editing the file does.

use std::path::Path;

use anyhow::{ensure, Context, Result};
use scrap::asset::{Bounds, MeshAsset, Submesh, Vertex};
use serde::Deserialize;

use crate::ImportSettings;

#[derive(Debug, Deserialize)]
struct TerrainSource {
    size: (f32, f32),
    #[serde(default = "default_resolution")]
    resolution: u32,
    height: f32,
    #[serde(default, deserialize_with = "plain")]
    noise: Option<Noise>,
    /// A greyscale image, relative to this file.
    #[serde(default, deserialize_with = "plain")]
    heightmap: Option<String>,
    #[serde(default)]
    edits: Vec<Edit>,
}

/// An optional field written as its value, not `Some(value)`.
fn plain<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// The files a terrain is built from besides itself: its heightmap. Whose
/// bytes go into its content hash, so a repainted heightmap is a changed
/// terrain.
pub fn dependencies(path: &Path) -> Vec<std::path::PathBuf> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(source) = ron::from_str::<TerrainSource>(&text) else {
        return Vec::new();
    };
    source
        .heightmap
        .map(|name| path.parent().unwrap_or(Path::new(".")).join(name))
        .into_iter()
        .collect()
}

/// The heightmap a terrain names, as written: relative to the terrain.
pub fn heightmap(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    ron::from_str::<TerrainSource>(&text).ok()?.heightmap
}

/// Point a terrain at another heightmap, changing that one string in the
/// file and nothing else — the comments and the line of edits a brush left
/// stay as they were.
pub fn set_heightmap(path: &Path, to: &str) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let (start, end) = heightmap_literal(&text)
        .with_context(|| format!("{} has no `heightmap: \"…\"` to change", path.display()))?;
    let changed = format!("{}{to:?}{}", &text[..start], &text[end..]);
    let read: TerrainSource =
        ron::from_str(&changed).with_context(|| format!("{}", path.display()))?;
    ensure!(
        read.heightmap.as_deref() == Some(to),
        "{}: the heightmap did not change to {to:?}",
        path.display()
    );
    std::fs::write(path, changed).with_context(|| format!("{}", path.display()))?;
    Ok(())
}

/// Where the string after `heightmap:` sits in the text, quotes included.
/// Not in a comment: a line's `//` hides everything after it.
fn heightmap_literal(text: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let code = line.split("//").next().unwrap_or("");
        if let Some(key) = code.find("heightmap") {
            let rest = code[key + "heightmap".len()..].trim_start();
            if let Some(value) = rest.strip_prefix(':') {
                let value = value.trim_start();
                if value.starts_with('"') {
                    let start = offset + (code.len() - value.len());
                    let mut escaped = false;
                    for (i, c) in value.char_indices().skip(1) {
                        match c {
                            '\\' if !escaped => escaped = true,
                            '"' if !escaped => return Some((start, start + i + 1)),
                            _ => escaped = false,
                        }
                    }
                }
            }
        }
        offset += line.len();
    }
    None
}

/// A greyscale image, sampled smoothly anywhere in `[0, 1]²`.
struct Heightmap {
    width: u32,
    height: u32,
    values: Vec<f32>,
}

impl Heightmap {
    fn load(path: &Path) -> Result<Self> {
        let image = image::open(path)
            .with_context(|| format!("heightmap {}", path.display()))?
            .into_luma16();
        let (width, height) = image.dimensions();
        Ok(Self {
            width,
            height,
            values: image.pixels().map(|p| p.0[0] as f32 / 65535.0).collect(),
        })
    }

    fn sample(&self, u: f32, v: f32) -> f32 {
        let x = u.clamp(0.0, 1.0) * (self.width - 1) as f32;
        let y = v.clamp(0.0, 1.0) * (self.height - 1) as f32;
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let at = |x: u32, y: u32| self.values[(y * self.width + x) as usize];
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        top + (bottom - top) * fy
    }
}

/// One brush stroke, in metres, with a smooth falloff to its radius.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, Deserialize)]
pub enum Edit {
    /// Up by `by` at the centre (down if negative).
    Raise {
        at: (f32, f32),
        radius: f32,
        by: f32,
    },
    /// Toward the height `to`, fully at the centre.
    Flatten {
        at: (f32, f32),
        radius: f32,
        to: f32,
    },
}

impl Edit {
    fn apply(&self, x: f32, z: f32, height: f32) -> f32 {
        let (at, radius) = match *self {
            Edit::Raise { at, radius, .. } | Edit::Flatten { at, radius, .. } => (at, radius),
        };
        let distance = ((x - at.0).powi(2) + (z - at.1).powi(2)).sqrt();
        if distance >= radius.max(1e-3) {
            return height;
        }
        // Smoothstep from the centre out: no crease where a stroke ends.
        let t = 1.0 - distance / radius;
        let weight = t * t * (3.0 - 2.0 * t);
        match *self {
            Edit::Raise { by, .. } => height + by * weight,
            Edit::Flatten { to, .. } => height + (to - height) * weight,
        }
    }
}

fn default_resolution() -> u32 {
    129
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct Noise {
    seed: u64,
    /// The width of the biggest hills, in metres.
    scale: f32,
    octaves: u32,
    /// How much each finer octave counts, relative to the one before.
    persistence: f32,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            seed: 1,
            scale: 50.0,
            octaves: 4,
            persistence: 0.5,
        }
    }
}

/// A value in `[0, 1)` for a lattice point: specified, so every machine
/// grows the same hills from the same seed.
fn lattice(seed: u64, x: i64, z: i64) -> f32 {
    let mut h = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (z as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth value noise at a point, in `[0, 1)`.
fn value(seed: u64, x: f32, z: f32) -> f32 {
    let (x0, z0) = (x.floor(), z.floor());
    let (fx, fz) = (x - x0, z - z0);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sz) = (smooth(fx), smooth(fz));
    let (ix, iz) = (x0 as i64, z0 as i64);
    let a = lattice(seed, ix, iz);
    let b = lattice(seed, ix + 1, iz);
    let c = lattice(seed, ix, iz + 1);
    let d = lattice(seed, ix + 1, iz + 1);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sz
}

/// Octaves of noise, summed: big hills with smaller ones on them.
fn fbm(noise: &Noise, x: f32, z: f32) -> f32 {
    let (mut total, mut amplitude, mut frequency, mut weight) =
        (0.0, 1.0, 1.0 / noise.scale.max(1e-3), 0.0);
    for octave in 0..noise.octaves.max(1) {
        total += value(
            noise.seed.wrapping_add(octave as u64),
            x * frequency,
            z * frequency,
        ) * amplitude;
        weight += amplitude;
        amplitude *= noise.persistence;
        frequency *= 2.0;
    }
    total / weight
}

pub fn mesh_from_terrain(path: &Path, settings: &ImportSettings) -> Result<MeshAsset> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let source: TerrainSource =
        ron::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let n = source.resolution;
    ensure!(
        (2..=1025).contains(&n),
        "{}: resolution is between 2 and 1025 vertices a side, not {n}",
        path.display()
    );
    let (width, depth) = source.size;
    ensure!(
        width > 0.0 && depth > 0.0,
        "{}: size must be positive",
        path.display()
    );

    // Heights first, normalised so the lowest point is at zero and the
    // highest at `height` — the number in the file is the number you get —
    // and then the edits, in metres on top.
    let at = |i: u32, j: u32| {
        (
            -width * 0.5 + width * i as f32 / (n - 1) as f32,
            -depth * 0.5 + depth * j as f32 / (n - 1) as f32,
        )
    };
    let painted = source
        .heightmap
        .as_ref()
        .map(|name| Heightmap::load(&path.parent().unwrap_or(Path::new(".")).join(name)))
        .transpose()?;
    // Noise alone when nothing is painted; with a painting, only if asked.
    let noise = match (&painted, source.noise) {
        (None, noise) => Some(noise.unwrap_or_default()),
        (Some(_), noise) => noise,
    };
    let mut heights: Vec<f32> = match &noise {
        Some(noise) => (0..n * n)
            .map(|k| {
                let (x, z) = at(k % n, k / n);
                fbm(noise, x, z)
            })
            .collect(),
        None => vec![0.0; (n * n) as usize],
    };
    let (low, high) = heights
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(*h), hi.max(*h)));
    let range = (high - low).max(1e-6);
    for (k, h) in heights.iter_mut().enumerate() {
        let noise = if noise.is_some() {
            (*h - low) / range
        } else {
            0.0
        };
        *h = match &painted {
            // A painting is taken as painted: black is the ground, white
            // is `height`. Noise, if any, is detail on top.
            Some(map) => {
                let (i, j) = (k as u32 % n, k as u32 / n);
                let (u, v) = (i as f32 / (n - 1) as f32, j as f32 / (n - 1) as f32);
                (map.sample(u, v) + noise * 0.1) * source.height
            }
            None => noise * source.height,
        };
        let (x, z) = at(k as u32 % n, k as u32 / n);
        for edit in &source.edits {
            *h = edit.apply(x, z, *h);
        }
    }
    let height = |i: u32, j: u32| heights[(j.min(n - 1) * n + i.min(n - 1)) as usize];

    let (dx, dz) = (width / (n - 1) as f32, depth / (n - 1) as f32);
    let mut vertices = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let (x, z) = at(i, j);
            // Central differences: a normal that leans away from the slope.
            let (l, r) = (height(i.saturating_sub(1), j), height(i + 1, j));
            let (b, f) = (height(i, j.saturating_sub(1)), height(i, j + 1));
            let normal =
                glam::Vec3::new((l - r) / (2.0 * dx), 1.0, (b - f) / (2.0 * dz)).normalize();
            vertices.push(Vertex {
                position: [x, height(i, j), z],
                normal: normal.to_array(),
                uv: [i as f32 / (n - 1) as f32, j as f32 / (n - 1) as f32],
            });
        }
    }
    let mut indices = Vec::with_capacity(((n - 1) * (n - 1) * 6) as usize);
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = j * n + i;
            let (b, c, d) = (a + 1, a + n, a + n + 1);
            // Counter-clockwise seen from above, like the plane.
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Ok(MeshAsset {
        id: settings.asset_id(),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "terrain".into()),
        bounds: Bounds::of(&vertices),
        submeshes: vec![Submesh {
            first_index: 0,
            index_count: indices.len() as u32,
            material: None,
        }],
        vertices,
        indices,
        skin: None,
        colors: Vec::new(),
        look: None,
    })
}
