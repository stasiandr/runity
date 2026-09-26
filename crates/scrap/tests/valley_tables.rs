//! The Valley's economy as tables: the materials and recipes of
//! docs/design/05-economy.md (6.2, 6.3) in `examples/valley/content/valley/crafting/`, read
//! into the types a game would give them — the first tables a game of this
//! engine has (docs/data.md).
//!
//! The example has no game code, so this test is its game: it reads the
//! tables, checks them as the game's own test would, and writes what they
//! hold to `library/tables.ron`, where the editor's Configs window and
//! `scrap check` find it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use scrap::{Link, Record, Table, Tables};
use serde::Deserialize;

/// A property of a material. Eight, and a ninth is not added lightly: each
/// multiplies what can stand in for what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
enum Tag {
    Fibrous,
    Burns,
    Hard,
    Sharp,
    Warm,
    Oily,
    Malleable,
    Edible,
}

/// What a thing is made of: each tag's grade (1…3) and units.
#[derive(Debug, Deserialize)]
struct Material {
    tags: BTreeMap<Tag, (u8, u8)>,
}

impl Record for Material {
    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.tags.is_empty() {
            out.push("has no tags: nothing could use it".into());
        }
        for (tag, (grade, units)) in &self.tags {
            if !(1..=3).contains(grade) {
                out.push(format!("{tag:?} has grade {grade}; grades are 1 to 3"));
            }
            if *units == 0 {
                out.push(format!("{tag:?} has no units"));
            }
        }
        out
    }
}

/// A step of a recipe: units of a tag at a grade or better.
#[derive(Debug, Deserialize)]
struct Step {
    #[serde(default)]
    part: String,
    need: Tag,
    units: u8,
    min_grade: u8,
    /// Not spent; its grade caps the result's.
    #[serde(default)]
    tool: Option<Tag>,
}

#[derive(Debug, Deserialize)]
struct Recipe {
    steps: Vec<Step>,
    /// What it makes, when that is a material too.
    #[serde(default)]
    makes: Option<Link<Material>>,
}

impl Record for Recipe {
    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.steps.is_empty() {
            out.push("has no steps".into());
        }
        for (i, step) in self.steps.iter().enumerate() {
            if !(1..=3).contains(&step.min_grade) {
                out.push(format!(
                    "steps[{i}]: min_grade is 1 to 3, not {}",
                    step.min_grade
                ));
            }
            if step.units == 0 {
                out.push(format!("steps[{i}]: takes no units"));
            }
        }
        out
    }
}

fn valley() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/valley")
}

fn tables() -> Tables {
    let mut tables = Tables::new();
    tables
        .register::<Material>("content/valley/crafting/materials.ron")
        .register::<Recipe>("content/valley/crafting/recipes.ron");
    tables
}

#[test]
fn the_valley_s_economy_reads_into_its_types() {
    let root = valley();
    let materials =
        Table::<Material>::load(root.join("content/valley/crafting/materials.ron")).unwrap();
    let recipes = Table::<Recipe>::load(root.join("content/valley/crafting/recipes.ron")).unwrap();
    assert_eq!(materials.len(), 14);
    assert_eq!(recipes.len(), 4);

    // Fat is oily, burns and is edible: burning it is not eating it.
    let fat = materials.named("Топлёный жир").unwrap();
    assert_eq!(fat.tags.len(), 3);
    assert_eq!(fat.tags[&Tag::Edible], (1, 2));

    // The axe: three steps, the handle fitted with something sharp.
    let axe = recipes.named("Топор").unwrap();
    assert_eq!(axe.steps.len(), 3);
    assert_eq!(axe.steps[1].part, "топорище");
    assert_eq!(axe.steps[1].tool, Some(Tag::Sharp));
    assert_eq!(axe.steps[0].need, Tag::Sharp);

    // Rope is a recipe that makes a material, linked by name and found.
    let rope = recipes.named("Верёвка").unwrap();
    let made = materials.find(rope.makes.as_ref().unwrap()).unwrap();
    assert_eq!(made.value.tags[&Tag::Fibrous], (1, 2));

    // Which materials would do for the rope's step: anything fibrous
    // enough — nettles and hide, sinew and wool.
    let step = &rope.steps[0];
    let mut will_do: Vec<&str> = materials
        .iter()
        .filter(|m| {
            m.value
                .tags
                .get(&step.need)
                .is_some_and(|(g, _)| *g >= step.min_grade)
        })
        .map(|m| m.name.as_str())
        .collect();
    will_do.sort();
    assert_eq!(
        will_do,
        [
            "Верёвка",
            "Клок шерсти",
            "Полоса шкуры",
            "Пучок крапивы",
            "Сухожилие"
        ]
    );
}

#[test]
fn the_valley_s_tables_hold_and_say_what_they_are() {
    let root = valley();
    let problems = tables().problems(&root);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    // Every record carries its id, as the examples' entities do.
    for file in [
        "content/valley/crafting/materials.ron",
        "content/valley/crafting/recipes.ron",
    ] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        assert_eq!(
            scrap::table::settle_ids(&text),
            None,
            "{file}: a record without its id"
        );
    }
    tables()
        .write_shapes(root.join(scrap::project::TABLE_SHAPES))
        .unwrap();
}
