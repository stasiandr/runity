//! `.blend` files, read through Blender itself (docs/blender.md, stage 2).
//!
//! Nobody but Blender reads a `.blend` properly — the format is Blender's
//! memory, and modifiers, instances and node trees only mean something
//! once Blender has evaluated them. So the importer starts Blender in the
//! background with the runity plugin's exporter (`tools/blender/runity/`),
//! and the plugin sends what Blender evaluated — meshes with modifiers
//! applied, the object tree, materials, images, properties — over a
//! socket on this machine. Nothing in between is written to disk, and the
//! `.rasset` is written here, by the engine's own code: the plugin never
//! learns the engine's formats (postulate 7, one truth per subsystem).
//!
//! The stream: `RUNITYBL`, a `u32` version, a `u32` length and that much
//! JSON describing the scene, then a `u32` count of blobs, each a `u64`
//! length and its bytes. The JSON names blobs by index. Plugin and
//! importer live in one repository and change together; the stream is
//! never kept.
//!
//! Blender's space is Z up and the engine's Y up: `(x, y, z)` becomes
//! `(x, z, -y)`, as Blender's own glTF exporter does it.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use glam::{Quat, Vec3};
use runity::asset::Vertex;
use runity::material::{Material, RenderFace, SurfaceType};
use runity::{EntityId, Transform};
use serde::Deserialize;

use crate::scene::{
    AssetData, ImageData, MaterialData, MeshData, NodeData, PrimitiveData, SceneData,
};

/// The plugin's exporter, run inside Blender.
pub const EXPORTER: &str = include_str!("../../../tools/blender/runity/export.py");

/// The add-on itself, as Blender installs it: what [`install`] writes.
pub const ADDON: [(&str, &str); 3] = [
    (
        "__init__.py",
        include_str!("../../../tools/blender/runity/__init__.py"),
    ),
    ("export.py", EXPORTER),
    (
        "blender_manifest.toml",
        include_str!("../../../tools/blender/runity/blender_manifest.toml"),
    ),
];

/// Install the runity add-on into this Blender and turn it on: an
/// extension in the user's `user_default` repository, enabled in the
/// user's preferences, which are saved with everything else in them as it
/// was. What the editor's Install Blender Plugin does. Returns the folder
/// it went into.
pub fn install(blender: &Path) -> Result<PathBuf> {
    const SCRIPT: &str = r#"
import bpy, json, os, addon_utils
files = json.loads(os.environ["RUNITY_ADDON_FILES"])
folder = os.path.join(bpy.utils.user_resource("EXTENSIONS", path="user_default", create=True), "runity")
os.makedirs(folder, exist_ok=True)
for name, text in files.items():
    with open(os.path.join(folder, name), "w", encoding="utf-8") as f:
        f.write(text)
try:
    bpy.ops.extensions.repo_refresh_all()
except Exception as e:
    print("refresh:", e)
addon_utils.enable("bl_ext.user_default.runity", default_set=True, persistent=True)
bpy.ops.wm.save_userpref()
print("RUNITY_INSTALLED " + folder)
"#;
    let files: serde_json::Map<String, serde_json::Value> = ADDON
        .iter()
        .map(|(name, text)| (name.to_string(), serde_json::Value::from(*text)))
        .collect();
    let out = Command::new(blender)
        .args([
            "--background",
            "--python-exit-code",
            "1",
            "--python-expr",
            SCRIPT,
        ])
        .env(
            "RUNITY_ADDON_FILES",
            serde_json::Value::Object(files).to_string(),
        )
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("starting {}", blender.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text
        .lines()
        .find_map(|l| l.strip_prefix("RUNITY_INSTALLED "))
    {
        Some(folder) if out.status.success() => Ok(PathBuf::from(folder.trim())),
        _ => bail!(
            "Blender could not install the plugin:\n{}",
            last_lines(
                &format!("{text}\n{}", String::from_utf8_lossy(&out.stderr)),
                12
            )
        ),
    }
}

/// Whether a Blender with a window is running on this machine. Installing
/// the plugin while one is open does not stick: Blender saves its
/// preferences when it quits, from memory, over the ones [`install`] just
/// wrote — the plugin stays in its folder, turned off. Blenders run with
/// `--background`, as the importer runs them, do not count.
pub fn blender_open() -> bool {
    if cfg!(windows) {
        // tasklist cannot tell a background Blender from one with a window;
        // the importer's are gone in seconds.
        return Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq blender.exe", "/NH"])
            .output()
            .is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .to_lowercase()
                    .contains("blender.exe")
            });
    }
    let Ok(out) = Command::new("ps").args(["-axo", "args="]).output() else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let mut args = line.split_whitespace();
        let program = args.next().unwrap_or_default();
        let name = program.rsplit('/').next().unwrap_or_default();
        name.eq_ignore_ascii_case("blender")
            && !args.any(|a| a == "--background" || a == "-b")
    })
}

/// The stream's version; the plugin writes the same.
pub const VERSION: u32 = 1;

const MAGIC: &[u8; 8] = b"RUNITYBL";

/// How long Blender gets to open a file and send it.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Where Blender is: `RUNITY_BLENDER`, then where macOS installs it, then
/// `blender` on the `PATH`.
pub fn blender() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RUNITY_BLENDER") {
        return Some(PathBuf::from(path));
    }
    for path in [
        "/Applications/Blender.app/Contents/MacOS/Blender",
        "C:\\Program Files\\Blender Foundation\\Blender\\blender.exe",
    ] {
        if Path::new(path).is_file() {
            return Some(PathBuf::from(path));
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| {
            dir.join(if cfg!(windows) {
                "blender.exe"
            } else {
                "blender"
            })
        })
        .find(|p| p.is_file())
}

/// Read a `.blend` through Blender in the background.
pub fn read(source: &Path) -> Result<SceneData> {
    let Some(blender) = blender() else {
        bail!(
            "Blender is needed to import a .blend and was not found — install it, or set RUNITY_BLENDER to its executable"
        );
    };
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let mut child = Command::new(&blender)
        .arg("--background")
        .arg("--factory-startup")
        .arg(source)
        .arg("--python-expr")
        .arg(EXPORTER)
        .arg("--")
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", blender.display()))?;

    let started = Instant::now();
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if let Some(status) = child.try_wait()? {
                    let mut err = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        let _ = e.read_to_string(&mut err);
                    }
                    let mut out = String::new();
                    if let Some(mut o) = child.stdout.take() {
                        let _ = o.read_to_string(&mut out);
                    }
                    bail!(
                        "Blender stopped ({status}) without sending the scene:\n{}",
                        last_lines(&format!("{out}\n{err}"), 12)
                    );
                }
                if started.elapsed() > TIMEOUT {
                    let _ = child.kill();
                    bail!(
                        "Blender did not send the scene within {}s",
                        TIMEOUT.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e.into()),
        }
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    let _ = stream.flush();
    let status = child.wait()?;
    let data = parse(&bytes)
        .with_context(|| format!("Blender ({status}) sent a stream that does not read"))?;
    Ok(data)
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

// --- the stream ------------------------------------------------------

#[derive(Deserialize)]
struct Header {
    version: u32,
    #[serde(default)]
    images: Vec<ImageHeader>,
    #[serde(default)]
    materials: Vec<MaterialHeader>,
    #[serde(default)]
    meshes: Vec<MeshHeader>,
    #[serde(default)]
    level: Vec<NodeHeader>,
    #[serde(default)]
    assets: Vec<AssetHeader>,
}

#[derive(Deserialize)]
struct ImageHeader {
    key: String,
    name: String,
    width: u32,
    height: u32,
    /// RGBA as `f32`, bottom row first, as Blender keeps it.
    pixels: usize,
}

#[derive(Deserialize)]
struct MaterialHeader {
    key: String,
    name: String,
    base_color: [f32; 3],
    #[serde(default = "one")]
    alpha: f32,
    #[serde(default)]
    metallic: f32,
    #[serde(default = "half")]
    roughness: f32,
    #[serde(default)]
    emission: [f32; 3],
    #[serde(default)]
    transparent: bool,
    #[serde(default)]
    double_sided: bool,
    base_map: Option<usize>,
    emission_map: Option<usize>,
    normal_map: Option<usize>,
    #[serde(default = "one")]
    normal_scale: f32,
    metallic_map: Option<(usize, usize)>,
    roughness_map: Option<(usize, usize)>,
    occlusion_map: Option<(usize, usize)>,
}

fn one() -> f32 {
    1.0
}

fn half() -> f32 {
    0.5
}

#[derive(Deserialize)]
struct MeshHeader {
    key: String,
    name: String,
    /// `f32 × 3` per vertex.
    positions: usize,
    /// `u32` per corner: its vertex.
    corners: usize,
    /// `f32 × 3` per corner.
    normals: usize,
    /// `f32 × 2` per corner, when the mesh has UVs.
    uvs: Option<usize>,
    /// `u32 × 3` per triangle: its corners.
    triangles: usize,
    /// `u32` per triangle: its material slot.
    slots: usize,
    /// Per slot, the material it holds, by index.
    materials: Vec<Option<usize>>,
}

#[derive(Deserialize)]
struct NodeHeader {
    name: String,
    id: Option<String>,
    location: [f32; 3],
    /// w, x, y, z — Blender's order.
    rotation: [f32; 4],
    scale: [f32; 3],
    mesh: Option<usize>,
    prefab: Option<String>,
    #[serde(default)]
    components: BTreeMap<String, String>,
    #[serde(default)]
    collider: bool,
    /// A material of the project's to draw with instead of the file's.
    material: Option<String>,
    #[serde(default)]
    children: Vec<NodeHeader>,
}

#[derive(Deserialize)]
struct AssetHeader {
    key: String,
    name: String,
    roots: Vec<NodeHeader>,
}

/// One material's triangles as they are gathered: vertices, indices, and
/// which corner became which vertex.
type Gathering = (Vec<Vertex>, Vec<u32>, BTreeMap<u32, u32>);

/// Blender's Z up to the engine's Y up.
fn axis(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[0], v[2], -v[1])
}

/// A placement as Blender gives it — location, rotation as w, x, y, z,
/// scale — in the engine's space.
pub fn placement(location: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> Transform {
    let [w, x, y, z] = rotation;
    let mut transform = Transform {
        position: axis(location),
        scale: Vec3::new(scale[0], scale[2], scale[1]),
        ..Default::default()
    };
    transform.set_rotation(Quat::from_xyzw(x, z, -y, w).normalize());
    transform
}

/// Import a `.blend` from a stream an open Blender sent — the plugin does
/// on every save — rather than by starting Blender: the same assets, a
/// second sooner. The sidecar is written as [`crate::import_into`] would,
/// with the hash of the file as saved, so the next poll finds it current.
pub fn import_stream(
    project: &runity::Project,
    source: &Path,
    bytes: &[u8],
) -> Result<runity::AssetId> {
    let data = parse(bytes)?;
    let name = project
        .relative(source)
        .with_context(|| format!("{} is not in the project", source.display()))?;
    let sidecar = crate::sidecar_for(source);
    let mut settings = crate::ImportSettings::load(&sidecar)
        .unwrap_or_else(|_| crate::ImportSettings::for_source(name.clone()));
    settings.source = name;
    settings.scene = true;
    settings.hash = crate::content_hash(source)?;
    settings.id = Some(settings.asset_id());
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scene".into());
    let id = crate::scene::build(data, &stem, &project.library(), &mut settings)?;
    settings.save(&sidecar)?;
    Ok(id)
}

/// Read the stream the plugin sends.
pub fn parse(bytes: &[u8]) -> Result<SceneData> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8]> {
        ensure!(at + n <= bytes.len(), "the stream ends early");
        let out = &bytes[at..at + n];
        at += n;
        Ok(out)
    };
    ensure!(take(8)? == MAGIC, "not a runity stream");
    let version = u32::from_le_bytes(take(4)?.try_into()?);
    ensure!(
        version == VERSION,
        "the plugin speaks version {version} and this importer {VERSION}: update the runity plugin in Blender"
    );
    let length = u32::from_le_bytes(take(4)?.try_into()?) as usize;
    let header: Header = serde_json::from_slice(take(length)?)?;
    ensure!(
        header.version == VERSION,
        "header version {}",
        header.version
    );
    let count = u32::from_le_bytes(take(4)?.try_into()?) as usize;
    let mut blobs: Vec<&[u8]> = Vec::with_capacity(count);
    for _ in 0..count {
        let n = u64::from_le_bytes(take(8)?.try_into()?) as usize;
        blobs.push(take(n)?);
    }
    let blob = |i: usize| -> Result<&[u8]> {
        blobs
            .get(i)
            .copied()
            .with_context(|| format!("no blob {i}"))
    };
    let floats = |i: usize| -> Result<Vec<f32>> {
        Ok(blob(i)?
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    };
    let words = |i: usize| -> Result<Vec<u32>> {
        Ok(blob(i)?
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    };

    let mut data = SceneData::default();
    for image in header.images {
        let pixels = floats(image.pixels)?;
        let (w, h) = (image.width as usize, image.height as usize);
        ensure!(
            pixels.len() == w * h * 4,
            "image `{}`: {} values for {w}×{h}",
            image.name,
            pixels.len()
        );
        // Top row first, as every other image the engine reads.
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in (0..h).rev() {
            for v in &pixels[y * w * 4..(y + 1) * w * 4] {
                rgba.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        data.images.push(ImageData {
            key: image.key,
            name: image.name,
            width: image.width,
            height: image.height,
            rgba,
        });
    }
    for m in header.materials {
        let [r, g, b] = m.base_color;
        let mut material = Material::new(r, g, b);
        material.alpha = m.alpha;
        material.metallic = m.metallic;
        material.smoothness = 1.0 - m.roughness;
        material.emission = m.emission;
        material.normal_scale = m.normal_scale;
        if m.transparent {
            material.surface = SurfaceType::Transparent;
        }
        if m.double_sided {
            material.render_face = RenderFace::Both;
        }
        data.materials.push(MaterialData {
            key: m.key,
            name: m.name,
            material,
            base_map: m.base_map,
            emission_map: m.emission_map,
            normal_map: m.normal_map,
            metallic_map: m.metallic_map,
            roughness_map: m.roughness_map,
            occlusion_map: m.occlusion_map,
        });
    }
    for mesh in header.meshes {
        let positions = floats(mesh.positions)?;
        let corners = words(mesh.corners)?;
        let normals = floats(mesh.normals)?;
        let uvs = mesh.uvs.map(floats).transpose()?;
        let triangles = words(mesh.triangles)?;
        let slots = words(mesh.slots)?;
        // A primitive per material slot used, in slot order; a vertex per
        // corner, since a corner carries its own normal and UV.
        let mut by_slot: BTreeMap<u32, Gathering> = BTreeMap::new();
        for (t, tri) in triangles.chunks_exact(3).enumerate() {
            let slot = slots.get(t).copied().unwrap_or(0);
            let (vertices, indices, seen) = by_slot.entry(slot).or_default();
            for &corner in tri {
                let index = *seen.entry(corner).or_insert_with(|| {
                    let c = corner as usize;
                    let v = corners.get(c).copied().unwrap_or(0) as usize;
                    let p = positions
                        .get(v * 3..v * 3 + 3)
                        .map_or([0.0; 3], |p| [p[0], p[1], p[2]]);
                    let n = normals
                        .get(c * 3..c * 3 + 3)
                        .map_or([0.0; 3], |n| [n[0], n[1], n[2]]);
                    let uv = uvs
                        .as_ref()
                        .and_then(|uv| uv.get(c * 2..c * 2 + 2))
                        .map_or([0.0; 2], |uv| [uv[0], 1.0 - uv[1]]);
                    vertices.push(Vertex {
                        position: axis(p).to_array(),
                        normal: axis(n).normalize_or_zero().to_array(),
                        uv,
                    });
                    vertices.len() as u32 - 1
                });
                indices.push(index);
            }
        }
        let primitives = by_slot
            .into_iter()
            .map(|(slot, (vertices, indices, _))| PrimitiveData {
                vertices,
                indices,
                material: mesh.materials.get(slot as usize).copied().flatten(),
                has_normals: true,
            })
            .collect();
        data.meshes.push(MeshData {
            key: mesh.key,
            name: mesh.name,
            primitives,
        });
    }
    fn node(n: NodeHeader) -> NodeData {
        let transform = placement(n.location, n.rotation, n.scale);
        NodeData {
            name: n.name,
            id: n.id.and_then(|id| id.parse::<EntityId>().ok()),
            transform,
            mesh: n.mesh,
            prefab: n.prefab,
            components: n.components,
            collider: n.collider,
            material: n.material,
            children: n.children.into_iter().map(node).collect(),
        }
    }
    data.level = header.level.into_iter().map(node).collect();
    data.assets = header
        .assets
        .into_iter()
        .map(|a| AssetData {
            key: a.key,
            name: a.name,
            roots: a.roots.into_iter().map(node).collect(),
        })
        .collect();
    Ok(data)
}
