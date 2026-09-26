//! The asset formats' tests: the core's header with the modules' formats.

#[cfg(test)]
mod tests {
    use crate::asset::*;

    fn cube() -> MeshAsset {
        let vertices: Vec<Vertex> = (0..8)
            .map(|i| Vertex {
                position: [
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                ],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
            })
            .collect();
        MeshAsset {
            id: AssetId::from_source("models/cube.obj", 0),
            name: "cube".into(),
            bounds: Bounds::of(&vertices),
            vertices,
            indices: (0..12u32).collect(),
            skin: None,
            colors: Vec::new(),
            look: None,
            submeshes: vec![Submesh {
                first_index: 0,
                index_count: 12,
                material: None,
            }],
        }
    }

    #[test]
    fn a_compressed_asset_is_read_back_as_it_was_and_only_through_read() {
        let mesh = cube();
        let bytes = to_bytes(&mesh, crate::asset::MESH).unwrap();
        let packed = compressed(&bytes, false, |body| ruzstd::encoding::compress_to_vec(body, ruzstd::encoding::CompressionLevel::Fastest)).unwrap();
        assert_ne!(packed, bytes);
        let shuffled = compressed(&bytes, true, |body| ruzstd::encoding::compress_to_vec(body, ruzstd::encoding::CompressionLevel::Fastest)).unwrap();
        assert_eq!(uncompressed(shuffled).unwrap(), bytes, "shuffled, and back");
        for n in [0, 1, 3, 4, 7, 64, 65] {
            let b: Vec<u8> = (0..n as u8).collect();
            assert_eq!(unshuffle(&shuffle(&b)), b, "{n} bytes");
        }
        assert!(split_header(&packed).is_err(), "its body is not rkyv's until unpacked");
        assert_eq!(head_of(&packed).unwrap().1, mesh.id, "its header says what it is all the same");
        let dir = std::env::temp_dir().join(format!("scrap-zstd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cube.scrasset");
        std::fs::write(&path, &packed).unwrap();
        assert!(is_current(&path));
        let read = read(&path).unwrap();
        assert_eq!(read, bytes, "unpacked on the way in");
        assert_eq!(view::<MeshAsset>(&read).unwrap().indices.len(), 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_asset_is_read_back_without_being_decoded() {
        let mesh = cube();
        let bytes = to_bytes(&mesh, crate::asset::MESH).unwrap();
        let archived = view::<MeshAsset>(&bytes).unwrap();
        assert_eq!(archived.vertices.len(), 8);
        assert_eq!(archived.name.as_str(), "cube");
        assert_eq!(archived.id, mesh.id);
        // Reading a coordinate costs no parse and no allocation.
        assert_eq!(archived.vertices[1].position[0], 1.0);
    }

    #[test]
    fn the_same_source_always_gets_the_same_id() {
        // A re-import has to update the asset in place. If ids were random,
        // every re-import would orphan every scene that referenced it.
        assert_eq!(
            AssetId::from_source("models/pine_large.obj", 0),
            AssetId::from_source("models/pine_large.obj", 0)
        );
        assert_ne!(
            AssetId::from_source("models/pine_large.obj", 0),
            AssetId::from_source("models/pine_small.obj", 0)
        );
    }

    #[test]
    fn a_file_that_is_not_ours_is_refused_rather_than_cast() {
        let png = b"\x89PNG\r\n\x1a\n and then some".to_vec();
        assert!(matches!(view::<MeshAsset>(&png), Err(AssetError::BadMagic)));
    }

    #[test]
    fn an_older_format_asks_for_a_re_import_instead_of_guessing() {
        let mut bytes = to_bytes(&cube(), crate::asset::MESH).unwrap();
        bytes[8] = 0; // pretend it was written by format v0
        match view::<MeshAsset>(&bytes) {
            Err(AssetError::Version { found, expected }) => {
                assert_eq!((found, expected), (0, FORMAT_VERSION));
            }
            Err(e) => panic!("expected a version error, got {e}"),
            Ok(_) => panic!("a v0 asset was read as if it were current"),
        }
    }

    #[test]
    fn an_empty_mesh_gets_a_box_at_the_origin_not_infinities() {
        let b = Bounds::of(&[]);
        assert_eq!(b.min, [0.0; 3]);
        assert_eq!(b.max, [0.0; 3]);
        assert_eq!(b.center(), [0.0; 3]);
    }

    #[test]
    fn a_lazy_library_reads_an_asset_when_asked_and_gives_it_back_when_told() {
        use crate::mesh_asset::MeshLibrary;
        let dir = std::env::temp_dir().join(format!("scrap-lazy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mesh = cube();
        let bytes = to_bytes(&mesh, crate::asset::MESH).unwrap();
        std::fs::write(dir.join("cube.scrasset"), &bytes).unwrap();
        let (mut library, problems) = crate::Library::open_lazy(&dir).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        // Known by its header alone: nothing read yet.
        assert_eq!(library.name(mesh.id), Some("cube"));
        assert_eq!(library.resident_bytes(), 0);
        // Asked for: read off disk, whole.
        assert_eq!(library.mesh_by_name("cube").unwrap().vertices.len(), 8);
        assert_eq!(library.resident_bytes(), bytes.len());
        // Kept while wanted, given back when not, and read again after.
        assert_eq!(library.release(|id| id == mesh.id), 0);
        assert_eq!(library.release(|_| false), bytes.len());
        assert_eq!(library.resident_bytes(), 0);
        assert!(library.mesh(mesh.id).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
