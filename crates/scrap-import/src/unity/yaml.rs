//! Unity's YAML: a file of documents, each `--- !u!<class> &<fileID>` and
//! one key naming its type.
//!
//! Unity writes a dialect: the `!u!` tag and a trailing `stripped` on the
//! header line are not YAML a parser takes, so the headers are read here
//! and each document's body goes to `yaml-rust2` on its own.

use std::collections::HashMap;

use yaml_rust2::{Yaml, YamlLoader};

/// One document of a Unity file.
#[derive(Debug, Clone)]
pub struct Doc {
    /// Unity's class ID: 1 GameObject, 4 Transform, 114 MonoBehaviour…
    pub class: u32,
    pub file_id: i64,
    /// A placeholder for an object inside a prefab instance.
    pub stripped: bool,
    /// The type's name: the document's one key.
    pub kind: String,
    /// What is under that key.
    pub body: Yaml,
}

/// Every document of a Unity YAML file, by fileID.
pub fn documents(text: &str) -> Vec<Doc> {
    let mut out = Vec::new();
    let mut header: Option<(u32, i64, bool)> = None;
    let mut body = String::new();
    let flush = |header: Option<(u32, i64, bool)>, body: &str, out: &mut Vec<Doc>| {
        let Some((class, file_id, stripped)) = header else {
            return;
        };
        let Ok(docs) = YamlLoader::load_from_str(body) else {
            return;
        };
        let Some(Yaml::Hash(hash)) = docs.into_iter().next() else {
            return;
        };
        if let Some((Yaml::String(kind), value)) = hash.into_iter().next() {
            out.push(Doc {
                class,
                file_id,
                stripped,
                kind,
                body: value,
            });
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("--- !u!") {
            flush(header.take(), &body, &mut out);
            body.clear();
            let mut words = rest.split_whitespace();
            let class = words.next().and_then(|c| c.parse().ok()).unwrap_or(0);
            let file_id = words
                .next()
                .and_then(|id| id.trim_start_matches('&').parse().ok())
                .unwrap_or(0);
            let stripped = words.any(|w| w == "stripped");
            header = Some((class, file_id, stripped));
        } else if header.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(header, &body, &mut out);
    out
}

/// A reference Unity writes: `{fileID: …, guid: …, type: …}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ref {
    pub file_id: i64,
    pub guid: Option<String>,
}

impl Ref {
    pub fn is_none(&self) -> bool {
        self.file_id == 0 && self.guid.is_none()
    }
}

/// Reading values out of a document's body.
pub trait Get {
    fn get(&self, key: &str) -> &Yaml;
    fn f32(&self, key: &str) -> Option<f32>;
    fn i64(&self, key: &str) -> Option<i64>;
    fn str(&self, key: &str) -> Option<&str>;
    fn vec3(&self, key: &str) -> Option<[f32; 3]>;
    fn quat(&self, key: &str) -> Option<[f32; 4]>;
    fn color(&self, key: &str) -> Option<[f32; 4]>;
    fn reference(&self, key: &str) -> Option<Ref>;
    fn list(&self, key: &str) -> &[Yaml];
}

pub fn number(y: &Yaml) -> Option<f64> {
    match y {
        Yaml::Integer(i) => Some(*i as f64),
        Yaml::Real(r) => r.parse().ok(),
        Yaml::String(s) => s.parse().ok(),
        Yaml::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// A whole number, exactly: Unity's fileIDs are 64-bit, past what an
/// `f64` holds without rounding.
pub fn integer(y: &Yaml) -> Option<i64> {
    match y {
        Yaml::Integer(i) => Some(*i),
        Yaml::String(s) | Yaml::Real(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn reference(y: &Yaml) -> Option<Ref> {
    let file_id = integer(&y["fileID"])?;
    let guid = y["guid"]
        .as_str()
        .map(str::to_string)
        .or_else(|| match &y["guid"] {
            Yaml::Integer(i) => Some(i.to_string()),
            // `0000000000000000e000000000000000`, Unity's built-in
            // resources, reads as a number in exponent form: its text is
            // kept.
            Yaml::Real(text) => Some(text.clone()),
            _ => None,
        });
    Some(Ref { file_id, guid })
}

impl Get for Yaml {
    fn get(&self, key: &str) -> &Yaml {
        &self[key]
    }
    fn f32(&self, key: &str) -> Option<f32> {
        number(&self[key]).map(|n| n as f32)
    }
    fn i64(&self, key: &str) -> Option<i64> {
        integer(&self[key]).or_else(|| number(&self[key]).map(|n| n as i64))
    }
    fn str(&self, key: &str) -> Option<&str> {
        self[key].as_str()
    }
    fn vec3(&self, key: &str) -> Option<[f32; 3]> {
        let v = &self[key];
        Some([v.f32("x")?, v.f32("y")?, v.f32("z")?])
    }
    fn quat(&self, key: &str) -> Option<[f32; 4]> {
        let v = &self[key];
        Some([v.f32("x")?, v.f32("y")?, v.f32("z")?, v.f32("w")?])
    }
    fn color(&self, key: &str) -> Option<[f32; 4]> {
        let v = &self[key];
        Some([
            v.f32("r")?,
            v.f32("g")?,
            v.f32("b")?,
            v.f32("a").unwrap_or(1.0),
        ])
    }
    fn reference(&self, key: &str) -> Option<Ref> {
        reference(&self[key])
    }
    fn list(&self, key: &str) -> &[Yaml] {
        match &self[key] {
            Yaml::Array(a) => a,
            _ => &[],
        }
    }
}

/// A `.meta` file's GUID, read from its `guid:` line.
pub fn meta_guid(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("guid: "))
        .map(|g| g.trim().to_string())
}

/// Documents by fileID, for following references inside one file.
pub fn by_id(docs: &[Doc]) -> HashMap<i64, &Doc> {
    docs.iter().map(|d| (d.file_id, d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1 &100
GameObject:
  m_Name: Crate
  m_Component:
  - component: {fileID: 200}
--- !u!4 &200
Transform:
  m_GameObject: {fileID: 100}
  m_LocalPosition: {x: 1, y: 2.5, z: -3}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_Father: {fileID: 0}
--- !u!4 &300 stripped
Transform:
  m_PrefabInstance: {fileID: 400}
";

    #[test]
    fn documents_come_with_their_class_id_and_stripped_flag() {
        let docs = documents(FILE);
        assert_eq!(docs.len(), 3);
        assert_eq!(
            (docs[0].class, docs[0].file_id, docs[0].kind.as_str()),
            (1, 100, "GameObject")
        );
        assert_eq!(docs[0].body.str("m_Name"), Some("Crate"));
        assert_eq!(
            docs[0].body.list("m_Component")[0]
                .reference("component")
                .unwrap()
                .file_id,
            200
        );
        assert_eq!(docs[1].body.vec3("m_LocalPosition"), Some([1.0, 2.5, -3.0]));
        assert!(docs[2].stripped);
        assert_eq!(
            meta_guid("fileFormatVersion: 2\nguid: abc123\n"),
            Some("abc123".into())
        );
        let big = documents("--- !u!4 &1\nTransform:\n  m_Father: {fileID: 5565795505742495678}\n");
        assert_eq!(
            big[0].body.reference("m_Father").unwrap().file_id,
            5565795505742495678
        );
    }
}
