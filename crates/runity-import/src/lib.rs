//! Turning source files into assets the engine can open.
//!
//! This crate is the only place that knows what an `.obj` is. The editor
//! calls it when something is dragged into the content browser, and the
//! command line calls it to rebuild a library from scratch; the game never
//! links it at all.
//!
//! Every import leaves two files behind:
//!
//! * `<name>.rasset` — the binary the runtime opens.
//! * `<name>.rimport` — the settings it was built with, and where it came
//!   from, in RON.
//!
//! The second one is the part that is easy to skip and expensive to add
//! later. Without it an import is a one-way trip: you can see that an asset
//! exists but not what produced it, so nothing can be rebuilt when the
//! importer improves, and a changed source file cannot be noticed. With it,
//! re-import is just "read the sidecar, run it again" — and because it is
//! text, a diff shows when someone changed a scale factor, which a binary
//! never would.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use runity::asset::{
    AssetId, AssetKind, Bounds, MeshAsset, Submesh, TextureAsset, TextureLevel, Vertex,
};
use serde::{Deserialize, Serialize};

/// What the importer was told to do, stored beside its output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportSettings {
    /// Where the source sat, relative to the project root. Kept so a
    /// re-import knows what to read, and so a missing source is a message
    /// rather than a mystery.
    pub source: String,
    /// Uniform scale applied on the way in. Kits disagree about units; the
    /// engine works in meters and the disagreement is settled here, once,
    /// rather than by a scale on every instance in every scene.
    pub scale: f32,
    /// Recompute normals from the faces instead of trusting the file's.
    /// Needed for sources that carry none, which is most hand-made OBJ.
    pub recompute_normals: bool,
    /// Whether an image holds colour, and so needs decoding from sRGB on
    /// the way to the GPU. True for an albedo map; false for a normal map, a
    /// roughness map or a mask, where decoding bends every value.
    #[serde(default = "yes")]
    pub srgb: bool,
    /// Move the mesh so its base sits at y = 0. A tree whose origin is in the
    /// middle of its trunk has to be placed by feel; one whose origin is at
    /// its foot can be dropped on the ground.
    pub origin_to_base: bool,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            source: String::new(),
            scale: 1.0,
            recompute_normals: false,
            srgb: true,
            origin_to_base: true,
        }
    }
}

impl ImportSettings {
    pub fn for_source(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            ..Self::default()
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("{}", path.as_ref().display()))?;
        Ok(ron::from_str(&text)?)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let pretty = ron::ser::PrettyConfig::new().struct_names(true);
        std::fs::write(path.as_ref(), ron::ser::to_string_pretty(self, pretty)?)?;
        Ok(())
    }
}

fn yes() -> bool {
    true
}

/// Where an import put its two files.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub id: AssetId,
    pub asset: PathBuf,
    pub sidecar: PathBuf,
}

/// Read an OBJ and build a mesh asset from it.
///
/// Vertices are de-duplicated on the way through: OBJ addresses position,
/// normal and UV with separate indices, and the GPU wants one index per
/// vertex, so every distinct triple becomes one vertex exactly once.
pub fn mesh_from_obj(path: impl AsRef<Path>, settings: &ImportSettings) -> Result<MeshAsset> {
    let path = path.as_ref();
    let (models, _materials) = tobj::load_obj(
        path,
        &tobj::LoadOptions {
            triangulate: true,
            single_index: false,
            ..Default::default()
        },
    )
    .with_context(|| format!("{}", path.display()))?;

    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut submeshes: Vec<Submesh> = Vec::new();
    // The key is the OBJ triple, which is what makes two references the same
    // vertex. Hashing floats would be wrong here and is not needed.
    let mut seen: std::collections::HashMap<(u32, u32, u32), u32> =
        std::collections::HashMap::new();

    for model in &models {
        let m = &model.mesh;
        let first_index = indices.len() as u32;
        for (face, &position_index) in m.indices.iter().enumerate() {
            let normal_index = m.normal_indices.get(face).copied().unwrap_or(u32::MAX);
            let uv_index = m.texcoord_indices.get(face).copied().unwrap_or(u32::MAX);
            let key = (position_index, normal_index, uv_index);
            let index = *seen.entry(key).or_insert_with(|| {
                let p = position_index as usize * 3;
                let position = [
                    m.positions[p] * settings.scale,
                    m.positions[p + 1] * settings.scale,
                    m.positions[p + 2] * settings.scale,
                ];
                let normal = if normal_index == u32::MAX || m.normals.is_empty() {
                    [0.0, 0.0, 0.0]
                } else {
                    let n = normal_index as usize * 3;
                    [m.normals[n], m.normals[n + 1], m.normals[n + 2]]
                };
                let uv = if uv_index == u32::MAX || m.texcoords.is_empty() {
                    [0.0, 0.0]
                } else {
                    let t = uv_index as usize * 2;
                    // OBJ's V axis points up, the GPU's points down.
                    [m.texcoords[t], 1.0 - m.texcoords[t + 1]]
                };
                vertices.push(Vertex {
                    position,
                    normal,
                    uv,
                });
                (vertices.len() - 1) as u32
            });
            indices.push(index);
        }
        submeshes.push(Submesh {
            first_index,
            index_count: indices.len() as u32 - first_index,
            material: None,
        });
    }

    let missing_normals = vertices.iter().all(|v| v.normal == [0.0, 0.0, 0.0]);
    if settings.recompute_normals || missing_normals {
        recompute_normals(&mut vertices, &indices);
    }
    if settings.origin_to_base {
        let base = Bounds::of(&vertices).min[1];
        for v in &mut vertices {
            v.position[1] -= base;
        }
    }

    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mesh".into());

    Ok(MeshAsset {
        id: AssetId::from_source(&settings.source, 0),
        name,
        bounds: Bounds::of(&vertices),
        vertices,
        indices,
        submeshes,
    })
}

/// Area-weighted vertex normals: each face adds its cross product, which is
/// already proportional to twice its area, so large faces pull harder than
/// slivers. Normalizing per face first would give a sliver the same vote as
/// the triangle next to it.
fn recompute_normals(vertices: &mut [Vertex], indices: &[u32]) {
    for v in vertices.iter_mut() {
        v.normal = [0.0; 3];
    }
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let (pa, pb, pc) = (
            vertices[a].position,
            vertices[b].position,
            vertices[c].position,
        );
        let u = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let v = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        for index in [a, b, c] {
            let normal = &mut vertices[index].normal;
            for (axis, component) in n.iter().enumerate() {
                normal[axis] += component;
            }
        }
    }
    for v in vertices.iter_mut() {
        let n = v.normal;
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        // A vertex used by no triangle, or by degenerate ones only, keeps a
        // usable normal rather than a NaN that would blacken every pixel it
        // touched.
        v.normal = if length > 1e-12 {
            [n[0] / length, n[1] / length, n[2] / length]
        } else {
            [0.0, 1.0, 0.0]
        };
    }
}

/// Read an image and build a texture asset from it.
///
/// Everything becomes RGBA8, including greyscale and palette images, because
/// one layout on the GPU is worth more than the bytes a narrower one saves —
/// and the place to save those bytes is block compression at import, not a
/// second code path at load.
pub fn texture_from_image(
    path: impl AsRef<Path>,
    settings: &ImportSettings,
) -> Result<TextureAsset> {
    let path = path.as_ref();
    let image = image::open(path)
        .with_context(|| format!("{}", path.display()))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let pixels = image.into_raw();
    let mips = build_mips(width, height, &pixels, settings.srgb);
    Ok(TextureAsset {
        id: AssetId::from_source(&settings.source, 0),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "texture".into()),
        width,
        height,
        pixels,
        mips,
        srgb: settings.srgb,
    })
}

/// Halve an image repeatedly, down to one pixel.
///
/// A colour map is averaged in linear space, not in sRGB. Averaging the
/// encoded bytes darkens every mip — the values are not proportional to
/// light — and the result is a texture that dims as it recedes.
fn build_mips(width: u32, height: u32, pixels: &[u8], srgb: bool) -> Vec<TextureLevel> {
    let to_linear = |b: u8| -> f32 {
        let c = b as f32 / 255.0;
        if !srgb {
            c
        } else if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let from_linear = |v: f32| -> u8 {
        let c = if !srgb {
            v
        } else if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };

    let mut levels = Vec::new();
    let (mut w, mut h) = (width, height);
    let mut source = pixels.to_vec();
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for channel in 0..4 {
                    // Alpha stays linear whatever the colour channels do:
                    // coverage is not light and encoding it would be wrong.
                    let mut sum = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let i = ((sy * w + sx) * 4 + channel) as usize;
                        sum += if channel == 3 {
                            source[i] as f32 / 255.0
                        } else {
                            to_linear(source[i])
                        };
                    }
                    let average = sum / 4.0;
                    next[((y * nw + x) * 4 + channel) as usize] = if channel == 3 {
                        (average.clamp(0.0, 1.0) * 255.0).round() as u8
                    } else {
                        from_linear(average)
                    };
                }
            }
        }
        levels.push(TextureLevel {
            width: nw,
            height: nh,
            pixels: next.clone(),
        });
        source = next;
        w = nw;
        h = nh;
    }
    levels
}

/// Import one source file into `library`, writing both the asset and its
/// sidecar.
pub fn import_file(
    source: impl AsRef<Path>,
    library: impl AsRef<Path>,
    settings: ImportSettings,
) -> Result<Imported> {
    let source = source.as_ref();
    let library = library.as_ref();
    std::fs::create_dir_all(library)?;

    let extension = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let (bytes, id, kind) = match extension.as_str() {
        "obj" => {
            let mesh = mesh_from_obj(source, &settings)?;
            (
                runity::asset::to_bytes(&mesh, AssetKind::Mesh)?,
                mesh.id,
                AssetKind::Mesh,
            )
        }
        "png" | "jpg" | "jpeg" | "tga" | "bmp" => {
            let texture = texture_from_image(source, &settings)?;
            (
                runity::asset::to_bytes(&texture, AssetKind::Texture)?,
                texture.id,
                AssetKind::Texture,
            )
        }
        other => anyhow::bail!("no importer for .{other} yet"),
    };

    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "asset".into());
    let asset_path = library.join(format!("{stem}.rasset"));
    let sidecar_path = library.join(format!("{stem}.rimport"));

    std::fs::write(&asset_path, bytes)?;
    settings.save(&sidecar_path)?;
    let _ = kind;

    Ok(Imported {
        id,
        asset: asset_path,
        sidecar: sidecar_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit square two meters up, two triangles, no normals — which is what
    /// most hand-written OBJ looks like.
    ///
    /// Wound counter-clockwise seen from above, which in a right-handed
    /// Y-up system is what makes a floor face the sky. Getting this backwards
    /// is the usual way a model ends up lit from underneath.
    const SQUARE: &str = "\
v -1.0 2.0 -1.0
v  1.0 2.0 -1.0
v  1.0 2.0  1.0
v -1.0 2.0  1.0
f 1 3 2
f 1 4 3
";

    fn write_square(dir: &Path) -> PathBuf {
        let path = dir.join("square.obj");
        std::fs::write(&path, SQUARE).unwrap();
        path
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("runity-import-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_obj_without_normals_gets_them_computed() {
        let dir = temp("normals");
        let settings = ImportSettings::for_source("square.obj");
        let mesh = mesh_from_obj(write_square(&dir), &settings).unwrap();
        assert_eq!(mesh.vertices.len(), 4, "the shared edge is not duplicated");
        assert_eq!(mesh.indices.len(), 6);
        for v in &mesh.vertices {
            assert!(
                (v.normal[1] - 1.0).abs() < 1e-5,
                "a floor faces up, got {:?}",
                v.normal
            );
        }
    }

    #[test]
    fn origin_to_base_puts_the_mesh_on_the_ground() {
        let dir = temp("base");
        let settings = ImportSettings::for_source("square.obj");
        let mesh = mesh_from_obj(write_square(&dir), &settings).unwrap();
        assert_eq!(mesh.bounds.min[1], 0.0, "y = 2 in the file, y = 0 in game");
    }

    #[test]
    fn scale_is_settled_at_import_not_per_instance() {
        let dir = temp("scale");
        let settings = ImportSettings {
            scale: 0.5,
            origin_to_base: false,
            ..ImportSettings::for_source("square.obj")
        };
        let mesh = mesh_from_obj(write_square(&dir), &settings).unwrap();
        assert_eq!(mesh.bounds.max[0], 0.5);
        assert_eq!(mesh.bounds.min[1], 1.0);
    }

    #[test]
    fn an_import_leaves_an_asset_and_a_recipe_for_rebuilding_it() {
        let dir = temp("roundtrip");
        let source = write_square(&dir);
        let library = dir.join("library");
        let settings = ImportSettings::for_source("square.obj");
        let out = import_file(&source, &library, settings.clone()).unwrap();

        // The runtime opens the asset without linking this crate.
        let bytes = runity::asset::read(&out.asset).unwrap();
        let archived = runity::asset::view::<MeshAsset>(&bytes).unwrap();
        assert_eq!(archived.vertices.len(), 4);
        assert_eq!(archived.id, out.id);

        // And the sidecar says exactly how to build it again.
        assert_eq!(ImportSettings::load(&out.sidecar).unwrap(), settings);
    }

    #[test]
    fn a_png_imports_as_a_texture_and_keeps_its_pixels() {
        let dir = temp("texture");
        // Two by two, one red pixel, so a flipped row or a swapped channel
        // shows up as a different number rather than as nothing.
        let path = dir.join("swatch.png");
        let mut image = image::RgbaImage::new(2, 2);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        image.save(&path).unwrap();

        let out = import_file(
            &path,
            dir.join("library"),
            ImportSettings::for_source("swatch.png"),
        )
        .unwrap();
        let bytes = runity::asset::read(&out.asset).unwrap();
        assert_eq!(
            runity::asset::kind_of(&bytes).unwrap(),
            runity::asset::AssetKind::Texture,
            "the header says what it is, so a library never casts one for the other"
        );
        let texture = runity::asset::view::<runity::asset::TextureAsset>(&bytes).unwrap();
        assert_eq!(texture.width.to_native(), 2);
        assert_eq!(texture.pixels.len(), 16, "two by two, four bytes each");
        assert_eq!(texture.pixels[0], 255, "the red pixel is first");
        assert!(texture.srgb, "a colour map is sRGB unless told otherwise");
    }

    #[test]
    fn a_texture_arrives_with_a_full_mip_chain() {
        let dir = temp("mips");
        let path = dir.join("checker.png");
        let mut image = image::RgbaImage::new(8, 8);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let on = (x + y) % 2 == 0;
            *pixel = image::Rgba(if on { [255; 4] } else { [0, 0, 0, 255] });
        }
        image.save(&path).unwrap();

        let settings = ImportSettings::for_source("checker.png");
        let texture = texture_from_image(&path, &settings).unwrap();
        // 8, 4, 2, 1 — three levels below the base.
        assert_eq!(texture.mips.len(), 3);
        assert_eq!((texture.mips[0].width, texture.mips[0].height), (4, 4));
        assert_eq!(
            texture.mips[2].pixels.len(),
            4,
            "the last level is one pixel"
        );

        // A checkerboard of black and white averages to mid grey. In sRGB
        // that is 188, not 128: averaging the encoded bytes instead would
        // give the darker number, and every texture would dim as it
        // receded.
        let grey = texture.mips[0].pixels[0];
        assert!(
            (grey as i32 - 188).abs() <= 2,
            "mips must be averaged in linear space, got {grey}"
        );
    }

    #[test]
    fn a_mask_can_be_imported_without_being_treated_as_colour() {
        let dir = temp("linear");
        let path = dir.join("mask.png");
        image::RgbaImage::new(1, 1).save(&path).unwrap();
        let settings = ImportSettings {
            srgb: false,
            ..ImportSettings::for_source("mask.png")
        };
        let out = import_file(&path, dir.join("library"), settings).unwrap();
        let bytes = runity::asset::read(&out.asset).unwrap();
        let texture = runity::asset::view::<runity::asset::TextureAsset>(&bytes).unwrap();
        assert!(!texture.srgb);
    }

    #[test]
    fn an_unknown_extension_says_so_instead_of_writing_an_empty_asset() {
        let dir = temp("unknown");
        let source = dir.join("thing.blend");
        std::fs::write(&source, b"not really").unwrap();
        let err = import_file(&source, dir.join("library"), ImportSettings::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("blend"), "{err}");
    }
}
