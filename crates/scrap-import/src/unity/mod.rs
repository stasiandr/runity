//! Importing a Unity project's content: scenes, prefabs, materials, models,
//! animator controllers (docs/unity-import.md).
//!
//! Content, not code: a game's scripts are rewritten by hand, and their
//! MonoBehaviours come over as components written as text, for `check` to
//! list until each has its Rust type.

mod animator;
mod look;
mod material;
mod mesh;
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
    /// Only the models whose name has this in it (with `models`): one
    /// model converted again without all the others.
    pub models_matching: Option<String>,
    /// Blender to use; found on the PATH when not given.
    pub blender: Option<PathBuf>,
    /// Shaders already written again, one `<name>.wgsl` a shader: put in
    /// place of the stubs (never over one written in the project itself).
    pub shaders: Option<PathBuf>,
}

/// A Unity project, indexed: every asset by GUID, and the name each gets in
/// the scrap project.
pub struct Unity {
    pub root: PathBuf,
    /// GUID → the asset's file.
    pub guids: HashMap<String, PathBuf>,
    /// GUID → its name in scrap: a file stem, made unique within its kind.
    pub names: HashMap<String, String>,
    /// Unity's layer numbers → scrap's layer names (from TagManager).
    pub layers: HashMap<i64, String>,
    /// A model's meshes each on its own, by the object they are on in it:
    /// `assets/models/<model>@<object>.glb`, in that object's own frame —
    /// what a MeshFilter that names one mesh of a model draws.
    pub pieces: HashMap<String, Vec<String>>,
    /// A mesh of a model (its GUID and fileID) → the piece it is, from the
    /// prefabs that draw it on an object named as one of the model's
    /// pieces — and only where they all agree.
    pub mesh_pieces: HashMap<(String, i64), String>,
    /// A material's name → what its shader written again says its eight
    /// numbers are (`// scrap:params`): where a particle's custom data goes.
    pub declared_params: HashMap<String, Vec<String>>,
    /// The `.asset` files that hold a Mesh: models, made without Blender.
    pub mesh_assets: std::collections::HashSet<PathBuf>,
}

/// The kind a Unity file becomes in scrap, by its extension.
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
        // The render pipeline's own materials (URP's Lit, the default a
        // cube is made with): a scene names them by GUID as it does its own.
        for package in std::fs::read_dir(root.join("Library/PackageCache")).into_iter().flatten().flatten() {
            let dir = package.path().join("Runtime/Materials");
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "meta")
                    && path.with_extension("").extension().is_some_and(|e| e == "mat")
                {
                    if let Some(guid) = std::fs::read_to_string(&path).ok().and_then(|t| yaml::meta_guid(&t)) {
                        guids.entry(guid).or_insert_with(|| path.with_extension(""));
                    }
                }
            }
        }
        let mesh_assets: std::collections::HashSet<PathBuf> = guids
            .values()
            .filter(|p| p.extension().is_some_and(|e| e == "asset") && mesh::is_mesh_asset(p))
            .cloned()
            .collect();
        let kind_at = |path: &Path| {
            if mesh_assets.contains(path) {
                Some("model")
            } else {
                kind_of(path)
            }
        };
        // Names: the file's stem, and where two of one kind share it, the
        // folder before it too — `props_crate`, `tools_crate`.
        let mut by_kind: HashMap<(&str, String), Vec<String>> = HashMap::new();
        for (guid, path) in &guids {
            let Some(kind) = kind_at(path) else { continue };
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
            mesh_pieces: HashMap::new(),
            declared_params: HashMap::new(),
            mesh_assets,
        })
    }

    /// The scrap name of the asset a GUID names, and its kind.
    pub fn named(&self, guid: &str) -> Option<(&'static str, &str)> {
        let path = self.guids.get(guid)?;
        Some((self.kind(path)?, self.names.get(guid)?.as_str()))
    }

    /// What a file becomes: by its extension, but a `.asset` holding a
    /// Mesh is a model.
    pub fn kind(&self, path: &Path) -> Option<&'static str> {
        if self.mesh_assets.contains(path) {
            Some("model")
        } else {
            kind_of(path)
        }
    }

    /// Every asset of a kind: its GUID and file.
    pub fn of_kind(&self, kind: &str) -> Vec<(&str, &Path)> {
        let mut out: Vec<(&str, &Path)> = self
            .guids
            .iter()
            .filter(|(_, p)| self.kind(p) == Some(kind))
            .map(|(g, p)| (g.as_str(), p.as_path()))
            .collect();
        out.sort_by(|a, b| a.1.cmp(b.1));
        out
    }
}

/// A layer's name as scrap writes it: `Ignore Raycast` → `ignore_raycast`.
fn layer_name(unity: &str) -> String {
    snake(&clean(unity)).replace("__", "_")
}

/// Unity's layers (`ProjectSettings/TagManager.asset`) and which pairs do
/// not collide (`DynamicsManager.asset`, its collision matrix), as a
/// `layers.ron`, with Unity's numbers → the names.
pub fn unity_layers(root: &Path) -> Option<(scrap::layers::Layers, HashMap<i64, String>)> {
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
    let layers = scrap::layers::Layers {
        layers: names.into_iter().map(|(_, n)| n).collect(),
        ignore,
    };
    Some((layers, by_number))
}

/// Bring a Unity project's content into a scrap project.
pub fn import_unity(unity: &Path, project: &scrap::Project, options: &Options) -> Result<Report> {
    let mut unity = Unity::open(unity)?;
    let mut report = Report::default();

    // Models first: a scene's renderer names one mesh of a model, and
    // which ones there are is known once they are converted.
    // Meshes kept as assets (exported terrain), made straight into glTF.
    let out = project.assets().join("models");
    for (guid, path) in unity.of_kind("model") {
        if !unity.mesh_assets.contains(path) {
            continue;
        }
        let to = out.join(format!("{}.glb", unity.names[guid]));
        let _ = std::fs::create_dir_all(&out);
        writable(&to);
        match mesh::convert_asset(path).and_then(|glb| Ok(std::fs::write(&to, glb)?)) {
            Ok(()) => report.models += 1,
            Err(e) => report.errors.push(format!("{}: {e:#}", path.display())),
        }
    }
    if options.models {
        models(&unity, project, options, &mut report);
    } else {
        let n = unity
            .of_kind("model")
            .iter()
            .filter(|(_, p)| !unity.mesh_assets.contains(*p))
            .count();
        if n > 0 {
            report.skip(format!(
                "{n} models: pass --models to convert them through Blender"
            ));
        }
    }
    unity.pieces = pieces(&project.assets().join("models"));
    unity.mesh_pieces = mesh_pieces(&unity);
    keep_origins(&project.assets().join("models"))?;

    if let Some((layers, _)) = unity_layers(&unity.root) {
        let text = ron::ser::to_string_pretty(&layers, ron::ser::PrettyConfig::new())?;
        write(
            &project.root().join(scrap::layers::FILE),
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
        // What the file is, by its bytes: Unity reads a JPEG named .png
        // without a word, and so must we.
        let read_as = image::ImageReader::open(path)
            .and_then(|r| r.with_guessed_format())
            .ok()
            .and_then(|r| r.format());
        let copies = matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "tga" | "bmp")
            && read_as == image::ImageFormat::from_extension(&extension);
        if !copies
            && !matches!(
                read_as,
                Some(
                    image::ImageFormat::Tiff
                        | image::ImageFormat::Png
                        | image::ImageFormat::Jpeg
                        | image::ImageFormat::Tga
                        | image::ImageFormat::Bmp
                )
            )
        {
            report.skip(format!("texture in .{extension} (convert it to PNG)"));
            continue;
        }
        std::fs::create_dir_all(&textures)?;
        let to = textures.join(if copies {
            format!("{name}.{extension}")
        } else {
            format!("{name}.png")
        });
        writable(&to);
        let written = if copies {
            std::fs::copy(path, &to)
                .map(|_| ())
                .map_err(anyhow::Error::from)
        } else {
            // A TIFF, or a file whose name lies about it: made a PNG.
            let picture = if read_as == Some(image::ImageFormat::Tiff) {
                tiff_picture(path)
            } else {
                image::ImageReader::open(path)
                    .and_then(|r| r.with_guessed_format())
                    .map_err(image::ImageError::from)
                    .and_then(|r| r.decode())
                    .map_err(anyhow::Error::from)
            };
            picture.and_then(|p| p.save(&to).map_err(anyhow::Error::from))
        };
        if let Err(e) = written {
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

    // Unity's own textures the materials name, made again (there is no
    // file of theirs to copy).
    for file_id in material::builtin_textures_used(&unity) {
        let Some((name, picture)) = material::builtin_texture(file_id) else {
            continue;
        };
        std::fs::create_dir_all(&textures)?;
        let to = textures.join(format!("{name}.png"));
        writable(&to);
        match picture.save(&to) {
            Ok(()) => report.textures += 1,
            Err(e) => report.errors.push(format!("{}: {e}", to.display())),
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
        writable(&sounds.join(format!("{name}.{extension}")));
        if let Err(e) = std::fs::copy(path, sounds.join(format!("{name}.{extension}"))) {
            report.errors.push(format!("{}: {e}", path.display()));
            continue;
        }
        report.sounds += 1;
    }

    let mut shaders: std::collections::BTreeMap<String, PathBuf> = Default::default();
    let mut declared_params: HashMap<String, Vec<String>> = HashMap::new();
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
                    project.root().join(scrap::project::SHADERS).join(&file),
                ))
                .find_map(|p| std::fs::read_to_string(p).ok())
        };
        match material::convert_with(&unity, path, &declared) {
            Ok(text) => {
                let name = &unity.names[guid];
                write(&project.materials().join(format!("{name}.scrmat")), &text)?;
                report.materials += 1;
                if let Some(own) = std::fs::read_to_string(path).ok().and_then(|t| {
                    yaml::documents(&t)
                        .into_iter()
                        .find(|d| d.kind == "Material")
                        .and_then(|d| material::own_shader(&unity, &d.body))
                }) {
                    if let Some(text) = declared(&own.0) {
                        declared_params.insert(name.clone(), material::declared_params(&text));
                    }
                    shaders.insert(own.0, own.1);
                }
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", path.display())),
        }
    }
    unity.declared_params = declared_params;
    // The shaders those materials had: a stub each to write again, never
    // over one already written.
    let dir = project.root().join(scrap::project::SHADERS);
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
            .join(format!("{name}.{}", scrap::prefab::EXTENSION));
        scrap::Prefabs::save(&root, &file).map_err(anyhow::Error::msg)?;
        report.prefabs += 1;
    }

    for (guid, path) in unity.of_kind("scene") {
        let Some(text) = read_text(path, &mut report) else {
            continue;
        };
        let entities = scene::convert_file(&unity, &text, &mut report);
        let mut scene = scrap::Scene {
            entities,
            ..Default::default()
        };
        let ambient = look::ambient(&text);
        if let Some(mut sun) = scene::sun(&unity, &text) {
            sun.ambient = ambient;
            scene.set_part(&sun);
        } else if ambient.is_some() {
            scene.set_part(&scrap::scene::Sun { ambient, ..Default::default() });
        }
        // How it looks: its fog, and its global Volume's grade.
        if let Some(fog) = look::fog(&text) {
            scene.set_part(&fog);
        }
        scene.set_part_opt(look::sky(&unity, &text).as_ref());
        scene.set_part_opt((look::post(&unity, &text, &mut report)).as_ref());
        scene.set_part_opt(look::shadows(&unity, &text).as_ref());
        scene.set_part_opt(look::ambient_occlusion(&unity).as_ref());
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
                .and_then(|t| scrap::ron::from_str(&t).ok())
                .unwrap_or_default();
            report.strings += words.len();
            all.extend(words);
            let body = scrap::ron::ser::to_string_pretty(&all, Default::default())?;
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
        match animator::convert(&unity, path, &mut report) {
            Ok(text) => {
                let dir = project.root().join(scrap::project::ANIMATORS);
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
                let dir = project.root().join(scrap::motion::DIR);
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
/// moved down to stand on y = 0, as a model dropped into a scrap project
/// is; and it is one mesh, not a scene of its nodes. Every converted model's sidecar says so — a new one, or an old one
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
        // One mesh, whatever its nodes: a scene names a model as one
        // thing — its pieces are there for a renderer that names one.
        // Its UVs as they are: the scene's materials read them.
        if settings.origin_to_base || settings.scene || !settings.keep_uvs || !sidecar.is_file() {
            settings.origin_to_base = false;
            settings.scene = false;
            settings.keep_uvs = true;
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

/// Which piece each mesh of a model is, by the prefabs that draw it on an
/// object named as a piece of that model (Unity's "(1)" copies aside). A
/// mesh two prefabs name differently is left out.
fn mesh_pieces(unity: &Unity) -> HashMap<(String, i64), String> {
    let mut seen: HashMap<(String, i64), Option<String>> = HashMap::new();
    for (_, path) in unity.of_kind("prefab") {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        if !text.contains("m_Mesh: {fileID") {
            continue;
        }
        let docs = yaml::documents(&text);
        let by_id = yaml::by_id(&docs);
        for doc in &docs {
            if !matches!(doc.kind.as_str(), "MeshFilter" | "SkinnedMeshRenderer") {
                continue;
            }
            let Some(mesh) = doc.body.reference("m_Mesh") else { continue };
            let Some(guid) = mesh.guid.clone() else { continue };
            let Some((kind, model)) = unity.named(&guid) else { continue };
            if kind != "model" {
                continue;
            }
            let Some(pieces) = unity.pieces.get(model) else { continue };
            let Some(object) = doc
                .body
                .reference("m_GameObject")
                .and_then(|g| by_id.get(&g.file_id))
                .and_then(|g| g.body.str("m_Name"))
            else {
                continue;
            };
            let bare = object.trim_end_matches(|c: char| c == ')' || c.is_ascii_digit());
            let bare = bare.strip_suffix(" (").unwrap_or(object).trim();
            let Some(piece) = [object, bare].into_iter().map(piece_name).find(|p| pieces.contains(p)) else {
                continue;
            };
            let entry = seen.entry((guid, mesh.file_id)).or_insert_with(|| Some(piece.clone()));
            if entry.as_deref() != Some(piece.as_str()) {
                *entry = None;
            }
        }
    }
    seen.into_iter().filter_map(|(k, v)| Some((k, v?))).collect()
}

/// A piece's file name for a Blender object's name: what a file name
/// can hold.
pub(crate) fn piece_name(object: &str) -> String {
    object
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

/// A file the import writes over made writable first: a project's binary
/// sources are lockable in LFS, and git leaves them read-only unless
/// locked.
fn writable(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut permissions = meta.permissions();
        if permissions.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = std::fs::set_permissions(path, permissions);
        }
    }
}

/// FBX (and friends) to GLB through Blender, into `assets/models/`.
fn models(unity: &Unity, project: &scrap::Project, options: &Options, report: &mut Report) {
    let blender = options
        .blender
        .clone()
        .unwrap_or_else(|| PathBuf::from("blender"));
    let out = project.assets().join("models");
    let _ = std::fs::create_dir_all(&out);
    let all: Vec<_> = unity
        .of_kind("model")
        .into_iter()
        .filter(|(_, p)| !unity.mesh_assets.contains(*p))
        .collect();
    let count = all.len();
    for (i, (guid, path)) in all.into_iter().enumerate() {
        let name = &unity.names[guid];
        if options.models_matching.as_ref().is_some_and(|m| !name.contains(m.as_str())) {
            continue;
        }
        eprintln!("model {}/{count} {name}", i + 1);
        let to = out.join(format!("{name}.glb"));
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if matches!(extension.as_str(), "gltf" | "glb" | "obj") {
            let to = out.join(format!("{name}.{extension}"));
            writable(&to);
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
        // What Blender writes over: the model and its pieces.
        writable(&to);
        for entry in std::fs::read_dir(&pieces_dir).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy().starts_with(&format!("{name}@")) {
                writable(&entry.path());
            }
        }
        // Vertex colours come through as the file has them, as Unity reads
        // them: taken in as plain numbers (Blender's default reads them as
        // sRGB, and its glTF writer then bends them to linear), and the
        // active set written as COLOR_0 — by default Blender writes only
        // colours its own material uses, and here that is none.
        let script = format!(
            "import bpy, math, mathutils, re\n\
             bpy.ops.wm.read_factory_settings(use_empty=True)\n\
             fbx_options = bpy.ops.import_scene.fbx.get_rna_type().properties.keys()\n\
             gltf_options = bpy.ops.export_scene.gltf.get_rna_type().properties.keys()\n\
             colors = {{'export_vertex_color': 'ACTIVE'}} if 'export_vertex_color' in gltf_options else {{'export_colors': True}}\n\
             bpy.ops.import_scene.fbx(filepath={:?}, **({{'colors_type': 'LINEAR'}} if 'colors_type' in fbx_options else {{}}))\n\
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
             \x20   arm = next((m.object for m in o.modifiers if m.type == 'ARMATURE' and m.object), None)\n\
             \x20   if arm: arm.select_set(True)\n\
             \x20   o.parent = None\n\
             \x20   o.matrix_world = frame\n\
             \x20   piece = re.sub(r'[^A-Za-z0-9_-]', '_', o.name)\n\
             \x20   bpy.ops.export_scene.gltf(filepath={:?} + '/' + {:?} + '@' + piece + '.glb', export_format='GLB', use_selection=True, export_animations=False, export_skins=bool(arm), **colors)\n\
             \x20   o.parent = parent\n\
             \x20   o.matrix_world = placed\n\
             for x in scene.objects: x.select_set(False)\n\
             roots = [o for o in bpy.context.scene.objects if o.parent is None]\n\
             turn = bpy.data.objects.new('unity_turn', None)\n\
             bpy.context.scene.collection.objects.link(turn)\n\
             for o in roots: o.parent = turn\n\
             turn.rotation_euler[2] = math.pi\n\
             bpy.ops.export_scene.gltf(filepath={:?}, export_format='GLB', export_animations=True, **colors)\n",
            path.to_string_lossy(),
            path.to_string_lossy(),
            pieces_dir.to_string_lossy().trim_end_matches('/'),
            name,
            to.to_string_lossy()
        );
        let result = std::process::Command::new(&blender)
            .args(["-b", "--factory-startup", "--python-expr", &script])
            .output();
        if std::env::var_os("SCRAP_BLENDER_LOG").is_some() {
            if let Ok(o) = &result {
                eprintln!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            }
        }
        // Blender ends well even when the script in it failed: its
        // traceback says so.
        let failed_inside = result.as_ref().ok().and_then(|o| {
            let out = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            out.lines()
                .find(|l| l.contains("Error:") || l.starts_with("PermissionError") || l.contains("Traceback"))
                .map(|_| {
                    out.lines()
                        .rev()
                        .find(|l| l.contains("Error"))
                        .unwrap_or("a Python error")
                        .to_string()
                })
        });
        if let Some(why) = failed_inside {
            report.errors.push(format!("{}: Blender's script failed: {why}", path.display()));
            continue;
        }
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

/// A name a scrap file can have: letters, digits, `_` and `-`; spaces and
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
/// scrap game writes it.
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
    fn names_are_what_a_scrap_file_can_be_called() {
        assert_eq!(snake("PlayerController"), "player_controller");
        assert_eq!(snake("HTTPClient"), "http_client");
        assert_eq!(snake("Item3D"), "item3_d");
        assert_eq!(clean("Big Rock (1)"), "Big_Rock_1");
    }
}

/// A TIFF as a picture. Photoshop's RGBA TIFFs mark the fourth channel an
/// unspecified extra sample, which the `tiff` crate reads as RGB with a
/// stride of three — stripes, and no alpha — so the common case (8 bits a
/// channel, interleaved strips, none, LZW or Deflate, with or without the
/// horizontal predictor) is read here by the file's own SamplesPerPixel;
/// anything else goes to the crate.
fn tiff_picture(path: &Path) -> Result<image::DynamicImage> {
    let data = std::fs::read(path)?;
    if let Some(picture) = tiff_strips(&data) {
        return picture;
    }
    use tiff::decoder::{Decoder, DecodingResult};
    let mut decoder = Decoder::new(std::io::Cursor::new(&data))?;
    let (w, h) = decoder.dimensions()?;
    let bytes: Vec<u8> = match decoder.read_image()? {
        DecodingResult::U8(b) => b,
        DecodingResult::U16(b) => b.into_iter().map(|v| (v >> 8) as u8).collect(),
        _ => anyhow::bail!("{}: a TIFF of a number type not read", path.display()),
    };
    picture_of(w, h, bytes).with_context(|| format!("{}", path.display()))
}

fn picture_of(w: u32, h: u32, bytes: Vec<u8>) -> Result<image::DynamicImage> {
    let pixels = (w as usize * h as usize).max(1);
    let picture = match bytes.len() / pixels {
        1 => image::GrayImage::from_raw(w, h, bytes).map(image::DynamicImage::from),
        2 => image::GrayAlphaImage::from_raw(w, h, bytes).map(image::DynamicImage::from),
        3 => image::RgbImage::from_raw(w, h, bytes).map(image::DynamicImage::from),
        4 => image::RgbaImage::from_raw(w, h, bytes).map(image::DynamicImage::from),
        n => anyhow::bail!("a picture of {n} channels"),
    };
    picture.context("its pixels do not fill it")
}

/// [`tiff_picture`]'s own reading: `None` when the file is not the case
/// it knows.
fn tiff_strips(data: &[u8]) -> Option<Result<image::DynamicImage>> {
    let little = match data.get(..4)? {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u32> {
        let b = data.get(o..o + 2)?;
        Some(if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) } as u32)
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = data.get(o..o + 4)?;
        let b = [b[0], b[1], b[2], b[3]];
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let mut tags: HashMap<u32, Vec<u32>> = HashMap::new();
    for i in 0..u16_at(ifd)? as usize {
        let e = ifd + 2 + i * 12;
        let (tag, kind, count) = (u16_at(e)?, u16_at(e + 2)?, u32_at(e + 4)? as usize);
        let size = match kind {
            3 => 2,
            4 => 4,
            _ => continue,
        };
        let at = if size * count <= 4 { e + 8 } else { u32_at(e + 8)? as usize };
        let values = (0..count)
            .map(|k| if size == 2 { u16_at(at + k * 2) } else { u32_at(at + k * 4) })
            .collect::<Option<Vec<u32>>>()?;
        tags.insert(tag, values);
    }
    let one = |tag: u32, default: u32| tags.get(&tag).and_then(|v| v.first().copied()).unwrap_or(default);
    let (w, h) = (one(256, 0), one(257, 0));
    let spp = one(277, 1) as usize;
    let compression = one(259, 1);
    let predictor = one(317, 1);
    if tags.get(&258).is_some_and(|b| b.iter().any(|&b| b != 8))
        || one(284, 1) != 1
        || tags.contains_key(&322)
        || !matches!(compression, 1 | 5 | 8 | 32946)
        || !matches!(predictor, 1 | 2)
        || !(1..=4).contains(&spp)
        || w == 0
        || h == 0
    {
        return None;
    }
    let (offsets, counts) = (tags.get(&273)?, tags.get(&279)?);
    let rows_per_strip = one(278, h) as usize;
    let row = w as usize * spp;
    let mut pixels = Vec::with_capacity(row * h as usize);
    for (strip, (&o, &n)) in offsets.iter().zip(counts).enumerate() {
        let raw = data.get(o as usize..o as usize + n as usize)?;
        let rows = rows_per_strip.min(h as usize - (strip * rows_per_strip).min(h as usize));
        let mut bytes = match compression {
            1 => raw.to_vec(),
            5 => {
                // TIFF's LZW switches code size one code early; some
                // writers do not. Whatever reads the whole strip is taken,
                // else the crate is left to try.
                let whole = |mut lzw: weezl::decode::Decoder| {
                    let mut out = Vec::new();
                    let _ = lzw.into_vec(&mut out).decode(raw);
                    out
                };
                [
                    whole(weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)),
                    whole(weezl::decode::Decoder::new(weezl::BitOrder::Msb, 8)),
                ]
                .into_iter()
                .find(|b| b.len() >= rows * row)?
            }
            _ => match miniz_oxide::inflate::decompress_to_vec_zlib(raw) {
                Ok(b) => b,
                Err(e) => return Some(Err(anyhow::anyhow!("Deflate: {e:?}"))),
            },
        };
        bytes.resize(rows * row, 0);
        if predictor == 2 {
            for r in bytes.chunks_mut(row) {
                for i in spp..r.len() {
                    r[i] = r[i].wrapping_add(r[i - spp]);
                }
            }
        }
        pixels.extend_from_slice(&bytes);
    }
    pixels.resize(row * h as usize, 0);
    Some(picture_of(w, h, pixels))
}

#[cfg(test)]
mod tiff_tests {
    /// Photoshop's RGBA TIFF: four samples, the fourth an unspecified
    /// extra sample, the horizontal predictor. Read as RGBA, not stripes.
    #[test]
    fn an_rgba_tiff_with_an_unspecified_extra_sample_reads_whole() {
        let (w, h) = (3u32, 2u32);
        let pixels: Vec<[u8; 4]> = vec![
            [10, 20, 30, 255], [40, 50, 60, 128], [70, 80, 90, 0],
            [1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12],
        ];
        // Horizontal differencing, per row, per sample.
        let mut strip = Vec::new();
        for row in pixels.chunks(w as usize) {
            for (x, p) in row.iter().enumerate() {
                for s in 0..4 {
                    strip.push(if x == 0 { p[s] } else { p[s].wrapping_sub(row[x - 1][s]) });
                }
            }
        }
        let entries: Vec<(u16, u16, u32, u32)> = vec![
            (256, 3, 1, w),
            (257, 3, 1, h),
            (258, 3, 4, 0), // offset filled below
            (259, 3, 1, 1),
            (262, 3, 1, 2),
            (273, 4, 1, 0), // strip offset below
            (277, 3, 1, 4),
            (278, 3, 1, h),
            (279, 4, 1, strip.len() as u32),
            (284, 3, 1, 1),
            (317, 3, 1, 2),
            (338, 3, 1, 0),
        ];
        let ifd = 8u32;
        let after_ifd = ifd + 2 + entries.len() as u32 * 12 + 4;
        let bps_at = after_ifd;
        let strip_at = bps_at + 8;
        let mut file = b"II*\0".to_vec();
        file.extend(ifd.to_le_bytes());
        file.extend((entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            let value = match tag {
                258 => bps_at,
                273 => strip_at,
                _ => value,
            };
            file.extend(tag.to_le_bytes());
            file.extend(kind.to_le_bytes());
            file.extend(count.to_le_bytes());
            if kind == 3 && count == 1 {
                file.extend((value as u16).to_le_bytes());
                file.extend([0, 0]);
            } else {
                file.extend(value.to_le_bytes());
            }
        }
        file.extend(0u32.to_le_bytes());
        for _ in 0..4 {
            file.extend(8u16.to_le_bytes());
        }
        file.extend(&strip);
        let picture = super::tiff_strips(&file).unwrap().unwrap().to_rgba8();
        assert_eq!(picture.dimensions(), (w, h));
        for (i, p) in pixels.iter().enumerate() {
            assert_eq!(picture.get_pixel(i as u32 % w, i as u32 / w).0, *p, "pixel {i}");
        }
    }
}
