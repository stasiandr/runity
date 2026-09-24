//! A source that is a scene, not a model: a glTF or a `.blend` with many
//! objects in it — a level, a kit — imported as the tree it is
//! (docs/blender.md).
//!
//! [`crate::mesh_from_gltf`] merges a whole file into one mesh, which is
//! right for a tree made of a trunk and a crown and wrong for a forest: a
//! hundred copies of one rock become one mesh with a hundred rocks' worth
//! of triangles, and nothing in it can be selected, moved or given a
//! component. Here instead:
//!
//! * every mesh becomes a model — one per material, since an entity draws
//!   with one — named `<file>/<mesh>`. A mesh a hundred objects share is
//!   **one** model, and the renderer draws its copies as instances: the
//!   instancing the artist set up survives the trip;
//! * every material becomes a material asset, `<file>/<material>`, URP Lit
//!   from the metallic-roughness model, with its images as textures; a
//!   file-wide override in the `.scrimport` (`materials:`) swaps one for a
//!   material of the project's;
//! * the objects become a prefab named after the file, written into the
//!   library as `<asset id>.prefab`: derived, never committed, found by
//!   [`scrap::Prefabs::of`] like a hand-written one. A scene places it
//!   with one line, `prefab: "forest"`. What a `.blend` marks as an asset
//!   is a prefab of its own, by its name — a kit.
//!
//! Two readers, one builder: [`from_gltf`] and [`crate::blend`] each turn
//! their file into a [`SceneData`] in the engine's space, and
//! [`build`] writes the assets. Every asset's ID is kept in the source's
//! `.scrimport` under `parts`, by a key made of what the file calls it, so
//! re-importing a changed file updates assets rather than orphaning them.
//! An object's entity ID is the one Blender's plugin stamped on it, or else
//! derived from its path of names, so overrides of a part survive a
//! re-import.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use glam::{Quat, Vec3};
use scrap::asset::{
    AssetId, Bounds, MaterialAsset, MeshAsset, Submesh, TextureAsset, Vertex,
};
use scrap::material::{Material, RenderFace, SurfaceType};
use scrap::scene::{Body, Collider, MaterialRef};
use scrap::{AssetLink, EntityDesc, EntityId, Transform};

use crate::{asset_for, build_mips, recompute_normals, ImportSettings};

/// A scene as a reader found it, in the engine's space (Y up, metres
/// before the import's `scale`), before any asset is written.
#[derive(Debug, Default)]
pub struct SceneData {
    pub images: Vec<ImageData>,
    pub materials: Vec<MaterialData>,
    pub meshes: Vec<MeshData>,
    /// The file's own tree: the level.
    pub level: Vec<NodeData>,
    /// What the file marks as assets: each a prefab of its own.
    pub assets: Vec<AssetData>,
}

/// A picture: RGBA8, top row first.
#[derive(Debug)]
pub struct ImageData {
    pub key: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A material in the metallic-roughness model, with the images it reads.
#[derive(Debug, Default)]
pub struct MaterialData {
    pub key: String,
    pub name: String,
    /// Its factors: colour, metal, emission, alpha, faces. `smoothness`
    /// here is one minus the file's roughness.
    pub material: Material,
    pub base_map: Option<usize>,
    pub emission_map: Option<usize>,
    pub normal_map: Option<usize>,
    /// Metal and roughness each from a channel of an image — glTF packs
    /// both into one (blue and green), Blender often has two.
    pub metallic_map: Option<(usize, usize)>,
    pub roughness_map: Option<(usize, usize)>,
    pub occlusion_map: Option<(usize, usize)>,
}

#[derive(Debug)]
pub struct MeshData {
    pub key: String,
    pub name: String,
    pub primitives: Vec<PrimitiveData>,
}

/// Triangles drawn with one material.
#[derive(Debug)]
pub struct PrimitiveData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
    /// Whether the file gave normals; without them they are computed.
    pub has_normals: bool,
}

#[derive(Debug, Default)]
pub struct NodeData {
    pub name: String,
    /// The ID Blender's plugin stamped on the object, if it did.
    pub id: Option<EntityId>,
    pub transform: Transform,
    pub mesh: Option<usize>,
    /// An instance of a prefab: another file's asset, or this one's.
    pub prefab: Option<String>,
    /// The game's components, by name, their values in RON.
    pub components: BTreeMap<String, String>,
    /// Collides as its own shape: a static body with a `Model` collider.
    pub collider: bool,
    /// Draws with this material of the project's, whatever the file's is.
    pub material: Option<String>,
    pub children: Vec<NodeData>,
}

#[derive(Debug)]
pub struct AssetData {
    pub key: String,
    pub name: String,
    pub roots: Vec<NodeData>,
}

/// Whether a glTF is a scene rather than a model: more than one node with
/// a mesh. Decided once, on the first import, and kept in the sidecar
/// (`scene: true`), so a file that grows a second object does not quietly
/// turn from the model scenes name into a prefab they do not. A `.blend`
/// is always a scene.
pub fn is_scene(path: &Path) -> Result<bool> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("blend"))
    {
        return Ok(true);
    }
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
    library.join(format!("{id}.{}", scrap::prefab::EXTENSION))
}

/// Import a scene source — glTF or `.blend` — into `library`.
pub fn import_scene(
    source: &Path,
    library: &Path,
    settings: &mut ImportSettings,
) -> Result<AssetId> {
    let data = if source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("blend"))
    {
        crate::blend::read(source)?
    } else {
        from_gltf(source)?
    };
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scene".into());
    build(data, &stem, library, settings).with_context(|| format!("{}", source.display()))
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

/// Write a scene's assets into `library`: its models, materials, textures
/// and prefabs. `settings.parts` is read for the IDs to keep and written
/// with the ones used now; assets of parts that are gone are removed.
/// Returns the level prefab's ID, the source's own.
pub fn build(
    data: SceneData,
    stem: &str,
    library: &Path,
    settings: &mut ImportSettings,
) -> Result<AssetId> {
    let source_name = settings.source.clone();
    let mut parts = Parts {
        source: &source_name,
        before: settings.parts.clone(),
        now: BTreeMap::new(),
    };
    let mut written: Vec<(AssetId, Vec<u8>)> = Vec::new();
    let mut prefabs: Vec<(AssetId, EntityDesc)> = Vec::new();

    // Textures, each built once per way it is read: an image that is both a
    // colour map and something else is two assets, sRGB and linear.
    let mut textures: BTreeMap<(usize, bool), AssetId> = BTreeMap::new();
    let mut texture = |index: usize,
                       srgb: bool,
                       parts: &mut Parts,
                       written: &mut Vec<(AssetId, Vec<u8>)>|
     -> Result<Option<AssetId>> {
        if let Some(id) = textures.get(&(index, srgb)) {
            return Ok(Some(*id));
        }
        let Some(image) = data.images.get(index) else {
            return Ok(None);
        };
        let id = parts.id(format!(
            "texture:{}:{}",
            image.key,
            if srgb { "colour" } else { "data" }
        ));
        let asset = TextureAsset {
            id,
            name: format!("{stem}/{}", image.name),
            width: image.width,
            height: image.height,
            mips: build_mips(image.width, image.height, &image.rgba, srgb),
            pixels: image.rgba.clone(),
            srgb,
        };
        written.push((id, scrap::asset::to_bytes(&asset, scrap::asset::TEXTURE)?));
        textures.insert((index, srgb), id);
        Ok(Some(id))
    };

    // Materials, as the file names them.
    let mut material_names: Vec<String> = Vec::new();
    let mut taken = HashSet::new();
    for md in &data.materials {
        let own = unique(md.name.clone(), &mut taken);
        let mut m = md.material;
        if let Some(i) = md.base_map {
            m.base_map = texture(i, true, &mut parts, &mut written)?;
        }
        if let Some(i) = md.emission_map {
            m.emission_map = texture(i, true, &mut parts, &mut written)?;
        }
        if let Some(i) = md.normal_map {
            m.normal_map = texture(i, false, &mut parts, &mut written)?;
        }
        // The engine's mask is metal in red, occlusion in green and
        // smoothness in alpha; packed here from wherever the file keeps
        // them, with the factors baked in.
        if md.metallic_map.is_some() || md.roughness_map.is_some() || md.occlusion_map.is_some() {
            let id = parts.id(format!("mask:{}", md.key));
            if let Some(asset) = mask(&data.images, md, id, format!("{stem}/{own} mask")) {
                if md.metallic_map.is_some() {
                    m.metallic = 1.0;
                }
                if md.roughness_map.is_some() {
                    m.smoothness = 1.0;
                }
                m.mask_map = Some(asset.id);
                written.push((
                    asset.id,
                    scrap::asset::to_bytes(&asset, scrap::asset::TEXTURE)?,
                ));
            }
        }
        let id = parts.id(format!("material:{}", md.key));
        let name = format!("{stem}/{own}");
        let asset = MaterialAsset {
            id,
            name: name.clone(),
            material: m,
        };
        written.push((id, scrap::asset::to_bytes(&asset, scrap::asset::MATERIAL)?));
        material_names.push(settings.materials.get(&own).cloned().unwrap_or(name));
    }

    // Models: one per material of every mesh, shared by every object that
    // draws it.
    let mut models: Vec<Vec<(AssetLink, Option<usize>)>> = Vec::new();
    let mut taken = HashSet::new();
    for mesh in data.meshes {
        let own = unique(mesh.name.clone(), &mut taken);
        let count = mesh.primitives.len();
        let mut these = Vec::new();
        for (j, mut primitive) in mesh.primitives.into_iter().enumerate() {
            let name = if count > 1 {
                format!("{stem}/{own}.{j}")
            } else {
                format!("{stem}/{own}")
            };
            let id = parts.id(format!("mesh:{}.{j}", mesh.key));
            for v in &mut primitive.vertices {
                v.position = (Vec3::from_array(v.position) * settings.scale).to_array();
            }
            if settings.recompute_normals || !primitive.has_normals {
                recompute_normals(&mut primitive.vertices, &primitive.indices);
            }
            let asset = MeshAsset {
                id,
                name: name.clone(),
                bounds: Bounds::of(&primitive.vertices),
                submeshes: vec![Submesh {
                    first_index: 0,
                    index_count: primitive.indices.len() as u32,
                    material: None,
                }],
                vertices: primitive.vertices,
                indices: primitive.indices,
                skin: None,
                look: None,
            };
            written.push((id, scrap::asset::to_bytes(&asset, scrap::asset::MESH)?));
            these.push((AssetLink::to(name, id), primitive.material));
        }
        models.push(these);
    }

    // The trees: the level, then each asset the file marks.
    let tree = Tree {
        models: &models,
        materials: &material_names,
        scale: settings.scale,
    };
    let mut level = EntityDesc {
        id: entity_id(""),
        name: stem.to_string(),
        ..Default::default()
    };
    tree.children(&mut level, data.level, "");
    prefabs.push((settings.asset_id(), level));
    for asset in data.assets {
        let id = parts.id(format!("prefab:{}", asset.key));
        let mut root = EntityDesc {
            id: entity_id(&format!("asset:{}", asset.key)),
            name: asset.name,
            ..Default::default()
        };
        tree.children(&mut root, asset.roots, "");
        prefabs.push((id, root));
    }

    // Written only once all of it has been read: a file that fails halfway
    // leaves the library as it was.
    std::fs::create_dir_all(library)?;
    for (id, bytes) in &written {
        std::fs::write(asset_for(*id, library), bytes)?;
    }
    for (id, root) in &prefabs {
        let path = prefab_for(*id, library);
        let _ = std::fs::remove_file(&path);
        scrap::Prefabs::save(root, &path).map_err(anyhow::Error::msg)?;
    }
    // What an earlier version of the file had and this one does not.
    for (key, old) in &parts.before {
        if !parts.now.contains_key(key) {
            let _ = std::fs::remove_file(asset_for(*old, library));
            let _ = std::fs::remove_file(prefab_for(*old, library));
        }
    }
    settings.parts = parts.now;
    Ok(settings.asset_id())
}

/// What turning nodes into entities needs to know.
struct Tree<'a> {
    models: &'a [Vec<(AssetLink, Option<usize>)>],
    materials: &'a [String],
    scale: f32,
}

impl Tree<'_> {
    fn material(&self, index: Option<usize>, own: Option<&str>) -> MaterialRef {
        if let Some(name) = own {
            return MaterialRef::Named(AssetLink::named(name));
        }
        match index.and_then(|i| self.materials.get(i)) {
            Some(name) => MaterialRef::Named(AssetLink::named(name.as_str())),
            None => MaterialRef::default(),
        }
    }

    fn children(&self, parent: &mut EntityDesc, nodes: Vec<NodeData>, path: &str) {
        let mut taken = HashSet::new();
        for node in nodes {
            let own = unique(node.name.clone(), &mut taken);
            let path = if path.is_empty() {
                own
            } else {
                format!("{path}/{own}")
            };
            parent.children.push(self.entity(node, &path));
        }
    }

    fn entity(&self, node: NodeData, path: &str) -> EntityDesc {
        let mut transform = node.transform;
        transform.position *= self.scale;
        let mut desc = EntityDesc {
            id: node.id.unwrap_or_else(|| entity_id(path)),
            name: node.name,
            transform,
            ..Default::default()
        };
        if let Some(prefab) = node.prefab {
            desc.prefab = AssetLink::named(prefab);
        }
        if let Some(mesh) = node.mesh.and_then(|m| self.models.get(m)) {
            for (j, (model, material)) in mesh.iter().enumerate() {
                if j == 0 {
                    desc.set_model(model.clone());
                    desc.set_material(self.material(*material, node.material.as_deref()));
                } else {
                    // The mesh's other materials: drawn by children in
                    // the same place, since an entity draws with one.
                    let mut part = EntityDesc {
                        id: desc.id.within(EntityId::from_raw(j as u64)),
                        name: format!("{} {}", desc.name, j + 1),
                        ..Default::default()
                    };
                    part.set_model(model.clone());
                    part.set_material(self.material(*material, node.material.as_deref()));
                    desc.children.push(part);
                }
            }
        }
        if node.collider {
            desc.set_part(&Body::Static);
            desc.set_part(&Collider::Model);
        }
        for (name, value) in node.components {
            if let Ok(value) = ron::value::RawValue::from_boxed_ron(value.into_boxed_str()) {
                desc.components.insert(name, value);
            }
        }
        self.children(&mut desc, node.children, path);
        desc
    }
}

/// A node's entity ID, from its path of names: the same file imported on
/// any machine, or again after an edit elsewhere in it, gives the same.
fn entity_id(path: &str) -> EntityId {
    let raw = AssetId::from_source(path, 0x7265_6500).0;
    EntityId::from_raw(((raw >> 64) as u64 ^ raw as u64).max(1))
}

/// The engine's mask map: metal in red, occlusion in green, smoothness in
/// alpha, each from its channel of its image with the factors multiplied
/// in. The first image's size is the mask's; a map of another size is left
/// out rather than resampled.
fn mask(
    images: &[ImageData],
    md: &MaterialData,
    id: AssetId,
    name: String,
) -> Option<TextureAsset> {
    let first = [md.metallic_map, md.roughness_map, md.occlusion_map]
        .into_iter()
        .flatten()
        .find_map(|(i, _)| images.get(i))?;
    let (width, height) = (first.width, first.height);
    let pick = |map: Option<(usize, usize)>| {
        map.and_then(|(i, channel)| {
            let image = images.get(i)?;
            (image.width == width && image.height == height).then_some((image, channel.min(3)))
        })
    };
    let (metal, rough, occlusion) = (
        pick(md.metallic_map),
        pick(md.roughness_map),
        pick(md.occlusion_map),
    );
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let at = |map: Option<(&ImageData, usize)>, i: usize| {
        map.map(|(image, c)| image.rgba[i * 4 + c] as f32 / 255.0)
    };
    let roughness = 1.0 - md.material.smoothness;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for i in 0..(width * height) as usize {
        let m = at(metal, i).map_or(255, |v| byte(md.material.metallic * v));
        let s = at(rough, i).map_or(255, |v| byte(1.0 - roughness * v));
        let o = at(occlusion, i).map_or(255, byte);
        pixels.extend([m, o, 0, s]);
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

/// Read a glTF into a [`SceneData`]. glTF is already Y up in metres.
pub fn from_gltf(source: &Path) -> Result<SceneData> {
    let (document, buffers, images) =
        gltf::import(source).with_context(|| format!("{}", source.display()))?;
    let mut data = SceneData::default();

    // Images an exporter writes: 8-bit. 16-bit and float ones a colour or
    // mask map does not need, and are left out.
    let mut image_index: Vec<Option<usize>> = Vec::new();
    for (i, (image, info)) in images.iter().zip(document.images()).enumerate() {
        image_index.push(rgba(image).map(|rgba| {
            data.images.push(ImageData {
                key: i.to_string(),
                name: info
                    .name()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("image {i}")),
                width: image.width,
                height: image.height,
                rgba,
            });
            data.images.len() - 1
        }));
    }
    let image = |t: gltf::Texture| image_index.get(t.source().index()).copied().flatten();

    let mut taken = HashSet::new();
    for (i, gm) in document.materials().enumerate() {
        let name = unique(
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
        let mut md = MaterialData {
            key: name.clone(),
            name,
            ..Default::default()
        };
        md.base_map = pbr.base_color_texture().and_then(|t| image(t.texture()));
        md.emission_map = gm.emissive_texture().and_then(|t| image(t.texture()));
        if let Some(normal) = gm.normal_texture() {
            md.normal_map = image(normal.texture());
            m.normal_scale = normal.scale();
        }
        // glTF keeps metal in blue and roughness in green of one map, and
        // occlusion in red of its own.
        if let Some(mr) = pbr
            .metallic_roughness_texture()
            .and_then(|t| image(t.texture()))
        {
            md.metallic_map = Some((mr, 2));
            md.roughness_map = Some((mr, 1));
        }
        if let Some(occlusion) = gm.occlusion_texture() {
            md.occlusion_map = image(occlusion.texture()).map(|i| (i, 0));
            m.occlusion_strength = occlusion.strength();
        }
        md.material = m;
        data.materials.push(md);
    }

    let mut taken = HashSet::new();
    for (i, mesh) in document.meshes().enumerate() {
        let name = unique(
            mesh.name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("mesh {i}")),
            &mut taken,
        );
        let mut primitives = Vec::new();
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                anyhow::bail!(
                    "mesh `{name}`: primitive mode {:?} is not triangles",
                    primitive.mode()
                );
            }
            primitives
                .push(triangles(&primitive, &buffers).with_context(|| format!("mesh `{name}`"))?);
        }
        data.meshes.push(MeshData {
            key: name.clone(),
            name,
            primitives,
        });
    }

    fn node(n: gltf::Node) -> NodeData {
        let (t, r, s) = n.transform().decomposed();
        let mut transform = Transform {
            position: Vec3::from_array(t),
            scale: Vec3::from_array(s),
            ..Default::default()
        };
        transform.set_rotation(Quat::from_array(r));
        NodeData {
            name: n
                .name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("node {}", n.index())),
            transform,
            mesh: n.mesh().map(|m| m.index()),
            children: n.children().map(node).collect(),
            ..Default::default()
        }
    }
    data.level = roots(&document).into_iter().map(node).collect();
    Ok(data)
}

/// One primitive's triangles in its mesh's own space.
fn triangles(primitive: &gltf::Primitive, buffers: &[gltf::buffer::Data]) -> Result<PrimitiveData> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| anyhow::anyhow!("a primitive has no positions"))?
        .collect();
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
    let uvs: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
    let vertices: Vec<Vertex> = positions
        .iter()
        .enumerate()
        .map(|(i, p)| Vertex {
            position: *p,
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
    Ok(PrimitiveData {
        vertices,
        indices,
        material: primitive.material().index(),
        has_normals: normals.is_some(),
    })
}

/// An image as RGBA8, from the formats exporters write.
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
