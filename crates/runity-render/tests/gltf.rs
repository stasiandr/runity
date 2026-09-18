//! Reading glTF, against files built byte by byte in the test.
//!
//! Checked-in binary fixtures would be opaque: when one fails you cannot see
//! why. Building them here means every field the loader reads is visible
//! beside the assertion about it.

use runity_render::encode_png;
use runity_render::gltf::{self, GltfError};

/// Base64, so the tests can write data URIs the way an exporter would.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut buffer = [0u8; 3];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let value = u32::from(buffer[0]) << 16 | u32::from(buffer[1]) << 8 | u32::from(buffer[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[((value >> (18 - index * 6)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn floats(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// A triangle: three positions and three indices.
fn triangle_buffer() -> Vec<u8> {
    let mut bytes = floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes
}

/// A document describing that triangle, with the buffer embedded.
fn triangle_document(extra_nodes: &str) -> String {
    let buffer = triangle_buffer();
    format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0,
            "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"mesh": 0, "name": "triangle"}}{extra}],
            "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1, "material": 0}}]}}],
            "materials": [{{
                "name": "clay",
                "pbrMetallicRoughness": {{
                    "baseColorFactor": [0.8, 0.2, 0.1, 1.0],
                    "metallicFactor": 0.0,
                    "roughnessFactor": 0.75
                }},
                "emissiveFactor": [0.1, 0.0, 0.0]
            }}],
            "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
            ],
            "bufferViews": [
                {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
            ],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        extra = extra_nodes,
        length = buffer.len(),
        data = base64(&buffer),
    )
}

#[test]
fn a_triangle_loads_with_its_vertices_and_indices() {
    let document = triangle_document("");
    let model = gltf::parse(document.as_bytes(), None).expect("a valid glTF");

    assert_eq!(model.meshes.len(), 1);
    assert_eq!(model.instances.len(), 1);
    let mesh = &model.meshes[0];
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.indices, vec![0, 1, 2]);
    assert_eq!(mesh.vertices[1].position.x, 1.0);
    assert_eq!(mesh.vertices[2].position.y, 1.0);
    assert_eq!(model.triangle_count(), 1);
    assert_eq!(model.instances[0].name, "triangle");
}

#[test]
fn material_factors_are_read() {
    let model = gltf::parse(triangle_document("").as_bytes(), None).unwrap();
    assert_eq!(model.materials.len(), 1);
    let material = &model.materials[0];
    assert_eq!(material.name, "clay");
    assert!((material.base_color.r - 0.8).abs() < 1e-6);
    assert_eq!(material.metallic, 0.0);
    assert_eq!(material.roughness, 0.75);
    assert!((material.emissive.r - 0.1).abs() < 1e-6);
    assert_eq!(model.instances[0].material, Some(0));
    assert!(material.base_color_texture.is_none());
}

#[test]
fn missing_normals_are_computed_rather_than_left_flat() {
    // A file without normals is legal, and unlit without them.
    let model = gltf::parse(triangle_document("").as_bytes(), None).unwrap();
    let normal = model.meshes[0].vertices[0].normal;
    assert!((normal.length() - 1.0).abs() < 1e-4, "{normal:?}");
    assert!(
        normal.z.abs() > 0.9,
        "a triangle in the xy plane faces along z: {normal:?}"
    );
}

#[test]
fn node_transforms_compose_down_the_tree() {
    // A child node under a translated, scaled parent.
    let document = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0,
            "scenes": [{{"nodes": [0]}}],
            "nodes": [
                {{"children": [1], "translation": [10.0, 0.0, 0.0], "scale": [2.0, 2.0, 2.0], "name": "cart"}},
                {{"mesh": 0, "translation": [1.0, 0.0, 0.0], "name": "wheel"}}
            ],
            "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}}],
            "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
            ],
            "bufferViews": [
                {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
            ],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        length = triangle_buffer().len(),
        data = base64(&triangle_buffer()),
    );

    let model = gltf::parse(document.as_bytes(), None).unwrap();
    assert_eq!(model.instances.len(), 1);
    let transform = model.instances[0].transform;
    let placed = transform.transform_point(runity_math::Vec3::ZERO);
    // Ten across, plus one scaled by two.
    assert!((placed.x - 12.0).abs() < 1e-4, "{placed:?}");
    assert_eq!(model.instances[0].name, "wheel");
}

#[test]
fn a_matrix_node_is_read_as_written() {
    let matrix = "[2,0,0,0, 0,2,0,0, 0,0,2,0, 5,6,7,1]";
    let document = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"mesh": 0, "matrix": {matrix}}}],
            "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}}}]}}],
            "accessors": [{{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}}],
            "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        length = triangle_buffer().len(),
        data = base64(&triangle_buffer()),
    );

    let model = gltf::parse(document.as_bytes(), None).unwrap();
    let placed = model.instances[0]
        .transform
        .transform_point(runity_math::Vec3::ZERO);
    assert!((placed.x - 5.0).abs() < 1e-5 && (placed.y - 6.0).abs() < 1e-5);
    // With no indices, the vertices are the triangle in order.
    assert_eq!(model.meshes[0].indices, vec![0, 1, 2]);
}

#[test]
fn interleaved_attributes_are_read_through_their_stride() {
    // Positions and normals in one buffer, alternating — which is how most
    // exporters write them, and where a loader that assumes packed data
    // produces a mesh of noise.
    let mut bytes = Vec::new();
    for (position, normal) in [
        ([0.0f32, 0.0, 0.0], [0.0f32, 1.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ] {
        bytes.extend(floats(&position));
        bytes.extend(floats(&normal));
    }

    let document = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"mesh": 0}}],
            "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1}}}}]}}],
            "accessors": [
                {{"bufferView": 0, "byteOffset": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                {{"bufferView": 0, "byteOffset": 12, "componentType": 5126, "count": 3, "type": "VEC3"}}
            ],
            "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": {length}, "byteStride": 24}}],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        length = bytes.len(),
        data = base64(&bytes),
    );

    let model = gltf::parse(document.as_bytes(), None).unwrap();
    let mesh = &model.meshes[0];
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.vertices[1].position.x, 1.0);
    assert_eq!(mesh.vertices[2].position.z, 1.0);
    for vertex in &mesh.vertices {
        assert!((vertex.normal.y - 1.0).abs() < 1e-5, "{:?}", vertex.normal);
    }
}

#[test]
fn indices_of_every_width_are_read() {
    for (component, width) in [(5121usize, 1usize), (5123, 2), (5125, 4)] {
        let mut bytes = floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        for index in [2u32, 1, 0] {
            match width {
                1 => bytes.push(index as u8),
                2 => bytes.extend_from_slice(&(index as u16).to_le_bytes()),
                _ => bytes.extend_from_slice(&index.to_le_bytes()),
            }
        }
        let document = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "scenes": [{{"nodes": [0]}}],
                "nodes": [{{"mesh": 0}}],
                "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}}],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                    {{"bufferView": 1, "componentType": {component}, "count": 3, "type": "SCALAR"}}
                ],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                    {{"buffer": 0, "byteOffset": 36, "byteLength": {index_length}}}
                ],
                "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
            }}"#,
            index_length = width * 3,
            length = bytes.len(),
            data = base64(&bytes),
        );
        let model = gltf::parse(document.as_bytes(), None)
            .unwrap_or_else(|error| panic!("component type {component}: {error}"));
        assert_eq!(
            model.meshes[0].indices,
            vec![2, 1, 0],
            "component type {component}"
        );
    }
}

#[test]
fn a_glb_container_is_taken_apart() {
    // The binary form: a JSON chunk and a BIN chunk, with the buffer having
    // no URI at all.
    let buffer = triangle_buffer();
    let json = r#"{
        "asset": {"version": "2.0"},
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 6}
        ],
        "buffers": [{"byteLength": 42}]
    }"#;

    let mut json_chunk = json.as_bytes().to_vec();
    while json_chunk.len() % 4 != 0 {
        json_chunk.push(b' ');
    }
    let mut binary_chunk = buffer.clone();
    while binary_chunk.len() % 4 != 0 {
        binary_chunk.push(0);
    }

    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    let total = 12 + 8 + json_chunk.len() + 8 + binary_chunk.len();
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json_chunk);
    glb.extend_from_slice(&(binary_chunk.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&binary_chunk);

    let model = gltf::parse(&glb, None).expect("a valid GLB");
    assert_eq!(model.meshes[0].vertices.len(), 3);
    assert_eq!(model.meshes[0].indices, vec![0, 1, 2]);

    // And a truncated one is refused rather than half-read.
    assert!(gltf::parse(&glb[..glb.len() - 10], None).is_err());
}

#[test]
fn an_embedded_png_becomes_a_texture() {
    let pixels = vec![
        0xFF_FF_00_00u32,
        0xFF_00_FF_00,
        0xFF_00_00_FF,
        0xFF_FF_FF_FF,
    ];
    let png = encode_png(2, 2, &pixels);
    let buffer = triangle_buffer();
    let document = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"mesh": 0}}],
            "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "material": 0}}]}}],
            "materials": [{{"pbrMetallicRoughness": {{"baseColorTexture": {{"index": 0}}}}}}],
            "textures": [{{"source": 0}}],
            "images": [{{"uri": "data:image/png;base64,{image}"}}],
            "accessors": [{{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}}],
            "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        image = base64(&png),
        length = buffer.len(),
        data = base64(&buffer),
    );

    let model = gltf::parse(document.as_bytes(), None).unwrap();
    let texture = model.materials[0]
        .base_color_texture
        .as_ref()
        .expect("the PNG should have been decoded");
    assert_eq!(texture.width(), 2);
    assert_eq!(texture.height(), 2);
}

#[test]
fn a_texture_this_engine_cannot_decode_is_skipped_not_fatal() {
    // A JPEG: the material should keep its factors and lose its texture,
    // which is far better than refusing the model.
    let buffer = triangle_buffer();
    let document = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"mesh": 0}}],
            "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "material": 0}}]}}],
            "materials": [{{
                "pbrMetallicRoughness": {{
                    "baseColorFactor": [0.5, 0.5, 0.5, 1.0],
                    "baseColorTexture": {{"index": 0}}
                }}
            }}],
            "textures": [{{"source": 0}}],
            "images": [{{"uri": "data:image/jpeg;base64,{image}"}}],
            "accessors": [{{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}}],
            "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        image = base64(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]),
        length = buffer.len(),
        data = base64(&buffer),
    );

    let model = gltf::parse(document.as_bytes(), None).unwrap();
    assert!(model.materials[0].base_color_texture.is_none());
    assert!((model.materials[0].base_color.r - 0.5).abs() < 1e-6);
}

#[test]
fn what_is_not_supported_is_refused_plainly() {
    let buffer = triangle_buffer();
    let with = |primitive: &str, accessor: &str| {
        format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "scenes": [{{"nodes": [0]}}],
                "nodes": [{{"mesh": 0}}],
                "meshes": [{{"primitives": [{primitive}]}}],
                "accessors": [{accessor}],
                "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
                "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
            }}"#,
            length = buffer.len(),
            data = base64(&buffer),
        )
    };

    // Lines rather than triangles.
    let lines = with(
        r#"{"attributes": {"POSITION": 0}, "mode": 1}"#,
        r#"{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}"#,
    );
    assert!(matches!(
        gltf::parse(lines.as_bytes(), None),
        Err(GltfError::Unsupported(_))
    ));

    // A sparse accessor.
    let sparse = with(
        r#"{"attributes": {"POSITION": 0}}"#,
        r#"{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "sparse": {"count": 1}}"#,
    );
    assert!(matches!(
        gltf::parse(sparse.as_bytes(), None),
        Err(GltfError::Unsupported(_))
    ));

    // A primitive with no positions.
    let empty = with(
        r#"{"attributes": {}}"#,
        r#"{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}"#,
    );
    assert!(matches!(
        gltf::parse(empty.as_bytes(), None),
        Err(GltfError::Missing(_))
    ));
}

#[test]
fn a_file_that_is_not_gltf_is_refused() {
    assert!(matches!(gltf::parse(b"{}", None), Err(GltfError::NotGltf)));
    assert!(matches!(
        gltf::parse(br#"{"asset": {"version": "1.0"}}"#, None),
        Err(GltfError::Version(_))
    ));
    assert!(matches!(
        gltf::parse(b"not json at all", None),
        Err(GltfError::Json(_))
    ));

    // An external buffer with nowhere to look for it.
    let external = r#"{
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": 4, "uri": "scene.bin"}]
    }"#;
    assert!(matches!(
        gltf::parse(external.as_bytes(), None),
        Err(GltfError::Buffer(_))
    ));
}

#[test]
fn a_cycle_in_the_node_graph_does_not_hang_the_loader() {
    // glTF says the nodes form a tree; nothing stops a file from lying.
    let buffer = triangle_buffer();
    let document = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scenes": [{{"nodes": [0]}}],
            "nodes": [
                {{"children": [1], "mesh": 0}},
                {{"children": [0], "mesh": 0}}
            ],
            "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}}}]}}],
            "accessors": [{{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}}],
            "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        length = buffer.len(),
        data = base64(&buffer),
    );

    let model = gltf::parse(document.as_bytes(), None).unwrap();
    assert_eq!(model.instances.len(), 2, "each node is visited once");
}

#[test]
fn rubbish_never_panics() {
    let good = triangle_document("");
    // Every prefix of a real document.
    for length in 0..good.len() {
        let _ = gltf::parse(&good.as_bytes()[..length], None);
    }
    // And random noise.
    let mut rng = runity_math::Rng::named(2, "gltf fuzz");
    for _ in 0..5_000 {
        let length = rng.below(64) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| rng.below(256) as u8).collect();
        let _ = gltf::parse(&bytes, None);
    }
}

/// A skinned triangle with two joints, one clip, and everything the loader
/// reads: influences, inverse bind matrices, a node hierarchy and keyframes.
fn skinned_document() -> String {
    // Three vertices: the first bound to the shoulder, the others to the elbow.
    let mut bytes = floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0]);
    // JOINTS_0 as unsigned bytes, four per vertex.
    bytes.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    // WEIGHTS_0 as floats, four per vertex.
    bytes.extend(floats(&[
        1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
    ]));
    // Inverse bind matrices: identity for the shoulder, a step back along x
    // for the elbow, which is one metre out in the bind pose.
    bytes.extend(floats(&[
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]));
    bytes.extend(floats(&[
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0,
    ]));
    // Keyframe times, then where the elbow is at each of them.
    bytes.extend(floats(&[0.0, 1.0]));
    bytes.extend(floats(&[1.0, 0.0, 0.0, 3.0, 0.0, 0.0]));

    format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0,
            "scenes": [{{"nodes": [0]}}],
            "nodes": [
                {{"mesh": 0, "skin": 0, "children": [1], "name": "body"}},
                {{"children": [2], "name": "shoulder"}},
                {{"translation": [1.0, 0.0, 0.0], "name": "elbow"}}
            ],
            "meshes": [{{"primitives": [{{
                "attributes": {{"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}}
            }}]}}],
            "skins": [{{"joints": [1, 2], "inverseBindMatrices": 3}}],
            "animations": [{{
                "name": "reach",
                "channels": [{{"sampler": 0, "target": {{"node": 2, "path": "translation"}}}}],
                "samplers": [{{"input": 4, "output": 5, "interpolation": "LINEAR"}}]
            }}],
            "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                {{"bufferView": 1, "componentType": 5121, "count": 3, "type": "VEC4"}},
                {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"}},
                {{"bufferView": 3, "componentType": 5126, "count": 2, "type": "MAT4"}},
                {{"bufferView": 4, "componentType": 5126, "count": 2, "type": "SCALAR"}},
                {{"bufferView": 5, "componentType": 5126, "count": 2, "type": "VEC3"}}
            ],
            "bufferViews": [
                {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                {{"buffer": 0, "byteOffset": 36, "byteLength": 12}},
                {{"buffer": 0, "byteOffset": 48, "byteLength": 48}},
                {{"buffer": 0, "byteOffset": 96, "byteLength": 128}},
                {{"buffer": 0, "byteOffset": 224, "byteLength": 8}},
                {{"buffer": 0, "byteOffset": 232, "byteLength": 24}}
            ],
            "buffers": [{{"byteLength": {length}, "uri": "data:application/octet-stream;base64,{data}"}}]
        }}"#,
        length = bytes.len(),
        data = base64(&bytes),
    )
}

#[test]
fn a_skeleton_is_read_with_its_joints_in_order() {
    let model = gltf::parse(skinned_document().as_bytes(), None).expect("a skinned glTF");
    assert_eq!(model.skins.len(), 1);
    let skin = &model.skins[0];
    assert_eq!(skin.skeleton.len(), 2);
    assert!(
        skin.skeleton.is_sorted(),
        "parents must come before children"
    );
    assert_eq!(
        skin.skeleton.joints[0].parent, None,
        "the shoulder is the root"
    );
    assert_eq!(
        skin.skeleton.joints[1].parent,
        Some(0),
        "and the elbow hangs off it"
    );
    assert_eq!(skin.nodes, vec![1, 2]);
    assert_eq!(model.instances[0].skin, Some(0));
}

#[test]
fn vertex_influences_are_read_and_normalized() {
    let model = gltf::parse(skinned_document().as_bytes(), None).unwrap();
    let influences = &model.influences[0];
    assert_eq!(influences.len(), 3);
    assert_eq!(influences[0].joints[0], 0);
    assert_eq!(influences[1].joints[0], 1);
    for influence in influences {
        let total: f32 = influence.weights.iter().sum();
        assert!((total - 1.0).abs() < 1e-5);
    }
    assert!(model.skinned_mesh(0).is_some());
}

#[test]
fn a_clip_is_retargeted_from_nodes_onto_joints() {
    // glTF animates nodes; a pose is in terms of joints. Without the
    // translation a clip cannot drive a skeleton at all.
    let model = gltf::parse(skinned_document().as_bytes(), None).unwrap();
    assert_eq!(model.animations.len(), 1);
    assert_eq!(model.animations[0].name, "reach");
    assert_eq!(model.animations[0].channels[0].node, 2);

    let clip = model.animation_for(0, 0).expect("the clip should retarget");
    assert_eq!(clip.channels.len(), 1);
    assert_eq!(
        clip.channels[0].joint, 1,
        "node 2 is the skin's second joint"
    );
    assert!((clip.duration - 1.0).abs() < 1e-6);
}

#[test]
fn the_loaded_skeleton_and_clip_actually_deform_the_mesh() {
    // The end-to-end check: file in, moved vertices out.
    let model = gltf::parse(skinned_document().as_bytes(), None).unwrap();
    let skin = &model.skins[0];
    let clip = model.animation_for(0, 0).unwrap();
    let skinned = model.skinned_mesh(0).unwrap();

    let mut matrices = Vec::new();
    let mut out = runity_render::Mesh::default();

    // At rest the skinning matrix is the joint's world transform times its
    // inverse bind, which cancel — so the mesh must come out where it was
    // modelled.
    skin.skeleton
        .skinning_matrices(&clip.pose_at(&skin.skeleton, 0.0), &mut matrices);
    skinned.apply(&matrices, &mut out);
    assert!(
        (out.vertices[1].position.x - 1.0).abs() < 1e-4,
        "at rest: {:?}",
        out.vertices[1].position
    );

    // The clip slides the elbow from one metre out to three, so everything
    // bound to it moves by two.
    skin.skeleton
        .skinning_matrices(&clip.pose_at(&skin.skeleton, 1.0), &mut matrices);
    skinned.apply(&matrices, &mut out);
    assert!(
        (out.vertices[1].position.x - 3.0).abs() < 1e-4,
        "reaching: {:?}",
        out.vertices[1].position
    );
    assert!(
        out.vertices[0].position.x.abs() < 1e-4,
        "the root vertex stays put"
    );
}

#[test]
fn an_unskinned_model_has_no_skins_and_no_clips() {
    let model = gltf::parse(triangle_document("").as_bytes(), None).unwrap();
    assert!(model.skins.is_empty());
    assert!(model.animations.is_empty());
    assert!(
        model.skinned_mesh(0).is_none(),
        "a mesh with no influences does not bend"
    );
    assert_eq!(model.animation_for(0, 0), None);
}
