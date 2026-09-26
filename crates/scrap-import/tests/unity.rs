//! A Unity project's content brought over, end to end: files in, a scrap
//! project out that opens and checks.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::path::Path;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn meta(path: &Path, guid: &str) {
    let mut name = path.as_os_str().to_owned();
    name.push(".meta");
    write(
        Path::new(&name),
        &format!("fileFormatVersion: 2\nguid: {guid}\n"),
    );
}

#[test]
fn a_unity_project_comes_over_as_scenes_prefabs_and_materials() {
    let root = std::env::temp_dir().join(format!("scrap-unity-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let unity = root.join("unity");
    let assets = unity.join("Assets");

    let lamp = assets.join("Prefabs/Lamp.prefab");
    write(
        &lamp,
        "%YAML 1.1
--- !u!1 &100
GameObject:
  m_Name: Lamp
--- !u!4 &101
Transform:
  m_GameObject: {fileID: 100}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 0}
--- !u!33 &102
MeshFilter:
  m_GameObject: {fileID: 100}
  m_Mesh: {fileID: 10206, guid: 0000000000000000e000000000000000, type: 0}
--- !u!23 &103
MeshRenderer:
  m_GameObject: {fileID: 100}
  m_Materials:
  - {fileID: 2100000, guid: brass000, type: 2}
",
    );
    meta(&lamp, "lamp0000");
    let brass = assets.join("Materials/Brass.mat");
    write(
        &brass,
        "%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: Brass
  m_SavedProperties:
    m_TexEnvs: []
    m_Floats:
    - _Metallic: 1
    - _Smoothness: 0.7
    m_Colors:
    - _BaseColor: {r: 0.8, g: 0.6, b: 0.2, a: 1}
",
    );
    meta(&brass, "brass000");
    let scene = assets.join("Scenes/Porch.unity");
    write(
        &scene,
        "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Floor
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 10, y: 1, z: 10}
  m_Father: {fileID: 0}
--- !u!33 &12
MeshFilter:
  m_GameObject: {fileID: 10}
  m_Mesh: {fileID: 10209, guid: 0000000000000000e000000000000000, type: 0}
--- !u!1001 &20
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 0}
    m_Modifications:
    - target: {fileID: 101, guid: lamp0000, type: 3}
      propertyPath: m_LocalPosition.x
      value: 2
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: lamp0000, type: 3}
",
    );
    meta(&scene, "porch000");

    let project = scrap::Project::create(root.join("game"), "game").unwrap();
    let report = scrap_import::unity::import_unity(&unity, &project, &Default::default()).unwrap();
    assert_eq!(
        (report.scenes, report.prefabs, report.materials),
        (1, 1, 1),
        "{report}"
    );
    scrap_import::sync(&project);

    // The scene opens with the prefab expanded and the material resolved.
    let porch = scrap::Scene::load(project.scenes().join("Porch.ron")).unwrap();
    let (prefabs, problems) = scrap::Prefabs::of(&project);
    assert!(problems.is_empty(), "{problems:?}");
    let expanded = scrap::instantiate(&porch, &prefabs);
    let names: Vec<String> = expanded
        .scene
        .flatten()
        .iter()
        .map(|(d, _)| d.name.clone())
        .collect();
    assert!(
        names.contains(&"Floor".to_string()) && names.contains(&"Lamp".to_string()),
        "{names:?}"
    );
    let library = scrap::Library::open(project.library()).unwrap().0;
    let brass = library
        .material_by_name("Brass")
        .expect("Brass is a material");
    assert_eq!(brass.metallic, 1.0);
    assert!(
        scrap_cli_free_check(&project),
        "the scene and prefab resolve"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// What `scrap check` would say about models and prefabs, without the CLI
/// crate: every model a line names is a builtin here.
fn scrap_cli_free_check(project: &scrap::Project) -> bool {
    let (prefabs, _) = scrap::Prefabs::of(project);
    let porch = scrap::Scene::load(project.scenes().join("Porch.ron")).unwrap();
    scrap::instantiate(&porch, &prefabs).problems.is_empty()
}

/// Unity reads a TIFF, and a JPEG whose name says `.png`, without a word:
/// both come over as PNGs the material's base map finds.
#[test]
fn a_tiff_and_a_misnamed_jpeg_come_over_as_pngs() {
    let root = std::env::temp_dir().join(format!("scrap-unity-tiff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let unity = root.join("unity");
    let assets = unity.join("Assets");
    let picture = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 40, 10]));
    let tiff = assets.join("Textures/Moss_Albedo.tif");
    std::fs::create_dir_all(tiff.parent().unwrap()).unwrap();
    picture.save_with_format(&tiff, image::ImageFormat::Tiff).unwrap();
    meta(&tiff, "moss0000");
    let jpeg = assets.join("Textures/Noise.png");
    picture.save_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
    meta(&jpeg, "noise000");
    for (name, texture) in [("Moss", "moss0000"), ("Noisy", "noise000")] {
        let mat = assets.join(format!("Materials/{name}.mat"));
        write(
            &mat,
            &format!(
                "%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: {name}
  m_SavedProperties:
    m_TexEnvs:
    - _MainTex:
        m_Texture: {{fileID: 2800000, guid: {texture}, type: 3}}
        m_Scale: {{x: 1, y: 1}}
        m_Offset: {{x: 0, y: 0}}
"
            ),
        );
        meta(&mat, &format!("{}mat", &texture[..5]));
    }
    // Something must use the materials for their textures to come over.
    let scene = assets.join("Scenes/Bog.unity");
    write(
        &scene,
        "%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Rock
--- !u!4 &11
Transform:
  m_GameObject: {fileID: 10}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Father: {fileID: 0}
--- !u!33 &12
MeshFilter:
  m_GameObject: {fileID: 10}
  m_Mesh: {fileID: 10202, guid: 0000000000000000e000000000000000, type: 0}
--- !u!23 &13
MeshRenderer:
  m_GameObject: {fileID: 10}
  m_Materials:
  - {fileID: 2100000, guid: moss0mat, type: 2}
",
    );
    meta(&scene, "bog00000");

    let project = scrap::Project::create(root.join("game"), "game").unwrap();
    let report = scrap_import::unity::import_unity(&unity, &project, &Default::default()).unwrap();
    assert!(report.errors.is_empty(), "{report}");
    let textures = project.assets().join("textures");
    for name in ["Moss_Albedo", "Noise"] {
        let png = textures.join(format!("{name}.png"));
        let read = image::open(&png).unwrap_or_else(|e| panic!("{}: {e}", png.display()));
        assert_eq!((read.width(), read.height()), (4, 4));
    }
    assert!(!report.skipped.keys().any(|k| k.contains("convert it to PNG")), "{report}");
    let _ = std::fs::remove_dir_all(&root);
}
