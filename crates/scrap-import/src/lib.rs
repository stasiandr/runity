//! Turning source files into assets the engine can open.
//!
//! This crate is the only place that knows what an `.obj` is. The editor
//! calls it when something is dragged into the content browser, and the
//! command line calls it to rebuild a library from scratch; the game never
//! links it at all.
//!
//! Every import leaves two files behind:
//!
//! * `<source>.scrasset` in the library — the binary the runtime opens.
//!   Derived: the library is never committed, and a clone rebuilds it.
//! * `<source>.scrimport` beside the source — the settings it was built with,
//!   where the source is, a hash of what the source held, and the asset's
//!   ID, in RON. Authored: committed, and diffed.
//!
//! The sidecar is the part that is easy to skip and expensive to add later.
//! Without it an import is a one-way trip: you can see that an asset exists
//! but not what produced it, so nothing can be rebuilt when the importer
//! improves, and a changed source file cannot be noticed. With it, re-import
//! is "read the sidecar, run it again" — and because it is text, a diff
//! shows when someone changed a scale factor, which a binary never would.
//!
//! It sits beside its source rather than in the library because the library
//! is derived and the settings are not (DNA, postulate 2): a sidecar in a
//! folder nobody commits is settings nobody keeps.

pub mod assets;
pub mod blend;
pub mod brush;
pub mod cook;
pub mod poly;
pub mod scene;
pub mod terrain;
pub mod unity;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use scrap::animation::{Channel, Clip, Joint, Path as AnimPath, PoseTransform, Skeleton};
use scrap::asset::{
    AssetId, Bounds, MaterialAsset, MeshAsset, MeshSkin, SoundAsset, Submesh,
    TextureAsset, TextureLevel, Vertex,
};
use scrap::material::{Material, Shading};
use serde::{Deserialize, Serialize};

/// What the importer was told to do: the `.scrimport` beside a source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportSettings {
    /// Where the source is: relative to the project, with forward slashes,
    /// or an absolute path for a source outside it. Kept so a re-import
    /// knows what to read, and so a missing source is a message rather than
    /// a mystery.
    pub source: String,
    /// A hash of what the source held when it was last imported.
    ///
    /// What decides whether a source changed — not its modification time,
    /// which a checkout, a copy or a sync tool moves without changing a
    /// byte (DNA, "Принятые решения"). And what finds a source that moved:
    /// the file that turned up with the same contents is the same file.
    #[serde(default)]
    pub hash: String,
    /// The asset's identity, minted on the first import and kept after.
    ///
    /// Stored rather than derived from the path, so moving a source does not
    /// turn its asset into a different one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<AssetId>,
    /// Uniform scale applied on the way in. Kits disagree about units; the
    /// engine works in meters and the disagreement is settled here, once,
    /// rather than by a scale on every instance in every scene.
    #[serde(default = "one")]
    pub scale: f32,
    /// Recompute normals from the faces instead of trusting the file's.
    /// Needed for sources that carry none, which is most hand-made OBJ.
    #[serde(default)]
    pub recompute_normals: bool,
    /// Whether an image holds colour, and so needs decoding from sRGB on
    /// the way to the GPU. True for an albedo map; false for a normal map, a
    /// roughness map or a mask, where decoding bends every value.
    #[serde(default = "yes")]
    pub srgb: bool,
    /// Move the mesh so its base sits at y = 0. A tree whose origin is in the
    /// middle of its trunk has to be placed by feel; one whose origin is at
    /// its foot can be dropped on the ground.
    #[serde(default = "yes")]
    pub origin_to_base: bool,
    /// Keep a glTF's own UVs even where its materials are colours only.
    /// Otherwise those colours become the model's look — a palette its UVs
    /// are moved onto — which is right for a kit whose model is all there
    /// is, and wrong for one a scene always dresses in materials of its own
    /// (a Unity project's: an atlas read by the UVs the palette would
    /// overwrite).
    #[serde(default, skip_serializing_if = "is_false")]
    pub keep_uvs: bool,
    /// A glTF that is a scene — several objects — rather than one model:
    /// imported as models, materials and a prefab (see [`scene`]). Set on
    /// the first import from what the file holds, and kept.
    #[serde(default, skip_serializing_if = "is_false")]
    pub scene: bool,
    /// For a scene: the ID of every asset it builds, by what the file calls
    /// it (`mesh:rock.0`, `material:moss`), so a re-import updates them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parts: BTreeMap<String, AssetId>,
    /// For a scene: its materials the project draws with one of its own
    /// instead — `{ "moss": "moss_wet" }` — everywhere this file uses them
    /// (docs/blender.md). A material with a custom look in the game and
    /// Blender's in Blender.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub materials: BTreeMap<String, String>,
    /// An image whose shape is in one of its colours, not its alpha: that
    /// colour copied into the alpha on the way in. A grass card whose
    /// blade is its green is cut by it everywhere the alpha cuts — the
    /// shadow pass too, which runs no material's own shader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_from: Option<ColourChannel>,
}

/// One of an image's colour channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColourChannel {
    Red,
    Green,
    Blue,
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            source: String::new(),
            hash: String::new(),
            id: None,
            scale: 1.0,
            recompute_normals: false,
            srgb: true,
            origin_to_base: true,
            keep_uvs: false,
            scene: false,
            parts: BTreeMap::new(),
            materials: BTreeMap::new(),
            alpha_from: None,
        }
    }
}

impl ImportSettings {
    /// The defaults for a source. An image named as a normal map or a mask
    /// (`bark_normal`, `bark_n`, `bark_mask`) is data, not colour, and is
    /// read linear from the start — the convention every kit follows.
    pub fn for_source(source: impl Into<String>) -> Self {
        let source = source.into();
        let stem = Path::new(&source)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let data = ["_normal", "_n", "_mask", "_nrm"]
            .iter()
            .any(|end| stem.ends_with(end));
        Self {
            source,
            srgb: !data,
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
        std::fs::write(
            path.as_ref(),
            ron::ser::to_string_pretty(self, pretty)? + "\n",
        )?;
        Ok(())
    }

    /// The asset's ID: the one stored, or on a first import one derived
    /// from where the source is — deterministic, so importing the same file
    /// twice in two fresh clones gives the same ID.
    pub fn asset_id(&self) -> AssetId {
        self.id
            .unwrap_or_else(|| AssetId::from_source(&self.source, 0))
    }
}

/// Where a source's sidecar goes: beside it, as `<file>.scrimport`.
pub fn sidecar_for(source: &Path) -> PathBuf {
    let mut name = source.as_os_str().to_owned();
    name.push(".scrimport");
    PathBuf::from(name)
}

/// Where an asset is built in a library: `<id>.scrasset`.
///
/// By the asset's ID, not its source's name (docs/refs.md): two sources
/// with one name in two folders build to two assets, and a source that is
/// renamed or moved keeps its built file. The library finds assets by the
/// ID and the name inside them, not by this.
pub fn asset_for(id: AssetId, library: &Path) -> PathBuf {
    library.join(format!("{id}.scrasset"))
}

/// Whether what an import built was written in an older asset format: its
/// `.scrasset`, or for a scene — whose own output is a prefab, with no asset
/// header — every asset it built.
fn outdated(settings: &ImportSettings, output: &Path, library: &Path) -> bool {
    if settings.scene {
        settings
            .parts
            .values()
            .map(|id| asset_for(*id, library))
            .any(|part| part.is_file() && !scrap::asset::is_current(&part))
    } else {
        !scrap::asset::is_current(output)
    }
}

/// Where a source's asset is built, by the ID its sidecar holds: `None`
/// for a source with no sidecar yet.
pub fn built_for(source: &Path, library: &Path) -> Option<PathBuf> {
    let settings = ImportSettings::load(sidecar_for(source)).ok()?;
    Some(output_for(&settings, library))
}

/// The file an import with these settings builds: its `.scrasset`, or for a
/// scene its prefab.
pub fn output_for(settings: &ImportSettings, library: &Path) -> PathBuf {
    if settings.scene {
        scene::prefab_for(settings.asset_id(), library)
    } else {
        asset_for(settings.asset_id(), library)
    }
}

/// Whether a library file is named the way [`asset_for`] names one. A
/// library built before assets were named by ID has files named after
/// their sources; they are removed and built again.
fn named_by_id(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A hash of a file's contents, as the hex a sidecar stores.
///
/// FNV-1a, 128 bits: specified, so every machine and every build computes
/// the same value for the same bytes — which a committed hash needs and
/// the standard library's hasher does not promise. Not cryptographic, and
/// it does not need to be: nobody is forging textures.
///
/// A source built from other files — a terrain from its heightmap — hashes
/// them too, so changing one is changing the source.
pub fn content_hash(path: &Path) -> std::io::Result<String> {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut files = vec![path.to_path_buf()];
    files.extend(dependencies(path));
    for file in files {
        for byte in std::fs::read(&file)? {
            hash ^= byte as u128;
            hash = hash.wrapping_mul(PRIME);
        }
    }
    Ok(format!("{hash:032x}"))
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
        id: settings.asset_id(),
        name,
        bounds: Bounds::of(&vertices),
        vertices,
        indices,
        submeshes,
        // OBJ has no concept of a skeleton.
        skin: None,
        colors: Vec::new(),
        look: None,
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

/// Read a glTF file and build one mesh asset from everything in it.
///
/// glTF is the format real tools export, so this is the path art actually
/// arrives by; OBJ stays because the kits this repository ships are OBJ.
///
/// Every primitive in every mesh becomes a submesh of one asset. Splitting a
/// file into several assets is the other reasonable choice and a worse one
/// here: a model exported as a dozen parts is still one thing to place, and
/// an importer that scatters it makes a scene author reassemble what the
/// artist already assembled.
pub fn mesh_from_gltf(path: impl AsRef<Path>, settings: &ImportSettings) -> Result<MeshAsset> {
    let path = path.as_ref();
    let (document, buffers, images) =
        gltf::import(path).with_context(|| format!("{}", path.display()))?;
    // Which vertices each primitive made, and its material: for the look.
    let mut painted: Vec<(std::ops::Range<usize>, Option<usize>)> = Vec::new();

    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut submeshes: Vec<Submesh> = Vec::new();
    let mut joint_indices: Vec<[u16; 4]> = Vec::new();
    let mut joint_weights: Vec<[f32; 4]> = Vec::new();
    // Each vertex's painted colour, white where its primitive has none.
    let mut colors: Vec<[u8; 4]> = Vec::new();

    // Walked through the scene graph rather than over `document.meshes()`,
    // because a node carries the transform that places its mesh. Reading the
    // meshes directly gives every part the same origin, and a model built
    // from placed parts collapses into a heap.
    let mut stack: Vec<(gltf::Node, glam::Mat4)> = document
        .default_scene()
        .map(|scene| scene.nodes().map(|n| (n, glam::Mat4::IDENTITY)).collect())
        .unwrap_or_else(|| {
            document
                .nodes()
                .map(|n| (n, glam::Mat4::IDENTITY))
                .collect()
        });

    // Where a skinned mesh's vertices are, at rest: glTF ignores the
    // transform of the node that carries a skinned mesh — its joints place
    // it — so its vertices go where its joints at rest and their inverse
    // binds put them, not under that node (which, from Blender, is often
    // the Armature at a hundredth scale).
    let bind = bind_space(&document, &buffers);
    while let Some((node, parent)) = stack.pop() {
        let local = glam::Mat4::from_cols_array_2d(&node.transform().matrix());
        let world = parent * local;
        let placed = match (node.skin().is_some(), bind) {
            (true, Some(bind)) => bind,
            _ => world,
        };
        let normal_matrix = glam::Mat3::from_mat4(placed).inverse().transpose();

        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    // Strips and fans are legal glTF and vanishingly rare
                    // from real exporters. Skipped loudly rather than
                    // misread as triangles.
                    anyhow::bail!(
                        "{}: primitive mode {:?} is not triangles",
                        path.display(),
                        primitive.mode()
                    );
                }
                let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
                let positions: Vec<[f32; 3]> = reader
                    .read_positions()
                    .ok_or_else(|| {
                        anyhow::anyhow!("{}: a primitive has no positions", path.display())
                    })?
                    .collect();
                let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
                let uvs: Option<Vec<[f32; 2]>> =
                    reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());

                let skin_joints: Option<Vec<[u16; 4]>> =
                    reader.read_joints(0).map(|j| j.into_u16().collect());
                let skin_weights: Option<Vec<[f32; 4]>> =
                    reader.read_weights(0).map(|w| w.into_f32().collect());
                // COLOR_0 as the file has it, in bytes as Unity keeps a
                // mesh's colours (rounded, as it rounds them): no curve
                // either way.
                let painted_colors: Option<Vec<[u8; 4]>> = reader.read_colors(0).map(|c| {
                    c.into_rgba_f32()
                        .map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
                        .collect()
                });

                let base = vertices.len() as u32;
                for (i, position) in positions.iter().enumerate() {
                    joint_indices.push(
                        skin_joints
                            .as_ref()
                            .and_then(|j| j.get(i))
                            .copied()
                            .unwrap_or([0; 4]),
                    );
                    joint_weights.push(
                        skin_weights
                            .as_ref()
                            .and_then(|w| w.get(i))
                            .copied()
                            // A vertex with no weights would collapse to the
                            // origin once skinning multiplies it by nothing.
                            // Bound entirely to joint zero leaves it where it
                            // is.
                            .unwrap_or([1.0, 0.0, 0.0, 0.0]),
                    );
                    colors.push(
                        painted_colors
                            .as_ref()
                            .and_then(|c| c.get(i))
                            .copied()
                            .unwrap_or([255; 4]),
                    );
                    let p =
                        placed.transform_point3(glam::Vec3::from_array(*position)) * settings.scale;
                    let n = normals
                        .as_ref()
                        .and_then(|n| n.get(i))
                        .map(|n| (normal_matrix * glam::Vec3::from_array(*n)).normalize_or_zero())
                        .unwrap_or(glam::Vec3::ZERO);
                    vertices.push(Vertex {
                        position: p.to_array(),
                        normal: n.to_array(),
                        uv: uvs
                            .as_ref()
                            .and_then(|uv| uv.get(i))
                            .copied()
                            .unwrap_or([0.0, 0.0]),
                    });
                }

                let first_index = indices.len() as u32;
                match reader.read_indices() {
                    Some(read) => indices.extend(read.into_u32().map(|i| i + base)),
                    // An unindexed primitive is a plain vertex list.
                    None => indices.extend(base..base + positions.len() as u32),
                }
                submeshes.push(Submesh {
                    first_index,
                    index_count: indices.len() as u32 - first_index,
                    material: None,
                });
                painted.push((base as usize..vertices.len(), primitive.material().index()));
            }
        }

        for child in node.children() {
            stack.push((child, world));
        }
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

    // A mesh painted all white — or not painted — keeps no colours: it
    // reads as white all the same, and costs nothing.
    if colors.iter().all(|c| *c == [255; 4]) {
        colors.clear();
    }
    let skin = read_skin(&document, &buffers, joint_indices, joint_weights, bind);
    let look = if settings.keep_uvs {
        None
    } else {
        gltf_look(&document, &images, &mut vertices, &painted, path)
    };

    Ok(MeshAsset {
        id: settings.asset_id(),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "mesh".into()),
        bounds: Bounds::of(&vertices),
        vertices,
        indices,
        submeshes,
        skin,
        colors,
        look,
    })
}

/// Every node's place in the file's world: its ancestors' transforms and
/// its own.
fn node_globals(document: &gltf::Document) -> Vec<glam::Mat4> {
    let mut parent_of = vec![None; document.nodes().len()];
    for node in document.nodes() {
        for child in node.children() {
            parent_of[child.index()] = Some(node.index());
        }
    }
    let locals: Vec<glam::Mat4> = document
        .nodes()
        .map(|n| glam::Mat4::from_cols_array_2d(&n.transform().matrix()))
        .collect();
    (0..locals.len())
        .map(|i| {
            let mut m = locals[i];
            let mut at = parent_of[i];
            // A file whose parents loop is broken; stop rather than spin.
            let mut steps = 0;
            while let (Some(p), true) = (at, steps < locals.len()) {
                m = locals[p] * m;
                at = parent_of[p];
                steps += 1;
            }
            m
        })
        .collect()
}

/// Where the first skin's mesh stands at rest, in the file's world: its
/// first joint's place times that joint's inverse bind — the same for
/// every joint of a well-made file.
fn bind_space(document: &gltf::Document, buffers: &[gltf::buffer::Data]) -> Option<glam::Mat4> {
    let skin = document.skins().next()?;
    let first = skin.joints().next()?;
    let inverse = skin
        .reader(|buffer| Some(&buffers[buffer.index()]))
        .read_inverse_bind_matrices()
        .and_then(|mut m| m.next())
        .map_or(glam::Mat4::IDENTITY, |m| glam::Mat4::from_cols_array_2d(&m));
    Some(node_globals(document)[first.index()] * inverse)
}

/// The skeleton and animations, if the file has any.
fn read_skin(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
    bind: Option<glam::Mat4>,
) -> Option<MeshSkin> {
    let gltf_skin = document.skins().next()?;
    let reader = gltf_skin.reader(|buffer| Some(&buffers[buffer.index()]));
    // The vertices were put in the bind space (see `bind_space`): each
    // inverse bind takes them out of it first, so at rest a skin moves
    // nothing.
    let unplace = bind.map_or(glam::Mat4::IDENTITY, |b| b.inverse());
    let inverse_binds: Vec<[[f32; 4]; 4]> = reader
        .read_inverse_bind_matrices()
        .map(|m| m.map(|m| (glam::Mat4::from_cols_array_2d(&m) * unplace).to_cols_array_2d()).collect())
        .unwrap_or_default();

    // A joint's parent is whichever node in the skin lists it as a child.
    let node_indices: Vec<usize> = gltf_skin.joints().map(|j| j.index()).collect();
    let slot_of = |node: usize| {
        node_indices
            .iter()
            .position(|n| *n == node)
            .map(|i| i as u16)
    };
    let mut parents: Vec<Option<u16>> = vec![None; node_indices.len()];
    for (slot, node) in gltf_skin.joints().enumerate() {
        for child in node.children() {
            if let Some(child_slot) = slot_of(child.index()) {
                parents[child_slot as usize] = Some(slot as u16);
            }
        }
    }

    let mut skeleton = Skeleton {
        joints: gltf_skin
            .joints()
            .enumerate()
            .map(|(slot, node)| {
                let (translation, rotation, scale) = node.transform().decomposed();
                Joint {
                    name: node.name().unwrap_or("joint").to_string(),
                    parent: parents[slot],
                    inverse_bind: inverse_binds
                        .get(slot)
                        .copied()
                        // The default is identity, which glTF allows and
                        // which means the bind pose is the model's own.
                        .unwrap_or_else(|| glam::Mat4::IDENTITY.to_cols_array_2d()),
                    rest: PoseTransform {
                        translation,
                        rotation,
                        scale,
                    },
                }
            })
            .collect(),
    };

    // What the skeleton hangs under without being part of it — Blender's
    // `Armature` at a hundredth scale and a quarter turn, the Unity
    // import's turn — is a joint too, the root's parent, which no clip
    // moves: else a skeleton in centimetres is posed a hundred times too
    // big. Put last, so no weight's joint index moves.
    let mut parent_of = vec![None; document.nodes().len()];
    for node in document.nodes() {
        for child in node.children() {
            parent_of[child.index()] = Some(node.index());
        }
    }
    let above = |node: usize| {
        let mut matrix = glam::Mat4::IDENTITY;
        let mut at = parent_of[node];
        while let Some(n) = at {
            if node_indices.contains(&n) {
                return None;
            }
            let local = document.nodes().nth(n).map(|n| n.transform().matrix()).unwrap_or(glam::Mat4::IDENTITY.to_cols_array_2d());
            matrix = glam::Mat4::from_cols_array_2d(&local) * matrix;
            at = parent_of[n];
        }
        Some(matrix)
    };
    let roots: Vec<usize> = (0..skeleton.joints.len()).filter(|i| skeleton.joints[*i].parent.is_none()).collect();
    if let Some(carrier) = roots.first().and_then(|r| above(node_indices[*r])) {
        if !carrier.abs_diff_eq(glam::Mat4::IDENTITY, 1e-6) && skeleton.joints.len() < u16::MAX as usize {
            let slot = skeleton.joints.len() as u16;
            let (scale, rotation, translation) = carrier.to_scale_rotation_translation();
            skeleton.joints.push(Joint {
                name: "(skeleton root)".into(),
                parent: None,
                // Nothing is weighted to it; bound where it stands.
                inverse_bind: carrier.inverse().to_cols_array_2d(),
                rest: PoseTransform {
                    translation: translation.to_array(),
                    rotation: rotation.to_array(),
                    scale: scale.to_array(),
                },
            });
            for r in roots {
                skeleton.joints[r].parent = Some(slot);
            }
        }
    }

    let clips = document
        .animations()
        .map(|animation| read_clip(&animation, buffers, &node_indices))
        .collect();

    Some(MeshSkin {
        joints,
        weights,
        skeleton,
        clips,
    })
}

fn read_clip(
    animation: &gltf::Animation,
    buffers: &[gltf::buffer::Data],
    node_indices: &[usize],
) -> Clip {
    let mut channels = Vec::new();
    let mut duration: f32 = 0.0;

    for channel in animation.channels() {
        let Some(joint) = node_indices
            .iter()
            .position(|n| *n == channel.target().node().index())
        else {
            // A channel animating something outside the skeleton — a camera,
            // a light, a prop. Not ours to apply.
            continue;
        };
        let path = match channel.target().property() {
            gltf::animation::Property::Translation => AnimPath::Translation,
            gltf::animation::Property::Rotation => AnimPath::Rotation,
            gltf::animation::Property::Scale => AnimPath::Scale,
            // Morph target weights are a different mechanism entirely.
            gltf::animation::Property::MorphTargetWeights => continue,
        };
        let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
        let Some(times) = reader.read_inputs().map(|t| t.collect::<Vec<f32>>()) else {
            continue;
        };
        let values: Vec<f32> = match reader.read_outputs() {
            Some(gltf::animation::util::ReadOutputs::Translations(v)) => v.flatten().collect(),
            Some(gltf::animation::util::ReadOutputs::Scales(v)) => v.flatten().collect(),
            Some(gltf::animation::util::ReadOutputs::Rotations(v)) => {
                v.into_f32().flatten().collect()
            }
            _ => continue,
        };
        duration = duration.max(times.last().copied().unwrap_or(0.0));
        channels.push(Channel {
            joint: joint as u16,
            path,
            times,
            values,
        });
    }

    Clip {
        name: animation.name().unwrap_or("clip").to_string(),
        duration,
        channels,
    }
}

/// A glTF model's own colours, as one texture its UVs read: the image its
/// materials share, when they have one; otherwise their base colours in a
/// palette — four texels square each, so filtering never mixes two — with
/// every vertex's UV moved to the middle of its material's square. `None`
/// for a model with no materials. Emission, metal and roughness are not
/// carried: the look is the colour.
fn gltf_look(
    document: &gltf::Document,
    images: &[gltf::image::Data],
    vertices: &mut [Vertex],
    painted: &[(std::ops::Range<usize>, Option<usize>)],
    path: &Path,
) -> Option<TextureAsset> {
    let used: Vec<usize> = {
        let mut u: Vec<usize> = painted.iter().filter_map(|(_, m)| *m).collect();
        u.sort();
        u.dedup();
        u
    };
    if used.is_empty() {
        return None;
    }
    let material = |i: usize| document.materials().nth(i);
    let id = AssetId::from_source(&format!("{}#look", path.display()), 0);
    let name = format!(
        "{} look",
        path.file_stem()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default()
    );
    // One image shared by every textured material: that is the look.
    let textured: Vec<usize> = used
        .iter()
        .filter_map(|&i| {
            let texture = material(i)?.pbr_metallic_roughness().base_color_texture()?;
            Some(texture.texture().source().index())
        })
        .collect();
    if let Some(&image) = textured.first() {
        if textured.iter().all(|&t| t == image) {
            let data = images.get(image)?;
            let pixels = rgba8(data)?;
            let mips = build_mips(data.width, data.height, &pixels, true);
            return Some(TextureAsset {
                id,
                name,
                width: data.width,
                height: data.height,
                pixels,
                mips,
                srgb: true,
                coding: Default::default(),
            });
        }
    }
    // Colours only: a palette, a square of four texels a material.
    const CELL: u32 = 4;
    let width = CELL * used.len() as u32;
    let mut pixels = vec![0u8; (width * CELL * 4) as usize];
    let encode = |linear: f32| -> u8 {
        let v = linear.clamp(0.0, 1.0);
        let c = if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (c * 255.0).round() as u8
    };
    for (k, &i) in used.iter().enumerate() {
        let [r, g, b, a] = material(i)
            .map(|m| m.pbr_metallic_roughness().base_color_factor())
            .unwrap_or([1.0; 4]);
        let texel = [
            encode(r),
            encode(g),
            encode(b),
            (a.clamp(0.0, 1.0) * 255.0) as u8,
        ];
        for y in 0..CELL {
            for x in 0..CELL {
                let at = ((y * width + k as u32 * CELL + x) * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&texel);
            }
        }
    }
    for (range, m) in painted {
        let Some(k) = m.and_then(|m| used.iter().position(|&u| u == m)) else {
            continue;
        };
        let uv = [
            (k as f32 * CELL as f32 + CELL as f32 / 2.0) / width as f32,
            0.5,
        ];
        for v in &mut vertices[range.clone()] {
            v.uv = uv;
        }
    }
    Some(TextureAsset {
        id,
        name,
        width,
        height: CELL,
        pixels,
        mips: Vec::new(),
        srgb: true,
        coding: Default::default(),
    })
}

/// A glTF image as RGBA8, from the layouts its loader gives.
fn rgba8(data: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format;
    let p = &data.pixels;
    Some(match data.format {
        Format::R8G8B8A8 => p.clone(),
        Format::R8G8B8 => p
            .chunks_exact(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        Format::R8G8 => p
            .chunks_exact(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        Format::R8 => p.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        _ => return None,
    })
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
    let mut pixels = image.into_raw();
    if let Some(channel) = settings.alpha_from {
        let from = channel as usize;
        for texel in pixels.chunks_exact_mut(4) {
            texel[3] = texel[from];
        }
    }
    let mips = build_mips(width, height, &pixels, settings.srgb);
    Ok(TextureAsset {
        id: settings.asset_id(),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "texture".into()),
        width,
        height,
        pixels,
        mips,
        srgb: settings.srgb,
        coding: Default::default(),
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

/// Read a WAV file and decode it to interleaved stereo floats.
///
/// Mono is duplicated to both channels here rather than at playback, so the
/// mixer has one layout and no branch — and so a sound that was mono is not
/// quietly louder than one that was stereo.
/// A sound file — WAV, MP3, Ogg Vorbis, FLAC — as a sound asset: decoded to
/// interleaved stereo when it is short, kept as its compressed bytes (to
/// be streamed) when it is longer than [`scrap::asset::LONG_SOUND_SECONDS`].
pub fn sound_from_file(path: impl AsRef<Path>, settings: &ImportSettings) -> Result<SoundAsset> {
    use symphonia::core::audio::sample::Sample;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let path = path.as_ref();
    let bytes = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
    let source = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.clone())),
        Default::default(),
    );
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let track = format
        .default_track(TrackType::Audio)
        .with_context(|| format!("{}: no sound in it", path.display()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .with_context(|| format!("{}: not a sound track", path.display()))?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let mut rate = params.sample_rate.unwrap_or(44_100);
    let mut samples: Vec<f32> = Vec::new();
    let mut chunk: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(e) => return Err(anyhow::anyhow!("{}: {e}", path.display())),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buffer) => {
                rate = buffer.spec().rate();
                let channels = buffer.spec().channels().count().max(1);
                chunk.resize(buffer.samples_interleaved(), f32::MID);
                buffer.copy_to_slice_interleaved(&mut chunk);
                // Stereo, as every sound is here: mono doubled, more than
                // two downmixed to the first two.
                for frame in chunk.chunks(channels) {
                    let left = frame[0];
                    samples.push(left);
                    samples.push(frame.get(1).copied().unwrap_or(left));
                }
            }
            Err(Error::DecodeError(_)) => {}
            Err(e) => return Err(anyhow::anyhow!("{}: {e}", path.display())),
        }
    }
    let seconds = samples.len() as f32 / 2.0 / rate.max(1) as f32;
    let long = seconds > scrap::asset::LONG_SOUND_SECONDS;
    Ok(SoundAsset {
        id: settings.asset_id(),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sound".into()),
        sample_rate: rate,
        samples: if long { Vec::new() } else { samples },
        encoded: if long { bytes } else { Vec::new() },
        seconds,
    })
}

pub fn sound_from_wav(path: impl AsRef<Path>, settings: &ImportSettings) -> Result<SoundAsset> {
    let path = path.as_ref();
    let mut reader = hound::WavReader::open(path).with_context(|| format!("{}", path.display()))?;
    let spec = reader.spec();

    let mono: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => {
            // Normalised by the format's own full scale, not by a guess: a
            // 24-bit file divided by 32768 clips into a wall of distortion.
            let scale = match spec.bits_per_sample {
                8 => i8::MAX as f32,
                16 => i16::MAX as f32,
                24 => 8_388_607.0,
                _ => i32::MAX as f32,
            };
            reader
                .samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / scale)
                .collect()
        }
    };

    let samples = match spec.channels {
        0 | 1 => mono.iter().flat_map(|s| [*s, *s]).collect(),
        2 => mono,
        // More than two channels are downmixed to the first two rather than
        // refused: a surround file is usable and a hard error is not.
        channels => mono
            .chunks(channels as usize)
            .flat_map(|frame| [frame[0], frame.get(1).copied().unwrap_or(frame[0])])
            .collect(),
    };

    Ok(SoundAsset {
        id: settings.asset_id(),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sound".into()),
        sample_rate: spec.sample_rate,
        seconds: samples.len() as f32 / 2.0 / spec.sample_rate.max(1) as f32,
        samples,
        encoded: Vec::new(),
    })
}

/// A colour as it is written down.
///
/// Two spellings, because there are two situations. A person or an agent
/// picking a colour has a hex code — that is what every colour picker, every
/// palette site and every screenshot gives you — and hex is sRGB. Something
/// that computed a colour has linear floats already. Accepting only the
/// second would mean every hand-written material carried a conversion done
/// by hand, which is exactly the arithmetic that silently comes out washed
/// out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    /// `"#7f8c99"`, or the same without the hash: sRGB, converted on import.
    Hex(String),
    /// Linear RGB, in `0..=1`, already converted.
    Linear([f32; 3]),
}

impl Color {
    /// The linear triple the shader wants.
    pub fn linear(&self) -> Result<[f32; 3]> {
        match self {
            Color::Linear(rgb) => Ok(*rgb),
            Color::Hex(text) => {
                let hex = text.trim().trim_start_matches('#');
                if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    anyhow::bail!("{text:?} is not a six-digit hex colour like \"#7f8c99\"");
                }
                let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
                Ok(Material::from_srgb(byte(0), byte(2), byte(4)).base_color)
            }
        }
    }
}

/// A material as it is authored: the source form of a `.scrmat`.
///
/// Its own type rather than reusing [`Material`] because the two are not the
/// same thing. This one is written by hand in sRGB and reads like a paint
/// swatch; the asset is linear and reads like something a GPU wants. Keeping
/// them apart is what lets the source stay editable and the asset stay fast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialSource {
    pub color: Color,
    /// Unlit surfaces ignore the sun and the fog. For embers and gizmos —
    /// things that are a light rather than something lit.
    #[serde(default)]
    pub unlit: bool,
    /// The metre grid on it: a greybox surface. Lit, so not with `unlit`.
    #[serde(default)]
    pub grid: bool,
    /// Water: waves, reflections, the colour of its depth, foam at the
    /// shore. `color` is the deep water's, `wind` the waves.
    #[serde(default)]
    pub water: bool,
    /// Sand: wind ripples, glinting grains, drifting sand in a gale.
    #[serde(default)]
    pub sand: bool,
    /// Clay: mud in the wet, cracking as it dries.
    #[serde(default)]
    pub clay: bool,
    /// Light under the surface, as the colour it comes out (sRGB, like
    /// `color`); none when not said.
    #[serde(default)]
    pub subsurface: Option<Color>,
    /// How far it travels under it, metres.
    #[serde(default = "subsurface_reach")]
    pub subsurface_radius: f32,
    /// For water: metres one sees down through it.
    #[serde(default = "clear_water")]
    pub clarity: f32,
    /// For water: foam where it meets the shore, 0 to 1.
    #[serde(default = "one")]
    pub foam: f32,
    /// URP Lit's properties, with its names; what is not said is a matte
    /// opaque surface (see [`Material`]).
    #[serde(default)]
    pub metallic: f32,
    #[serde(default)]
    pub smoothness: f32,
    /// What it gives off, as a colour like `color`, times
    /// `emission_intensity`: past white it blooms.
    #[serde(default = "black")]
    pub emission: Color,
    #[serde(default = "one")]
    pub emission_intensity: f32,
    #[serde(default = "one")]
    pub alpha: f32,
    #[serde(default)]
    pub surface: scrap::material::SurfaceType,
    #[serde(default)]
    pub blend: scrap::material::Blend,
    #[serde(default)]
    pub alpha_clip: f32,
    #[serde(default)]
    pub render_face: scrap::material::RenderFace,
    #[serde(default = "yes")]
    pub specular_highlights: bool,
    #[serde(default = "yes")]
    pub environment_reflections: bool,
    #[serde(default = "yes")]
    pub receive_shadows: bool,
    /// Texture assets by file stem, URP's maps: the colour, the normal
    /// map, the mask (red metallic, green occlusion, alpha smoothness), the
    /// emission. Empty is none.
    #[serde(default)]
    pub base_map: String,
    #[serde(default)]
    pub normal_map: String,
    #[serde(default)]
    pub mask_map: String,
    #[serde(default)]
    pub emission_map: String,
    /// Its own shader: `shaders/<name>.wgsl` in the project, a `surface`
    /// function over the standard one. Empty is the standard one.
    #[serde(default)]
    pub shader: String,
    /// Up to eight numbers for its shader (`in.params`), in the order its
    /// `// scrap:params` line names them.
    #[serde(default)]
    pub params: Vec<f32>,
    /// Textures for its shader, by the names it reads them by (Unity's
    /// property names): `textures: {"_Road": "T_Road_01"}`, each a
    /// texture's file stem or its asset id. The shader's
    /// `// scrap:textures` line says which it reads, in which slot.
    /// Imported as colour unless their sidecars say otherwise.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub textures: BTreeMap<String, String>,
    /// The base map on the screen, not the mesh: `Screen`, or `Mirror` for
    /// a mirror's picture.
    #[serde(default)]
    pub screen_map: scrap::material::ScreenMap,
    /// Over everything, walls included (transparent only).
    #[serde(default)]
    pub on_top: bool,
    #[serde(default = "one")]
    pub normal_scale: f32,
    #[serde(default = "one")]
    pub occlusion_strength: f32,
    /// How much it sways in the wind; see `scrap::foliage`.
    #[serde(default)]
    pub wind: f32,
    /// How much light comes through it from behind, 0 to 1.
    #[serde(default)]
    pub translucency: f32,
    #[serde(default = "no_tiling")]
    pub tiling: [f32; 2],
    #[serde(default)]
    pub offset: [f32; 2],
}

fn no_tiling() -> [f32; 2] {
    [1.0, 1.0]
}

/// The texture a material names, as the asset id it has or will have: from
/// its sidecar once imported, or the id its first import will mint. `data`
/// maps (normal, mask) must be imported linear, and it says so if not.
fn texture_id(material: &Path, name: &str, data: bool) -> Result<Option<scrap::asset::AssetId>> {
    if name.is_empty() {
        return Ok(None);
    }
    // A camera's picture: `render:mirror`, what a camera with
    // `render_texture: (name: "mirror")` draws.
    if let Some(target) = name.strip_prefix("render:") {
        return Ok(Some(scrap::asset::AssetId::render_target(target)));
    }
    let project = scrap::Project::find(material).map_err(|e| {
        anyhow::anyhow!(
            "{}: `{name}` is a texture, and a material that uses one has to be in a project: {e}",
            material.display()
        )
    })?;
    let mut found = Vec::new();
    let mut names = Vec::new();
    let mut stack = vec![project.assets()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let image = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .is_some_and(|e| ["png", "jpg", "jpeg", "tga", "bmp"].contains(&e.as_str()));
            if !image {
                continue;
            }
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if stem == name {
                found.push(path);
            } else {
                names.push(stem);
            }
        }
    }
    match found.len() {
        0 => {
            let near = scrap::spelling::closest(name, names.iter().map(String::as_str))
                .map(|n| format!(" — did you mean `{n}`?"))
                .unwrap_or_default();
            anyhow::bail!(
                "{}: no texture `{name}` under assets/{near}",
                material.display()
            )
        }
        1 => {}
        _ => anyhow::bail!(
            "{}: more than one texture is called `{name}`: {}",
            material.display(),
            found
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
    let texture = &found[0];
    let settings = match ImportSettings::load(sidecar_for(texture)) {
        Ok(settings) => settings,
        Err(_) => ImportSettings::for_source(
            project
                .relative(texture)
                .unwrap_or_else(|| texture.to_string_lossy().into_owned()),
        ),
    };
    if data && settings.srgb {
        anyhow::bail!(
            "{}: `{name}` is a normal map or a mask, but it is imported as colour — set `srgb: false` in {}; read as colour, every value in it bends",
            material.display(),
            sidecar_for(texture).display()
        );
    }
    Ok(Some(settings.asset_id()))
}

/// A texture a material hands its shader: an asset id as it is — 32 hex
/// digits, which no texture is named — or a texture by name, as the maps
/// are found.
fn shader_texture_id(material: &Path, texture: &str) -> Result<Option<scrap::asset::AssetId>> {
    let text = texture.trim();
    if text.len() == 32 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(Some(text.parse().map_err(anyhow::Error::msg)?));
    }
    texture_id(material, text, false)
}

fn subsurface_reach() -> f32 {
    0.01
}

fn clear_water() -> f32 {
    3.0
}
fn one() -> f32 {
    1.0
}

fn black() -> Color {
    Color::Linear([0.0; 3])
}

/// Read a `.scrmat` and build a material asset from it.
/// A material's source with its parent's under it: Unreal's Material
/// Instance (docs/artist.md). A `.scrmat` that says `parent: "stone"` — or
/// `parent: ("stone", "<id>")`, a link (docs/refs.md) — is the parent with
/// only what it states changed, so a dozen stones are one material and a
/// dozen colours, and changing the parent's smoothness changes them all.
pub fn material_source(path: &Path) -> Result<MaterialSource> {
    let text = material_text(path, 0)?;
    ron::from_str(&text).with_context(|| format!("{}", path.display()))
}

/// A material seen as an instance: the parent it names, and each of its
/// parameters with the value it takes and whether this file sets it
/// (rather than inheriting it). What the editor's material panel shows.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialLayers {
    pub parent: Option<scrap::AssetLink>,
    /// `(name, value as RON, set here)`, in the source's field order.
    pub fields: Vec<(String, String, bool)>,
}

pub fn material_layers(path: &Path) -> Result<MaterialLayers> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let own: ron::Value = ron::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let parent = parent_link(&own);
    let own_keys: Vec<String> = match &own {
        ron::Value::Map(map) => map
            .keys()
            .filter_map(|k| match k {
                ron::Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    // Every parameter, defaults filled in, as the source would say it.
    let effective = material_source(path)?;
    let text = ron::to_string(&effective)?;
    let ron::Value::Map(all) = ron::from_str::<ron::Value>(&text)? else {
        anyhow::bail!("{}: not a material", path.display());
    };
    let order: Vec<String> = {
        // Field order as the struct declares it: serialise and read keys in turn.
        let mut keys = Vec::new();
        let inner = text.trim().trim_start_matches('(').trim_end_matches(')');
        let mut depth = 0i32;
        let mut in_string = false;
        let mut start = 0;
        let bytes = inner.as_bytes();
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'"' => in_string = !in_string,
                b'(' | b'[' | b'{' if !in_string => depth += 1,
                b')' | b']' | b'}' if !in_string => depth -= 1,
                b',' if !in_string && depth == 0 => {
                    if let Some(k) = inner[start..i].split(':').next() {
                        keys.push(k.trim().to_string());
                    }
                    start = i + 1;
                }
                _ => {}
            }
        }
        if let Some(k) = inner[start..].split(':').next() {
            if !k.trim().is_empty() {
                keys.push(k.trim().to_string());
            }
        }
        keys
    };
    let fields = order
        .into_iter()
        .filter_map(|key| {
            let value = all.get(&ron::Value::String(key.clone()))?;
            let text = ron::to_string(value).ok()?;
            let set = own_keys.contains(&key);
            Some((key, text, set))
        })
        .collect();
    Ok(MaterialLayers { parent, fields })
}

/// The files a material is made of: itself, then its parent, its parent's
/// parent… What its hash covers, so an edited parent rebuilds its
/// instances.
pub fn material_parents(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut at = path.to_path_buf();
    while out.len() < MATERIAL_DEPTH {
        let Some(parent) = std::fs::read_to_string(&at)
            .ok()
            .and_then(|text| ron::from_str::<ron::Value>(&text).ok())
            .and_then(|value| parent_link(&value))
            .and_then(|link| find_material(&at, &link))
        else {
            break;
        };
        if parent == path || out.contains(&parent) {
            break;
        }
        out.push(parent.clone());
        at = parent;
    }
    out
}

/// How deep a chain of parents may go: deep enough for any real palette,
/// shallow enough that a loop is an error and not a hang.
const MATERIAL_DEPTH: usize = 8;

/// A material's text with its parents' under it: the parent's text, each
/// field this file states set over it in place. As text rather than as
/// RON's generic value, which cannot hold a bare enum variant
/// (`render_face: Both`).
fn material_text(path: &Path, depth: usize) -> Result<String> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let value: ron::Value = ron::from_str(&text).with_context(|| format!("{}", path.display()))?;
    let Some(link) = parent_link(&value) else {
        return Ok(text);
    };
    anyhow::ensure!(
        depth < MATERIAL_DEPTH,
        "{}: parents go more than {MATERIAL_DEPTH} deep — does a material name itself?",
        path.display()
    );
    let parent = find_material(path, &link).with_context(|| {
        format!(
            "{}: no material `{}` to be the parent",
            path.display(),
            link.name
        )
    })?;
    let mut under = material_text(&parent, depth + 1)?;
    let open = scrap::ron_edit::outer_open(&text)
        .with_context(|| format!("{}: not a material", path.display()))?;
    let own = scrap::ron_edit::items(&text, open)
        .with_context(|| format!("{}: not a material", path.display()))?;
    for span in own.items {
        let Some((key, value)) = text[span].split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key == "parent" {
            continue;
        }
        under =
            scrap::ron_edit::set_field(&under, key, Some(value.trim())).with_context(|| {
                format!("{}: could not lay `{key}` over its parent", path.display())
            })?;
    }
    Ok(under)
}

/// What a material's `parent:` says, as a link.
fn parent_link(value: &ron::Value) -> Option<scrap::AssetLink> {
    let ron::Value::Map(map) = value else {
        return None;
    };
    let parent = map.get(&ron::Value::String("parent".into()))?;
    parent.clone().into_rust::<scrap::AssetLink>().ok()
}

/// A `.scrmat` of the project by link: the one whose sidecar has the ID,
/// or the one with the name.
fn find_material(near: &Path, link: &scrap::AssetLink) -> Option<PathBuf> {
    let root = scrap::Project::find(near)
        .map(|p| p.materials())
        .unwrap_or_else(|_| near.parent().map(Path::to_path_buf).unwrap_or_default());
    let mut found = Vec::new();
    walk(&root, &mut |p| {
        if p.extension().is_some_and(|e| e == "scrmat") {
            found.push(p.to_path_buf());
        }
    });
    if let Some(id) = link.id {
        if let Some(p) = found
            .iter()
            .find(|p| scrap::asset::sidecar_id(sidecar_for(p)) == Some(id))
        {
            return Some(p.clone());
        }
    }
    found
        .into_iter()
        .find(|p| p.file_stem().is_some_and(|s| s == link.as_str()))
}

pub fn material_from_ron(
    path: impl AsRef<Path>,
    settings: &ImportSettings,
) -> Result<MaterialAsset> {
    let path = path.as_ref();
    let source = material_source(path)?;
    Ok(MaterialAsset {
        id: settings.asset_id(),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "material".into()),
        material: Material {
            base_color: source.color.linear()?,
            shading: match (source.unlit, source.grid, source.water, source.sand) {
                (false, false, false, false) => Shading::Lit,
                (true, false, false, false) => Shading::Unlit,
                (false, true, false, false) => Shading::Grid,
                (false, false, true, false) => Shading::Water,
                (false, false, false, true) => Shading::Sand,
                _ => anyhow::bail!(
                    "{}: more than one of unlit, grid, water and sand — pick one",
                    path.display()
                ),
            },
            metallic: source.metallic,
            smoothness: source.smoothness,
            emission: source
                .emission
                .linear()?
                .map(|c| c * source.emission_intensity.max(0.0)),
            alpha: source.alpha,
            surface: source.surface,
            blend: source.blend,
            alpha_clip: source.alpha_clip,
            render_face: source.render_face,
            specular_highlights: source.specular_highlights,
            environment_reflections: source.environment_reflections,
            receive_shadows: source.receive_shadows,
            base_map: texture_id(path, &source.base_map, false)?,
            normal_map: texture_id(path, &source.normal_map, true)?,
            mask_map: texture_id(path, &source.mask_map, true)?,
            emission_map: texture_id(path, &source.emission_map, false)?,
            shader: (!source.shader.is_empty()).then(|| scrap::asset::shader_id(&source.shader)),
            params: std::array::from_fn(|i| source.params.get(i).copied().unwrap_or(0.0)),
            textures: scrap::material::MaterialTextures::new(
                source
                    .textures
                    .iter()
                    .map(|(name, texture)| {
                        let id = shader_texture_id(path, texture).with_context(|| {
                            format!("{}: its texture `{name}`", path.display())
                        })?;
                        Ok(id.map(|id| (name.clone(), id)))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .flatten(),
            ),
            screen_map: source.screen_map,
            on_top: source.on_top,
            normal_scale: source.normal_scale,
            occlusion_strength: source.occlusion_strength.clamp(0.0, 1.0),
            tiling: source.tiling,
            offset: source.offset,
            wind: source.wind.max(0.0),
            translucency: source.translucency.clamp(0.0, 1.0),
            clarity: if source.water {
                source.clarity.max(0.05)
            } else {
                0.0
            },
            foam: if source.water {
                source.foam.clamp(0.0, 1.0)
            } else {
                0.0
            },
            clay: source.clay,
            subsurface: match &source.subsurface {
                Some(color) => color.linear()?,
                None => [0.0; 3],
            },
            subsurface_radius: source.subsurface_radius.max(0.0005),
        },
    })
}

/// Import one source file into `library`, with its sidecar beside it.
pub fn import_file(
    source: impl AsRef<Path>,
    library: impl AsRef<Path>,
    settings: ImportSettings,
) -> Result<Imported> {
    let source = source.as_ref();
    import_to(source, library.as_ref(), &sidecar_for(source), settings)
}

/// Import one source file into `library`, writing its sidecar at `sidecar`.
///
/// The hash and the ID are filled in on the way: the hash of what was just
/// read, and the ID the settings already had or the one this import mints.
pub fn import_to(
    source: &Path,
    library: &Path,
    sidecar: &Path,
    mut settings: ImportSettings,
) -> Result<Imported> {
    std::fs::create_dir_all(library)?;
    let first = settings.id.is_none() && settings.hash.is_empty();
    settings.hash = content_hash(source).with_context(|| format!("{}", source.display()))?;
    settings.id = Some(settings.asset_id());

    let extension = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if extension == "blend" {
        settings.scene = true;
    } else if matches!(extension.as_str(), "gltf" | "glb") && first && !settings.scene {
        settings.scene = scene::is_scene(source)?;
    }
    if settings.scene {
        let id = scene::import_scene(source, library, &mut settings)?;
        settings.save(sidecar)?;
        return Ok(Imported {
            id,
            asset: scene::prefab_for(id, library),
            sidecar: sidecar.to_path_buf(),
        });
    }
    let (bytes, id, kind) = match extension.as_str() {
        "gltf" | "glb" => {
            let mesh = mesh_from_gltf(source, &settings)?;
            (
                scrap::asset::to_bytes(&mesh, scrap::asset::MESH)?,
                mesh.id,
                scrap::asset::MESH,
            )
        }
        "obj" => {
            let mesh = mesh_from_obj(source, &settings)?;
            (
                scrap::asset::to_bytes(&mesh, scrap::asset::MESH)?,
                mesh.id,
                scrap::asset::MESH,
            )
        }
        "scrpoly" => {
            let mesh = poly::mesh_from_poly(source, &settings)?;
            (
                scrap::asset::to_bytes(&mesh, scrap::asset::MESH)?,
                mesh.id,
                scrap::asset::MESH,
            )
        }
        "scrbrush" => {
            let mesh = brush::mesh_from_brushes(source, &settings)?;
            (
                scrap::asset::to_bytes(&mesh, scrap::asset::MESH)?,
                mesh.id,
                scrap::asset::MESH,
            )
        }
        "scrterrain" => {
            let mesh = terrain::mesh_from_terrain(source, &settings)?;
            (
                scrap::asset::to_bytes(&mesh, scrap::asset::MESH)?,
                mesh.id,
                scrap::asset::MESH,
            )
        }
        "wav" | "mp3" | "ogg" | "flac" => {
            let sound = sound_from_file(source, &settings)?;
            (
                scrap::asset::to_bytes(&sound, scrap::asset::SOUND)?,
                sound.id,
                scrap::asset::SOUND,
            )
        }
        "scrmat" => {
            let material = material_from_ron(source, &settings)?;
            (
                scrap::asset::to_bytes(&material, scrap::asset::MATERIAL)?,
                material.id,
                scrap::asset::MATERIAL,
            )
        }
        "png" | "jpg" | "jpeg" | "tga" | "bmp" => {
            let texture = texture_from_image(source, &settings)?;
            (
                scrap::asset::to_bytes(&texture, scrap::asset::TEXTURE)?,
                texture.id,
                scrap::asset::TEXTURE,
            )
        }
        other => anyhow::bail!("no importer for .{other} yet"),
    };

    let asset_path = asset_for(id, library);
    std::fs::write(&asset_path, bytes)?;
    settings.save(sidecar)?;
    let _ = kind;

    Ok(Imported {
        id,
        asset: asset_path,
        sidecar: sidecar.to_path_buf(),
    })
}

/// What importing into a project did, and what the editor should say about
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    pub imported: Imported,
    /// Set when the source is outside the project. It is imported, by
    /// absolute path — and a clone of the project anywhere else will not find
    /// it, which is worth a sentence before it is worth a surprise.
    pub warning: Option<String>,
}

/// Import a source into a project's library.
///
/// A source inside the project is named by its path relative to the project
/// and gets its sidecar beside it. A source outside is named by its absolute
/// path, gets its sidecar in the project's `assets/` under its own file
/// name, and comes back with a warning (DNA, "Принятые решения": relative
/// paths, absolute only outside the project, with a warning).
///
/// An existing sidecar keeps its asset's ID always, and its settings unless
/// `options` gives new ones — so a re-import without options rebuilds the
/// asset the way it was built, not the default way.
pub fn import_into(
    project: &scrap::Project,
    source: &Path,
    options: Option<&ImportSettings>,
) -> Result<ImportReport> {
    let (name, sidecar, warning) = match project.relative(source) {
        Some(relative) => (relative, sidecar_for(source), None),
        None => {
            let absolute = std::path::absolute(source)?;
            let file = source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "source".into());
            let warning = format!(
                "{} is outside the project, so it is referenced by absolute path and a clone \
                 of the project elsewhere will not find it — copy it into assets/ to keep it",
                absolute.display()
            );
            (
                absolute.to_string_lossy().into_owned(),
                project.assets().join(format!("{file}.scrimport")),
                Some(warning),
            )
        }
    };
    let existing = ImportSettings::load(&sidecar).ok();
    let mut settings = match (options, &existing) {
        (Some(options), _) => options.clone(),
        (None, Some(existing)) => existing.clone(),
        (None, None) => ImportSettings::default(),
    };
    settings.id = existing.and_then(|e| e.id);
    settings.source = name;
    let imported = import_to(source, &project.library(), &sidecar, settings)?;
    Ok(ImportReport { imported, warning })
}

/// What happened to one source when a project's library was brought up to
/// date.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A source nothing had imported yet: dropped into `assets/`, or a
    /// fresh clone with an empty library.
    New,
    /// Its contents changed since it was built.
    Changed,
    /// Its asset was missing from the library and has been built.
    Built,
    /// Its asset was written by an older build's format and has been built
    /// again.
    Outdated,
    /// The sidecar's source was gone and the same contents turned up
    /// elsewhere: the sidecar followed, with its settings and its asset's ID.
    Moved { from: String },
    /// The sidecar's source is gone and nothing with its contents is
    /// anywhere. The asset is left alone — a moved file should not blank a
    /// model — and this says so.
    Gone,
}

/// What one source's sync did.
#[derive(Debug, Clone, PartialEq)]
pub struct Reimported {
    pub source: PathBuf,
    pub change: Change,
    pub result: std::result::Result<scrap::AssetId, String>,
}

/// Bring a project's library up to date with its sources.
///
/// The library is derived (DNA, postulate 2): everything in it comes from a
/// source in `assets/` or `materials/` and the sidecar beside it, so a fresh
/// clone with no library at all ends up with a whole one, and a library
/// deleted by hand is rebuilt. What counts as changed is the content hash;
/// a modification time only says whether hashing is worth doing, because
/// polling this every frame must not read every texture every frame.
pub fn sync(project: &scrap::Project) -> Vec<Reimported> {
    sync_settled(project, std::time::Duration::ZERO)
}

/// [`sync`], leaving alone a `.blend` written less than `settle` ago.
///
/// What an editor polls with while an open Blender can send it the file
/// it just saved (docs/blender.md): that arrives in a moment and costs
/// nothing, where importing it here would start a second Blender for the
/// same work. A `.blend` changed any other way — a pull, a Blender without
/// the plugin — is imported once it has settled.
pub fn sync_settled(project: &scrap::Project, settle: std::time::Duration) -> Vec<Reimported> {
    let fresh = |path: &Path| {
        !settle.is_zero()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("blend"))
            && modified(path)
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age < settle)
    };
    let library = project.library();
    // Built before assets were named by ID: gone, and built again below.
    if let Ok(entries) = std::fs::read_dir(&library) {
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().and_then(|e| e.to_str()) == Some("scrasset") && !named_by_id(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let mut sidecars = Vec::new();
    let mut sources = Vec::new();
    for root in [project.assets(), project.materials()] {
        walk(&root, &mut |path| {
            if path.extension().and_then(|e| e.to_str()) == Some("scrimport") {
                sidecars.push(path.to_path_buf());
            } else if importable(path) {
                sources.push(path.to_path_buf());
            }
        });
    }
    sidecars.sort();
    sources.sort();

    let resolve = |name: &str| -> PathBuf {
        let path = Path::new(name);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            project.resolve(name)
        }
    };
    let same = |a: &Path, b: &Path| {
        std::path::absolute(a)
            .ok()
            .is_some_and(|a| std::path::absolute(b).ok() == Some(a))
    };

    let mut out = Vec::new();
    let mut claimed: Vec<PathBuf> = Vec::new();
    let mut orphans: Vec<(PathBuf, ImportSettings)> = Vec::new();

    for sidecar in &sidecars {
        let settings = match ImportSettings::load(sidecar) {
            Ok(settings) => settings,
            Err(e) => {
                out.push(Reimported {
                    source: sidecar.clone(),
                    change: Change::Gone,
                    result: Err(format!("{}: {e:#}", sidecar.display())),
                });
                continue;
            }
        };
        let source = resolve(&settings.source);
        if !source.is_file() {
            orphans.push((sidecar.clone(), settings));
            continue;
        }
        claimed.push(source.clone());
        if fresh(&source) {
            continue;
        }

        let asset = output_for(&settings, &library);
        let change = if !asset.is_file() {
            Some(Change::Built)
        } else if outdated(&settings, &asset, &library) {
            Some(Change::Outdated)
        } else if settings.hash.is_empty() || modified(&source) > modified(sidecar) {
            // The clock says maybe; the hash decides.
            match content_hash(&source) {
                Ok(hash) if hash == settings.hash => {
                    // Unchanged after all — a checkout, a copy. Moving the
                    // sidecar's clock forward stops the next poll from
                    // hashing the same bytes again, without touching a byte
                    // of the sidecar.
                    touch(sidecar);
                    None
                }
                _ => Some(Change::Changed),
            }
        } else {
            None
        };
        if let Some(change) = change {
            let result = import_to(&source, &library, sidecar, settings)
                .map(|imported| imported.id)
                .map_err(|e| format!("{e:#}"));
            out.push(Reimported {
                source,
                change,
                result,
            });
        }
    }

    let mut unclaimed: Vec<PathBuf> = sources
        .into_iter()
        .filter(|source| !claimed.iter().any(|c| same(c, source)))
        .filter(|source| !fresh(source))
        .collect();

    // A sidecar whose source is gone, and a source with no sidecar holding
    // the same bytes: one file that moved. The sidecar follows it, keeping
    // the settings someone chose and the ID scenes point at.
    for (old_sidecar, mut settings) in orphans {
        let moved_to = (!settings.hash.is_empty())
            .then(|| {
                unclaimed.iter().position(|candidate| {
                    content_hash(candidate).is_ok_and(|hash| hash == settings.hash)
                })
            })
            .flatten();
        let Some(position) = moved_to else {
            out.push(Reimported {
                source: resolve(&settings.source),
                change: Change::Gone,
                result: Err(format!(
                    "{} is gone, and nothing in the project has its contents",
                    settings.source
                )),
            });
            continue;
        };
        let source = unclaimed.remove(position);
        let from = settings.source.clone();
        settings.source = project
            .relative(&source)
            .unwrap_or_else(|| source.to_string_lossy().into_owned());
        let new_sidecar = sidecar_for(&source);
        let result = import_to(&source, &library, &new_sidecar, settings)
            .map(|imported| imported.id)
            .map_err(|e| format!("{e:#}"));
        if result.is_ok() {
            // The asset is built under its ID, so the one built before the
            // move has just been written over: only the old sidecar goes.
            let _ = std::fs::remove_file(&old_sidecar);
        }
        out.push(Reimported {
            source,
            change: Change::Moved { from },
            result,
        });
    }

    // Whatever is left is new: imported with the defaults, and given a
    // sidecar so the next person sees the settings it was built with.
    for source in unclaimed {
        let name = project
            .relative(&source)
            .unwrap_or_else(|| source.to_string_lossy().into_owned());
        let result = import_file(&source, &library, ImportSettings::for_source(name))
            .map(|imported| imported.id)
            .map_err(|e| format!("{e:#}"));
        out.push(Reimported {
            source,
            change: Change::New,
            result,
        });
    }
    // Prefabs, scenes, graphs and screens get their IDs too. Not reported:
    // nothing in the library changed for them.
    identify(project);
    out
}

/// Where the project's own text assets are — prefabs, scenes, animator
/// graphs, screens — with the extension each folder's files have.
fn text_assets(project: &scrap::Project) -> [(PathBuf, &'static str); 4] {
    [
        (project.prefabs(), scrap::prefab::EXTENSION),
        (project.scenes(), "ron"),
        (project.root().join(scrap::project::ANIMATORS), "ron"),
        (project.root().join(scrap::project::UI), "ron"),
    ]
}

/// Give every prefab, scene, animator graph and screen a sidecar with its
/// ID, as models and materials have (docs/refs.md): what a link to it
/// holds. These are not imported — the files are read as they are — so the
/// sidecar holds only where the file is and its ID, and no hash: a scene
/// changes with every save, and a hash would put its sidecar in every diff.
///
/// A sidecar whose file is gone follows a file of the same name that turned
/// up without one — moved to another folder outside the editor. What this
/// cannot follow, a file renamed outside the editor, a link finds by the
/// name it keeps beside the ID.
pub fn identify(project: &scrap::Project) -> Vec<Reimported> {
    let mut sidecars = Vec::new();
    let mut files = Vec::new();
    for (root, extension) in text_assets(project) {
        walk(
            &root,
            &mut |path| match path.extension().and_then(|e| e.to_str()) {
                Some("scrimport") => sidecars.push(path.to_path_buf()),
                Some(e) if e == extension => files.push(path.to_path_buf()),
                _ => {}
            },
        );
    }
    sidecars.sort();
    files.sort();
    let name = |path: &Path| {
        project
            .relative(path)
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    };
    let mut out = Vec::new();
    let mut orphans = Vec::new();
    for sidecar in sidecars {
        let Ok(settings) = ImportSettings::load(&sidecar) else {
            continue;
        };
        let source = project.resolve(&settings.source);
        if source.is_file() {
            files.retain(|f| *f != source);
        } else {
            orphans.push((sidecar, settings));
        }
    }
    for (sidecar, mut settings) in orphans {
        let file_name = Path::new(&settings.source)
            .file_name()
            .map(|n| n.to_owned());
        let Some(at) = files
            .iter()
            .position(|f| f.file_name().map(|n| n.to_owned()) == file_name)
        else {
            continue;
        };
        let source = files.remove(at);
        let from = std::mem::replace(&mut settings.source, name(&source));
        let result = write_identity(&source, &settings).map(|()| settings.asset_id());
        if result.is_ok() {
            let _ = std::fs::remove_file(&sidecar);
        }
        out.push(Reimported {
            source,
            change: Change::Moved { from },
            result: result.map_err(|e| format!("{e:#}")),
        });
    }
    for source in files {
        let mut settings = ImportSettings::for_source(name(&source));
        settings.id = Some(settings.asset_id());
        let result = write_identity(&source, &settings).map(|()| settings.asset_id());
        out.push(Reimported {
            source,
            change: Change::New,
            result: result.map_err(|e| format!("{e:#}")),
        });
    }
    out
}

/// A text asset's sidecar: where it is and its ID, nothing an importer
/// would read.
fn write_identity(source: &Path, settings: &ImportSettings) -> Result<()> {
    let text = format!(
        "ImportSettings(\n    source: {:?},\n    id: Some({:?}),\n)\n",
        settings.source,
        settings.asset_id().to_string()
    );
    std::fs::write(sidecar_for(source), text)?;
    Ok(())
}

/// Whether the importer knows what to do with a file.
pub fn importable(path: &Path) -> bool {
    matches!(
        path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .as_deref(),
        Some(
            "gltf"
                | "glb"
                | "blend"
                | "obj"
                | "scrterrain"
                | "scrpoly"
                | "scrbrush"
                | "wav"
                | "mp3"
                | "ogg"
                | "flac"
                | "scrmat"
                | "png"
                | "jpg"
                | "jpeg"
                | "tga"
                | "bmp"
        )
    )
}

/// Every file under `root`, depth first. Hidden files and folders are
/// skipped: `.gitkeep`, editor droppings, a `.git` someone nested.
pub fn walk(root: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}

/// When a source last changed: itself, or — for a terrain — the latest of
/// it and its heightmap, so repainting the image counts.
fn modified(path: &Path) -> Option<std::time::SystemTime> {
    let own = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        dependencies(path)
            .iter()
            .filter_map(|d| std::fs::metadata(d).ok()?.modified().ok())
            .fold(own, |latest, t| latest.max(t)),
    )
}

/// The other files a source is built from: a terrain's heightmap, a
/// material's parents.
fn dependencies(path: &Path) -> Vec<PathBuf> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("scrterrain") => terrain::dependencies(path),
        Some("scrmat") => material_parents(path),
        _ => Vec::new(),
    }
}

fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
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
        let dir = std::env::temp_dir().join(format!("scrap-import-{name}"));
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
        let bytes = scrap::asset::read(&out.asset).unwrap();
        let archived = scrap::asset::view::<MeshAsset>(&bytes).unwrap();
        assert_eq!(archived.vertices.len(), 4);
        assert_eq!(archived.id, out.id);

        // And the sidecar, beside the source, says exactly how to build it
        // again — with what the source held and which asset it became.
        assert_eq!(out.sidecar, sidecar_for(&source));
        let written = ImportSettings::load(&out.sidecar).unwrap();
        assert_eq!(
            written,
            ImportSettings {
                hash: content_hash(&source).unwrap(),
                id: Some(out.id),
                ..settings
            }
        );
    }

    #[test]
    fn a_content_hash_is_the_same_for_the_same_bytes_and_only_them() {
        let dir = temp("hash");
        let (a, b, c) = (dir.join("a.obj"), dir.join("b.obj"), dir.join("c.obj"));
        std::fs::write(&a, SQUARE).unwrap();
        std::fs::write(&b, SQUARE).unwrap();
        std::fs::write(&c, "v 0 0 0\n").unwrap();
        let hash = |p: &Path| content_hash(p).unwrap();
        assert_eq!(hash(&a), hash(&b), "where it lives does not matter");
        assert_ne!(hash(&a), hash(&c));
        // Fixed, because it is committed: every machine must agree on it.
        let empty = dir.join("empty");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(hash(&empty), "6c62272e07bb014262b821756295c58d");
    }

    #[test]
    fn a_gltf_node_transform_reaches_the_vertices() {
        // The fixture's quad sits at the origin under a node that lifts it
        // three metres. Reading `document.meshes()` directly — the obvious
        // way — loses that, and a model exported as placed parts collapses
        // into a heap.
        let settings = ImportSettings {
            origin_to_base: false,
            ..ImportSettings::for_source("floating_quad.gltf")
        };
        let mesh = mesh_from_gltf(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floating_quad.gltf"),
            &settings,
        )
        .unwrap();

        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(mesh.submeshes.len(), 1);
        assert_eq!(mesh.bounds.min[1], 3.0, "the node's translation applied");
        assert_eq!(mesh.bounds.max[1], 3.0);
    }

    #[test]
    fn a_skinned_gltf_brings_its_skeleton_and_its_animation() {
        let settings = ImportSettings {
            origin_to_base: false,
            ..ImportSettings::for_source("skinned_banner.gltf")
        };
        let mesh = mesh_from_gltf(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skinned_banner.gltf"),
            &settings,
        )
        .unwrap();

        let skin = mesh.skin.expect("the file has a skin");
        assert_eq!(skin.skeleton.len(), 2);
        assert!(skin.skeleton.is_sorted(), "root before tip");
        // The parent link comes from the node graph, not from the joint
        // list's order: a skin that lists joints in any order still has to
        // come out with the right hierarchy.
        assert_eq!(skin.skeleton.joints[0].parent, None);
        assert_eq!(skin.skeleton.joints[1].parent, Some(0));
        assert_eq!(skin.skeleton.joints[1].name, "tip");

        assert_eq!(skin.joints.len(), mesh.vertices.len());
        assert_eq!(skin.weights.len(), mesh.vertices.len());
        // The middle pair is shared evenly between the two joints.
        assert_eq!(skin.weights[2], [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(skin.joints[2], [0, 1, 0, 0]);

        assert_eq!(skin.clips.len(), 1);
        let clip = &skin.clips[0];
        assert_eq!(clip.name, "furl");
        assert_eq!(clip.duration, 1.0);
        assert_eq!(clip.channels[0].joint, 1, "the channel points at the tip");

        // And the pose it produces actually turns that joint.
        let posed = clip.sample(&skin.skeleton, 1.0, false);
        let rotated = scrap::glam::Quat::from_array(posed[1].rotation);
        let angle = rotated.to_euler(scrap::glam::EulerRot::ZYX).0;
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "a quarter turn, got {angle}"
        );
    }

    #[test]
    fn a_gltf_models_colours_come_with_it_as_its_look() {
        // The quad fixture twice over, one red and one blue material.
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floating_quad.gltf");
        let mut root =
            gltf::json::Root::from_str(&std::fs::read_to_string(&fixture).unwrap()).unwrap();
        let primitive = root.meshes[0].primitives[0].clone();
        let painted = |i: u32| gltf::json::mesh::Primitive {
            material: Some(gltf::json::Index::new(i)),
            ..primitive.clone()
        };
        root.meshes[0].primitives = vec![painted(0), painted(1)];
        let colour = |c: [f32; 4]| gltf::json::Material {
            pbr_metallic_roughness: gltf::json::material::PbrMetallicRoughness {
                base_color_factor: gltf::json::material::PbrBaseColorFactor(c),
                ..Default::default()
            },
            ..Default::default()
        };
        root.materials = vec![colour([1.0, 0.0, 0.0, 1.0]), colour([0.0, 0.0, 1.0, 1.0])];
        let dir = temp("look");
        for entry in std::fs::read_dir(fixture.parent().unwrap())
            .unwrap()
            .flatten()
        {
            let _ = std::fs::copy(entry.path(), dir.join(entry.file_name()));
        }
        let source = dir.join("painted.gltf");
        std::fs::write(&source, gltf::json::serialize::to_string(&root).unwrap()).unwrap();
        let mesh = mesh_from_gltf(&source, &ImportSettings::for_source("painted.gltf")).unwrap();
        let look = mesh.look.expect("its colours");
        assert_eq!(
            (look.width, look.height),
            (8, 4),
            "a square of four a colour"
        );
        assert_eq!(&look.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&look.pixels[16..20], &[0, 0, 255, 255]);
        // Each primitive's vertices read the middle of its colour.
        assert_eq!(mesh.vertices[0].uv, [0.25, 0.5]);
        assert_eq!(mesh.vertices.last().unwrap().uv, [0.75, 0.5]);
        // No materials, no look.
        let plain =
            mesh_from_gltf(&fixture, &ImportSettings::for_source("floating_quad.gltf")).unwrap();
        assert!(plain.look.is_none());
    }

    /// A triangle whose COLOR_0 is `colors` (floats, as Blender writes a
    /// float colour attribute), or none.
    fn painted_triangle(dir: &Path, colors: Option<[[f32; 4]; 3]>) -> PathBuf {
        let mut bin: Vec<u8> = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] {
            bin.extend(p.iter().flat_map(|f| f.to_le_bytes()));
        }
        let mut attributes = r#""POSITION": 0"#.to_string();
        let mut accessors = r#"{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 0, 1]}"#.to_string();
        let mut views = r#"{"buffer": 0, "byteOffset": 0, "byteLength": 36}"#.to_string();
        if let Some(colors) = colors {
            for c in colors {
                bin.extend(c.iter().flat_map(|f| f.to_le_bytes()));
            }
            attributes += r#", "COLOR_0": 1"#;
            accessors += r#", {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC4"}"#;
            views += r#", {"buffer": 0, "byteOffset": 36, "byteLength": 48}"#;
        }
        std::fs::write(dir.join("triangle.bin"), &bin).unwrap();
        let json = format!(
            r#"{{"asset": {{"version": "2.0"}}, "scene": 0, "scenes": [{{"nodes": [0]}}], "nodes": [{{"mesh": 0}}],
            "meshes": [{{"primitives": [{{"attributes": {{{attributes}}}}}]}}],
            "accessors": [{accessors}], "bufferViews": [{views}],
            "buffers": [{{"uri": "triangle.bin", "byteLength": {}}}]}}"#,
            bin.len()
        );
        let path = dir.join("triangle.gltf");
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn a_gltf_keeps_the_colours_painted_on_its_vertices_as_the_file_has_them() {
        let dir = temp("vertex-colours");
        let source = painted_triangle(
            &dir,
            Some([[0.29, 0.0, 1.0, 1.0], [1.0, 0.5, 0.0, 1.0], [0.0, 0.0, 0.0, 0.25]]),
        );
        let settings = ImportSettings::for_source("triangle.gltf");
        let mesh = mesh_from_gltf(&source, &settings).unwrap();
        // As numbers, not bent through a curve: 0.29 is 74 of 255, not
        // the 147 that 0.29 made sRGB would be.
        assert_eq!(mesh.colors, vec![[74, 0, 255, 255], [255, 128, 0, 255], [0, 0, 0, 64]]);
        // Unpainted, or painted all white: no colours kept.
        let plain = mesh_from_gltf(painted_triangle(&dir, None), &settings).unwrap();
        assert!(plain.colors.is_empty());
        let white = mesh_from_gltf(painted_triangle(&dir, Some([[1.0; 4]; 3])), &settings).unwrap();
        assert!(white.colors.is_empty(), "white everywhere is what no colours read as");
    }

    #[test]
    fn an_unskinned_gltf_carries_no_skeleton_and_costs_nothing() {
        let settings = ImportSettings::for_source("floating_quad.gltf");
        let mesh = mesh_from_gltf(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floating_quad.gltf"),
            &settings,
        )
        .unwrap();
        assert!(mesh.skin.is_none());
    }

    #[test]
    fn a_gltf_without_normals_gets_them_computed_like_an_obj_does() {
        let settings = ImportSettings::for_source("floating_quad.gltf");
        let mesh = mesh_from_gltf(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floating_quad.gltf"),
            &settings,
        )
        .unwrap();
        for v in &mesh.vertices {
            assert!(
                (v.normal[1] - 1.0).abs() < 1e-5,
                "a floor faces up, got {:?}",
                v.normal
            );
        }
        assert_eq!(mesh.bounds.min[1], 0.0, "origin_to_base still applies");
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
        let bytes = scrap::asset::read(&out.asset).unwrap();
        assert_eq!(
            scrap::asset::kind_of(&bytes).unwrap(),
            scrap::asset::TEXTURE,
            "the header says what it is, so a library never casts one for the other"
        );
        let texture = scrap::asset::view::<scrap::asset::TextureAsset>(&bytes).unwrap();
        assert_eq!(texture.width.to_native(), 2);
        assert_eq!(texture.pixels.len(), 16, "two by two, four bytes each");
        assert_eq!(texture.pixels[0], 255, "the red pixel is first");
        assert!(texture.srgb, "a colour map is sRGB unless told otherwise");
    }

    #[test]
    fn a_shape_in_the_green_becomes_the_alpha_when_asked() {
        let dir = temp("alpha-from");
        let path = dir.join("blade.png");
        let mut image = image::RgbImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgb([200, 255, 10]));
        image.put_pixel(1, 0, image::Rgb([200, 0, 10]));
        image.save(&path).unwrap();

        let plain = texture_from_image(&path, &ImportSettings::for_source("blade.png")).unwrap();
        assert_eq!((plain.pixels[3], plain.pixels[7]), (255, 255), "no alpha of its own: opaque");
        let settings = ImportSettings {
            alpha_from: Some(ColourChannel::Green),
            ..ImportSettings::for_source("blade.png")
        };
        let cut = texture_from_image(&path, &settings).unwrap();
        assert_eq!((cut.pixels[3], cut.pixels[7]), (255, 0), "the blade where green is, nothing where not");
        assert_eq!(&cut.pixels[..3], &[200, 255, 10], "the colours stay");
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
        let bytes = scrap::asset::read(&out.asset).unwrap();
        let texture = scrap::asset::view::<scrap::asset::TextureAsset>(&bytes).unwrap();
        assert!(!texture.srgb);
    }

    #[test]
    fn a_short_sound_is_decoded_and_a_long_one_kept_to_stream() {
        let dir = temp("sound-long");
        let write = |name: &str, seconds: u32| {
            let path = dir.join(name);
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..8000 * seconds {
                writer
                    .write_sample(((i as f32 * 0.1).sin() * 8000.0) as i16)
                    .unwrap();
            }
            writer.finalize().unwrap();
            path
        };
        let step = sound_from_file(
            write("step.wav", 1),
            &ImportSettings::for_source("step.wav"),
        )
        .unwrap();
        assert_eq!(step.frames(), 8000, "decoded");
        assert!(step.encoded.is_empty());
        let wind_path = write("wind.wav", 11);
        let wind = sound_from_file(&wind_path, &ImportSettings::for_source("wind.wav")).unwrap();
        assert!(wind.samples.is_empty(), "too long to hold decoded");
        assert_eq!(
            wind.encoded,
            std::fs::read(&wind_path).unwrap(),
            "the file's own bytes"
        );
        assert!(
            (wind.duration_seconds() - 11.0).abs() < 0.01,
            "{}",
            wind.duration_seconds()
        );
    }

    #[test]
    fn a_wav_imports_as_decoded_stereo_whatever_it_started_as() {
        let dir = temp("sound");
        // Mono, 16-bit: the common case, and the one that has to come out
        // stereo so the mixer has one layout and no branch.
        let path = dir.join("beep.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..800 {
            writer
                .write_sample(((i as f32 * 0.1).sin() * 16384.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();

        let sound = sound_from_wav(&path, &ImportSettings::for_source("beep.wav")).unwrap();
        assert_eq!(sound.sample_rate, 8000);
        assert_eq!(sound.frames(), 800, "one frame per mono sample");
        assert_eq!(sound.samples.len(), 1600, "duplicated to both channels");
        assert_eq!(sound.samples[0], sound.samples[1], "left equals right");
        assert!((sound.duration_seconds() - 0.1).abs() < 1e-4);
        // Normalised by the format's own full scale: dividing a 16-bit
        // sample by the wrong number either clips or whispers.
        assert!(sound.samples.iter().all(|s| s.abs() <= 1.0));
        assert!(sound.samples.iter().any(|s| s.abs() > 0.4));
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
