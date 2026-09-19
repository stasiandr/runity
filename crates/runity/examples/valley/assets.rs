//! Valley content: the models and the color atlas the first ten minutes need.
//!
//! The files themselves live in `assets/valley/`, outside any crate — a scene
//! is data, not code (`look.rs` keeps only what changes with the hour). This
//! module is the loading side of that split: where the files are, what each of
//! them is for, and the two rules the valley loads them under — one shared
//! atlas, sampled [`Filter::Nearest`], and no materials in the OBJ at all.
//!
//! The content was prepared from three CC0 kits by
//! `tools/valley-assets/build_assets.py`; `assets/valley/CREDITS.txt` names the
//! source of every committed file. Placing any of it in a scene is the next
//! card's work — here the question is only whether the files are there and in
//! the shape the checklist asks for, which is what the tests at the bottom
//! answer.

use runity::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

// The palette threshold lives with the rest of the valley's color; the atlas
// test asks it whether a cell reads as loud. Only the tests need it, and
// look.rs is an example root of its own, so its `main` goes unused here.
#[cfg(test)]
#[path = "look.rs"]
#[allow(dead_code)]
mod look;

/// One thing the valley needs on screen, and the file that supplies it.
pub struct Item {
    /// What it is, in the words of the content list.
    pub what: &'static str,
    /// Its file under `assets/valley/models/`.
    pub file: &'static str,
}

/// Every prop of the first ten minutes, each with a real model behind it.
pub const CONTENT: &[Item] = &[
    Item {
        what: "костёр: камни и поленья (угли — в коде)",
        file: "campfire.obj",
    },
    Item {
        what: "шалаш",
        file: "shelter.obj",
    },
    Item {
        what: "недостроенная стена: остов и куча брёвен",
        file: "wall_unfinished.obj",
    },
    Item {
        what: "брёвна россыпью",
        file: "logs_loose.obj",
    },
    Item {
        what: "брёвна в связке",
        file: "logs_bundle.obj",
    },
    Item {
        what: "топор",
        file: "axe.obj",
    },
    Item {
        what: "поселенец: стоит",
        file: "settler_stand.obj",
    },
    Item {
        what: "поселенец: несёт",
        file: "settler_carry.obj",
    },
    Item {
        what: "поселенец: идёт",
        file: "settler_walk.obj",
    },
    Item {
        what: "руки-предплечья от первого лица",
        file: "hands_first_person.obj",
    },
    Item {
        what: "ель, малая",
        file: "pine_small.obj",
    },
    Item {
        what: "ель, средняя",
        file: "pine_medium.obj",
    },
    Item {
        what: "ель, большая",
        file: "pine_large.obj",
    },
    Item {
        what: "лиственное дерево, малое",
        file: "broadleaf_small.obj",
    },
    Item {
        what: "лиственное дерево, среднее",
        file: "broadleaf_medium.obj",
    },
    Item {
        what: "лиственное дерево, большое",
        file: "broadleaf_large.obj",
    },
    Item {
        what: "трава",
        file: "grass_tuft.obj",
    },
    Item {
        what: "куст",
        file: "bush.obj",
    },
    Item {
        what: "ягодный куст",
        file: "berry_bush.obj",
    },
    Item {
        what: "валун",
        file: "boulder.obj",
    },
    Item {
        what: "валун, малый",
        file: "boulder_small.obj",
    },
];

/// The one atlas every model samples.
pub const ATLAS: &str = "valley_atlas.png";

/// Triangle budget per prop: the renderer is a single-threaded rasterizer
/// (`docs/design/07-look.md` part 3), and the whole valley has to fit in a
/// frame alongside the ground.
pub const MAX_TRIANGLES: usize = 1500;

/// `assets/valley/`, found relative to this crate rather than the working
/// directory, so tests and examples agree on where it is.
pub fn valley_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/valley")
}

pub fn models_dir() -> PathBuf {
    valley_dir().join("models")
}

pub fn textures_dir() -> PathBuf {
    valley_dir().join("textures")
}

/// The files of one kind under `directory`, sorted, so a walk of the content
/// is the same on every machine.
pub fn files_with_extension(directory: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|e| panic!("{}: {e}", directory.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    found.sort();
    found
}

/// Load a valley atlas: nearest-neighbour, clamped to its edge.
///
/// Both rules follow from what the atlas *is* — a grid of flat color cells,
/// one per surface. Bilinear filtering would blend neighbouring cells along
/// every cell border and paint a thin wrong-colored seam onto each face, and
/// there is nothing outside the grid worth repeating.
pub fn load_texture(path: &Path) -> Result<Texture, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let image = decode_png(&bytes).map_err(|e| format!("{}: {e:?}", path.display()))?;
    let texels = image
        .pixels
        .iter()
        .copied()
        .map(Color::from_argb8)
        .collect();
    let mut texture = Texture::new(image.width, image.height, texels);
    texture.filter = Filter::Nearest;
    texture.wrap = Wrap::Clamp;
    Ok(texture)
}

/// The valley's atlas, or a panic naming the file that is wrong: the content
/// is committed next to the code, so a missing atlas is a broken checkout
/// rather than a condition to recover from.
pub fn load_atlas() -> Texture {
    let path = textures_dir().join(ATLAS);
    load_texture(&path).unwrap_or_else(|e| panic!("{e}"))
}

/// Load one prepared model. Normals come from the file — every committed OBJ
/// carries `vn`, so nothing here depends on `Mesh::recompute_normals`.
pub fn load_model(file: &str) -> Result<Mesh, String> {
    let path = models_dir().join(file);
    let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Mesh::from_obj(&source).map_err(|e| format!("{}: {e}", path.display()))
}

fn main() {
    let atlas = load_atlas();
    println!(
        "valley content: {} models, atlas {}x{} ({:?})",
        CONTENT.len(),
        atlas.width(),
        atlas.height(),
        atlas.filter
    );
    for item in CONTENT {
        match load_model(item.file) {
            Ok(mesh) => println!(
                "  {:<24} {:>5} tris  {}",
                item.file,
                mesh.triangle_count(),
                item.what
            ),
            Err(e) => println!("  {:<24} {e}", item.file),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bounding box of a mesh, as `(min, max)`.
    fn bounds(mesh: &Mesh) -> (Vec3, Vec3) {
        let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
        for v in &mesh.vertices {
            min = Vec3::new(
                min.x.min(v.position.x),
                min.y.min(v.position.y),
                min.z.min(v.position.z),
            );
            max = Vec3::new(
                max.x.max(v.position.x),
                max.y.max(v.position.y),
                max.z.max(v.position.z),
            );
        }
        (min, max)
    }

    #[test]
    fn every_item_of_the_first_ten_minutes_has_a_model_file() {
        for item in CONTENT {
            let path = models_dir().join(item.file);
            assert!(
                path.exists(),
                "{} ({}) has no file — a prop of the first ten minutes is missing",
                item.what,
                path.display()
            );
        }
    }

    #[test]
    fn no_model_file_is_left_out_of_the_content_list() {
        for path in files_with_extension(&models_dir(), "obj") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            assert!(
                CONTENT.iter().any(|item| item.file == name),
                "models/{name} is committed but nothing in CONTENT claims it"
            );
        }
    }

    /// The checklist every committed OBJ has to pass: authored normals, no
    /// materials, a triangle budget, meters with Y up, and the support on the
    /// ground.
    #[test]
    fn every_model_passes_the_obj_checklist() {
        for path in files_with_extension(&models_dir(), "obj") {
            let name = path.display().to_string();
            let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));

            let lines = || source.lines().map(str::trim);
            assert!(
                lines().any(|l| l.starts_with("vn ")),
                "{name}: no `vn` lines — normals have to be in the file"
            );
            assert!(
                lines().all(|l| !l.starts_with("mtllib") && !l.starts_with("usemtl")),
                "{name}: color belongs in the atlas, not in a material"
            );
            for face in lines().filter(|l| l.starts_with("f ")) {
                for token in face.split_whitespace().skip(1) {
                    let fields: Vec<&str> = token.split('/').collect();
                    assert!(
                        fields.len() == 3 && !fields[2].is_empty() && !fields[1].is_empty(),
                        "{name}: face corner `{token}` misses its UV or normal"
                    );
                }
            }

            let mesh = Mesh::from_obj(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
            let triangles = mesh.triangle_count();
            assert!(triangles > 0, "{name}: no triangles");
            assert!(
                triangles <= MAX_TRIANGLES,
                "{name}: {triangles} triangles is over the {MAX_TRIANGLES} budget"
            );
            assert!(
                mesh.vertices
                    .iter()
                    .all(|v| (v.normal.length() - 1.0).abs() < 1e-3),
                "{name}: a normal is not unit length"
            );
            assert!(
                mesh.vertices
                    .iter()
                    .all(|v| (0.0..=1.0).contains(&v.uv.x) && (0.0..=1.0).contains(&v.uv.y)),
                "{name}: a UV points outside the atlas"
            );

            let (min, max) = bounds(&mesh);
            assert!(
                min.y.abs() < 1e-3,
                "{name}: rests at y = {}, not on the ground",
                min.y
            );
            // A meter is a unit: the flattest prop here is a fire ring and the
            // tallest a pine, and nothing in the valley is smaller than a tuft
            // of grass or larger than a tree.
            let size = max - min;
            let largest = size.x.max(size.y).max(size.z);
            assert!(
                (0.3..=12.0).contains(&largest),
                "{name}: {largest} m at its largest — that is not a meter scale"
            );
            assert!(
                size.y <= 12.0 && size.x.max(size.z) <= 8.0,
                "{name}: {size:?} m — that is not a meter scale"
            );
        }
    }

    #[test]
    fn every_texture_decodes_and_loads_with_nearest_filtering() {
        let textures = files_with_extension(&textures_dir(), "png");
        assert!(!textures.is_empty(), "the valley has no atlas at all");
        for path in textures {
            let name = path.display().to_string();
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            // 16-bit and interlaced PNGs are rejected by the decoder, so this
            // is also the 8-bit, non-interlaced half of the checklist.
            let image = decode_png(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(image.width > 0 && image.height > 0, "{name}: empty image");

            let texture = load_texture(&path).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(
                texture.filter,
                Filter::Nearest,
                "{name}: an atlas of flat cells must not be blended"
            );
        }
    }

    /// The valley is a muted place: the atlas is allowed exactly one loud
    /// patch, and it is the berry. `look.rs` owns the threshold — the embers
    /// are the only other thing over it, and they are a light in code, not a
    /// surface here.
    #[test]
    fn the_atlas_has_at_most_one_saturated_patch() {
        let path = textures_dir().join(ATLAS);
        let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let image = decode_png(&bytes).unwrap_or_else(|e| panic!("{e:?}"));

        let loud: Vec<bool> = image
            .pixels
            .iter()
            .map(|p| look::is_saturated_color(Color::from_argb8(*p)))
            .collect();

        // Flood fill: a patch is a connected run of loud pixels, so the three
        // tiers of one color count once, and two unrelated cells count twice.
        let mut seen = vec![false; loud.len()];
        let mut patches: Vec<Color> = Vec::new();
        for start in 0..loud.len() {
            if !loud[start] || seen[start] {
                continue;
            }
            patches.push(Color::from_argb8(image.pixels[start]));
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(i) = stack.pop() {
                let (x, y) = (i % image.width, i / image.width);
                let mut visit = |nx: usize, ny: usize| {
                    let n = ny * image.width + nx;
                    if loud[n] && !seen[n] {
                        seen[n] = true;
                        stack.push(n);
                    }
                };
                if x > 0 {
                    visit(x - 1, y);
                }
                if x + 1 < image.width {
                    visit(x + 1, y);
                }
                if y > 0 {
                    visit(x, y - 1);
                }
                if y + 1 < image.height {
                    visit(x, y + 1);
                }
            }
        }

        assert!(
            patches.len() <= 1,
            "{} saturated patches in the atlas: {patches:?} — only the berry may be loud",
            patches.len()
        );
        // And exactly one, not zero: the berry has to be in there, or the
        // threshold above would be passing on an atlas that lost its berries.
        let berry = patches
            .first()
            .expect("the atlas has no saturated patch at all — where are the berries?");
        assert!(
            berry.r > berry.g && berry.r > berry.b,
            "the one loud patch should be the berry, but it is {berry:?}"
        );
    }

    #[test]
    fn every_committed_asset_has_a_line_in_credits() {
        let path = valley_dir().join("CREDITS.txt");
        let credits =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let listed = |needle: &str| credits.lines().any(|line| line.contains(needle));

        for (directory, folder) in [(models_dir(), "models"), (textures_dir(), "textures")] {
            for asset in fs::read_dir(&directory)
                .unwrap_or_else(|e| panic!("{}: {e}", directory.display()))
                .filter_map(|entry| entry.ok().map(|e| e.path()))
            {
                let name = asset.file_name().unwrap().to_string_lossy().to_string();
                assert!(
                    listed(&format!("{folder}/{name}")),
                    "{folder}/{name} has no line in CREDITS.txt — every committed file needs one"
                );
            }
        }
    }
}
