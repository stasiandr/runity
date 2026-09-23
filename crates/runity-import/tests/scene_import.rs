//! A glTF that is a scene — a level, a kit — comes in as the tree it is
//! (docs/blender.md, stage 1): a model per mesh however many objects share
//! it, a material per material, and a prefab of the nodes that a scene
//! places with one line.

use std::path::{Path, PathBuf};

use runity::scene::MaterialRef;
use runity::{EntityDesc, Library, Prefabs, Project, Scene};
use runity_import::{sidecar_for, sync, ImportSettings};

fn project(name: &str) -> (Project, PathBuf) {
    let root = std::env::temp_dir().join(format!("runity-scene-import-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, name).unwrap();
    (project, root)
}

/// A forest: a hundred rocks sharing one mesh, a stump whose mesh has two
/// materials, and a mushroom on the stump sharing the rocks' mesh. The
/// geometry is one triangle; what matters is who points at what.
fn forest(dir: &Path, stump_x: f32) -> PathBuf {
    let mut bin = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            bin.extend(c.to_le_bytes());
        }
    }
    for i in [0u16, 1, 2, 0] {
        bin.extend(i.to_le_bytes());
    }
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("forest.bin"), &bin).unwrap();

    let mut nodes: Vec<String> = (0..100)
        .map(|i| {
            format!(
                r#"{{"name": "rock", "mesh": 0, "translation": [{}, 0, {}]}}"#,
                (i % 10) as f32 * 2.0,
                (i / 10) as f32 * 2.0
            )
        })
        .collect();
    nodes.push(format!(
        r#"{{"name": "stump", "mesh": 1, "translation": [{stump_x}, 0, 0], "children": [101]}}"#
    ));
    nodes.push(
        r#"{"name": "mushroom", "mesh": 0, "translation": [0, 1, 0], "scale": [0.2, 0.2, 0.2]}"#
            .into(),
    );
    let roots: Vec<String> = (0..101).map(|i| i.to_string()).collect();
    let prim =
        |m: usize| format!(r#"{{"attributes": {{"POSITION": 0}}, "indices": 1, "material": {m}}}"#);
    let gltf = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [{roots}]}}],
  "nodes": [{nodes}],
  "meshes": [
    {{"name": "rock", "primitives": [{p0}]}},
    {{"name": "stump", "primitives": [{p0}, {p1}]}}
  ],
  "materials": [
    {{"name": "bark", "pbrMetallicRoughness": {{"baseColorFactor": [0.3, 0.2, 0.1, 1.0], "roughnessFactor": 0.9, "metallicFactor": 0.0}}}},
    {{"name": "moss", "doubleSided": true, "alphaMode": "MASK", "alphaCutoff": 0.4, "pbrMetallicRoughness": {{"baseColorFactor": [0.1, 0.5, 0.1, 1.0], "roughnessFactor": 0.25, "metallicFactor": 0.0}}}}
  ],
  "buffers": [{{"uri": "forest.bin", "byteLength": {len}}}],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
  ],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}},
    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ]
}}"#,
        roots = roots.join(", "),
        nodes = nodes.join(",\n    "),
        p0 = prim(0),
        p1 = prim(1),
        len = bin.len(),
    );
    let path = dir.join("forest.gltf");
    std::fs::write(&path, gltf).unwrap();
    path
}

fn find<'a>(desc: &'a EntityDesc, name: &str) -> Option<&'a EntityDesc> {
    if desc.name == name {
        return Some(desc);
    }
    desc.children.iter().find_map(|c| find(c, name))
}

#[test]
fn a_gltf_scene_is_a_prefab_of_its_nodes_with_one_model_per_shared_mesh() {
    let (project, _root) = project("forest");
    let source = forest(&project.assets().join("levels"), 30.0);
    let synced = sync(&project);
    assert!(synced.iter().all(|r| r.result.is_ok()), "{synced:?}");

    let settings = ImportSettings::load(sidecar_for(&source)).unwrap();
    assert!(
        settings.scene,
        "a hundred objects: a scene, decided on the first import"
    );
    assert!(
        settings.parts.contains_key("mesh:rock.0"),
        "{:?}",
        settings.parts
    );

    // A hundred and one rocks, one model: the instancing survived.
    let (library, problems) = Library::open(project.library()).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let rock = settings.parts["mesh:rock.0"];
    assert!(library.mesh(rock).is_some());
    assert_eq!(library.name(rock), Some("forest/rock"));
    assert!(
        library.mesh(settings.parts["mesh:stump.1"]).is_some(),
        "a model per material"
    );
    let moss = library
        .material_by_name("forest/moss")
        .expect("materials come along");
    assert!(
        (moss.smoothness - 0.75).abs() < 1e-4,
        "smoothness is one minus roughness"
    );
    assert_eq!(moss.render_face, runity::material::RenderFace::Both);
    assert!((moss.alpha_clip - 0.4).abs() < 1e-4);

    let (prefabs, problems) = Prefabs::of(&project);
    assert!(problems.is_empty(), "{problems:?}");
    let tree = prefabs
        .get("forest")
        .expect("the file is a prefab by its name");
    assert_eq!(tree.children.len(), 101);
    let rocks: Vec<&EntityDesc> = tree
        .children
        .iter()
        .filter(|c| c.name.starts_with("rock"))
        .collect();
    assert_eq!(rocks.len(), 100);
    assert!(
        rocks.iter().all(|r| r.model.id == Some(rock)),
        "every rock draws the one model"
    );
    assert_eq!(
        rocks[11].transform.position,
        runity::glam::Vec3::new(2.0, 0.0, 2.0)
    );
    let stump = find(tree, "stump").unwrap();
    assert_eq!(stump.material, MaterialRef::Named("forest/bark".into()));
    let second = find(tree, "stump 2").expect("the stump's moss, as a child");
    assert_eq!(second.material, MaterialRef::Named("forest/moss".into()));
    let mushroom = find(tree, "mushroom").unwrap();
    assert_eq!(
        mushroom.model.id,
        Some(rock),
        "a child sharing the mesh shares the model"
    );
    assert!((mushroom.transform.scale.x - 0.2).abs() < 1e-5);

    // Placed with one line; its parts can be overridden by their ids.
    let scene: Scene = runity::ron::from_str(&format!(
        r#"(entities: [(name: "the forest", prefab: "forest", overrides: {{ "{}": (material: "stone") }})])"#,
        mushroom.id
    ))
    .unwrap();
    let expanded = runity::instantiate(&scene, &prefabs);
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    let drawn: Vec<&EntityDesc> = expanded
        .scene
        .flatten()
        .into_iter()
        .map(|(d, _)| d)
        .filter(|d| !d.model.is_empty())
        .collect();
    assert_eq!(
        drawn.len(),
        103,
        "100 rocks, a stump in two parts, a mushroom"
    );
    assert!(drawn
        .iter()
        .any(|d| d.name == "mushroom" && d.material == MaterialRef::Named("stone".into())));
    let mushroom_id = mushroom.id;

    // The artist moves the stump and saves: the same assets and the same
    // parts, so what the scene says about the mushroom still lands.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    forest(&project.assets().join("levels"), 12.0);
    std::fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let synced = sync(&project);
    assert_eq!(synced.len(), 1, "{synced:?}");
    let again = ImportSettings::load(sidecar_for(&source)).unwrap();
    assert_eq!(again.parts, settings.parts, "the same IDs");
    let (prefabs, _) = Prefabs::of(&project);
    let tree = prefabs.get("forest").unwrap();
    assert_eq!(find(tree, "stump").unwrap().transform.position.x, 12.0);
    assert_eq!(find(tree, "mushroom").unwrap().id, mushroom_id);
}

#[test]
fn a_file_wide_override_swaps_a_material_for_the_projects_own() {
    let (project, _root) = project("override");
    let source = forest(&project.assets(), 0.0);
    sync(&project);
    let mut settings = ImportSettings::load(sidecar_for(&source)).unwrap();
    settings.materials.insert("moss".into(), "moss_wet".into());
    runity_import::import_into(&project, &source, Some(&settings)).unwrap();

    let (prefabs, _) = Prefabs::of(&project);
    let tree = prefabs.get("forest").unwrap();
    assert_eq!(
        find(tree, "stump 2").unwrap().material,
        MaterialRef::Named("moss_wet".into()),
        "the game's material, not Blender's"
    );
    assert_eq!(
        find(tree, "stump").unwrap().material,
        MaterialRef::Named("forest/bark".into()),
        "the rest as the file has it"
    );
}

#[test]
fn a_gltf_of_one_object_is_still_one_model() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floating_quad.gltf");
    assert!(!runity_import::scene::is_scene(&fixture).unwrap());
}

#[test]
fn a_scenes_maps_become_textures_and_metal_roughness_packs_into_the_mask() {
    let (project, _root) = project("maps");
    let dir = project.assets();
    let source = forest(&dir, 0.0);
    // One pixel each: a red colour map, and glTF's metal (blue) and
    // roughness (green) at full and a quarter.
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]))
        .save(dir.join("bark_colour.png"))
        .unwrap();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 64, 255, 255]))
        .save(dir.join("bark_mr.png"))
        .unwrap();
    let text = std::fs::read_to_string(&source).unwrap().replace(
        r#""roughnessFactor": 0.9, "metallicFactor": 0.0}}"#,
        r#""roughnessFactor": 1.0, "metallicFactor": 1.0, "baseColorTexture": {"index": 0}, "metallicRoughnessTexture": {"index": 1}}}"#,
    );
    let text = text.replacen(
        r#""buffers""#,
        r#""images": [{"uri": "bark_colour.png"}, {"uri": "bark_mr.png"}],
  "textures": [{"source": 0}, {"source": 1}],
  "buffers""#,
        1,
    );
    std::fs::write(&source, text).unwrap();
    runity_import::import_into(&project, &source, None).unwrap();

    let (library, _) = Library::open(project.library()).unwrap();
    let bark = library.material_by_name("forest/bark").unwrap();
    let base = library.texture(bark.base_map.expect("a base map")).unwrap();
    assert!(base.srgb, "colour is sRGB");
    assert_eq!(&base.pixels[..4], &[255, 0, 0, 255]);
    let mask = library.texture(bark.mask_map.expect("a mask map")).unwrap();
    assert!(!mask.srgb, "a mask is data");
    assert_eq!(mask.pixels[0], 255, "metal from glTF's blue, in red");
    assert_eq!(mask.pixels[1], 255, "no occlusion map: none");
    assert_eq!(mask.pixels[3], 191, "smoothness is one minus roughness, in alpha");
    assert_eq!((bark.metallic, bark.smoothness), (1.0, 1.0), "the factors are in the map");
}
