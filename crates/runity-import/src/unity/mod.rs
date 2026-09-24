//! Importing a Unity project's content: scenes, prefabs, materials, models,
//! animator controllers (docs/unity-import.md).
//!
//! Content, not code: a game's scripts are rewritten by hand, and their
//! MonoBehaviours come over as components written as text, for `check` to
//! list until each has its Rust type.

mod animator;
mod look;
mod material;
mod motion;
mod scene;
pub mod yaml;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub use scene::convert_file;
use yaml::Get;

/// What an import did, and what it could not.
#[derive(Debug, Default)]
pub struct Report {
    pub scenes: usize,
    pub prefabs: usize,
    pub materials: usize,
    pub textures: usize,
    pub sounds: usize,
    pub models: usize,
    pub animators: usize,
    pub motions: usize,
    /// ScriptableObjects, as data files under `data/`.
    pub data: usize,
    /// Words from string tables, over every language.
    pub strings: usize,
    /// What was left behind, by kind, with how many times: a component
    /// with no counterpart, a modification it could not carry.
    pub skipped: BTreeMap<String, usize>,
    /// Files that could not be read or written, in words.
    pub errors: Vec<String>,
}

impl Report {
    pub(crate) fn skip(&mut self, what: impl Into<String>) {
        *self.skipped.entry(what.into()).or_default() += 1;
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "{} scenes, {} prefabs, {} materials, {} textures, {} sounds, {} models, {} animators, {} clips, {} data, {} strings",
            self.scenes,
            self.prefabs,
            self.materials,
            self.textures,
            self.sounds,
            self.models,
            self.animators,
            self.motions,
            self.data,
            self.strings
        )?;
        if !self.skipped.is_empty() {
            writeln!(f, "left behind:")?;
            let mut skipped: Vec<_> = self.skipped.iter().collect();
            skipped.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (what, n) in skipped {
                writeln!(f, "  {n:>5} × {what}")?;
            }
        }
        for e in &self.errors {
            writeln!(f, "error: {e}")?;
        }
        Ok(())
    }
}

/// What to bring over.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Convert FBX models through Blender: slow, so asked for.
    pub models: bool,
    /// Blender to use; found on the PATH when not given.
    pub blender: Option<PathBuf>,
    /// Shaders already written again, one `<name>.wgsl` a shader: put in
    /// place of the stubs (never over one written in the project itself).
    pub shaders: Option<PathBuf>,
}

/// A Unity project, indexed: every asset by GUID, and the name each gets in
/// the runity project.
pub struct Unity {
    pub root: PathBuf,
    /// GUID → the asset's file.
    pub guids: HashMap<String, PathBuf>,
    /// GUID → its name in runity: a file stem, made unique within its kind.
    pub names: HashMap<String, String>,
    /// Unity's layer numbers → runity's layer names (from TagManager).
    pub layers: HashMap<i64, String>,
    /// A model's meshes each on its own, by the object they are on in it:
    /// `assets/models/<model>@<object>.glb`, in that object's own frame —
    /// what a MeshFilter that names one mesh of a model draws.
    pub pieces: HashMap<String, Vec<String>>,
}

/// The kind a Unity file becomes in runity, by its extension.
/// The assets of `kind` the project's scenes and prefabs name by GUID —
/// what their components link to, beyond what materials use.
fn referenced(unity: &Unity, kind: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for which in ["scene", "prefab"] {
        for (_, path) in unity.of_kind(which) {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for part in text.split("guid: ").skip(1) {
                let guid: String = part.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
                if unity.named(&guid).is_some_and(|(k, _)| k == kind) {
                    out.insert(guid);
                }
            }
        }
    }
    out
}

pub fn kind_of(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    Some(match extension.as_str() {
        "unity" => "scene",
        "prefab" => "prefab",
        "mat" => "material",
        "fbx" | "obj" | "gltf" | "glb" | "blend" => "model",
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "psd" | "tif" | "tiff" | "exr" => "texture",
        "wav" | "ogg" | "mp3" | "flac" => "sound",
        "controller" => "animator",
        "anim" => "motion",
        "cs" => "script",
        "asset" => "data",
        _ => return None,
    })
}

impl Unity {
    /// Index a Unity project's `Assets/` by the GUIDs in its `.meta` files.
    pub fn open(root: &Path) -> Result<Self> {
        let assets = root.join("Assets");
        anyhow::ensure!(
            assets.is_dir(),
            "{} has no Assets/: not a Unity project",
            root.display()
        );
        let mut guids = HashMap::new();
        walk_all(&assets, &mut |path| {
            if path.extension().is_some_and(|e| e == "meta") {
                if let Some(guid) = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|t| yaml::meta_guid(&t))
                {
                    guids.insert(guid, path.with_extension(""));
                }
            }
        });
        // Names: the file's stem, and where two of one kind share it, the
        // folder before it too — `props_crate`, `tools_crate`.
        let mut by_kind: HashMap<(&str, String), Vec<String>> = HashMap::new();
        for (guid, path) in &guids {
            let Some(kind) = kind_of(path) else { continue };
            let stem = stem(path);
            by_kind.entry((kind, stem)).or_default().push(guid.clone());
        }
        let mut names = HashMap::new();
        for ((_, stem), mut clash) in by_kind {
            clash.sort();
            if clash.len() == 1 {
                names.insert(clash.remove(0), stem);
                continue;
            }
            for guid in clash {
                let folder = guids[&guid]
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                names.insert(guid.clone(), clean(&format!("{folder}_{stem}")));
            }
        }
        let layers = unity_layers(root)
            .map(|(_, by_number)| by_number)
            .unwrap_or_default();
        Ok(Self {
            root: root.to_path_buf(),
            guids,
            names,
            layers,
            pieces: HashMap::new(),
        })
    }

    /// The runity name of the asset a GUID names, and its kind.
    pub fn named(&self, guid: &str) -> Option<(&'static str, &str)> {
        let path = self.guids.get(guid)?;
        Some((kind_of(path)?, self.names.get(guid)?.as_str()))
    }

    /// Every asset of a kind: its GUID and file.
    pub fn of_kind(&self, kind: &str) -> Vec<(&str, &Path)> {
        let mut out: Vec<(&str, &Path)> = self
            .guids
            .iter()
            .filter(|(_, p)| kind_of(p) == Some(kind))
            .map(|(g, p)| (g.as_str(), p.as_path()))
            .collect();
        out.sort_by(|a, b| a.1.cmp(b.1));
        out
    }
}

/// A layer's name as runity writes it: `Ignore Raycast` → `ignore_raycast`.
fn layer_name(unity: &str) -> String {
    snake(&clean(unity)).replace("__", "_")
}

/// Unity's layers (`ProjectSettings/TagManager.asset`) and which pairs do
/// not collide (`DynamicsManager.asset`, its collision matrix), as a
/// `layers.ron`, with Unity's numbers → the names.
pub fn unity_layers(root: &Path) -> Option<(runity::layers::Layers, HashMap<i64, String>)> {
    let tags = std::fs::read_to_string(root.join("ProjectSettings/TagManager.asset")).ok()?;
    let doc = yaml::documents(&tags).into_iter().next()?;
    let mut by_number = HashMap::new();
    let mut names = Vec::new();
    for (i, layer) in doc.body.list("layers").iter().enumerate() {
        let Some(name) = layer.as_str().map(str::trim).filter(|n| !n.is_empty()) else {
            continue;
        };
        let name = if i == 0 {
            "default".to_string()
        } else {
            layer_name(name)
        };
        by_number.insert(i as i64, name.clone());
        names.push((i, name));
    }
    let mut ignore = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join("ProjectSettings/DynamicsManager.asset")) {
        let matrix = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("m_LayerCollisionMatrix:"))
            .map(|m| m.trim().to_string())
            .unwrap_or_default();
        // 32 masks of 8 hex digits, each a little-endian u32: bit j of
        // mask i, layer i collides with layer j.
        let mask = |i: usize| -> Option<u32> {
            let hex = matrix.get(i * 8..i * 8 + 8)?;
            let bytes = u32::from_str_radix(hex, 16).ok()?;
            Some(bytes.swap_bytes())
        };
        for (a, (i, first)) in names.iter().enumerate() {
            for (j, second) in names.iter().skip(a) {
                // Either side saying no is no: the file's two halves need
                // not agree.
                let off = |a: usize, b: usize| mask(a).is_some_and(|m| m & (1 << b) == 0);
                if off(*i, *j) || off(*j, *i) {
                    ignore.push((first.clone(), second.clone()));
                }
            }
        }
    }
    let layers = runity::layers::Layers {
        layers: names.into_iter().map(|(_, n)| n).collect(),
        ignore,
    };
    Some((layers, by_number))
}

/// Bring a Unity project's content into a runity project.
pub fn import_unity(unity: &Path, project: &runity::Project, options: &Options) -> Result<Report> {
    let mut unity = Unity::open(unity)?;
    let mut report = Report::default();

    // Models first: a scene's renderer names one mesh of a model, and
    // which ones there are is known once they are converted.
    if options.models {
        models(&unity, project, options, &mut report);
    } else {
        let n = unity.of_kind("model").len();
        if n > 0 {
            report.skip(format!(
                "{n} models: pass --models to convert them through Blender"
            ));
        }
    }
    unity.pieces = pieces(&project.assets().join("models"));
    keep_origins(&project.assets().join("models"))?;

    if let Some((layers, _)) = unity_layers(&unity.root) {
        let text = ron::ser::to_string_pretty(&layers, ron::ser::PrettyConfig::new())?;
        write(
            &project.root().join(runity::layers::FILE),
            &format!("{text}\n"),
        )?;
    }

    // Textures first, and only those materials — or the scenes' and
    // prefabs' components — use: a project's Assets/ holds many a picture
    // nothing draws.
    let (mut used, data_only) = material::textures_used(&unity);
    used.extend(referenced(&unity, "texture"));
    let textures = project.assets().join("textures");
    for guid in &used {
        let (Some(path), Some(name)) = (unity.guids.get(guid), unity.names.get(guid)) else {
            continue;
        };
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "tga" | "bmp") {
            report.skip(format!("texture in .{extension} (convert it to PNG)"));
            continue;
        }
        std::fs::create_dir_all(&textures)?;
        let to = textures.join(format!("{name}.{extension}"));
        if let Err(e) = std::fs::copy(path, &to) {
            report.errors.push(format!("{}: {e}", path.display()));
            continue;
        }
        report.textures += 1;
        // Data, not colour — a normal map, a mask — as Unity's importer
        // said, or as every material using it uses it: not decoded from
        // sRGB.
        let unity_meta = std::fs::read_to_string(meta_of(path)).unwrap_or_default();
        let data = data_only.contains(guid)
            || unity_meta.lines().any(|l| {
                let l = l.trim();
                l == "sRGBTexture: 0" || l == "textureType: 1"
            });
        if data {
            let mut settings = crate::ImportSettings::for_source(
                project
                    .relative(&to)
                    .unwrap_or_else(|| to.to_string_lossy().into_owned()),
            );
            settings.srgb = false;
            let _ = settings.save(crate::sidecar_for(&to));
        }
    }

    // Sounds the scenes and prefabs name: short ones decoded on sync, long
    // ones kept compressed to stream.
    let sounds = project.assets().join("sounds");
    for guid in referenced(&unity, "sound") {
        let (Some(path), Some(name)) = (unity.guids.get(&guid), unity.names.get(&guid)) else {
            continue;
        };
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        std::fs::create_dir_all(&sounds)?;
        if let Err(e) = std::fs::copy(path, sounds.join(format!("{name}.{extension}"))) {
            report.errors.push(format!("{}: {e}", path.display()));
            continue;
        }
        report.sounds += 1;
    }

    let mut shaders: std::collections::BTreeMap<String, PathBuf> = Default::default();
    for (guid, path) in unity.of_kind("material") {
        // A shader's parameters, from the one written again if there is
        // one, else the project's own.
        let declared = |name: &str| {
            let file = format!("{name}.wgsl");
            options
                .shaders
                .iter()
                .map(|d| d.join(&file))
                .chain(std::iter::once(
                    project.root().join(runity::project::SHADERS).join(&file),
                ))
                .find_map(|p| std::fs::read_to_string(p).ok())
        };
        match material::convert_with(&unity, path, &declared) {
            Ok(text) => {
                let name = &unity.names[guid];
                write(&project.materials().join(format!("{name}.rmat")), &text)?;
                report.materials += 1;
                if let Some(own) = std::fs::read_to_string(path).ok().and_then(|t| {
                    yaml::documents(&t)
                        .into_iter()
                        .find(|d| d.kind == "Material")
                        .and_then(|d| material::own_shader(&unity, &d.body))
                }) {
                    shaders.insert(own.0, own.1);
                }
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", path.display())),
        }
    }
    // The shaders those materials had: a stub each to write again, never
    // over one already written.
    let dir = project.root().join(runity::project::SHADERS);
    for (name, path) in &shaders {
        let file = dir.join(format!("{name}.wgsl"));
        let stub =
            |f: &Path| std::fs::read_to_string(f).is_ok_and(|t| t.contains(material::STUB_MARK));
        let written = options
            .shaders
            .as_ref()
            .map(|d| d.join(format!("{name}.wgsl")))
            .filter(|f| f.is_file());
        match written {
            Some(from) if !file.exists() || stub(&file) => {
                std::fs::create_dir_all(&dir)?;
                std::fs::copy(&from, &file)?;
            }
            _ if !file.exists() => {
                write(&file, &material::shader_stub(&unity, name, path))?;
                report.skip(format!("a shader to write again (shaders/{name}.wgsl)"));
            }
            _ if stub(&file) => {
                report.skip(format!("a shader to write again (shaders/{name}.wgsl)"))
            }
            _ => {}
        }
    }

    for (guid, path) in unity.of_kind("prefab") {
        let Some(text) = read_text(path, &mut report) else {
            continue;
        };
        let roots = scene::convert_file(&unity, &text, &mut report);
        let Some(root) = roots.into_iter().next() else {
            report
                .errors
                .push(format!("{}: no root object", path.display()));
            continue;
        };
        let name = &unity.names[guid];
        let file = project
            .prefabs()
            .join(format!("{name}.{}", runity::prefab::EXTENSION));
        runity::Prefabs::save(&root, &file).map_err(anyhow::Error::msg)?;
        report.prefabs += 1;
    }

    for (guid, path) in unity.of_kind("scene") {
        let Some(text) = read_text(path, &mut report) else {
            continue;
        };
        let entities = scene::convert_file(&unity, &text, &mut report);
        let mut scene = runity::Scene {
            entities,
            ..Default::default()
        };
        if let Some(sun) = scene::sun(&text) {
            scene.set_part(&sun);
        }
        // How it looks: its fog, and its global Volume's grade.
        if let Some(fog) = look::fog(&text) {
            scene.set_part(&fog);
        }
        scene.set_part_opt((look::post(&unity, &text, &mut report)).as_ref());
        let name = &unity.names[guid];
        scene
            .save(project.scenes().join(format!("{name}.ron")))
            .with_context(|| format!("scene {name}"))?;
        report.scenes += 1;
    }

    // ScriptableObjects: a game's configs and graphs, its own fields as
    // RON under `data/`, named as Unity names the asset. Other `.asset`
    // files (lighting, terrain, settings) are not a MonoBehaviour and pass.
    // String tables: a CSV of `key,<language>,<language>…` rows, as
    // `strings/<language>.ron`, merged into what is there.
    let mut tables: Vec<&PathBuf> = unity
        .guids
        .values()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")))
        .collect();
    tables.sort();
    for path in tables {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        for (language, words) in string_table(&text) {
            let file = project.root().join("strings").join(format!("{language}.ron"));
            let mut all: BTreeMap<String, String> = std::fs::read_to_string(&file)
                .ok()
                .and_then(|t| runity::ron::from_str(&t).ok())
                .unwrap_or_default();
            report.strings += words.len();
            all.extend(words);
            let body = runity::ron::ser::to_string_pretty(&all, Default::default())?;
            write(&file, &format!("// The game's words in `{language}`.\n{body}\n"))?;
        }
    }

    for (guid, path) in unity.of_kind("data") {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some((script, body)) = scene::data_asset(&unity, &text) else {
            continue;
        };
        let name = &unity.names[guid];
        let text = format!("// {script}, from {}\n{body}\n", path.display());
        write(&project.root().join("data").join(format!("{name}.ron")), &text)?;
        report.data += 1;
    }

    for (guid, path) in unity.of_kind("animator") {
        match animator::convert(&unity, path) {
            Ok(text) => {
                let dir = project.root().join(runity::project::ANIMATORS);
                let name = &unity.names[guid];
                write(&dir.join(format!("{name}.ron")), &text)?;
                report.animators += 1;
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", path.display())),
        }
    }

    for (guid, path) in unity.of_kind("motion") {
        match motion::convert(path, &mut report) {
            Ok(clip) => {
                let dir = project.root().join(runity::motion::DIR);
                let name = &unity.names[guid];
                write(&dir.join(format!("{name}.ron")), &motion::text(&clip))?;
                report.motions += 1;
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", path.display())),
        }
    }

    Ok(report)
}

/// A Unity model's origin is where its scenes put it: its mesh is not
/// moved down to stand on y = 0, as a model dropped into a runity project
/// is. Every converted model's sidecar says so — a new one, or an old one
/// set right and made to import again.
fn keep_origins(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "glb") {
            continue;
        }
        let sidecar = crate::sidecar_for(&path);
        let mut settings = match crate::ImportSettings::load(&sidecar) {
            Ok(settings) => settings,
            Err(_) => crate::ImportSettings::for_source(format!(
                "assets/models/{}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        };
        if settings.origin_to_base || !sidecar.is_file() {
            settings.origin_to_base = false;
            settings.hash = String::new();
            settings.save(&sidecar)?;
        }
    }
    Ok(())
}

/// The pieces converted models have, from the files: `<model>@<object>.glb`.
fn pieces(dir: &Path) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "glb") {
            continue;
        }
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        if let Some((model, piece)) = stem.split_once('@') {
            out.entry(model.to_string()).or_default().push(piece.to_string());
        }
    }
    for list in out.values_mut() {
        list.sort();
    }
    out
}

/// A piece's file name for a Blender object's name: what a file name
/// can hold.
pub(crate) fn piece_name(object: &str) -> String {
    object
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

/// FBX (and friends) to GLB through Blender, into `assets/models/`.
fn models(unity: &Unity, project: &runity::Project, options: &Options, report: &mut Report) {
    let blender = options
        .blender
        .clone()
        .unwrap_or_else(|| PathBuf::from("blender"));
    let out = project.assets().join("models");
    let _ = std::fs::create_dir_all(&out);
    let all = unity.of_kind("model");
    let count = all.len();
    for (i, (guid, path)) in all.into_iter().enumerate() {
        let name = &unity.names[guid];
        eprintln!("model {}/{count} {name}", i + 1);
        let to = out.join(format!("{name}.glb"));
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if matches!(extension.as_str(), "gltf" | "glb" | "obj") {
            let to = out.join(format!("{name}.{extension}"));
            match std::fs::copy(path, &to) {
                Ok(_) => report.models += 1,
                Err(e) => report.errors.push(format!("{}: {e}", path.display())),
            }
            continue;
        }
        // Unity takes an FBX in by mirroring X; the scene comes over by
        // mirroring Z (docs/unity-import.md). Between the two the model is
        // turned half a turn about up: done here, under a parent the
        // glTF importer bakes into the mesh.
        //
        // Each mesh also on its own, in its object's own frame: a Unity
        // renderer that names one mesh of a model draws it there, the
        // object's turn and place being the GameObject's. Blender keeps an
        // FBX mesh's vertices as the file has them — Unity's, but for its
        // mirror — so the piece is those vertices turned half about up,
        // put through the matrix that undoes glTF's own Z-up to Y-up, and
        // in metres as Unity reads the file's units (its UnitScaleFactor,
        // centimetres to the unit).
        let pieces_dir = to.with_file_name("");
        let script = format!(
            "import bpy, math, mathutils, re\n\
             bpy.ops.wm.read_factory_settings(use_empty=True)\n\
             bpy.ops.import_scene.fbx(filepath={:?})\n\
             scene = bpy.context.scene\n\
             import struct\n\
             data = open({:?}, 'rb').read()\n\
             unit = 100.0\n\
             at = data.find(b'UnitScaleFactor')\n\
             if at >= 0:\n\
             \x20   for k in range(at + 15, at + 120):\n\
             \x20       if data[k:k+1] == b'D':\n\
             \x20           unit = struct.unpack('<d', data[k+1:k+9])[0]\n\
             \x20           break\n\
             frame = mathutils.Matrix(((-1,0,0,0),(0,0,1,0),(0,1,0,0),(0,0,0,1))) @ mathutils.Matrix.Scale(unit / 100.0, 4)\n\
             for o in [o for o in scene.objects if o.type == 'MESH']:\n\
             \x20   parent, placed = o.parent, o.matrix_world.copy()\n\
             \x20   for x in scene.objects: x.select_set(False)\n\
             \x20   o.select_set(True)\n\
             \x20   o.parent = None\n\
             \x20   o.matrix_world = frame\n\
             \x20   piece = re.sub(r'[^A-Za-z0-9_-]', '_', o.name)\n\
             \x20   bpy.ops.export_scene.gltf(filepath={:?} + '/' + {:?} + '@' + piece + '.glb', export_format='GLB', use_selection=True, export_animations=False, export_skins=False)\n\
             \x20   o.parent = parent\n\
             \x20   o.matrix_world = placed\n\
             for x in scene.objects: x.select_set(False)\n\
             roots = [o for o in bpy.context.scene.objects if o.parent is None]\n\
             turn = bpy.data.objects.new('unity_turn', None)\n\
             bpy.context.scene.collection.objects.link(turn)\n\
             for o in roots: o.parent = turn\n\
             turn.rotation_euler[2] = math.pi\n\
             bpy.ops.export_scene.gltf(filepath={:?}, export_format='GLB', export_animations=True)\n",
            path.to_string_lossy(),
            path.to_string_lossy(),
            pieces_dir.to_string_lossy().trim_end_matches('/'),
            name,
            to.to_string_lossy()
        );
        let result = std::process::Command::new(&blender)
            .args(["-b", "--factory-startup", "--python-expr", &script])
            .output();
        match result {
            Ok(o) if o.status.success() && to.is_file() => report.models += 1,
            Ok(o) => report.errors.push(format!(
                "{}: Blender could not convert it: {}",
                path.display(),
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
            )),
            Err(e) => {
                report.errors.push(format!(
                    "Blender ({}) did not start: {e}",
                    blender.display()
                ));
                return;
            }
        }
    }
}

/// A Unity file as text, or why not: a file saved in Unity's binary
/// serialization is not YAML, and is named in the report.
fn read_text(path: &Path, report: &mut Report) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.starts_with(b"%YAML") => String::from_utf8(bytes).ok(),
        Ok(_) => {
            report.errors.push(format!(
                "{}: saved as binary, not text — in Unity, Project Settings › Editor › Asset Serialization: Force Text",
                path.display()
            ));
            None
        }
        Err(e) => {
            report.errors.push(format!("{}: {e}", path.display()));
            None
        }
    }
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text).with_context(|| format!("{}", path.display()))
}

/// A string table's words by language: a CSV whose header is `key,en,ru…`,
/// `#` lines and blank ones passed over, fields quoted as CSV quotes them.
/// Anything else is not a string table and gives nothing.
pub fn string_table(text: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut rows = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(csv_row);
    let Some(header) = rows.next() else { return Vec::new() };
    if header.first().map(String::as_str) != Some("key") || header.len() < 2 {
        return Vec::new();
    }
    let mut out: Vec<(String, BTreeMap<String, String>)> =
        header[1..].iter().map(|l| (l.clone(), BTreeMap::new())).collect();
    for row in rows {
        let Some(key) = row.first().filter(|k| !k.is_empty()) else { continue };
        for (i, (_, words)) in out.iter_mut().enumerate() {
            if let Some(text) = row.get(i + 1).filter(|t| !t.is_empty()) {
                words.insert(key.clone(), text.clone());
            }
        }
    }
    out
}

/// One CSV line's fields.
fn csv_row(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    out.push(field);
    out
}

/// A Unity asset's `.meta` file.
pub(crate) fn meta_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".meta");
    PathBuf::from(name)
}

pub(crate) fn stem(path: &Path) -> String {
    clean(
        &path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

/// A name a runity file can have: letters, digits, `_` and `-`; spaces and
/// the rest become `_`.
pub(crate) fn clean(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

/// `PlayerController` → `player_controller`: a component's name as a
/// runity game writes it.
pub(crate) fn snake(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev_lower =
                i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if i > 0 && (prev_lower || (next_lower && chars[i - 1].is_uppercase())) {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(*c);
        }
    }
    out
}

/// Every file under `root`, hidden or not except `.git`: Unity keeps
/// nothing that matters in dotfiles, but `walk` skips them all.
fn walk_all(root: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if path.is_dir() {
            walk_all(&path, visit);
        } else {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_csv_string_table_is_words_by_language() {
        let text = "key,en,ru\n# a note\n\nmenu.play,Play,Играть\ntutorial.b1,\"Wake up, crew! Say \"\"hi\"\".\",Подъём\nonly.en,Only,\n";
        let tables = string_table(text);
        assert_eq!(tables.len(), 2);
        let (en, words) = &tables[0];
        assert_eq!(en, "en");
        assert_eq!(words["tutorial.b1"], "Wake up, crew! Say \"hi\".");
        assert_eq!(tables[1].1["menu.play"], "Играть");
        assert!(!tables[1].1.contains_key("only.en"), "an empty cell is no word");
        assert!(string_table("name,age\nbob,3\n").is_empty());
    }

    #[test]
    fn names_are_what_a_runity_file_can_be_called() {
        assert_eq!(snake("PlayerController"), "player_controller");
        assert_eq!(snake("HTTPClient"), "http_client");
        assert_eq!(snake("Item3D"), "item3_d");
        assert_eq!(clean("Big Rock (1)"), "Big_Rock_1");
    }
}
