//! A Mesh Unity keeps as its own asset (`.asset`, class 43) or inside a
//! scene (ProBuilder's) as a `.glb`: exported terrain, meshes made in the
//! editor. Its vertices are a hex string in the YAML, laid out by channels.

use anyhow::{bail, Context, Result};
use yaml_rust2::Yaml;

use super::yaml::{self, Get};

/// Whether a `.asset` holds a Mesh: its first document says so.
pub fn is_mesh_asset(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 256];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let n = file.read(&mut head).unwrap_or(0);
    String::from_utf8_lossy(&head[..n]).contains("\nMesh:")
}

/// The text after `key: ` on its line: the hex strings, read as text (as
/// YAML, a string of digits would be a number).
fn raw<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mark = format!("{key}: ");
    text.lines()
        .find_map(|l| l.trim_start().strip_prefix(&mark))
        .map(str::trim)
}

fn hex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("odd hex string");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("not hex"))
        .collect()
}

/// A vertex attribute as Unity lays it: which stream, where in the
/// vertex, what number type, how many.
#[derive(Clone, Copy, Debug)]
struct Channel {
    stream: usize,
    offset: usize,
    format: i64,
    dimension: usize,
}

/// The mesh as triangles in scrap's frame (Unity's z mirrored), with
/// glTF's v (down from the top). The winding stays: Unity's front face is
/// clockwise in its left-handed frame, and mirrored — the camera with it —
/// that is counter-clockwise in a right-handed one, glTF's front.
pub struct Mesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub colors: Vec<[u8; 4]>,
    /// One index list per sub-mesh.
    pub submeshes: Vec<Vec<u32>>,
}

/// The Mesh document `body`, whose text is `text`.
pub fn read(body: &Yaml, text: &str) -> Result<Mesh> {
    let name = body.str("m_Name").unwrap_or("mesh").to_string();
    if body.i64("m_MeshCompression").unwrap_or(0) != 0 {
        bail!("{name}: a compressed mesh");
    }
    let data = &body["m_VertexData"];
    let count = data.i64("m_VertexCount").context("no vertex count")? as usize;
    let channels: Vec<Channel> = data
        .list("m_Channels")
        .iter()
        .map(|c| Channel {
            stream: c.i64("stream").unwrap_or(0) as usize,
            offset: c.i64("offset").unwrap_or(0) as usize,
            format: c.i64("format").unwrap_or(0),
            dimension: c.i64("dimension").unwrap_or(0) as usize & 0xf,
        })
        .collect();
    let bytes = hex(raw(text, "_typelessdata").unwrap_or(""))?;
    // Channel order: Unity 2018 on — position, normal, tangent, colour,
    // eight UVs (and skin weights and indices); before — position,
    // normal, colour, four UVs, tangent. Formats then: 0 float, 1 half,
    // 2 unorm8; before, 2 was the colour's four bytes too.
    let modern = channels.len() >= 12;
    let (normal, color, uv0) = if modern { (1, 3, 4) } else { (1, 2, 3) };
    let size = |c: &Channel| {
        c.dimension
            * match (c.format, modern) {
                (0, _) => 4,
                (1, _) => 2,
                (2 | 3 | 6 | 7, true) => 1,
                (2, false) => 1,
                (4 | 5 | 8 | 9, true) => 2,
                _ => 4,
            }
    };
    // Each stream's stride is where its last channel ends; streams follow
    // one another, each started on 16 bytes.
    let streams = channels.iter().map(|c| c.stream + 1).max().unwrap_or(1);
    let mut strides = vec![0usize; streams];
    for c in channels.iter().filter(|c| c.dimension > 0) {
        strides[c.stream] = strides[c.stream].max(c.offset + size(c));
    }
    let mut starts = vec![0usize; streams];
    let mut at = 0;
    for s in 0..streams {
        starts[s] = at;
        at += strides[s] * count;
        at = (at + 15) & !15;
    }
    let read = |ci: usize, v: usize, k: usize| -> f32 {
        let c = channels[ci];
        let base = starts[c.stream] + strides[c.stream] * v + c.offset;
        match (c.format, modern) {
            (0, _) => {
                let o = base + k * 4;
                bytes
                    .get(o..o + 4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .unwrap_or(0.0)
            }
            (1, _) => {
                let o = base + k * 2;
                bytes
                    .get(o..o + 2)
                    .map(|b| half(u16::from_le_bytes([b[0], b[1]])))
                    .unwrap_or(0.0)
            }
            _ => bytes.get(base + k).map(|b| *b as f32 / 255.0).unwrap_or(0.0),
        }
    };
    let has = |ci: usize| channels.get(ci).is_some_and(|c| c.dimension > 0);
    if !has(0) {
        bail!("{name}: no positions");
    }
    let mut mesh = Mesh {
        name,
        positions: (0..count)
            .map(|v| [read(0, v, 0), read(0, v, 1), -read(0, v, 2)])
            .collect(),
        normals: Vec::new(),
        uvs: Vec::new(),
        colors: Vec::new(),
        submeshes: Vec::new(),
    };
    if has(normal) {
        mesh.normals = (0..count)
            .map(|v| {
                let n = [read(normal, v, 0), read(normal, v, 1), -read(normal, v, 2)];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                [n[0] / l, n[1] / l, n[2] / l]
            })
            .collect();
    }
    if has(uv0) {
        mesh.uvs = (0..count)
            .map(|v| [read(uv0, v, 0), 1.0 - read(uv0, v, 1)])
            .collect();
    }
    if has(color) {
        let dims = channels[color].dimension;
        mesh.colors = (0..count)
            .map(|v| {
                let mut c = [255u8; 4];
                for (k, out) in c.iter_mut().enumerate().take(dims) {
                    *out = (read(color, v, k).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                c
            })
            .collect();
    }
    let indices = hex(raw(text, "m_IndexBuffer").unwrap_or(""))?;
    let wide = body.i64("m_IndexFormat") == Some(1);
    let index = |i: usize| -> u32 {
        if wide {
            indices
                .get(i * 4..i * 4 + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .unwrap_or(0)
        } else {
            indices
                .get(i * 2..i * 2 + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]) as u32)
                .unwrap_or(0)
        }
    };
    for sub in body.list("m_SubMeshes") {
        // Triangles only (topology 0).
        if sub.i64("topology").unwrap_or(0) != 0 {
            continue;
        }
        let first = sub.i64("firstByte").unwrap_or(0) as usize / if wide { 4 } else { 2 };
        let n = sub.i64("indexCount").unwrap_or(0) as usize;
        let base = sub.i64("baseVertex").unwrap_or(0) as u32;
        let mut list = Vec::with_capacity(n);
        for t in 0..n / 3 {
            let i = first + t * 3;
            let triangle = [index(i) + base, index(i + 1) + base, index(i + 2) + base];
            if triangle.iter().all(|&v| (v as usize) < count) {
                list.extend(triangle);
            }
        }
        if !list.is_empty() {
            mesh.submeshes.push(list);
        }
    }
    if mesh.submeshes.is_empty() {
        bail!("{}: no triangles", mesh.name);
    }
    Ok(mesh)
}

fn half(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let frac = (h & 0x3ff) as f32;
    sign * match exp {
        0 => frac * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// The mesh as a binary glTF: one node, one mesh, a primitive a sub-mesh.
pub fn glb(mesh: &Mesh) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut push = |bin: &mut Vec<u8>, bytes: &[u8], target: u32| -> usize {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        views.push(serde_json::json!({
            "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len(), "target": target
        }));
        bin.extend_from_slice(bytes);
        views.len() - 1
    };
    let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let count = mesh.positions.len();
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in &mesh.positions {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let flat: Vec<f32> = mesh.positions.iter().flatten().copied().collect();
    let v = push(&mut bin, &floats(&flat), 34962);
    accessors.push(serde_json::json!({
        "bufferView": v, "componentType": 5126, "count": count, "type": "VEC3", "min": min, "max": max
    }));
    let mut attributes = serde_json::json!({ "POSITION": 0 });
    if mesh.normals.len() == count {
        let flat: Vec<f32> = mesh.normals.iter().flatten().copied().collect();
        let v = push(&mut bin, &floats(&flat), 34962);
        accessors.push(serde_json::json!({ "bufferView": v, "componentType": 5126, "count": count, "type": "VEC3" }));
        attributes["NORMAL"] = (accessors.len() - 1).into();
    }
    if mesh.uvs.len() == count {
        let flat: Vec<f32> = mesh.uvs.iter().flatten().copied().collect();
        let v = push(&mut bin, &floats(&flat), 34962);
        accessors.push(serde_json::json!({ "bufferView": v, "componentType": 5126, "count": count, "type": "VEC2" }));
        attributes["TEXCOORD_0"] = (accessors.len() - 1).into();
    }
    if mesh.colors.len() == count {
        let flat: Vec<u8> = mesh.colors.iter().flatten().copied().collect();
        let v = push(&mut bin, &flat, 34962);
        accessors.push(serde_json::json!({
            "bufferView": v, "componentType": 5121, "normalized": true, "count": count, "type": "VEC4"
        }));
        attributes["COLOR_0"] = (accessors.len() - 1).into();
    }
    let mut primitives = Vec::new();
    for list in &mesh.submeshes {
        let bytes: Vec<u8> = list.iter().flat_map(|i| i.to_le_bytes()).collect();
        let v = push(&mut bin, &bytes, 34963);
        accessors.push(serde_json::json!({ "bufferView": v, "componentType": 5125, "count": list.len(), "type": "SCALAR" }));
        primitives.push(serde_json::json!({ "attributes": attributes, "indices": accessors.len() - 1 }));
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let json = serde_json::json!({
        "asset": { "version": "2.0", "generator": "scrap import-unity" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": mesh.name, "mesh": 0 }],
        "meshes": [{ "name": mesh.name, "primitives": primitives }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": accessors,
    });
    let mut json = serde_json::to_vec(&json).unwrap_or_default();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut out = Vec::with_capacity(28 + json.len() + bin.len());
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((28 + json.len() + bin.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// A `.asset` Mesh file as a `.glb`.
pub fn convert_asset(path: &std::path::Path) -> Result<Vec<u8>> {
    let text = std::fs::read_to_string(path)?;
    let doc = yaml::documents(&text)
        .into_iter()
        .find(|d| d.kind == "Mesh")
        .context("no Mesh in it")?;
    Ok(glb(&read(&doc.body, &text)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad of two triangles, as Unity 2018 writes one: float position
    /// and normal, unorm8 colour, float uv.
    #[test]
    fn a_unity_mesh_asset_reads_as_mirrored_triangles() {
        let mut data = Vec::new();
        let verts = [
            ([0.0f32, 0.0, 0.0], [0.0f32, 0.0], [255u8, 0, 0, 255]),
            ([1.0, 0.0, 0.0], [1.0, 0.0], [0, 255, 0, 255]),
            ([1.0, 0.0, 1.0], [1.0, 1.0], [0, 0, 255, 255]),
            ([0.0, 0.0, 1.0], [0.0, 1.0], [255, 255, 255, 0]),
        ];
        for (p, uv, c) in verts {
            for f in p.iter().chain([0.0f32, 1.0, 0.0].iter()) {
                data.extend(f.to_le_bytes());
            }
            data.extend(c);
            for f in uv {
                data.extend(f.to_le_bytes());
            }
        }
        let hexed: String = data.iter().map(|b| format!("{b:02x}")).collect();
        let indices: String = [0u16, 2, 1, 0, 3, 2]
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .map(|b| format!("{b:02x}"))
            .collect();
        let empty = "    - stream: 0\n      offset: 0\n      format: 0\n      dimension: 0\n";
        let text = format!(
            "%YAML 1.1\n--- !u!43 &4300000\nMesh:\n  m_Name: Quad\n  m_SubMeshes:\n  - firstByte: 0\n    indexCount: 6\n    topology: 0\n    baseVertex: 0\n  m_MeshCompression: 0\n  m_IndexFormat: 0\n  m_IndexBuffer: {indices}\n  m_VertexData:\n    m_VertexCount: 4\n    m_Channels:\n    - stream: 0\n      offset: 0\n      format: 0\n      dimension: 3\n    - stream: 0\n      offset: 12\n      format: 0\n      dimension: 3\n{empty}    - stream: 0\n      offset: 24\n      format: 2\n      dimension: 4\n    - stream: 0\n      offset: 28\n      format: 0\n      dimension: 2\n{}    m_DataSize: {}\n    _typelessdata: {hexed}\n",
            empty.repeat(7),
            data.len()
        );
        let doc = yaml::documents(&text).into_iter().next().unwrap();
        let mesh = read(&doc.body, &text).unwrap();
        assert_eq!(mesh.positions[2], [1.0, 0.0, -1.0], "z mirrored");
        assert_eq!(mesh.normals[0], [0.0, 1.0, 0.0]);
        assert_eq!(mesh.colors[1], [0, 255, 0, 255]);
        assert_eq!(mesh.uvs[3], [0.0, 0.0], "v from the top");
        assert_eq!(mesh.submeshes, vec![vec![0, 2, 1, 0, 3, 2]], "wound as Unity winds it");
        let glb = glb(&mesh);
        let (document, buffers, _) = gltf::import_slice(&glb).unwrap();
        let prim = document.meshes().next().unwrap().primitives().next().unwrap();
        let reader = prim.reader(|b| Some(&buffers[b.index()]));
        assert_eq!(reader.read_positions().unwrap().count(), 4);
        assert_eq!(reader.read_indices().unwrap().into_u32().count(), 6);
        assert!(reader.read_colors(0).is_some());
    }
}
