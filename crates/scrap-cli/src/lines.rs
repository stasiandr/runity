//! `scrap lines`: everything the project's dialogues say, as a sheet per
//! language — for recording voices and for translators. A row a line or
//! an answer: its stable name (`<dialogue>/<line>`, what the recording is
//! called), who says it, the `content/localization/` key, and the words in that
//! language. The sheets are made, not kept: `content/localization/` stays the truth.

use std::path::{Path, PathBuf};

use scrap::dialogue::Dialogue;
use scrap::Project;

/// A dialogue's name: its file's, less `.dialogue.ron` — `captain`; in the
/// old `dialogues/`, its path there — `harbour/captain`.
pub fn name(root: &Path, path: &Path) -> String {
    scrap::layout::name_in(root, scrap::layout::Kind::Dialogue, path)
}

/// Whether a file in `dialogues/` is a dialogue's cases, not a dialogue.
pub fn is_cases(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".cases.ron")
}

/// Every dialogue of the project that reads, named, with its path; and
/// what did not read.
pub fn dialogues(project: &Project) -> (Vec<(PathBuf, Dialogue)>, Vec<String>) {
    let dir = project.root();
    let paths = project.files(scrap::layout::Kind::Dialogue);
    let (mut found, mut failed) = (Vec::new(), Vec::new());
    for path in paths {
        match Dialogue::load(&path) {
            Ok(mut d) => {
                d.name = name(dir, &path);
                found.push((path, d));
            }
            Err(e) => failed.push(e),
        }
    }
    (found, failed)
}

fn cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// The sheet for one language (`None`: the texts as written): a header,
/// then a row for everything said.
pub fn sheet(dialogues: &[Dialogue], table: Option<&scrap::strings::Table>) -> String {
    let say = |text: &str| -> String {
        match (text.strip_prefix('@'), table) {
            (Some(key), Some(t)) => t.get(key).cloned().unwrap_or_default(),
            (Some(_), None) => String::new(),
            (None, _) => text.to_string(),
        }
    };
    let mut out = String::from("id,speaker,key,text\n");
    for d in dialogues {
        for said in d.said() {
            let key = said.text.strip_prefix('@').unwrap_or("");
            out += &format!(
                "{},{},{},{}\n",
                cell(&said.id),
                cell(&say(&said.speaker)),
                cell(key),
                cell(&say(&said.text))
            );
        }
    }
    out
}

/// Write a sheet per language of `content/localization/` into `out` (`<language>.csv`),
/// or one `lines.csv` of the texts as written when there are none. What
/// was written.
pub fn export(project: &Project, out: &Path) -> Result<Vec<PathBuf>, String> {
    let (dialogues, failed) = dialogues(project);
    if let Some(e) = failed.first() {
        return Err(e.clone());
    }
    let dialogues: Vec<Dialogue> = dialogues.into_iter().map(|(_, d)| d).collect();
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut written = Vec::new();
    let tables = scrap::strings::tables(project.root().join(scrap::strings::DIR));
    let mut put = |file: PathBuf, text: String| -> Result<(), String> {
        std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
        written.push(file);
        Ok(())
    };
    if tables.is_empty() {
        put(out.join("lines.csv"), sheet(&dialogues, None))?;
    }
    for (language, path) in tables {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let table: scrap::strings::Table =
            scrap::ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        put(
            out.join(format!("{language}.csv")),
            sheet(&dialogues, Some(&table)),
        )?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_names_each_line_by_dialogue_and_line_and_says_it_in_the_language() {
        let mut d: Dialogue = scrap::ron::from_str(
            r#"(start: "hello", lines: {
                "hello": (speaker: "@chef", text: "@chef.hello", next: "ask"),
                "ask": (speaker: "@chef", text: "Ready, \"chef\"?", choices: [(text: "@yes", to: "hello")]),
            })"#,
        )
        .unwrap();
        d.name = "kitchen/chef".into();
        let table: scrap::strings::Table = [
            ("chef".to_string(), "Head chef".to_string()),
            ("chef.hello".to_string(), "Morning, all".to_string()),
            ("yes".to_string(), "Yes".to_string()),
        ]
        .into();
        assert_eq!(
            sheet(&[d], Some(&table)),
            "id,speaker,key,text\n\
             kitchen/chef/hello,Head chef,chef.hello,\"Morning, all\"\n\
             kitchen/chef/ask,Head chef,,\"Ready, \"\"chef\"\"?\"\n\
             kitchen/chef/ask/1,,yes,Yes\n"
        );
        assert_eq!(
            name(Path::new("/p"), Path::new("/p/dialogues/kitchen/chef.ron")),
            "kitchen/chef",
            "in the old dialogues/, its path there"
        );
        assert_eq!(
            name(
                Path::new("/p"),
                Path::new("/p/content/game/kitchen/chef.dialogue.ron")
            ),
            "chef",
            "anywhere else, its file's name"
        );
    }
}
