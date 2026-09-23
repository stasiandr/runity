//! A Unity project's content brought over, end to end: files in, a runity
//! project out that opens and checks.

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
    let root = std::env::temp_dir().join(format!("runity-unity-import-{}", std::process::id()));
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

    let project = runity::Project::create(root.join("game"), "game").unwrap();
    let report = runity_import::unity::import_unity(&unity, &project, &Default::default()).unwrap();
    assert_eq!(
        (report.scenes, report.prefabs, report.materials),
        (1, 1, 1),
        "{report}"
    );
    runity_import::sync(&project);

    // The scene opens with the prefab expanded and the material resolved.
    let porch = runity::Scene::load(project.scenes().join("Porch.ron")).unwrap();
    let (prefabs, problems) = runity::Prefabs::of(&project);
    assert!(problems.is_empty(), "{problems:?}");
    let expanded = runity::instantiate(&porch, &prefabs);
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
    let library = runity::Library::open(project.library()).unwrap().0;
    let brass = library
        .material_by_name("Brass")
        .expect("Brass is a material");
    assert_eq!(brass.metallic, 1.0);
    assert!(
        runity_cli_free_check(&project),
        "the scene and prefab resolve"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// What `runity check` would say about models and prefabs, without the CLI
/// crate: every model a line names is a builtin here.
fn runity_cli_free_check(project: &runity::Project) -> bool {
    let (prefabs, _) = runity::Prefabs::of(project);
    let porch = runity::Scene::load(project.scenes().join("Porch.ron")).unwrap();
    runity::instantiate(&porch, &prefabs).problems.is_empty()
}
