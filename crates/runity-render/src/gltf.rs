//! Loading glTF 2.0 models.
//!
//! glTF is the format worth implementing: it is JSON plus a blob of vertex
//! data laid out the way a renderer wants it, so there is no mesh
//! optimisation, no compression and no scene-graph philosophy to reverse
//! engineer. The whole loader is a few hundred lines because the format was
//! designed to be read, unlike FBX.
//!
//! What is read: meshes (positions, normals, texture coordinates, indices),
//! the node hierarchy with its transforms, material factors and base-colour
//! textures that happen to be PNG. What is not: animation, skinning, cameras,
//! lights, sparse accessors, and any image format this engine cannot decode —
//! each of which is refused plainly rather than half-loaded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use runity_math::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::color::Color;
use crate::json::{Json, JsonError};
use crate::mesh::Mesh;
use crate::shader::Vertex;
use crate::texture::Texture;

/// Why a model could not be loaded.
#[derive(Clone, Debug, PartialEq)]
pub enum GltfError {
    /// The JSON was not JSON.
    Json(JsonError),
    /// Not a glTF file at all.
    NotGltf,
    /// A glTF of a version this loader does not read.
    Version(String),
    /// A GLB container whose chunks do not add up.
    Container(&'static str),
    /// A buffer's bytes could not be found or read.
    Buffer(String),
    /// Something the file refers to does not exist.
    Missing(&'static str),
    /// A feature this loader does not support.
    Unsupported(String),
}

impl core::fmt::Display for GltfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GltfError::Json(error) => write!(f, "the glTF JSON is malformed: {error}"),
            GltfError::NotGltf => write!(f, "not a glTF document"),
            GltfError::Version(found) => write!(f, "glTF version {found} is not supported"),
            GltfError::Container(reason) => write!(f, "malformed GLB container: {reason}"),
            GltfError::Buffer(what) => write!(f, "could not read a buffer: {what}"),
            GltfError::Missing(what) => write!(f, "the file refers to a {what} that is not there"),
            GltfError::Unsupported(what) => write!(f, "{what} is not supported"),
        }
    }
}

impl std::error::Error for GltfError {}

impl From<JsonError> for GltfError {
    fn from(error: JsonError) -> Self {
        GltfError::Json(error)
    }
}

/// What a material says about a surface.
///
/// Kept separate from [`Material`](crate::Material) because that one borrows
/// its textures; this owns them, and the caller builds the borrowing one when
/// it draws.
#[derive(Clone, Debug)]
pub struct MaterialData {
    /// Name from the file, for finding it again.
    pub name: String,
    /// Base colour, in linear space.
    pub base_color: Color,
    /// How metallic, from 0 to 1.
    pub metallic: f32,
    /// How rough, from 0 to 1.
    pub roughness: f32,
    /// Light the surface gives off.
    pub emissive: Color,
    /// Base-colour texture, when the file carried one this engine can decode.
    pub base_color_texture: Option<Texture>,
}

impl Default for MaterialData {
    fn default() -> Self {
        // glTF's own defaults: white, fully rough, not metal.
        Self {
            name: String::new(),
            base_color: Color::WHITE,
            metallic: 1.0,
            roughness: 1.0,
            emissive: Color::BLACK,
            base_color_texture: None,
        }
    }
}

/// One mesh placed in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// Which mesh of the model.
    pub mesh: usize,
    /// Which material, if the file named one.
    pub material: Option<usize>,
    /// Where it goes, with the node hierarchy already applied.
    pub transform: Mat4,
    /// The node's name, for finding a particular part.
    pub name: String,
}

/// A loaded model: meshes, where they go, and what they are made of.
#[derive(Clone, Debug, Default)]
pub struct Model {
    /// One mesh per glTF primitive, since a primitive is one material's worth
    /// of triangles and that is what a draw call is.
    pub meshes: Vec<Mesh>,
    /// Every placement of every mesh, flattened out of the node tree.
    pub instances: Vec<Instance>,
    /// Materials, indexed by [`Instance::material`].
    pub materials: Vec<MaterialData>,
}

impl Model {
    /// How many triangles the whole model holds, counting instances.
    pub fn triangle_count(&self) -> usize {
        self.instances
            .iter()
            .filter_map(|instance| self.meshes.get(instance.mesh))
            .map(|mesh| mesh.indices.len() / 3)
            .sum()
    }

    /// The bounds of every instance together, in model space.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for instance in &self.instances {
            let Some(mesh) = self.meshes.get(instance.mesh) else {
                continue;
            };
            for vertex in &mesh.vertices {
                let point = instance.transform.transform_point(vertex.position);
                let point = Vec3::new(point.x, point.y, point.z);
                min = min.min(point);
                max = max.max(point);
                any = true;
            }
        }
        if any {
            (min, max)
        } else {
            (Vec3::ZERO, Vec3::ZERO)
        }
    }
}

/// Load a `.gltf` or `.glb` file from disk.
///
/// External buffers and images are resolved relative to the file, which is
/// what the format assumes and what every exporter produces.
pub fn load(path: impl AsRef<Path>) -> Result<Model, GltfError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .map_err(|error| GltfError::Buffer(format!("{}: {error}", path.display())))?;
    let base = path.parent().map(PathBuf::from).unwrap_or_default();
    parse(&bytes, Some(&base))
}

/// Parse a model already in memory.
///
/// `base` is the directory external files are resolved against; without one,
/// a file referring to an external buffer is refused rather than guessed at.
pub fn parse(bytes: &[u8], base: Option<&Path>) -> Result<Model, GltfError> {
    let (document, binary) = split(bytes)?;
    let json = Json::parse(&document)?;
    build(&json, binary, base)
}

/// Take a GLB apart, or hand back the text of a plain glTF.
fn split(bytes: &[u8]) -> Result<(String, Option<Vec<u8>>), GltfError> {
    if bytes.len() >= 12 && &bytes[0..4] == b"glTF" {
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version != 2 {
            return Err(GltfError::Version(version.to_string()));
        }
        let mut position = 12;
        let mut json = None;
        let mut binary = None;
        while position + 8 <= bytes.len() {
            let length = u32::from_le_bytes([
                bytes[position],
                bytes[position + 1],
                bytes[position + 2],
                bytes[position + 3],
            ]) as usize;
            let kind = &bytes[position + 4..position + 8];
            let body = position + 8;
            if body + length > bytes.len() {
                return Err(GltfError::Container("a chunk runs past the end"));
            }
            match kind {
                b"JSON" => {
                    json = Some(
                        String::from_utf8(bytes[body..body + length].to_vec())
                            .map_err(|_| GltfError::Container("the JSON chunk is not UTF-8"))?,
                    )
                }
                b"BIN\0" => binary = Some(bytes[body..body + length].to_vec()),
                _ => {}
            }
            // Chunks are padded to four bytes.
            position = body + length + ((4 - length % 4) % 4);
        }
        let json = json.ok_or(GltfError::Container("no JSON chunk"))?;
        return Ok((json, binary));
    }

    let text = String::from_utf8(bytes.to_vec()).map_err(|_| GltfError::NotGltf)?;
    Ok((text, None))
}

/// Turn a parsed document into a model.
fn build(json: &Json, binary: Option<Vec<u8>>, base: Option<&Path>) -> Result<Model, GltfError> {
    let version = json
        .path(&["asset", "version"])
        .and_then(Json::as_str)
        .ok_or(GltfError::NotGltf)?;
    if !version.starts_with('2') {
        return Err(GltfError::Version(version.to_string()));
    }

    let buffers = read_buffers(json, binary, base)?;
    let views = json
        .get("bufferViews")
        .and_then(Json::as_array)
        .unwrap_or(&[]);
    let accessors = json
        .get("accessors")
        .and_then(Json::as_array)
        .unwrap_or(&[]);
    let materials = read_materials(json, &buffers, views, base)?;

    // Each primitive becomes one mesh: a primitive is one material's worth of
    // triangles, which is exactly what a draw call is.
    let mut meshes = Vec::new();
    let mut primitive_index: Vec<Vec<usize>> = Vec::new();
    let mut primitive_material: Vec<Vec<Option<usize>>> = Vec::new();

    for mesh in json.get("meshes").and_then(Json::as_array).unwrap_or(&[]) {
        let mut indices = Vec::new();
        let mut used_materials = Vec::new();
        for primitive in mesh
            .get("primitives")
            .and_then(Json::as_array)
            .unwrap_or(&[])
        {
            // Mode 4 is TRIANGLES, and is the default when absent.
            let mode = primitive.get("mode").and_then(Json::as_usize).unwrap_or(4);
            if mode != 4 {
                return Err(GltfError::Unsupported(format!("primitive mode {mode}")));
            }
            let built = read_primitive(primitive, &buffers, views, accessors)?;
            indices.push(meshes.len());
            used_materials.push(primitive.get("material").and_then(Json::as_usize));
            meshes.push(built);
        }
        primitive_index.push(indices);
        primitive_material.push(used_materials);
    }

    let nodes = json.get("nodes").and_then(Json::as_array).unwrap_or(&[]);
    let mut instances = Vec::new();
    // Walk from the scene's roots so that node transforms compose; a file
    // without a scene still has nodes, and rendering them at the origin is
    // better than rendering nothing.
    let roots: Vec<usize> = match scene_roots(json) {
        Some(roots) => roots,
        None => (0..nodes.len()).collect(),
    };
    let mut stack: Vec<(usize, Mat4)> = roots
        .into_iter()
        .rev()
        .map(|n| (n, Mat4::IDENTITY))
        .collect();
    let mut visited = vec![false; nodes.len()];

    while let Some((index, parent)) = stack.pop() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        // A node graph with a cycle is not a tree; stop rather than loop.
        if visited.get(index).copied().unwrap_or(true) {
            continue;
        }
        visited[index] = true;

        let world = parent * node_transform(node);
        if let Some(mesh) = node.get("mesh").and_then(Json::as_usize) {
            let name = node
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string();
            let primitives = primitive_index.get(mesh).cloned().unwrap_or_default();
            let materials_for = primitive_material.get(mesh).cloned().unwrap_or_default();
            for (slot, mesh_index) in primitives.into_iter().enumerate() {
                instances.push(Instance {
                    mesh: mesh_index,
                    material: materials_for.get(slot).copied().flatten(),
                    transform: world,
                    name: name.clone(),
                });
            }
        }
        for child in node.get("children").and_then(Json::as_array).unwrap_or(&[]) {
            if let Some(child) = child.as_usize() {
                stack.push((child, world));
            }
        }
    }

    Ok(Model {
        meshes,
        instances,
        materials,
    })
}

/// The indices of the default scene's root nodes.
fn scene_roots(json: &Json) -> Option<Vec<usize>> {
    let scenes = json.get("scenes")?.as_array()?;
    let index = json.get("scene").and_then(Json::as_usize).unwrap_or(0);
    let scene = scenes.get(index)?;
    Some(
        scene
            .get("nodes")?
            .as_array()?
            .iter()
            .filter_map(Json::as_usize)
            .collect(),
    )
}

/// A node's local transform, from either a matrix or its parts.
fn node_transform(node: &Json) -> Mat4 {
    if let Some(values) = node.get("matrix").and_then(Json::as_array) {
        if values.len() == 16 {
            let mut m = [0.0f32; 16];
            for (slot, value) in m.iter_mut().zip(values) {
                *slot = value.as_f64().unwrap_or(0.0) as f32;
            }
            // glTF matrices are column-major, which is this engine's layout
            // too, so they go straight in.
            return Mat4::from_array(m);
        }
    }

    let translation = node
        .get("translation")
        .and_then(Json::as_array)
        .filter(|values| values.len() == 3)
        .map(|values| {
            Vec3::new(
                values[0].as_f64().unwrap_or(0.0) as f32,
                values[1].as_f64().unwrap_or(0.0) as f32,
                values[2].as_f64().unwrap_or(0.0) as f32,
            )
        })
        .unwrap_or(Vec3::ZERO);
    let rotation = node
        .get("rotation")
        .and_then(Json::as_array)
        .filter(|values| values.len() == 4)
        .map(|values| Quat {
            x: values[0].as_f64().unwrap_or(0.0) as f32,
            y: values[1].as_f64().unwrap_or(0.0) as f32,
            z: values[2].as_f64().unwrap_or(0.0) as f32,
            w: values[3].as_f64().unwrap_or(1.0) as f32,
        })
        .unwrap_or(Quat::IDENTITY);
    let scale = node
        .get("scale")
        .and_then(Json::as_array)
        .filter(|values| values.len() == 3)
        .map(|values| {
            Vec3::new(
                values[0].as_f64().unwrap_or(1.0) as f32,
                values[1].as_f64().unwrap_or(1.0) as f32,
                values[2].as_f64().unwrap_or(1.0) as f32,
            )
        })
        .unwrap_or(Vec3::ONE);

    Mat4::from_translation(translation) * rotation.to_mat4() * Mat4::from_scale(scale)
}

/// Every buffer's bytes: embedded, external, or the GLB's own chunk.
fn read_buffers(
    json: &Json,
    binary: Option<Vec<u8>>,
    base: Option<&Path>,
) -> Result<Vec<Vec<u8>>, GltfError> {
    let mut buffers = Vec::new();
    for buffer in json.get("buffers").and_then(Json::as_array).unwrap_or(&[]) {
        match buffer.get("uri").and_then(Json::as_str) {
            None => {
                // No URI means the GLB's binary chunk, and there is only one.
                let bytes = binary.clone().ok_or_else(|| {
                    GltfError::Buffer("a buffer with no URI and no GLB chunk".into())
                })?;
                buffers.push(bytes);
            }
            Some(uri) if uri.starts_with("data:") => {
                let comma = uri
                    .find(',')
                    .ok_or_else(|| GltfError::Buffer("a malformed data URI".into()))?;
                if !uri[..comma].ends_with(";base64") {
                    return Err(GltfError::Unsupported(
                        "a data URI that is not base64".into(),
                    ));
                }
                buffers.push(base64(&uri[comma + 1..])?);
            }
            Some(uri) => {
                let base = base.ok_or_else(|| {
                    GltfError::Buffer(format!(
                        "{uri} is external and there is no directory to look in"
                    ))
                })?;
                let path = base.join(percent_decode(uri));
                buffers.push(
                    std::fs::read(&path).map_err(|error| {
                        GltfError::Buffer(format!("{}: {error}", path.display()))
                    })?,
                );
            }
        }
    }
    Ok(buffers)
}

/// Materials, with their textures decoded where possible.
fn read_materials(
    json: &Json,
    buffers: &[Vec<u8>],
    views: &[Json],
    base: Option<&Path>,
) -> Result<Vec<MaterialData>, GltfError> {
    let images = json.get("images").and_then(Json::as_array).unwrap_or(&[]);
    let textures = json.get("textures").and_then(Json::as_array).unwrap_or(&[]);
    let mut decoded: HashMap<usize, Option<Texture>> = HashMap::new();

    let mut materials = Vec::new();
    for material in json
        .get("materials")
        .and_then(Json::as_array)
        .unwrap_or(&[])
    {
        let mut data = MaterialData {
            name: material
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            ..MaterialData::default()
        };

        if let Some(pbr) = material.get("pbrMetallicRoughness") {
            if let Some(factor) = pbr.get("baseColorFactor").and_then(Json::as_array) {
                if factor.len() >= 3 {
                    data.base_color = Color::rgb(
                        factor[0].as_f64().unwrap_or(1.0) as f32,
                        factor[1].as_f64().unwrap_or(1.0) as f32,
                        factor[2].as_f64().unwrap_or(1.0) as f32,
                    );
                }
            }
            if let Some(value) = pbr.get("metallicFactor").and_then(Json::as_f64) {
                data.metallic = value as f32;
            }
            if let Some(value) = pbr.get("roughnessFactor").and_then(Json::as_f64) {
                data.roughness = value as f32;
            }
            if let Some(index) = pbr
                .path(&["baseColorTexture", "index"])
                .and_then(Json::as_usize)
            {
                let source = textures
                    .get(index)
                    .and_then(|texture| texture.get("source").and_then(Json::as_usize));
                if let Some(source) = source {
                    let texture = decoded
                        .entry(source)
                        .or_insert_with(|| read_image(images.get(source), buffers, views, base));
                    data.base_color_texture = texture.clone();
                }
            }
        }
        if let Some(factor) = material.get("emissiveFactor").and_then(Json::as_array) {
            if factor.len() >= 3 {
                data.emissive = Color::rgb(
                    factor[0].as_f64().unwrap_or(0.0) as f32,
                    factor[1].as_f64().unwrap_or(0.0) as f32,
                    factor[2].as_f64().unwrap_or(0.0) as f32,
                );
            }
        }
        materials.push(data);
    }
    Ok(materials)
}

/// Decode one image, if this engine can read its format.
///
/// A JPEG is simply skipped: the material keeps its factors and loses its
/// texture, which is better than refusing to load the model at all.
fn read_image(
    image: Option<&Json>,
    buffers: &[Vec<u8>],
    views: &[Json],
    base: Option<&Path>,
) -> Option<Texture> {
    let image = image?;
    let bytes: Vec<u8> = match image.get("uri").and_then(Json::as_str) {
        Some(uri) if uri.starts_with("data:") => {
            let comma = uri.find(',')?;
            base64(&uri[comma + 1..]).ok()?
        }
        Some(uri) => std::fs::read(base?.join(percent_decode(uri))).ok()?,
        None => {
            let view = views.get(image.get("bufferView").and_then(Json::as_usize)?)?;
            let buffer = buffers.get(view.get("buffer").and_then(Json::as_usize)?)?;
            let offset = view.get("byteOffset").and_then(Json::as_usize).unwrap_or(0);
            let length = view.get("byteLength").and_then(Json::as_usize)?;
            buffer.get(offset..offset + length)?.to_vec()
        }
    };

    let decoded = crate::png::decode_png(&bytes).ok()?;
    Some(Texture::from_fn(decoded.width, decoded.height, |x, y| {
        Color::from_srgb8(decoded.pixels[y * decoded.width + x])
    }))
}

/// One primitive's vertices and indices.
fn read_primitive(
    primitive: &Json,
    buffers: &[Vec<u8>],
    views: &[Json],
    accessors: &[Json],
) -> Result<Mesh, GltfError> {
    let attributes = primitive
        .get("attributes")
        .ok_or(GltfError::Missing("primitive attribute"))?;
    let position_index = attributes
        .get("POSITION")
        .and_then(Json::as_usize)
        .ok_or(GltfError::Missing("POSITION attribute"))?;

    let positions = read_accessor(position_index, buffers, views, accessors)?;
    let normals = attributes
        .get("NORMAL")
        .and_then(Json::as_usize)
        .map(|index| read_accessor(index, buffers, views, accessors))
        .transpose()?;
    let uvs = attributes
        .get("TEXCOORD_0")
        .and_then(Json::as_usize)
        .map(|index| read_accessor(index, buffers, views, accessors))
        .transpose()?;

    let count = positions.len() / 3;
    let mut vertices = Vec::with_capacity(count);
    for index in 0..count {
        let position = Vec3::new(
            positions[index * 3],
            positions[index * 3 + 1],
            positions[index * 3 + 2],
        );
        let normal = normals
            .as_ref()
            .filter(|values| values.len() >= (index + 1) * 3)
            .map(|values| {
                Vec3::new(
                    values[index * 3],
                    values[index * 3 + 1],
                    values[index * 3 + 2],
                )
            })
            .unwrap_or(Vec3::Y);
        let uv = uvs
            .as_ref()
            .filter(|values| values.len() >= (index + 1) * 2)
            .map(|values| Vec2::new(values[index * 2], values[index * 2 + 1]))
            .unwrap_or(Vec2::ZERO);
        vertices.push(Vertex {
            position,
            normal,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            uv_density: 1.0,
            uv,
            color: Color::WHITE,
        });
    }

    let indices = match primitive.get("indices").and_then(Json::as_usize) {
        Some(index) => read_accessor(index, buffers, views, accessors)?
            .into_iter()
            .map(|value| value as u32)
            .collect(),
        // Without an index buffer the vertices are the triangles, in order.
        None => (0..count as u32).collect(),
    };

    let mut mesh = Mesh::new(vertices, indices);
    if normals.is_none() {
        // A file without normals is legal, and unlit without them.
        mesh.recompute_normals();
    }
    mesh.recompute_tangents();
    Ok(mesh)
}

/// Read an accessor as floats, whatever it is stored as.
fn read_accessor(
    index: usize,
    buffers: &[Vec<u8>],
    views: &[Json],
    accessors: &[Json],
) -> Result<Vec<f32>, GltfError> {
    let accessor = accessors.get(index).ok_or(GltfError::Missing("accessor"))?;
    if accessor.get("sparse").is_some() {
        return Err(GltfError::Unsupported("sparse accessors".into()));
    }
    let count = accessor.get("count").and_then(Json::as_usize).unwrap_or(0);
    let component = accessor
        .get("componentType")
        .and_then(Json::as_usize)
        .ok_or(GltfError::Missing("componentType"))?;
    let kind = accessor
        .get("type")
        .and_then(Json::as_str)
        .unwrap_or("SCALAR");
    let components = match kind {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        other => return Err(GltfError::Unsupported(format!("accessor type {other}"))),
    };
    let size = match component {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        other => return Err(GltfError::Unsupported(format!("component type {other}"))),
    };

    let Some(view_index) = accessor.get("bufferView").and_then(Json::as_usize) else {
        // A bufferView-less accessor is all zeroes, by the specification.
        return Ok(vec![0.0; count * components]);
    };
    let view = views
        .get(view_index)
        .ok_or(GltfError::Missing("bufferView"))?;
    let buffer = buffers
        .get(view.get("buffer").and_then(Json::as_usize).unwrap_or(0))
        .ok_or(GltfError::Missing("buffer"))?;

    let view_offset = view.get("byteOffset").and_then(Json::as_usize).unwrap_or(0);
    let accessor_offset = accessor
        .get("byteOffset")
        .and_then(Json::as_usize)
        .unwrap_or(0);
    // A stride lets attributes be interleaved; without one they are packed.
    let stride = view
        .get("byteStride")
        .and_then(Json::as_usize)
        .unwrap_or(size * components);

    let mut values = Vec::with_capacity(count * components);
    for element in 0..count {
        let start = view_offset + accessor_offset + element * stride;
        for component_index in 0..components {
            let at = start + component_index * size;
            let bytes = buffer
                .get(at..at + size)
                .ok_or_else(|| GltfError::Buffer("an accessor reads past the end".into()))?;
            values.push(decode_component(component, bytes));
        }
    }
    Ok(values)
}

/// One component, converted to a float.
///
/// Normalized integer attributes are not rescaled here: positions and indices
/// are what this loader reads, and neither is ever normalized.
fn decode_component(component: usize, bytes: &[u8]) -> f32 {
    match component {
        5120 => bytes[0] as i8 as f32,
        5121 => bytes[0] as f32,
        5122 => i16::from_le_bytes([bytes[0], bytes[1]]) as f32,
        5123 => u16::from_le_bytes([bytes[0], bytes[1]]) as f32,
        5125 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
        _ => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    }
}

/// Decode base64, the only encoding a glTF data URI may use.
fn base64(text: &str) -> Result<Vec<u8>, GltfError> {
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => u32::from(byte - b'A'),
            b'a'..=b'z' => u32::from(byte - b'a') + 26,
            b'0'..=b'9' => u32::from(byte - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };

    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let Some(digit) = value(byte) else {
            return Err(GltfError::Buffer(
                "a data URI contains something that is not base64".into(),
            ));
        };
        accumulator = (accumulator << 6) | digit;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    Ok(out)
}

/// Undo the percent-escaping exporters put in file names.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = |byte: u8| -> Option<u8> {
                Some(match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    _ => return None,
                })
            };
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}
