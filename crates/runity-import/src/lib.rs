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
use runity::animation::{Channel, Clip, Joint, Path as AnimPath, PoseTransform, Skeleton};
use runity::asset::{
    AssetId, AssetKind, Bounds, MeshAsset, MeshSkin, SoundAsset, Submesh, TextureAsset,
    TextureLevel, Vertex,
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
        // OBJ has no concept of a skeleton.
        skin: None,
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
    let (document, buffers, _images) =
        gltf::import(path).with_context(|| format!("{}", path.display()))?;

    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut submeshes: Vec<Submesh> = Vec::new();
    let mut joint_indices: Vec<[u16; 4]> = Vec::new();
    let mut joint_weights: Vec<[f32; 4]> = Vec::new();

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

    while let Some((node, parent)) = stack.pop() {
        let local = glam::Mat4::from_cols_array_2d(&node.transform().matrix());
        let world = parent * local;
        let normal_matrix = glam::Mat3::from_mat4(world).inverse().transpose();

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
                    let p =
                        world.transform_point3(glam::Vec3::from_array(*position)) * settings.scale;
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

    let skin = read_skin(&document, &buffers, joint_indices, joint_weights);

    Ok(MeshAsset {
        id: AssetId::from_source(&settings.source, 0),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "mesh".into()),
        bounds: Bounds::of(&vertices),
        vertices,
        indices,
        submeshes,
        skin,
    })
}

/// The skeleton and animations, if the file has any.
fn read_skin(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
) -> Option<MeshSkin> {
    let gltf_skin = document.skins().next()?;
    let reader = gltf_skin.reader(|buffer| Some(&buffers[buffer.index()]));
    let inverse_binds: Vec<[[f32; 4]; 4]> = reader
        .read_inverse_bind_matrices()
        .map(|m| m.collect())
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

    let skeleton = Skeleton {
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

/// Read a WAV file and decode it to interleaved stereo floats.
///
/// Mono is duplicated to both channels here rather than at playback, so the
/// mixer has one layout and no branch — and so a sound that was mono is not
/// quietly louder than one that was stereo.
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
        id: AssetId::from_source(&settings.source, 0),
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sound".into()),
        sample_rate: spec.sample_rate,
        samples,
    })
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
        "gltf" | "glb" => {
            let mesh = mesh_from_gltf(source, &settings)?;
            (
                runity::asset::to_bytes(&mesh, AssetKind::Mesh)?,
                mesh.id,
                AssetKind::Mesh,
            )
        }
        "obj" => {
            let mesh = mesh_from_obj(source, &settings)?;
            (
                runity::asset::to_bytes(&mesh, AssetKind::Mesh)?,
                mesh.id,
                AssetKind::Mesh,
            )
        }
        "wav" => {
            let sound = sound_from_wav(source, &settings)?;
            (
                runity::asset::to_bytes(&sound, AssetKind::Sound)?,
                sound.id,
                AssetKind::Sound,
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

/// Re-import everything in a library whose source has changed since the
/// asset was built.
///
/// This is the drag-and-drop workflow without the dragging: the sidecar
/// says where each asset came from and with which settings, so a changed
/// source can be rebuilt without anyone remembering what was done to it.
/// It is also what makes the importer improvable — a better importer is
/// worth nothing if every asset has to be re-added by hand.
///
/// `root` is what the sidecars' relative source paths are relative to.
pub fn reimport_changed(library: impl AsRef<Path>, root: impl AsRef<Path>) -> Vec<Reimported> {
    let (library, root) = (library.as_ref(), root.as_ref());
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(library) else {
        return out;
    };
    for entry in entries.flatten() {
        let sidecar = entry.path();
        if sidecar.extension().and_then(|e| e.to_str()) != Some("rimport") {
            continue;
        }
        let Ok(settings) = ImportSettings::load(&sidecar) else {
            continue;
        };
        let source = root.join(&settings.source);
        let asset = sidecar.with_extension("rasset");

        let newer = match (modified(&source), modified(&asset)) {
            (Some(source_time), Some(asset_time)) => source_time > asset_time,
            // A source that has gone is reported rather than rebuilt: the
            // asset still works, and deleting it because a file moved would
            // be worse than saying so.
            (None, _) => {
                out.push(Reimported {
                    source: source.clone(),
                    result: Err(format!("{} is gone", settings.source)),
                });
                continue;
            }
            // No asset yet — build it.
            (Some(_), None) => true,
        };
        if !newer {
            continue;
        }

        let result = import_file(&source, library, settings)
            .map(|imported| imported.id)
            .map_err(|e| e.to_string());
        out.push(Reimported { source, result });
    }
    out
}

/// What one re-import attempt did.
#[derive(Debug, Clone, PartialEq)]
pub struct Reimported {
    pub source: PathBuf,
    pub result: std::result::Result<runity::AssetId, String>,
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
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
        let rotated = runity::glam::Quat::from_array(posed[1].rotation);
        let angle = rotated.to_euler(runity::glam::EulerRot::ZYX).0;
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "a quarter turn, got {angle}"
        );
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
