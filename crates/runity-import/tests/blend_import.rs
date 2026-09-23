//! A `.blend` in `assets/` imports itself through Blender and the runity
//! plugin (docs/blender.md, stage 2): linked duplicates stay one model,
//! what is marked as an asset is a prefab of its own and its instances are
//! instances of it, IDs stamped by the plugin become the entities' IDs,
//! and `runity.<name>` properties are components.
//!
//! Needs Blender; without it the test says so and passes.

use std::path::{Path, PathBuf};
use std::process::Command;

use runity::scene::{Collider, MaterialRef};
use runity::{EntityDesc, Library, Prefabs, Project};
use runity_import::{sidecar_for, sync, ImportSettings};

/// Build the test file in Blender: the scene below, IDs stamped, saved.
const BUILD: &str = r#"
import sys, bpy
sys.path.insert(0, PLUGIN)
import export

for o in list(bpy.data.objects):
    bpy.data.objects.remove(o)

moss = bpy.data.materials.new("moss")
moss.use_nodes = True
bsdf = moss.node_tree.nodes["Principled BSDF"]
bsdf.inputs["Base Color"].default_value = (0.1, 0.5, 0.1, 1.0)
bsdf.inputs["Roughness"].default_value = 0.3
moss.use_backface_culling = True

bpy.ops.mesh.primitive_ico_sphere_add()
rock = bpy.context.object
rock.name = "rock"
rock.data.name = "rock"
rock.data.materials.append(moss)
rock.location = (1.0, 2.0, 3.0)
rock["runity.door"] = "(open_angle: 90.0)"
for i in range(2):
    twin = rock.copy()
    twin.location = (i * 3.0, -4.0, 0.0)
    bpy.context.scene.collection.objects.link(twin)

bpy.ops.mesh.primitive_cube_add()
wall = bpy.context.object
wall.name = "wall-col"
wall.modifiers.new("bevel", "BEVEL")

post = bpy.data.objects.new("lamp post", None)
bpy.context.scene.collection.objects.link(post)
post.location = (0.0, 0.0, 2.0)
bulb = rock.copy()
bulb.name = "bulb"
bpy.context.scene.collection.objects.link(bulb)
bulb.parent = post
bulb.location = (0.0, 0.0, 1.0)

kit = bpy.data.collections.new("crate kit")
bpy.ops.mesh.primitive_cube_add()
crate = bpy.context.object
crate.name = "crate"
for c in list(crate.users_collection):
    c.objects.unlink(crate)
kit.objects.link(crate)
kit.asset_mark()
here = bpy.data.objects.new("crate here", None)
here.instance_type = "COLLECTION"
here.instance_collection = kit
here.location = (5.0, 0.0, 0.0)
bpy.context.scene.collection.objects.link(here)

export.stamp()
bpy.ops.wm.save_as_mainfile(filepath=OUT)
"#;

fn plugin() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/blender/runity")
}

fn find<'a>(desc: &'a EntityDesc, name: &str) -> Option<&'a EntityDesc> {
    if desc.name == name {
        return Some(desc);
    }
    desc.children.iter().find_map(|c| find(c, name))
}

#[test]
fn a_blend_imports_as_a_level_and_a_kit_through_blender() {
    let Some(blender) = runity_import::blend::blender() else {
        eprintln!("no Blender on this machine: skipped");
        return;
    };
    let root = std::env::temp_dir().join("runity-blend-import");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "blend").unwrap();
    let source = project.assets().join("yard.blend");
    let script = format!(
        "PLUGIN = {:?}\nOUT = {:?}\n{BUILD}",
        plugin().to_string_lossy(),
        source.to_string_lossy()
    );
    let made = Command::new(&blender)
        .args(["--background", "--factory-startup", "--python-expr"])
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        source.is_file(),
        "Blender did not write the file:\n{}\n{}",
        String::from_utf8_lossy(&made.stdout),
        String::from_utf8_lossy(&made.stderr)
    );

    let synced = sync(&project);
    let yard: Vec<_> = synced.iter().filter(|r| r.source == source).collect();
    assert_eq!(yard.len(), 1, "{synced:?}");
    assert!(yard[0].result.is_ok(), "{:?}", yard[0].result);
    let settings = ImportSettings::load(sidecar_for(&source)).unwrap();
    assert!(settings.scene);

    let (library, _) = Library::open(project.library()).unwrap();
    let (prefabs, problems) = Prefabs::of(&project);
    assert!(problems.is_empty(), "{problems:?}");
    let level = prefabs.get("yard").expect("the level, by the file's name");

    // Three rocks and a bulb on one mesh block: one model.
    let rocks: Vec<&EntityDesc> = level
        .children
        .iter()
        .filter(|c| c.name.starts_with("rock"))
        .collect();
    assert_eq!(
        rocks.len(),
        3,
        "{:#?}",
        level.children.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    let model = rocks[0].model.id.expect("a model");
    assert!(rocks.iter().all(|r| r.model.id == Some(model)));
    let bulb = find(level, "bulb").expect("the post's child");
    assert_eq!(
        bulb.model.id,
        Some(model),
        "a child on the same mesh shares the model"
    );
    assert_eq!(library.name(model), Some("yard/rock"));
    assert_eq!(
        find(level, "lamp post").unwrap().children.len(),
        1,
        "the bulb under its post"
    );

    // Blender's Z up is the engine's Y up.
    let first = find(level, "rock").unwrap();
    let p = first.transform.position;
    assert!(
        (p - runity::glam::Vec3::new(1.0, 3.0, -2.0)).length() < 1e-4,
        "{p}"
    );

    // The plugin's IDs are the entities' IDs.
    assert!(!first.id.is_unassigned());
    assert!(
        first.components.contains_key("door"),
        "runity.door is a component"
    );
    assert_eq!(first.material, MaterialRef::Named("yard/moss".into()));
    let moss = library.material_by_name("yard/moss").unwrap();
    assert!((moss.smoothness - 0.7).abs() < 1e-3, "{}", moss.smoothness);

    // A modifier makes a mesh of its own; -col makes it solid.
    let wall = find(level, "wall-col").unwrap();
    assert!(wall.model.id.is_some() && wall.model.id != Some(model));
    assert_eq!(wall.collider, Collider::Model);

    // The kit: an asset of its own, and its instance in the level is one.
    let kit = prefabs
        .get("crate kit")
        .expect("an asset collection is a prefab");
    assert!(find(kit, "crate").is_some_and(|c| c.model.id.is_some()));
    let here = find(level, "crate here").unwrap();
    assert_eq!(here.prefab.as_str(), "crate kit");
    let expanded = runity::instantiate(
        &runity::ron::from_str(r#"(entities: [(name: "yard", prefab: "yard")])"#).unwrap(),
        &prefabs,
    );
    assert!(expanded.problems.is_empty(), "{:?}", expanded.problems);
    assert!(expanded
        .scene
        .flatten()
        .iter()
        .any(|(d, _)| d.name == "crate" && d.model.id.is_some()));
}

/// The runity tab's buttons, driven as a person would: a component added
/// from the game's list becomes a property per field, the collider and
/// the game material are settings of the object, and the engine reads all
/// three back.
#[test]
fn the_runity_panel_adds_components_a_field_at_a_time() {
    let Some(blender) = runity_import::blend::blender() else {
        eprintln!("no Blender on this machine: skipped");
        return;
    };
    let root = std::env::temp_dir().join("runity-blend-panel");
    let _ = std::fs::remove_dir_all(&root);
    let project = Project::create(&root, "panel").unwrap();
    // What the editor writes for the plugin.
    std::fs::create_dir_all(project.library()).unwrap();
    std::fs::write(
        project.library().join("blender.json"),
        r#"{
  "materials": ["stone", "moss"],
  "components": {
    "door": {
      "shape": {"Struct": [["open_angle", "Float"], ["locked", "Bool"], ["table", "Text"], ["mode", {"Enum": ["Open", "Shut"]}]]},
      "example": "(open_angle: 0.0, locked: false, table: \"\", mode: Open)"
    },
    "tag": {"shape": "Text", "example": "\"\""}
  }
}"#,
    )
    .unwrap();
    let source = project.assets().join("room.blend");
    let script = format!(
        r#"
import sys, bpy
sys.path.insert(0, {tools:?})
import runity
runity.register()
for o in list(bpy.data.objects):
    bpy.data.objects.remove(o)
bpy.ops.wm.save_as_mainfile(filepath={out:?})
bpy.ops.mesh.primitive_cube_add()
door = bpy.context.object
door.name = "door"
bpy.context.view_layer.objects.active = door
assert bpy.ops.runity.add_component(name="door") == {{"FINISHED"}}
assert bpy.ops.runity.add_component(name="tag") == {{"FINISHED"}}
assert isinstance(door["runity.door.open_angle"], float)
assert door["runity.door.locked"] is False or door["runity.door.locked"] == 0
door["runity.door.open_angle"] = 90.0
door["runity.door.table"] = "chest"
bpy.ops.runity.toggle_collider()
bpy.ops.runity.set_material(material="stone")
bpy.ops.wm.save_mainfile()
"#,
        tools = plugin().parent().unwrap().to_string_lossy(),
        out = source.to_string_lossy(),
    );
    let ran = Command::new(&blender)
        .args([
            "--background",
            "--factory-startup",
            "--python-exit-code",
            "1",
            "--python-expr",
        ])
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );

    let synced = sync(&project);
    assert!(
        synced
            .iter()
            .filter(|r| r.source == source)
            .all(|r| r.result.is_ok()),
        "{synced:?}"
    );
    let (prefabs, _) = Prefabs::of(&project);
    let door = find(prefabs.get("room").unwrap(), "door").unwrap();
    assert_eq!(
        door.components["door"].get_ron(),
        r#"(locked: false, mode: Open, open_angle: 90.0, table: "chest")"#
    );
    assert_eq!(door.components["tag"].get_ron(), r#""""#);
    assert_eq!(door.collider, Collider::Model);
    assert_eq!(door.material, MaterialRef::Named("stone".into()));
}
