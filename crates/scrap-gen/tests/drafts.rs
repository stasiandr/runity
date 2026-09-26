//! A draft lands in the project and takes the greybox's place — tested with
//! a provider that answers at once with a box, so no network and no key.

#[allow(unused_imports)]
use scrap::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

use scrap::glam::Vec3;
use scrap::scene::MaterialRef;
use scrap_editor::Session;
use scrap_gen::{drafts, Generator, Input, Output, Provider, Request};

/// `std::fs::write`, the folders on the way made first: a new project has
/// only the folders its layout needs (docs/layout.md).
#[allow(dead_code)]
fn write_all(path: impl AsRef<std::path::Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// A box 1 × 2 × 1 m, nowhere near the origin, in one colour: what a
/// network sends back, minus the network.
struct Boxes;

impl Provider for Boxes {
    fn name(&self) -> &str {
        "boxes"
    }
    fn generate(&self, _: &Input, progress: &mut dyn FnMut(String)) -> anyhow::Result<Output> {
        progress("building a box".into());
        Ok(Output {
            glb: glb_box(
                Vec3::new(3.0, 5.0, 3.0),
                Vec3::new(4.0, 7.0, 4.0),
                [0.5, 0.2, 0.1],
            ),
            picture: None,
            model: "test-box".into(),
            seed: Some(7),
        })
    }
}

struct Fails;

impl Provider for Fails {
    fn name(&self) -> &str {
        "fails"
    }
    fn generate(&self, _: &Input, _: &mut dyn FnMut(String)) -> anyhow::Result<Output> {
        anyhow::bail!("the service said no")
    }
}

const SCENE: &str = r#"(
    entities: [
        (
            name: "block",
            model: "builtin:cube",
            transform: (position: (5.0, 1.0, 0.0), rotation_deg: (0.0, 90.0, 0.0), scale: (2.0, 2.0, 2.0)),
            body: Static,
            collider: Box(half: (0.5, 0.5, 0.5)),
        ),
    ],
)"#;

fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("scrap-gen-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    scrap::Project::create(&root, name).unwrap();
    let path = root.join("maps/scene.scene.ron");
    write_all(&path, SCENE).unwrap();
    path
}

fn session(name: &str) -> (Session, PathBuf) {
    let scene = project(name);
    let mut session = Session::offscreen(64, 64).unwrap();
    session.open_scene(&scene).unwrap();
    (
        session,
        scene.parent().unwrap().parent().unwrap().to_path_buf(),
    )
}

#[test]
fn a_draft_takes_the_greybox_place_in_one_undo_step() {
    let (mut session, _root) = session("fit");
    let block = session.find("block").unwrap();
    let mut generator = Generator::new(Boxes);

    let job = generator
        .start(
            &mut session,
            Request::text("well", "an old stone well").fit(block),
        )
        .unwrap();
    let finished = generator
        .wait(&mut session, job, Duration::from_secs(10))
        .expect("the box comes back at once");
    let applied = finished.result.unwrap();

    // Scaled evenly to fit the 2 m cube: the 2 m tall box keeps its size.
    assert!(
        (applied.size - Vec3::new(1.0, 2.0, 1.0)).length() < 1e-4,
        "{:?}",
        applied.size
    );
    let desc = session.scene().get(block).unwrap();
    assert_eq!(desc.model(), "well");
    assert!(
        matches!(desc.material_ref(), MaterialRef::Inline(_)),
        "painted its colour, not left the grid"
    );

    // Standing on the cube's floor, at the cube's middle.
    let (low, high) = session.world_bounds(block).unwrap();
    assert!(
        low.y.abs() < 1e-3,
        "on the floor the cube stood on: {low:?}"
    );
    assert!((high.y - 2.0).abs() < 1e-3, "{high:?}");
    let middle = (low + high) * 0.5;
    assert!(
        (middle.x - 5.0).abs() < 1e-3 && middle.z.abs() < 1e-3,
        "{middle:?}"
    );

    // The file and what made it, where a release will look for drafts.
    assert!(session.project().unwrap().assets().join("drafts/well.glb").exists());
    let found = drafts(session.project().unwrap());
    assert_eq!(found.len(), 1);
    let record = found[0]
        .record
        .as_ref()
        .expect("the record is beside the draft");
    assert_eq!(record.from, "an old stone well");
    assert_eq!(record.provider, "boxes");
    assert_eq!(record.color, "#bc7c59", "the linear 0.5, 0.2, 0.1, in sRGB");

    // The greybox is one undo away.
    session.undo().unwrap();
    assert_eq!(session.scene().get(block).unwrap().model(), "builtin:cube");
}

#[test]
fn a_name_a_real_model_has_is_refused_before_anything_is_paid_for() {
    let (mut session, root) = session("taken");
    std::fs::create_dir_all(root.join("assets/models")).unwrap();
    write_all(root.join("assets/models/well.obj"), "v 0 0 0\n").unwrap();
    let mut generator = Generator::new(Boxes);
    let error = generator
        .start(&mut session, Request::text("well", "a well"))
        .unwrap_err();
    assert!(error.contains("assets/models/well.obj"), "{error}");
    assert!(generator.running().is_empty());
}

#[test]
fn a_failure_is_said_and_changes_nothing() {
    let (mut session, _) = session("fails");
    let block = session.find("block").unwrap();
    let mut generator = Generator::new(Fails);
    let job = generator
        .start(&mut session, Request::text("well", "a well").fit(block))
        .unwrap();
    let finished = generator
        .wait(&mut session, job, Duration::from_secs(10))
        .unwrap();
    assert!(finished.result.unwrap_err().contains("the service said no"));
    assert_eq!(session.scene().get(block).unwrap().model(), "builtin:cube");
    assert!(!session.can_undo(), "no edit was made");
}

/// A binary glTF of an axis-aligned box.
fn glb_box(min: Vec3, max: Vec3, color: [f32; 3]) -> Vec<u8> {
    let corners: Vec<[f32; 3]> = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            ]
        })
        .collect();
    let faces: [[u16; 4]; 6] = [
        [0, 2, 6, 4],
        [1, 5, 7, 3],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 1, 3, 2],
        [4, 6, 7, 5],
    ];
    let indices: Vec<u16> = faces
        .iter()
        .flat_map(|f| [f[0], f[1], f[2], f[0], f[2], f[3]])
        .collect();
    let mut bin: Vec<u8> = corners
        .iter()
        .flat_map(|c| c.iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    let positions_len = bin.len();
    bin.extend(indices.iter().flat_map(|i| i.to_le_bytes()));
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let json = serde_json::json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "materials": [{ "pbrMetallicRoughness": { "baseColorFactor": [color[0], color[1], color[2], 1.0] } }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1, "material": 0 }] }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": positions_len },
            { "buffer": 0, "byteOffset": positions_len, "byteLength": indices.len() * 2 },
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3",
              "min": min.to_array(), "max": max.to_array() },
            { "bufferView": 1, "componentType": 5123, "count": indices.len(), "type": "SCALAR" },
        ],
    });
    let mut json = serde_json::to_vec(&json).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}
