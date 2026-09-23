//! A source that is a scene, not a model: a glTF with many objects in it —
//! a level, a kit — imported as the tree it is (docs/blender.md, stage 1).
//!
//! [`crate::mesh_from_gltf`] merges a whole file into one mesh, which is
//! right for a tree made of a trunk and a crown and wrong for a forest: a
//! hundred copies of one rock become one mesh with a hundred rocks' worth
//! of triangles, and nothing in it can be selected, moved or given a
//! component. Here instead:
//!
//! * every glTF mesh becomes a model — one per primitive, since an entity
//!   draws with one material — named `<file>/<mesh>`. A mesh a hundred
//!   nodes share is **one** model, and the renderer draws its copies as
//!   instances: the instancing the artist set up survives the trip;
//! * every glTF material becomes a material asset, `<file>/<material>`,
//!   URP Lit from the metallic-roughness model, with its images as
//!   textures; a file-wide override in the `.rimport` (`materials:`)
//!   swaps one for a material of the project's;
//! * the nodes become a prefab named after the file, written into the
//!   library as `<asset id>.prefab`: derived, never committed, found by
//!   [`runity::Prefabs::of`] like a hand-written one. A scene places it
//!   with one line, `prefab: "forest"`.
//!
//! Every asset's ID is kept in the source's `.rimport` under `parts`, by a
//! key made of what the file calls it, so re-importing a changed file
//! updates assets rather than orphaning them. A node's entity ID is derived
//! from its path of names, so overrides of a part survive a re-import.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use glam::{Quat, Vec3};
use runity::asset::{
    AssetId, AssetKind, Bounds, MaterialAsset, MeshAsset, Submesh, TextureAsset, Vertex,
};
use runity::material::{Material, RenderFace, SurfaceType};
use runity::scene::MaterialRef;
use runity::{AssetLink, EntityDesc, EntityId, Transform};

use crate::{asset_for, build_mips, recompute_normals, ImportSettings};

/// Whether a glTF is a scene rather than a model: more than one node with
/// a mesh. Decided once, on the first import, and kept in the sidecar
/// (`scene: true`), so a file that grows a second object does not quietly
/// turn from the model scenes name into a prefab they do not.
pub fn is_scene(path: &Path) -> Result<bool> {
    let document = gltf::Gltf::open(path).with_context(|| format!("{}", path.display()))?;
    let mut count = 0;
    let mut stack: Vec<gltf::Node> = roots(&document.document);
    while let Some(node) = stack.pop() {
        if node.mesh().is_some() {
            count += 1;
        }
        stack.extend(node.children());
    }
    Ok(count > 1)
}

fn roots(document: &gltf::Document) -> Vec<gltf::Node<'_>> {
    document
        .default_scene()
        .or_else(|| document.scenes().next())
        .map(|scene| scene.nodes().collect())
        .unwrap_or_else(|| {
            // No scene: every node nobody parents.
            let children: HashSet<usize> = document
                .nodes()
                .flat_map(|n| n.children().map(|c| c.index()).collect::<Vec<_>>())
                .collect();
            document
                .nodes()
                .filter(|n| !children.contains(&n.index()))
                .collect()
        })
}

/// Where a scene source's prefab is built: `<id>.prefab` in the library.
pub fn prefab_for(id: AssetId, library: &Path) -> std::path::PathBuf {
    library.join(format!("{id}.{}", runity::prefab::EXTENSION))
}

/// The IDs of what one import builds, by key, minted the first time a key
/// is seen and kept after.
struct Parts<'a> {
    source: &'a str,
    before: BTreeMap<String, AssetId>,
    now: BTreeMap<String, AssetId>,
}

impl Parts<'_> {
    fn id(&mut self, key: String) -> AssetId {
        let id = self
            .before
            .get(&key)
            .copied()
            .unwrap_or_else(|| AssetId::from_source(&format!("{}#{key}", self.source), 0));
        self.now.insert(key, id);
        id
    }
}

/// A name no sibling has yet: `rock`, then `rock 2`.
fn unique(name: String, taken: &mut HashSet<String>) -> String {
    let mut candidate = name.clone();
    let mut n = 1;
    while !taken.insert(candidate.clone()) {
        n += 1;
        candidate = format!("{name} {n}");
    }
    candidate
}

/// Import a glTF scene into `library`: its models, materials, textures and
/// prefab. `settings.parts` is read for the IDs to keep and written with
/// the ones used now; assets of parts that are gone are removed. Returns
/// the prefab's ID, the source's own.
pub fn import_scene(
    source: &Path,
    library: &Path,
    settings: &mut ImportSettings,
) -> Result<AssetId> {
    let (document, buffers, images) =
        gltf::import(source).with_context(|| format!("{}", source.display()))?;
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scene".into());
    let source_name = settings.source.clone();
    let mut parts = Parts {
        source: &source_name,
        before: settings.parts.clone(),
        now: BTreeMap::new(),
    };
    let mut written: Vec<(AssetId, Vec<u8>)> = Vec::new();

    // Textures, each built once per way it is read: an image that is both a
    // colour map and something else would be two assets, sRGB and linear.
    let mut textures: HashMap<(usize, bool), AssetId> = HashMap::new();
    let mut texture = |index: usize,
                       srgb: bool,
                       parts: &mut Parts,
                       written: &mut Vec<(AssetId, Vec<u8>)>|
     -> Result<Option<AssetId>> {
        if let Some(id) = textures.get(&(index, srgb)) {
            return Ok(Some(*id));
        }
        let Some(image) = images.get(index) else {
            return Ok(None);
        };
        let Some(pixels) = rgba(image) else {
            return Ok(None);
        };
        let name = document
            .images()
            .nth(index)
            .and_then(|i| i.name().map(str::to_string))
            .unwrap_or_else(|| format!("image {index}"));
        let id = parts.id(format!(
            "texture:{index}:{}",
            if srgb { "colour" } else { "data" }
        ));
        let asset = TextureAsset {
            id,
            name: format!("{stem}/{name}"),
            width: image.width,
            height: image.height,
            mips: build_mips(image.width, image.height, &pixels, srgb),
            pixels,
            srgb,
        };
        written.push((id, runity::asset::to_bytes(&asset, AssetKind::Texture)?));
        textures.insert((index, srgb), id);
        Ok(Some(id))
    };

    // Materials, as the scene names them.
    let mut material_names: Vec<String> = Vec::new();
    let mut taken = HashSet::new();
    for (i, gm) in document.materials().enumerate() {
        let own = unique(
            gm.name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("material {i}")),
            &mut taken,
        );
        let pbr = gm.pbr_metallic_roughness();
        let [r, g, b, a] = pbr.base_color_factor();
        let mut m = Material::new(r, g, b);
        m.alpha = a;
        m.metallic = pbr.metallic_factor();
        m.smoothness = 1.0 - pbr.roughness_factor();
        m.emission = gm.emissive_factor();
        match gm.alpha_mode() {
            gltf::material::AlphaMode::Opaque => {}
            gltf::material::AlphaMode::Mask => m.alpha_clip = gm.alpha_cutoff().unwrap_or(0.5),
            gltf::material::AlphaMode::Blend => m.surface = SurfaceType::Transparent,
        }
        if gm.double_sided() {
            m.render_face = RenderFace::Both;
        }
        if let Some(info) = pbr.base_color_texture() {
            m.base_map = texture(
                info.texture().source().index(),
                true,
                &mut parts,
                &mut written,
            )?;
        }
        if let Some(info) = gm.emissive_texture() {
            m.emission_map = texture(
                info.texture().source().index(),
                true,
                &mut parts,
                &mut written,
            )?;
        }
        if let Some(normal) = gm.normal_texture() {
            m.normal_map = texture(
                normal.texture().source().index(),
                false,
                &mut parts,
                &mut written,
            )?;
            m.normal_scale = normal.scale();
        }
        // glTF keeps metal in blue and roughness in green, occlusion in red
        // of its own map; the engine's mask is metal in red, occlusion in
        // green, smoothness in alpha. Packed here, factors baked in.
        let metal_rough = pbr
            .metallic_roughness_texture()
            .map(|t| t.texture().source().index());
        let occlusion = gm
            .occlusion_texture()
            .map(|t| (t.texture().source().index(), t.strength()));
        if metal_rough.is_some() || occlusion.is_some() {
            let key = format!("mask:{i}");
            if let Some(asset) = mask(
                &images,
                metal_rough,
                occlusion.map(|(o, _)| o),
                m.metallic,
                1.0 - m.smoothness,
                parts.id(key.clone()),
                format!("{stem}/{own} mask"),
            ) {
                if metal_rough.is_some() {
                    m.metallic = 1.0;
                    m.smoothness = 1.0;
                }
                m.mask_map = Some(asset.id);
                if let Some((_, strength)) = occlusion {
                    m.occlusion_strength = strength;
                }
                written.push((
                    asset.id,
                    runity::asset::to_bytes(&asset, AssetKind::Texture)?,
                ));
            }
        }
        let id = parts.id(format!("material:{own}"));
        let name = format!("{stem}/{own}");
        let asset = MaterialAsset {
            id,
            name: name.clone(),
            material: m,
        };
        written.push((id, runity::asset::to_bytes(&asset, AssetKind::Material)?));
        material_names.push(settings.materials.get(&own).cloned().unwrap_or(name));
    }

    // Models: one per primitive of every mesh, shared by every node that
    // draws it.
    let mut models: Vec<Vec<(AssetLink, Option<usize>)>> = Vec::new();
    let mut taken = HashSet::new();
    for (i, mesh) in document.meshes().enumerate() {
        let own = unique(
            mesh.name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("mesh {i}")),
            &mut taken,
        );
        let count = mesh.primitives().len();
        let mut these = Vec::new();
        for (j, primitive) in mesh.primitives().enumerate() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                anyhow::bail!(
                    "{}: mesh `{own}`: primitive mode {:?} is not triangles",
                    source.display(),
                    primitive.mode()
                );
            }
            let name = if count > 1 {
                format!("{stem}/{own}.{j}")
            } else {
                format!("{stem}/{own}")
            };
            let id = parts.id(format!("mesh:{own}.{j}"));
            let (vertices, indices) = triangles(&primitive, &buffers, settings)
                .with_context(|| format!("{}: mesh `{own}`", source.display()))?;
            let asset = MeshAsset {
                id,
                name: name.clone(),
                bounds: Bounds::of(&vertices),
                submeshes: vec![Submesh {
                    first_index: 0,
                    index_count: indices.len() as u32,
                    material: None,
                }],
                vertices,
                indices,
                skin: None,
            };
            written.push((id, runity::asset::to_bytes(&asset, AssetKind::Mesh)?));
            these.push((AssetLink::to(name, id), primitive.material().index()));
        }
        models.push(these);
    }

    // The tree.
    let material_of = |index: Option<usize>| match index.and_then(|i| material_names.get(i)) {
        Some(name) => MaterialRef::Named(name.clone()),
        None => MaterialRef::default(),
    };
    fn entity(
        node: gltf::Node,
        path: &str,
        scale: f32,
        models: &[Vec<(AssetLink, Option<usize>)>],
        material_of: &dyn Fn(Option<usize>) -> MaterialRef,
    ) -> EntityDesc {
        let (t, r, s) = node.transform().decomposed();
        let mut transform = Transform {
            position: Vec3::from_array(t) * scale,
            scale: Vec3::from_array(s),
            ..Default::default()
        };
        transform.set_rotation(Quat::from_array(r));
        let mut desc = EntityDesc {
            id: entity_id(path),
            name: node
                .name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("node {}", node.index())),
            transform,
            ..Default::default()
        };
        if let Some(mesh) = node.mesh().and_then(|m| models.get(m.index())) {
            for (j, (model, material)) in mesh.iter().enumerate() {
                if j == 0 {
                    desc.model = model.clone();
                    desc.material = material_of(*material);
                } else {
                    // The mesh's other materials: drawn by children in
                    // the same place, since an entity draws with one.
                    desc.children.push(EntityDesc {
                        id: entity_id(&format!("{path}#{j}")),
                        name: format!("{} {}", desc.name, j + 1),
                        model: model.clone(),
                        material: material_of(*material),
                        ..Default::default()
                    });
                }
            }
        }
        let mut taken = HashSet::new();
        for child in node.children() {
            let own = unique(
                child
                    .name()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("node {}", child.index())),
                &mut taken,
            );
            desc.children.push(entity(
                child,
                &format!("{path}/{own}"),
                scale,
                models,
                material_of,
            ));
        }
        desc
    }
    let mut root = EntityDesc {
        id: entity_id(""),
        name: stem.clone(),
        ..Default::default()
    };
    let mut taken = HashSet::new();
    for node in roots(&document) {
        let own = unique(
            node.name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("node {}", node.index())),
            &mut taken,
        );
        root.children
            .push(entity(node, &own, settings.scale, &models, &material_of));
    }

    // Written only once all of it has been read: a file that fails halfway
    // leaves the library as it was.
    std::fs::create_dir_all(library)?;
    for (id, bytes) in &written {
        std::fs::write(asset_for(*id, library), bytes)?;
    }
    let id = settings.asset_id();
    let prefab = prefab_for(id, library);
    let _ = std::fs::remove_file(&prefab);
    runity::Prefabs::save(&root, &prefab).map_err(anyhow::Error::msg)?;
    // What an earlier version of the file had and this one does not.
    for (key, old) in &parts.before {
        if !parts.now.contains_key(key) {
            let _ = std::fs::remove_file(asset_for(*old, library));
        }
    }
    settings.parts = parts.now;
    Ok(id)
}

/// A node's entity ID, from its path of names: the same file imported on
/// any machine, or again after an edit elsewhere in it, gives the same.
fn entity_id(path: &str) -> EntityId {
    let raw = AssetId::from_source(path, 0x7265_6500).0;
    EntityId::from_raw(((raw >> 64) as u64 ^ raw as u64).max(1))
}

/// One primitive's triangles in its mesh's own space, scaled.
fn triangles(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    settings: &ImportSettings,
) -> Result<(Vec<Vertex>, Vec<u32>)> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| anyhow::anyhow!("a primitive has no positions"))?
        .collect();
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
    let uvs: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
    let mut vertices: Vec<Vertex> = positions
        .iter()
        .enumerate()
        .map(|(i, p)| Vertex {
            position: (Vec3::from_array(*p) * settings.scale).to_array(),
            normal: normals
                .as_ref()
                .and_then(|n| n.get(i))
                .map(|n| Vec3::from_array(*n).normalize_or_zero().to_array())
                .unwrap_or([0.0; 3]),
            uv: uvs
                .as_ref()
                .and_then(|uv| uv.get(i))
                .copied()
                .unwrap_or([0.0; 2]),
        })
        .collect();
    let indices: Vec<u32> = match reader.read_indices() {
        Some(read) => read.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    if settings.recompute_normals || normals.is_none() {
        recompute_normals(&mut vertices, &indices);
    }
    Ok((vertices, indices))
}

/// An image as RGBA8, from the formats exporters write. `None` for 16-bit
/// and float images, which a colour or mask map does not need.
fn rgba(image: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format;
    let p = &image.pixels;
    Some(match image.format {
        Format::R8G8B8A8 => p.clone(),
        Format::R8G8B8 => p
            .chunks_exact(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        Format::R8G8 => p
            .chunks_exact(2)
            .flat_map(|c| [c[0], c[1], 0, 255])
            .collect(),
        Format::R8 => p.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        _ => return None,
    })
}

/// The engine's mask map from glTF's metallic-roughness and occlusion
/// maps: metal (glTF blue) to red, occlusion (its red) to green, and
/// smoothness — one minus roughness (glTF green) — to alpha, with the
/// factors multiplied in. Occlusion of another size than the rest is left
/// out rather than resampled.
fn mask(
    images: &[gltf::image::Data],
    metal_rough: Option<usize>,
    occlusion: Option<usize>,
    metallic: f32,
    roughness: f32,
    id: AssetId,
    name: String,
) -> Option<TextureAsset> {
    let mr = metal_rough.and_then(|i| Some((images.get(i)?, rgba(images.get(i)?)?)));
    let oc = occlusion.and_then(|i| Some((images.get(i)?, rgba(images.get(i)?)?)));
    let (width, height) = match (&mr, &oc) {
        (Some((image, _)), _) | (None, Some((image, _))) => (image.width, image.height),
        (None, None) => return None,
    };
    let oc = oc.filter(|(image, _)| image.width == width && image.height == height);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for i in 0..(width * height) as usize {
        let (metal, smooth) = match &mr {
            Some((_, p)) => (
                byte(metallic * p[i * 4 + 2] as f32 / 255.0),
                byte(1.0 - roughness * p[i * 4 + 1] as f32 / 255.0),
            ),
            None => (255, 255),
        };
        let occ = oc.as_ref().map_or(255, |(_, p)| p[i * 4]);
        pixels.extend([metal, occ, 0, smooth]);
    }
    Some(TextureAsset {
        id,
        name,
        width,
        height,
        mips: build_mips(width, height, &pixels, false),
        pixels,
        srgb: false,
    })
}
