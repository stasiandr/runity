//! Cooking a library for a platform: its textures encoded into the blocks
//! the platform's GPUs sample as they are — BC7 on a desktop's, ASTC 4x4 on
//! a phone's and an Apple one's — a quarter of RGBA8's bytes on disk, in
//! the download and in video memory.
//!
//! Done when a game is built for a platform (`scrap build --platform`,
//! `scrap cook`), not at import: the library stays RGBA8, so importing and
//! reloading a texture in the editor stays as quick as it was (DNA,
//! postulate 1), and the encoders — Intel's ISPC ones, native code — never
//! reach a game (postulate 7: importers are not linked into the runtime).
//! What was encoded is kept in a cache by the bytes it was encoded from,
//! so the next build encodes only what changed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use scrap::asset::{MeshAsset, TextureAsset, TextureCoding, TextureLevel};

/// What a platform's build samples its textures as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Windows, macOS and Linux: BC7.
    Desktop,
    /// iOS and Android: ASTC 4x4.
    Mobile,
}

impl Platform {
    pub fn coding(self) -> TextureCoding {
        match self {
            Platform::Desktop => TextureCoding::Bc7,
            Platform::Mobile => TextureCoding::Astc4x4,
        }
    }
}

/// `texture` in `coding`'s blocks, or `None` when it stays as it is: it is
/// in them already, or its size is not whole blocks (a GPU's block texture
/// must be).
pub fn cook_texture(texture: &TextureAsset, coding: TextureCoding) -> Option<TextureAsset> {
    let block = coding.block();
    if texture.coding != TextureCoding::Rgba8
        || coding == TextureCoding::Rgba8
        || texture.width % block != 0
        || texture.height % block != 0
        || texture.pixels.len() != texture.width as usize * texture.height as usize * 4
    {
        return None;
    }
    let alpha = texture.pixels.chunks_exact(4).any(|p| p[3] < 255);
    let srgb = texture.srgb;
    let encode = |w: u32, h: u32, rgba: &[u8]| encode_level(coding, alpha, srgb, w, h, rgba);
    Some(TextureAsset {
        id: texture.id,
        name: texture.name.clone(),
        width: texture.width,
        height: texture.height,
        pixels: encode(texture.width, texture.height, &texture.pixels),
        mips: texture
            .mips
            .iter()
            .map(|m| TextureLevel { width: m.width, height: m.height, pixels: encode(m.width, m.height, &m.pixels) })
            .collect(),
        srgb: texture.srgb,
        coding,
    })
}

/// One level's blocks: the level padded out to whole blocks by its last
/// row and column (a mip smaller than a block, or not a multiple of it),
/// then encoded.
fn encode_level(coding: TextureCoding, alpha: bool, srgb: bool, width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let (across, down) = coding.blocks(width, height);
    let (pw, ph) = (across * coding.block(), down * coding.block());
    let (w, h) = (width.max(1) as usize, height.max(1) as usize);
    let mut padded = vec![0u8; pw as usize * ph as usize * 4];
    for y in 0..ph as usize {
        let sy = y.min(h - 1);
        for x in 0..pw as usize {
            let sx = x.min(w - 1);
            let from = (sy * w + sx) * 4;
            let to = (y * pw as usize + x) * 4;
            if let Some(texel) = rgba.get(from..from + 4) {
                padded[to..to + 4].copy_from_slice(texel);
            }
        }
    }
    let surface = intel_tex_2::RgbaSurface { data: &padded, width: pw, height: ph, stride: pw * 4 };
    match coding {
        TextureCoding::Rgba8 => rgba.to_vec(),
        TextureCoding::Bc7 => {
            let settings = if alpha { intel_tex_2::bc7::alpha_basic_settings() } else { intel_tex_2::bc7::opaque_basic_settings() };
            intel_tex_2::bc7::compress_blocks(&settings, &surface)
        }
        TextureCoding::Astc4x4 => encode_astc(pw, ph, &padded, srgb).unwrap_or_else(|e| {
            eprintln!("an ASTC block did not encode ({e:?}): the level is left black");
            vec![0; coding.level_bytes(width, height)]
        }),
    }
}

/// A mesh's normals on a grid of a 4096th and its texture coordinates on
/// one of a 65536th (a sixteenth of a texel of a 4096 atlas), for zstd:
/// still f32 where they were, so nothing that reads a mesh changes, their
/// low bits zeros now, and a shuffled body of zeros compresses to little.
/// Not the positions: what collides is built from them, and a body resting
/// a millimetre lower is a game that plays out otherwise.
pub fn snap_mesh(mesh: &mut MeshAsset) {
    let (turn, texel) = (2f32.powi(-12), 2f32.powi(-16));
    let snap = |v: f32, step: f32| (v / step).round() * step;
    for v in &mut mesh.vertices {
        v.normal = v.normal.map(|n| snap(n, turn));
        v.uv = v.uv.map(|u| snap(u, texel));
    }
}

/// ASTC 4x4 blocks of an image of whole blocks, by ARM's encoder (Intel's
/// has none): sRGB colour maps encoded as sRGB, the rest as data. One
/// thread: the library's textures are encoded side by side already.
fn encode_astc(width: u32, height: u32, rgba: &[u8], srgb: bool) -> Result<Vec<u8>, astcenc_rs::Error> {
    let config = astcenc_rs::ConfigBuilder::new()
        .with_profile(if srgb { astcenc_rs::Profile::LdrSrgb } else { astcenc_rs::Profile::LdrRgba })
        .with_preset(astcenc_rs::PRESET_FAST)
        .with_block_size(astcenc_rs::Extents::new(4, 4))
        .build()?;
    let mut context = astcenc_rs::Context::with_threads(1, config)?;
    let image = astcenc_rs::Image { extents: astcenc_rs::Extents::new(width, height), data: vec![rgba] };
    context.compress(&image, astcenc_rs::Swizzle::rgba())
}

/// How hard zstd works on a build's assets: once, kept in the cache, for
/// every download after — near its best, short of its slowest levels.
const ZSTD_LEVEL: i32 = 17;

/// What cooking a library did.
#[derive(Debug, Default)]
pub struct Cooked {
    /// Textures encoded now.
    pub encoded: usize,
    /// Textures taken from the cache.
    pub cached: usize,
    /// Assets not re-encoded but compressed (meshes, textures that stay
    /// RGBA8), with zstd on.
    pub compressed: usize,
    /// Files copied as they were: prefabs, and assets without zstd.
    pub copied: usize,
    /// Bytes of the library, and of what it became.
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// Every file of `library` into `out`, its textures in `coding`'s blocks —
/// each encoded once and kept in `cache` by the bytes it came from — and,
/// with `zstd`, every asset's body compressed (a download's, a phone's
/// storage: [`scrap::asset::FLAG_ZSTD`], unpacked as it is read).
pub fn cook_library(library: &Path, out: &Path, coding: TextureCoding, zstd: bool, cache: &Path) -> Result<Cooked> {
    std::fs::create_dir_all(out).with_context(|| format!("making {}", out.display()))?;
    let mut files: Vec<PathBuf> = Vec::new();
    collect(library, &mut files)?;
    let cooked = scrap_core::jobs::map(&files, 1, |file| cook_file(library, file, out, coding, zstd, cache));
    let mut report = Cooked::default();
    for done in cooked {
        let (what, bytes_in, bytes_out) = done?;
        match what {
            Done::Encoded => report.encoded += 1,
            Done::Cached => report.cached += 1,
            Done::Compressed => report.compressed += 1,
            Done::Copied => report.copied += 1,
        }
        report.bytes_in += bytes_in;
        report.bytes_out += bytes_out;
    }
    Ok(report)
}

enum Done {
    Encoded,
    Cached,
    Compressed,
    Copied,
}

fn collect(dir: &Path, into: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, into)?;
        } else {
            into.push(path);
        }
    }
    Ok(())
}

fn cook_file(library: &Path, file: &Path, out: &Path, coding: TextureCoding, zstd: bool, cache: &Path) -> Result<(Done, u64, u64)> {
    let to = out.join(file.strip_prefix(library).unwrap_or(file));
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let asset = file.extension().is_some_and(|e| e == "scrasset") && scrap::asset::split_header(&bytes).is_ok();
    let texture = asset && scrap::asset::kind_of(&bytes).is_ok_and(|k| k == scrap::asset::TEXTURE);
    let mesh = asset && scrap::asset::kind_of(&bytes).is_ok_and(|k| k == scrap::asset::MESH);
    if !texture && !(asset && zstd) {
        std::fs::write(&to, &bytes)?;
        return Ok((Done::Copied, bytes.len() as u64, bytes.len() as u64));
    }
    // Kept by what it was made from, and how.
    let key = format!("{:032x}-{coding:?}{}.scrasset", fnv128(&bytes), if zstd { "-zstd5" } else { "" });
    let kept = cache.join(&key);
    if let Ok(done) = std::fs::read(&kept) {
        std::fs::write(&to, &done)?;
        return Ok((Done::Cached, bytes.len() as u64, done.len() as u64));
    }
    let (done, mut written) = if texture {
        let body = scrap::asset::split_header(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let asset: TextureAsset = rkyv::from_bytes::<TextureAsset, rkyv::rancor::Error>(body)
            .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        match cook_texture(&asset, coding) {
            Some(encoded) => {
                let header = &bytes[..bytes.len() - body.len()];
                let body = rkyv::to_bytes::<rkyv::rancor::Error>(&encoded).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
                (Done::Encoded, [header, body.as_slice()].concat())
            }
            None => (Done::Copied, bytes.clone()),
        }
    } else if mesh && zstd {
        let body = scrap::asset::split_header(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let mut asset: MeshAsset = rkyv::from_bytes::<MeshAsset, rkyv::rancor::Error>(body)
            .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        snap_mesh(&mut asset);
        // Its own header kept — the name it is found by is the importer's,
        // not always the mesh's — and a new body after it.
        let header = &bytes[..bytes.len() - body.len()];
        let body = rkyv::to_bytes::<rkyv::rancor::Error>(&asset).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        (Done::Compressed, [header, body.as_slice()].concat())
    } else {
        (Done::Compressed, bytes.clone())
    };
    if zstd {
        // Shuffled but where the body is GPU blocks, which it does not help.
        let blocks = matches!(done, Done::Encoded);
        written = scrap::asset::compressed(&written, !blocks, |body| zstd::bulk::compress(body, ZSTD_LEVEL).unwrap_or_default())?;
    }
    std::fs::create_dir_all(cache)?;
    std::fs::write(&kept, &written)?;
    std::fs::write(&to, &written)?;
    Ok((done, bytes.len() as u64, written.len() as u64))
}

/// FNV-1a over 128 bits: the importer's hash of a source, here of the
/// bytes a texture was encoded from.
fn fnv128(bytes: &[u8]) -> u128 {
    let mut hash: u128 = 0x6c62272e07bb014262b821756295c58d;
    for &b in bytes {
        hash ^= b as u128;
        hash = hash.wrapping_mul(0x0000000001000000000000000000013B);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(side: u32) -> TextureAsset {
        let pixels: Vec<u8> = (0..side * side)
            .flat_map(|i| {
                let on = ((i % side) / 4 + (i / side) / 4) % 2 == 0;
                if on { [230, 40, 30, 255] } else { [20, 60, 200, 255] }
            })
            .collect();
        let mips = (1..)
            .map(|l| (side >> l).max(1))
            .take_while(|_| true)
            .take(side.ilog2() as usize)
            .map(|s| TextureLevel { width: s, height: s, pixels: vec![128; (s * s * 4) as usize] })
            .collect();
        TextureAsset {
            id: scrap::asset::AssetId(7),
            name: "checker".into(),
            width: side,
            height: side,
            pixels,
            mips,
            srgb: true,
            coding: TextureCoding::Rgba8,
        }
    }

    #[test]
    fn a_texture_is_a_quarter_the_bytes_in_blocks_and_its_small_mips_are_whole_blocks() {
        let texture = checker(64);
        for coding in [TextureCoding::Bc7, TextureCoding::Astc4x4] {
            let cooked = cook_texture(&texture, coding).expect("64 is whole blocks");
            assert_eq!(cooked.coding, coding);
            assert_eq!(cooked.pixels.len() * 4, texture.pixels.len(), "{coding:?}: a quarter");
            for (mip, level) in cooked.mips.iter().zip(&texture.mips) {
                assert_eq!(mip.pixels.len(), coding.level_bytes(level.width, level.height), "{coding:?} {}", level.width);
            }
            // The 1x1 level is one block.
            assert_eq!(cooked.mips.last().unwrap().pixels.len(), 16);
        }
    }

    #[test]
    fn a_meshs_positions_stay_exactly_and_its_normals_and_texture_coordinates_move_a_hair() {
        use scrap::asset::{Bounds, Vertex};
        let vertices = vec![
            Vertex { position: [0.123456, 7.654321, -3.3], normal: [0.577, 0.5773, 0.57735], uv: [0.3141592, 12.718281] };
            3
        ];
        let mut mesh = MeshAsset {
            id: scrap::asset::AssetId(1),
            name: "m".into(),
            bounds: Bounds::of(&vertices),
            vertices: vertices.clone(),
            indices: vec![0, 1, 2],
            submeshes: Vec::new(),
            skin: None,
            colors: Vec::new(),
            look: None,
        };
        snap_mesh(&mut mesh);
        for (a, b) in mesh.vertices.iter().zip(&vertices) {
            assert_eq!(a.position, b.position, "what collides is built from them");
            for (x, y) in a.normal.iter().zip(b.normal) {
                assert!((x - y).abs() <= 2f32.powi(-13), "{x} {y}");
            }
            for (x, y) in a.uv.iter().zip(b.uv) {
                assert!((x - y).abs() <= 2f32.powi(-17), "{x} {y}");
                assert_eq!(x.to_bits() & 0xf, 0, "low bits zeros, for zstd");
            }
        }
    }

    #[test]
    fn a_texture_not_of_whole_blocks_or_cooked_already_stays() {
        let mut odd = checker(64);
        odd.width = 62;
        odd.pixels.truncate(62 * 64 * 4);
        assert!(cook_texture(&odd, TextureCoding::Bc7).is_none());
        let cooked = cook_texture(&checker(8), TextureCoding::Bc7).unwrap();
        assert!(cook_texture(&cooked, TextureCoding::Astc4x4).is_none());
    }
}
