//! Tables of records: the game's data by name — materials, recipes,
//! wolves — each record a block of RON with an ID that outlives its name
//! (docs/data.md).
//!
//! A table is a file in `configs/`, or a folder of them, holding a map from
//! a record's name to its fields:
//!
//! ```ron
//! {
//!     "Палка": (id: "4c1e0e9a7b3d2e65", hard: (1, 2), burns: (2, 2)),
//!     "Доска": (id: "9a02c1d77e0b5f13", base: "Палка", hard: (2, 4)),
//! }
//! ```
//!
//! What a record is, the game says, with a type of its own
//! (`Table::<Material>::load`); `id` and `base` are the table's. A record
//! with a `base` holds only what it changes: the base's fields are under
//! its own, each of its own taking the base's place whole. A record written
//! without an `id` has the one its name gives ([`RecordId::from_name`])
//! until a save writes it down ([`settle_ids`]) — the same one, so nothing
//! that linked to it in between loses it.
//!
//! One link to a record from anywhere — another record, a component — is a
//! [`Link`]: the name for the person reading the line, the ID for finding
//! it, as an [`crate::AssetLink`] is for an asset.
//!
//! Loading is all or nothing: a table with one record that does not fit is
//! reported, record and line, and the game keeps the table it had
//! (DNA, postulate 1).
//!
//! ```
//! # use serde::Deserialize;
//! #[derive(Deserialize)]
//! struct Wolf { speed: f32, pack: u32 }
//! impl scrap_core::table::Record for Wolf {}
//!
//! let wolves = scrap_core::Table::<Wolf>::from_text(
//!     r#"{ "grey": (id: "1", speed: 4.0, pack: 5), "old": (base: "grey", speed: 2.5) }"#,
//! )
//! .unwrap();
//! assert_eq!(wolves.named("old").unwrap().pack, 5);
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::marker::PhantomData;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::de::{DeserializeOwned, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use crate::ron_edit::{self, Change};
use crate::ron_text::{outline, Kind, Part};
use crate::shape::Shape;

// --- identity -----------------------------------------------------------

/// A record's identity: sixteen hex digits, like an entity's, written in
/// the record as `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(u64);

impl RecordId {
    /// The identity a name gives a record that has none written yet. The
    /// same name gives the same one on every machine, so a record added by
    /// hand is linked to by the ID it will be saved with.
    pub fn from_name(name: &str) -> Self {
        // FNV-1a, then mixed so that near names are far apart.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        let mut z = hash;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        RecordId((z ^ (z >> 31)).max(1))
    }

    pub const fn from_raw(raw: u64) -> Self {
        RecordId(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl std::str::FromStr for RecordId {
    type Err = String;

    /// Up to sixteen hex digits; shorter is accepted, as for an entity.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.trim();
        if digits.is_empty() || digits.len() > 16 || !digits.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(format!(
                "record id {text:?} is not up to sixteen hex digits, like \"4c1e0e9a7b3d2e65\""
            ));
        }
        u64::from_str_radix(digits, 16)
            .map(RecordId)
            .map_err(|e| e.to_string())
    }
}

impl Serialize for RecordId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.collect_str(self)
        } else {
            serializer.serialize_u64(self.0)
        }
    }
}

impl<'de> Deserialize<'de> for RecordId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let text = String::deserialize(deserializer)?;
            text.parse().map_err(serde::de::Error::custom)
        } else {
            u64::deserialize(deserializer).map(RecordId)
        }
    }
}

// --- the game's side ----------------------------------------------------

/// A type a table holds. What the type cannot say about itself — a grade
/// from 1 to 3, a recipe with at least one step — it says here, in words;
/// a record with a problem is a table that does not load.
///
/// ```
/// # use serde::Deserialize;
/// #[derive(Deserialize)]
/// struct Step { units: u8, min_grade: u8 }
///
/// impl scrap_core::table::Record for Step {
///     fn problems(&self) -> Vec<String> {
///         (!(1..=3).contains(&self.min_grade))
///             .then(|| format!("min_grade is 1 to 3, not {}", self.min_grade))
///             .into_iter()
///             .collect()
///     }
/// }
/// ```
pub trait Record: DeserializeOwned + 'static {
    fn problems(&self) -> Vec<String> {
        Vec::new()
    }
}

/// A type's own name, without its path or generics: what a [`Link`] says
/// it links to, and what `library/tables.ron` knows a table's records by.
pub fn record_name<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base)
}

/// What a [`Link`] read from text says it expects, before the type's
/// name: how [`crate::shape`] knows a field holds a link to a record.
pub const RECORD_EXPECTING: &str = "a record, by its name or (name, id), of";

/// A link to a record of a table of `T`: its name, for the person reading
/// the line, and its ID, for finding it. In a file it is `("Кремень",
/// "e310…")`, or a bare `"Кремень"` as written by hand — found by name,
/// and given its ID when saved through the editor. [`Table::find`] tries
/// the ID first and the name second.
pub struct Link<T> {
    pub name: String,
    pub id: Option<RecordId>,
    of: PhantomData<fn() -> T>,
}

impl<T> Link<T> {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: None,
            of: PhantomData,
        }
    }

    pub fn to(name: impl Into<String>, id: RecordId) -> Self {
        Self {
            name: name.into(),
            id: Some(id),
            of: PhantomData,
        }
    }

    /// Whether it links nothing: `""`.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.id.is_none()
    }
}

impl<T> Clone for Link<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            id: self.id,
            of: PhantomData,
        }
    }
}

impl<T> Default for Link<T> {
    fn default() -> Self {
        Self::named("")
    }
}

impl<T> PartialEq for Link<T> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.id == other.id
    }
}

impl<T> Eq for Link<T> {}

impl<T> std::hash::Hash for Link<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.id.hash(state);
    }
}

impl<T> fmt::Debug for Link<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.id {
            Some(id) => write!(f, "Link<{}>({:?}, {id})", record_name::<T>(), self.name),
            None => write!(f, "Link<{}>({:?})", record_name::<T>(), self.name),
        }
    }
}

impl<T> fmt::Display for Link<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl<T> Serialize for Link<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        match self.id {
            None => serializer.serialize_str(&self.name),
            Some(id) => {
                let mut s = serializer.serialize_tuple(2)?;
                s.serialize_element(&self.name)?;
                s.serialize_element(&id.to_string())?;
                s.end()
            }
        }
    }
}

impl<'de, T> Deserialize<'de> for Link<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Read<T>(PhantomData<fn() -> T>);
        impl<'de, T> Visitor<'de> for Read<T> {
            type Value = Link<T>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{RECORD_EXPECTING} `{}`", record_name::<T>())
            }

            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<Link<T>, E> {
                Ok(Link::named(name))
            }

            fn visit_string<E: serde::de::Error>(self, name: String) -> Result<Link<T>, E> {
                Ok(Link::named(name))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Link<T>, A::Error> {
                let name: String = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let id: Option<String> = seq.next_element()?;
                let id = id
                    .map(|text| text.parse().map_err(serde::de::Error::custom))
                    .transpose()?;
                Ok(Link {
                    name,
                    id,
                    of: PhantomData,
                })
            }
        }
        deserializer.deserialize_any(Read(PhantomData))
    }
}

// --- a table as written -------------------------------------------------

/// A record as its file writes it, before any type reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    pub name: String,
    pub id: Option<RecordId>,
    pub base: Option<String>,
    /// Its own fields as text, in order; `id` and `base` are not among them.
    pub fields: Vec<(String, String)>,
    /// The line its entry starts on, from 1.
    pub line: usize,
    /// Where its value, `( … )`, is in the file.
    pub span: Range<usize>,
}

/// The line `at` is on, from 1.
pub fn line_of(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].matches('\n').count() + 1
}

/// Whether a file is written as a table: a map whose values are records.
/// A struct of numbers (`world.ron`) is not.
pub fn is_table(text: &str) -> bool {
    matches!(outline(text).and_then(|t| t.group), Some((Kind::Map, _)))
}

/// A table's records as written. An error in words for a file that is not
/// RON, or not a map of records by name.
pub fn written(text: &str) -> Result<Vec<Written>, String> {
    // The scan is linear; serde's reading of a whole file as a value is
    // not (a large map takes seconds), so it only says what is wrong.
    let Some(top) = outline(text) else {
        return Err(match ron::from_str::<ron::Value>(text) {
            Err(e) => e.to_string(),
            Ok(_) => "the file does not scan as RON".into(),
        });
    };
    let Some((Kind::Map, entries)) = top.group else {
        return Err(
            "a table is a map of records by name: { \"name\": (field: value, …), … }".into(),
        );
    };
    let mut out = Vec::new();
    // Lines counted on from the last record's, not from the top each time.
    let (mut line, mut counted) = (1, 0);
    for entry in entries {
        line += text[counted..entry.at].matches('\n').count();
        counted = entry.at;
        let key = entry.key.clone().unwrap_or_default();
        let name: String = ron::from_str(&key)
            .map_err(|_| format!("line {line}: a record's name is text in quotes, not {key}"))?;
        let fields = match &entry.group {
            Some((Kind::Struct, fields)) => fields,
            _ => {
                return Err(format!(
                    "line {line}: `{name}` is (field: value, …), not {}",
                    entry.text
                ))
            }
        };
        let mut record = Written {
            name,
            id: None,
            base: None,
            fields: Vec::new(),
            line,
            span: entry.span.clone(),
        };
        for field in fields {
            let key = field.key.clone().unwrap_or_default();
            match key.as_str() {
                "id" => {
                    let text: String = ron::from_str(field.text).map_err(|_| {
                        format!("line {line}: `{}`: id is text in quotes", record.name)
                    })?;
                    record.id = Some(
                        text.parse()
                            .map_err(|e| format!("line {line}: `{}`: {e}", record.name))?,
                    );
                }
                "base" => {
                    let text: String = ron::from_str(field.text).map_err(|_| {
                        format!(
                            "line {line}: `{}`: base is a record's name in quotes",
                            record.name
                        )
                    })?;
                    record.base = Some(text);
                }
                _ => record.fields.push((key, field.text.to_string())),
            }
        }
        out.push(record);
    }
    Ok(out)
}

/// What is wrong with a table as written, before any type reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct TableProblem {
    pub line: usize,
    /// The record's name.
    pub record: String,
    pub message: String,
    /// Works today, costs later: a record without its `id` written.
    pub warning: bool,
}

impl fmt::Display for TableProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: `{}`: {}", self.line, self.record, self.message)
    }
}

/// Names and IDs said twice, bases that name nothing or go round, records
/// whose `id` is not written yet.
pub fn problems_of(records: &[Written]) -> Vec<TableProblem> {
    let mut out = Vec::new();
    let problem = |r: &Written, message: String, warning: bool| TableProblem {
        line: r.line,
        record: r.name.clone(),
        message,
        warning,
    };
    let mut names: HashMap<&str, usize> = HashMap::new();
    let mut ids: HashMap<RecordId, &str> = HashMap::new();
    for r in records {
        if let Some(line) = names.insert(&r.name, r.line) {
            out.push(problem(
                r,
                format!("the name is said twice; the other is on line {line}"),
                false,
            ));
        }
        let id = r.id.unwrap_or_else(|| RecordId::from_name(&r.name));
        if let Some(other) = ids.insert(id, &r.name) {
            out.push(problem(r, format!("has the id of `{other}`, {id}"), false));
        }
        if r.id.is_none() {
            out.push(problem(
                r,
                format!("no `id` written yet; a save through the editor writes id: \"{id}\""),
                true,
            ));
        }
    }
    let (_, base_problems) = resolved(records);
    for (i, message) in base_problems {
        out.push(problem(&records[i], message, false));
    }
    out
}

/// A record's fields as text, by name, in order.
pub type Fields = Vec<(String, String)>;

/// Each record's fields with its base's under them — the base's first, in
/// their order, a field of its own in the base's field's place — and what
/// is wrong with a `base`, by record.
pub fn resolved(records: &[Written]) -> (Vec<Fields>, Vec<(usize, String)>) {
    let by_name: HashMap<&str, usize> = records
        .iter()
        .enumerate()
        .map(|(i, r)| (r.name.as_str(), i))
        .collect();
    let mut problems = Vec::new();
    let mut out = Vec::with_capacity(records.len());
    for (i, record) in records.iter().enumerate() {
        // The chain up from this record: it, its base, the base's base.
        let mut chain = vec![i];
        let mut at = i;
        while let Some(base) = &records[at].base {
            match by_name.get(base.as_str()) {
                Some(&next) if chain.contains(&next) => {
                    let mut names: Vec<&str> =
                        chain.iter().map(|&c| records[c].name.as_str()).collect();
                    names.push(&records[next].name);
                    problems.push((i, format!("`base` goes round: {}", names.join(" → "))));
                    break;
                }
                Some(&next) => {
                    chain.push(next);
                    at = next;
                }
                None => {
                    // Said once, on the record that has the base.
                    if at == i {
                        let near =
                            crate::spelling::closest(base, records.iter().map(|r| r.name.as_str()))
                                .map(|n| format!(" (did you mean `{n}`?)"))
                                .unwrap_or_default();
                        problems.push((i, format!("base `{base}` is no record here{near}")));
                    }
                    break;
                }
            }
        }
        let mut fields: Vec<(String, String)> = Vec::new();
        for &link in chain.iter().rev() {
            for (key, text) in &records[link].fields {
                match fields.iter_mut().find(|(k, _)| k == key) {
                    Some(slot) => slot.1 = text.clone(),
                    None => fields.push((key.clone(), text.clone())),
                }
            }
        }
        let _ = record;
        out.push(fields);
    }
    (out, problems)
}

/// Fields as one struct's text: `(a: 1, b: "x")`.
pub fn compose(fields: &[(String, String)]) -> String {
    let inner: Vec<String> = fields.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    format!("({})", inner.join(", "))
}

// --- a table, read ------------------------------------------------------

/// When each file of a table was last changed, as read.
type Stamps = Vec<(PathBuf, Option<SystemTime>)>;

/// A record read into its type: its name, its identity, its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry<T> {
    pub name: String,
    pub id: RecordId,
    pub value: T,
}

/// A table of `T`, from a file or a folder of them, and kept up with it
/// ([`Table::poll`]), as [`crate::Tuned`] keeps one value.
#[derive(Debug, Clone)]
pub struct Table<T> {
    entries: Vec<Entry<T>>,
    by_id: HashMap<RecordId, usize>,
    by_name: HashMap<String, usize>,
    path: PathBuf,
    stamps: Stamps,
    since_poll: f32,
}

/// The files a table at `path` is read from: the file, or every RON file
/// in the folder and under it, in order.
pub fn files_of(path: &Path) -> Vec<PathBuf> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().is_some_and(|e| e == "ron") {
                out.push(path);
            }
        }
    }
    if path.is_dir() {
        let mut out = Vec::new();
        collect(path, &mut out);
        out.sort();
        out
    } else {
        vec![path.to_path_buf()]
    }
}

impl<T: Record> Table<T> {
    /// Records from one file's text. `file` names it in what goes wrong.
    pub fn read(text: &str, file: &str) -> Result<Vec<Entry<T>>, Vec<String>> {
        let records = written(text).map_err(|e| vec![format!("{file}: {e}")])?;
        let mut problems: Vec<String> = problems_of(&records)
            .into_iter()
            .filter(|p| !p.warning)
            .map(|p| format!("{file}:{p}"))
            .collect();
        let (fields, _) = resolved(&records);
        let mut entries = Vec::with_capacity(records.len());
        for (record, fields) in records.iter().zip(fields) {
            let composed = compose(&fields);
            let at = |message: String| {
                format!("{file}:line {}: `{}`: {message}", record.line, record.name)
            };
            match ron::from_str::<T>(&composed) {
                Ok(value) => {
                    for problem in value.problems() {
                        problems.push(at(problem));
                    }
                    entries.push(Entry {
                        name: record.name.clone(),
                        id: record
                            .id
                            .unwrap_or_else(|| RecordId::from_name(&record.name)),
                        value,
                    });
                }
                Err(e) => {
                    // The shape says it in words, field by field; serde's
                    // own message is for what the shape does not catch.
                    let said = crate::shape::of::<T>().problems(&composed);
                    if said.is_empty() {
                        problems.push(at(e.code.to_string()));
                    } else {
                        problems.extend(said.into_iter().map(at));
                    }
                }
            }
        }
        if problems.is_empty() {
            Ok(entries)
        } else {
            Err(problems)
        }
    }

    /// A table from text, as if it were one file.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let entries = Self::read(text, "the table").map_err(|p| p.join("\n"))?;
        Self::with(entries, PathBuf::new(), Vec::new())
    }

    /// The table at `path`: a file, or a folder whose files are one table.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let (entries, stamps) = Self::read_all(&path)?;
        Self::with(entries, path, stamps)
    }

    fn read_all(path: &Path) -> Result<(Vec<Entry<T>>, Stamps), String> {
        let files = files_of(path);
        if files.is_empty() {
            return Err(format!("{}: no .ron files in the folder", path.display()));
        }
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        let mut stamps = Vec::new();
        for file in files {
            stamps.push((file.clone(), crate::files::modified(&file)));
            let label = file.display().to_string();
            match crate::files::read_to_string(&file) {
                Ok(text) => match Self::read(&text, &label) {
                    Ok(more) => entries.extend(more),
                    Err(p) => problems.extend(p),
                },
                Err(e) => problems.push(format!("{label}: {e}")),
            }
        }
        if problems.is_empty() {
            Ok((entries, stamps))
        } else {
            Err(problems.join("\n"))
        }
    }

    fn with(entries: Vec<Entry<T>>, path: PathBuf, stamps: Stamps) -> Result<Self, String> {
        let mut by_id = HashMap::with_capacity(entries.len());
        let mut by_name = HashMap::with_capacity(entries.len());
        for (i, entry) in entries.iter().enumerate() {
            if let Some(other) = by_name.insert(entry.name.clone(), i) {
                return Err(format!(
                    "`{}` is in the table twice (records {} and {})",
                    entry.name,
                    other + 1,
                    i + 1
                ));
            }
            if let Some(other) = by_id.insert(entry.id, i) {
                return Err(format!(
                    "`{}` and `{}` have one id, {}",
                    entries[other].name, entry.name, entry.id
                ));
            }
        }
        Ok(Self {
            entries,
            by_id,
            by_name,
            path,
            stamps,
            since_poll: 0.0,
        })
    }

    /// Reread the table if a file of it changed, came or went: `None` when
    /// none did, the problems in words when the new text does not load —
    /// with the old records kept.
    pub fn reload_if_changed(&mut self) -> Option<Result<(), String>> {
        let now: Stamps = files_of(&self.path)
            .into_iter()
            .map(|f| {
                let stamp = crate::files::modified(&f);
                (f, stamp)
            })
            .collect();
        if now == self.stamps {
            return None;
        }
        self.stamps = now;
        Some(Self::read_all(&self.path).and_then(|(entries, stamps)| {
            let fresh = Self::with(entries, self.path.clone(), stamps)?;
            *self = fresh;
            Ok(())
        }))
    }

    /// [`Table::reload_if_changed`] at most every
    /// [`crate::tuned::POLL_SECONDS`]: call it every frame with the delta.
    pub fn poll(&mut self, delta: f32) -> Option<Result<(), String>> {
        self.since_poll += delta;
        if self.since_poll < crate::tuned::POLL_SECONDS {
            return None;
        }
        self.since_poll = 0.0;
        self.reload_if_changed()
    }
}

impl<T> Table<T> {
    pub fn get(&self, id: RecordId) -> Option<&T> {
        self.entry(id).map(|e| &e.value)
    }

    pub fn entry(&self, id: RecordId) -> Option<&Entry<T>> {
        self.by_id.get(&id).map(|&i| &self.entries[i])
    }

    pub fn named(&self, name: &str) -> Option<&T> {
        self.by_name.get(name).map(|&i| &self.entries[i].value)
    }

    /// What a link links to: by its ID, and by its name when the ID finds
    /// nothing (a link written by hand, or to a record that was re-added).
    pub fn find(&self, link: &Link<T>) -> Option<&Entry<T>> {
        link.id
            .and_then(|id| self.by_id.get(&id))
            .or_else(|| self.by_name.get(&link.name))
            .map(|&i| &self.entries[i])
    }

    /// A link to the record called `name`, with its ID.
    pub fn link(&self, name: &str) -> Option<Link<T>> {
        self.by_name
            .get(name)
            .map(|&i| Link::to(name, self.entries[i].id))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entry<T>> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl<'a, T> IntoIterator for &'a Table<T> {
    type Item = &'a Entry<T>;
    type IntoIter = std::slice::Iter<'a, Entry<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

// --- the game's tables, for the editor and `check` ----------------------

/// What `library/tables.ron` says of a table: where it is, what its records
/// are called in code, and their shape — what the editor and `scrap check`
/// know it by without linking the game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableShape {
    /// Relative to the project, forward slashes: `configs/materials.ron`
    /// or a folder, `configs/materials`.
    pub path: String,
    /// The records' type, as [`record_name`] says it.
    pub record: String,
    pub shape: Shape,
}

impl TableShape {
    /// Whether the file at `file` (relative, forward slashes) is this
    /// table or one of its folder's.
    pub fn holds(&self, file: &str) -> bool {
        file == self.path
            || file
                .strip_prefix(self.path.trim_end_matches('/'))
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// The shape of one field of its records.
    pub fn field(&self, name: &str) -> Option<&Shape> {
        match &self.shape {
            Shape::Struct(fields) => fields.iter().find(|(n, _)| n == name).map(|(_, s)| s),
            _ => None,
        }
    }
}

/// Read what the game wrote of its tables, where it wrote it.
pub fn read_shapes(path: impl AsRef<Path>) -> Option<Vec<TableShape>> {
    let text = std::fs::read_to_string(path).ok()?;
    ron::from_str(&text).ok()
}

struct Registered {
    path: String,
    record: &'static str,
    shape: fn() -> Shape,
    problems: fn(&Path, &str) -> Vec<String>,
}

/// The tables a game has, by where they are: what it writes for the editor
/// ([`Tables::write_shapes`]) and what its test checks
/// ([`Tables::problems`]), as [`crate::Components`] is for components.
#[derive(Default)]
pub struct Tables {
    tables: Vec<Registered>,
}

fn load_problems<T: Record>(root: &Path, path: &str) -> Vec<String> {
    match Table::<T>::load(root.join(path)) {
        Ok(_) => Vec::new(),
        Err(e) => e.lines().map(str::to_string).collect(),
    }
}

impl Tables {
    pub fn new() -> Self {
        Self::default()
    }

    /// Say that the table at `path` (relative to the project) holds `T`.
    pub fn register<T: Record>(&mut self, path: &str) -> &mut Self {
        self.tables.push(Registered {
            path: path.trim_end_matches('/').to_string(),
            record: record_name::<T>(),
            shape: crate::shape::of::<T>,
            problems: load_problems::<T>,
        });
        self
    }

    pub fn shapes(&self) -> Vec<TableShape> {
        self.tables
            .iter()
            .map(|t| TableShape {
                path: t.path.clone(),
                record: t.record.to_string(),
                shape: (t.shape)(),
            })
            .collect()
    }

    /// Write [`Tables::shapes`] where the editor and `scrap check` read
    /// them: `library/tables.ron` ([`crate::project::TABLE_SHAPES`]).
    pub fn write_shapes(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let text = ron::ser::to_string_pretty(&self.shapes(), ron::ser::PrettyConfig::new())
            .map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Everything wrong with the tables of the project at `root`: a table
    /// that does not load into its type or breaks its rules, and a link to
    /// a record that is not there. What the game's test asserts is empty.
    pub fn problems(&self, root: &Path) -> Vec<String> {
        let mut out: Vec<String> = self
            .tables
            .iter()
            .flat_map(|t| (t.problems)(root, &t.path))
            .collect();
        let shapes = self.shapes();
        for (file, line, message) in link_problems(&shapes, &|path| read_table_files(root, path)) {
            out.push(format!("{file}:line {line}: {message}"));
        }
        out
    }
}

/// The files of the table at `path` under `root`, relative and with their
/// text.
pub fn read_table_files(root: &Path, path: &str) -> Vec<(String, String)> {
    files_of(&root.join(path))
        .into_iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(&file).ok()?;
            let rel = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            Some((rel, text))
        })
        .collect()
}

// --- links between records ---------------------------------------------

/// The records of each type the tables hold, by name and by ID.
#[derive(Debug, Default)]
pub struct Index {
    by_type: HashMap<String, Known>,
}

/// One type's records: each name's ID, and every ID.
#[derive(Debug, Default)]
struct Known {
    names: HashMap<String, RecordId>,
    ids: HashSet<RecordId>,
}

impl Index {
    pub fn build(shapes: &[TableShape], read: &dyn Fn(&str) -> Vec<(String, String)>) -> Self {
        let mut by_type: HashMap<String, Known> = HashMap::new();
        for table in shapes {
            let known = by_type.entry(table.record.clone()).or_default();
            for (_, text) in read(&table.path) {
                for r in written(&text).unwrap_or_default() {
                    let id = r.id.unwrap_or_else(|| RecordId::from_name(&r.name));
                    known.ids.insert(id);
                    known.names.insert(r.name, id);
                }
            }
        }
        Self { by_type }
    }

    /// The names of the records of a type, sorted: what a picker offers.
    pub fn names(&self, record: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .by_type
            .get(record)
            .map(|k| k.names.keys().cloned().collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The ID of the record of a type called `name`.
    pub fn id_of(&self, record: &str, name: &str) -> Option<RecordId> {
        self.by_type.get(record)?.names.get(name).copied()
    }

    /// What is wrong with a link's text to a record of `record`: `None`
    /// when it finds one, or links nothing.
    pub fn problem(&self, record: &str, text: &str) -> Option<String> {
        let (name, id) = parse_link(text)?;
        if name.is_empty() && id.is_none() {
            return None;
        }
        let Some(known) = self.by_type.get(record) else {
            return Some(format!("no table holds `{record}` records"));
        };
        if id.is_some_and(|id| known.ids.contains(&id)) || known.names.contains_key(&name) {
            return None;
        }
        let near = crate::spelling::closest(&name, known.names.keys().map(String::as_str))
            .map(|n| format!(" (did you mean `{n}`?)"))
            .unwrap_or_default();
        Some(format!("no `{record}` called `{name}`{near}"))
    }
}

/// A link's name and ID from its text: `"name"` or `("name", "id")`.
fn parse_link(text: &str) -> Option<(String, Option<RecordId>)> {
    let link: Link<()> = ron::from_str(text.trim()).ok()?;
    Some((link.name, link.id))
}

/// Every link to a record in a value of this shape, by where it is in the
/// value (`steps[1].tool`) and the type it links to, with its text.
pub fn links_in(shape: &Shape, text: &str, at: &str, out: &mut Vec<(String, String, String)>) {
    let text = text.trim();
    let join = |key: &str| {
        if at.is_empty() {
            key.to_string()
        } else {
            format!("{at}.{key}")
        }
    };
    match shape {
        Shape::Record(record) => out.push((at.to_string(), record.clone(), text.to_string())),
        Shape::Option(inner) => {
            if text == "None" {
                return;
            }
            let inside = call_inside(text, "Some").unwrap_or(text);
            links_in(inner, inside, at, out);
        }
        Shape::List(item) => {
            for (i, text) in items_of(text).into_iter().enumerate() {
                links_in(item, text, &format!("{at}[{i}]"), out);
            }
        }
        Shape::Tuple(shapes) => {
            for (i, (shape, text)) in shapes.iter().zip(items_of(text)).enumerate() {
                links_in(shape, text, &format!("{at}.{i}"), out);
            }
        }
        Shape::Map(_, value) => {
            if let Some(Part {
                group: Some((Kind::Map, entries)),
                ..
            }) = outline(text)
            {
                for entry in entries {
                    let key = entry.key.clone().unwrap_or_default();
                    links_in(value, entry.text, &format!("{at}[{key}]"), out);
                }
            }
        }
        Shape::Struct(fields) => {
            if let Some(Part {
                group: Some((Kind::Struct, entries)),
                ..
            }) = outline(text)
            {
                for entry in entries {
                    let key = entry.key.clone().unwrap_or_default();
                    if let Some((_, shape)) = fields.iter().find(|(n, _)| *n == key) {
                        links_in(shape, entry.text, &join(&key), out);
                    }
                }
            }
        }
        Shape::Tagged(variants) => {
            let name: String = text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let Some((_, content)) = variants.iter().find(|(v, _)| *v == name) else {
                return;
            };
            let at = join(&name);
            match content {
                Shape::Unit => {}
                // `Burn(per_second: 1.0)` reads as a struct with a name.
                Shape::Struct(_) => links_in(content, &text[name.len()..], &at, out),
                Shape::Tuple(_) => links_in(content, &text[name.len()..], &at, out),
                other => {
                    if let Some(inside) = call_inside(text, &name) {
                        links_in(other, inside, &at, out);
                    }
                }
            }
        }
        _ => {}
    }
}

/// `Name(inside)`'s inside.
fn call_inside<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    text.strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')
}

/// A list's or a tuple's items as text.
fn items_of(text: &str) -> Vec<&str> {
    let open = text.find(['[', '(']).unwrap_or(0);
    ron_edit::items(text, open)
        .map(|found| found.items.into_iter().map(|r| &text[r]).collect())
        .unwrap_or_default()
}

/// Every link in these tables' records to a record that is not there, as
/// (file, line, what is wrong). `read` gives a table's files and texts.
pub fn link_problems(
    shapes: &[TableShape],
    read: &dyn Fn(&str) -> Vec<(String, String)>,
) -> Vec<(String, usize, String)> {
    let index = Index::build(shapes, read);
    let mut out = Vec::new();
    for table in shapes {
        for (file, text) in read(&table.path) {
            let Ok(records) = written(&text) else {
                continue;
            };
            let (fields, _) = resolved(&records);
            for (record, fields) in records.iter().zip(fields) {
                // Links in what the record itself says: a base's broken
                // link is the base's to report.
                let own: HashSet<&str> = record.fields.iter().map(|(k, _)| k.as_str()).collect();
                for (key, value) in fields.iter().filter(|(k, _)| own.contains(k.as_str())) {
                    let Some(shape) = table.field(key) else {
                        continue;
                    };
                    let mut links = Vec::new();
                    links_in(shape, value, key, &mut links);
                    for (at, of, text) in links {
                        if let Some(problem) = index.problem(&of, &text) {
                            out.push((
                                file.clone(),
                                record.line,
                                format!("`{}`: `{at}`: {problem}", record.name),
                            ));
                        }
                    }
                }
            }
        }
    }
    out
}

// --- editing a table's text where it changes ----------------------------

/// Where the record called `name` is: its value's span.
fn record_span(text: &str, name: &str) -> Option<Range<usize>> {
    written(text)
        .ok()?
        .into_iter()
        .find(|r| r.name == name)
        .map(|r| r.span)
}

/// The map's `{`: where a table's records are.
fn map_open(text: &str) -> Option<usize> {
    let top = outline(text)?;
    matches!(top.group, Some((Kind::Map, _))).then_some(top.span.start)
}

/// `text` with the record `name`'s field `key` set to `value` (RON text),
/// or taken away with `None`; the rest of the file as it was.
pub fn set_field(text: &str, name: &str, key: &str, value: Option<&str>) -> Option<String> {
    let span = record_span(text, name)?;
    let record = &text[span.clone()];
    let changed = ron_edit::set_field(record, key, value)?;
    Some(format!(
        "{}{changed}{}",
        &text[..span.start],
        &text[span.end..]
    ))
}

/// `text` with a record added at the end of the map.
pub fn add_record(text: &str, name: &str, value: &str) -> Option<String> {
    let open = map_open(text)?;
    let key = ron::to_string(name).ok()?;
    ron_edit::apply(text, open, &[Change::Append(format!("{key}: {value}"))])
}

/// `text` without the record called `name`.
pub fn remove_record(text: &str, name: &str) -> Option<String> {
    let open = map_open(text)?;
    let at = written(text).ok()?.iter().position(|r| r.name == name)?;
    ron_edit::apply(text, open, &[Change::Remove(at)])
}

/// `text` with the record `from` called `to`: its key only, the ID stays.
pub fn rename_record(text: &str, from: &str, to: &str) -> Option<String> {
    let open = map_open(text)?;
    let found = ron_edit::items(text, open)?;
    let records = written(text).ok()?;
    let at = records.iter().position(|r| r.name == from)?;
    let item = found.items.get(at)?.clone();
    let value = &text[records[at].span.clone()];
    let key = ron::to_string(to).ok()?;
    let _ = value;
    // The key alone is replaced — a string, so the end of it is its
    // closing quote; what is between it and the value stays.
    let entry = &text[item.clone()];
    let mut end = None;
    let mut escaped = false;
    for (i, c) in entry.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => {
                end = Some(i + 1);
                break;
            }
            _ => {}
        }
    }
    let end = item.start + end?;
    Some(format!("{}{key}{}", &text[..item.start], &text[end..]))
}

/// `text` with every record's `id` written, first in the record, where it
/// was not: the one its name gives. `None` when every record has one.
pub fn settle_ids(text: &str) -> Option<String> {
    let records = written(text).ok()?;
    let mut out = text.to_string();
    let mut changed = false;
    // From the end back: the spans before stay where they were.
    for record in records.iter().rev().filter(|r| r.id.is_none()) {
        let id = RecordId::from_name(&record.name);
        let value = &out[record.span.clone()];
        let open = value.find('(')?;
        let with = ron_edit::apply(value, open, &[Change::Prepend(format!("id: \"{id}\""))])?;
        out.replace_range(record.span.clone(), &with);
        changed = true;
    }
    changed.then_some(out)
}

// --- merging ------------------------------------------------------------

/// What a three-way merge of a table came to.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    /// Ours, with theirs' changes that did not collide.
    pub text: String,
    /// Where both changed the same thing differently, in words; ours kept.
    pub conflicts: Vec<String>,
}

/// A record's identity across three versions: its ID, the one its name
/// gives when none is written.
fn identity(r: &Written) -> RecordId {
    r.id.unwrap_or_else(|| RecordId::from_name(&r.name))
}

/// Merge `theirs` into `ours`, `base` their common ancestor, as tables: by
/// record (its ID) and field. What only one side changed is taken; what
/// both changed the same way is taken once; what both changed differently
/// keeps ours and is a conflict. A top-level struct of numbers (`world.ron`)
/// merges by field the same way. `None` when a text is not a table or a
/// struct: the caller merges by lines.
pub fn merge(base: &str, ours: &str, theirs: &str) -> Option<Merged> {
    let as_struct = |text: &str| -> Option<Vec<Written>> {
        let top = outline(text)?;
        let Some((Kind::Struct, fields)) = top.group else {
            return None;
        };
        Some(vec![Written {
            name: String::new(),
            id: Some(RecordId(1)),
            base: None,
            fields: fields
                .iter()
                .map(|f| (f.key.clone().unwrap_or_default(), f.text.to_string()))
                .collect(),
            line: 1,
            span: top.span,
        }])
    };
    let tables = [base, ours, theirs].map(|t| written(t).ok());
    if let [Some(b), Some(o), Some(t)] = tables {
        return Some(merge_records(&b, &o, &t, ours, theirs, false));
    }
    let structs = [base, ours, theirs].map(as_struct);
    if let [Some(b), Some(o), Some(t)] = structs {
        return Some(merge_records(&b, &o, &t, ours, theirs, true));
    }
    None
}

/// A record's fields, `id` and `base` among them, as text.
fn all_fields(r: &Written) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(base) = &r.base {
        out.push(("base".to_string(), ron::to_string(base).unwrap_or_default()));
    }
    out.extend(r.fields.iter().cloned());
    out
}

fn merge_records(
    base: &[Written],
    ours: &[Written],
    theirs: &[Written],
    ours_text: &str,
    theirs_text: &str,
    one_struct: bool,
) -> Merged {
    let find = |set: &[Written], id: RecordId| set.iter().find(|r| identity(r) == id).cloned();
    let mut text = ours_text.to_string();
    let mut conflicts = Vec::new();
    // The field edits within a record, made on the text as it stands.
    let set = |text: &mut String, name: &str, key: &str, value: Option<&str>| {
        let next = if one_struct {
            ron_edit::set_field(text, key, value)
        } else {
            set_field(text, name, key, value)
        };
        if let Some(next) = next {
            *text = next;
        }
    };
    for t in theirs {
        let id = identity(t);
        let (b, o) = (find(base, id), find(ours, id));
        match (b, o) {
            (Some(b), Some(o)) => {
                let label = if one_struct {
                    String::new()
                } else {
                    format!("`{}`: ", o.name)
                };
                // Renamed on their side only: the key follows.
                if t.name != b.name && o.name == b.name && !one_struct {
                    if let Some(next) = rename_record(&text, &o.name, &t.name) {
                        text = next;
                    }
                } else if t.name != b.name && o.name != b.name && t.name != o.name {
                    conflicts.push(format!(
                        "`{}`: both renamed it, ours to `{}`, theirs to `{}`",
                        b.name, o.name, t.name
                    ));
                }
                let name = if t.name != b.name && o.name == b.name {
                    t.name.clone()
                } else {
                    o.name.clone()
                };
                let (bf, of, tf) = (all_fields(&b), all_fields(&o), all_fields(t));
                let get = |set: &[(String, String)], k: &str| {
                    set.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
                };
                let mut keys: Vec<String> = tf.iter().map(|(k, _)| k.clone()).collect();
                keys.extend(
                    bf.iter()
                        .map(|(k, _)| k.clone())
                        .filter(|k| get(&tf, k).is_none()),
                );
                for key in keys {
                    let (bv, ov, tv) = (get(&bf, &key), get(&of, &key), get(&tf, &key));
                    if tv == bv || tv == ov {
                        continue;
                    }
                    if ov == bv {
                        set(&mut text, &name, &key, tv.as_deref());
                    } else {
                        conflicts.push(format!(
                            "{label}both changed `{key}`: ours {}, theirs {}",
                            ov.as_deref().unwrap_or("took it away"),
                            tv.as_deref().unwrap_or("took it away")
                        ));
                    }
                }
            }
            (None, None) => {
                // Theirs added it: in it goes, as they wrote it.
                let value = &theirs_text[t.span.clone()];
                if let Some(next) = add_record(&text, &t.name, value) {
                    text = next;
                }
            }
            (Some(b), None) => {
                if all_fields(&b) != all_fields(t) || b.name != t.name {
                    conflicts.push(format!(
                        "`{}`: ours took it away, theirs changed it",
                        t.name
                    ));
                }
            }
            (None, Some(o)) => {
                if all_fields(&o) != all_fields(t) {
                    conflicts.push(format!("`{}`: both added it, differently", o.name));
                }
            }
        }
    }
    // What theirs took away, ours left as it was: gone.
    for b in base {
        let id = identity(b);
        if find(theirs, id).is_some() {
            continue;
        }
        if let Some(o) = find(ours, id) {
            if all_fields(&o) == all_fields(b) && o.name == b.name {
                if let Some(next) = remove_record(&text, &o.name) {
                    text = next;
                }
            } else {
                conflicts.push(format!(
                    "`{}`: theirs took it away, ours changed it",
                    o.name
                ));
            }
        }
    }
    Merged { text, conflicts }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
    enum Tag {
        Hard,
        Burns,
        Sharp,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Material {
        tags: Vec<(Tag, u8, u8)>,
        #[serde(default)]
        note: String,
    }

    impl Record for Material {
        fn problems(&self) -> Vec<String> {
            self.tags
                .iter()
                .filter(|(_, grade, _)| !(1..=3).contains(grade))
                .map(|(tag, grade, _)| format!("{tag:?} has grade {grade}; grades are 1 to 3"))
                .collect()
        }
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    enum Need {
        Tag(Tag),
        Made(Link<Material>),
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Recipe {
        steps: Vec<Need>,
        tool: Option<Link<Material>>,
    }

    impl Record for Recipe {}

    const MATERIALS: &str = r#"// What things are made of.
{
    "Палка": (id: "4c1e", tags: [(Hard, 1, 2), (Burns, 2, 2)]),
    // A board is a better stick.
    "Доска": (
        id: "9a02",
        base: "Палка",
        tags: [(Hard, 2, 4), (Burns, 2, 3)],
        note: "planed",
    ),
    "Обломок доски": (base: "Доска", note: "split"),
}
"#;

    #[test]
    fn records_read_into_their_type_with_their_base_under_them() {
        let table = Table::<Material>::from_text(MATERIALS).unwrap();
        assert_eq!(table.len(), 3);
        let board = table.named("Доска").unwrap();
        assert_eq!(board.tags[0], (Tag::Hard, 2, 4));
        let piece = table.named("Обломок доски").unwrap();
        assert_eq!(
            piece.tags, board.tags,
            "what it does not say, its base does"
        );
        assert_eq!(piece.note, "split");
        let id: RecordId = "9a02".parse().unwrap();
        assert_eq!(table.entry(id).unwrap().name, "Доска");
        // Written without an id: the one its name gives.
        let piece = table.link("Обломок доски").unwrap();
        assert_eq!(piece.id, Some(RecordId::from_name("Обломок доски")));
        assert_eq!(
            table.find(&Link::named("Палка")).unwrap().id,
            "4c1e".parse().unwrap()
        );
        assert_eq!(
            table.find(&Link::to("Board", id)).unwrap().name,
            "Доска",
            "by id first"
        );
    }

    #[test]
    fn a_record_that_does_not_fit_or_breaks_a_rule_names_itself_and_its_line() {
        let text = r#"{
    "Палка": (tags: [(Hard, 1, 2)]),
    "Кремень": (tags: [(Sharp, 4, 1)]),
    "Смола": (tagz: []),
}"#;
        let e = Table::<Material>::from_text(text).unwrap_err();
        assert!(e.contains("line 3: `Кремень`: Sharp has grade 4"), "{e}");
        assert!(
            e.contains("line 4: `Смола`: no field `tagz` (did you mean `tags`?)"),
            "{e}"
        );
        assert!(!e.contains("Палка"), "{e}");
    }

    #[test]
    fn what_is_wrong_with_a_table_as_written_is_said_by_record() {
        let text = r#"{
    "a": (id: "1", base: "c"),
    "b": (id: "1", base: "z"),
    "c": (base: "a"),
}"#;
        let problems: Vec<String> = problems_of(&written(text).unwrap())
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("line 3: `b`: has the id of `a`")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`b`: base `z` is no record here")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`base` goes round: a → c → a")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`c`: no `id` written yet")),
            "{problems:?}"
        );
        assert!(written("(gravity: 1.0)")
            .unwrap_err()
            .contains("a table is a map"));
        assert!(!is_table("(gravity: 1.0)"));
        assert!(is_table(MATERIALS));
    }

    #[test]
    fn ids_are_written_first_in_each_record_that_has_none_and_nowhere_else() {
        let settled = settle_ids(MATERIALS).unwrap();
        let id = RecordId::from_name("Обломок доски");
        assert!(
            settled.contains(&format!(
                "\"Обломок доски\": (id: \"{id}\", base: \"Доска\""
            )),
            "{settled}"
        );
        assert!(settled.contains("// A board is a better stick."));
        assert_eq!(settled.matches("id:").count(), 3);
        assert_eq!(settle_ids(&settled), None);
        // The same records, the same ids: a table reads the same before
        // and after.
        let before = Table::<Material>::from_text(MATERIALS).unwrap();
        let after = Table::<Material>::from_text(&settled).unwrap();
        let ids = |t: &Table<Material>| t.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(&before), ids(&after));
        let multi = "{\n    \"a\": (\n        x: 1,\n    ),\n}";
        let id = RecordId::from_name("a");
        assert_eq!(
            settle_ids(multi).unwrap(),
            format!("{{\n    \"a\": (\n        id: \"{id}\",\n        x: 1,\n    ),\n}}")
        );
    }

    #[test]
    fn a_field_a_record_and_a_name_change_in_place() {
        let out = set_field(MATERIALS, "Доска", "note", Some("\"sanded\"")).unwrap();
        assert!(out.contains("        note: \"sanded\",\n"), "{out}");
        assert!(out.contains("// What things are made of."));
        let out = set_field(&out, "Палка", "note", Some("\"raw\"")).unwrap();
        assert!(
            out.contains("(id: \"4c1e\", tags: [(Hard, 1, 2), (Burns, 2, 2)], note: \"raw\")"),
            "{out}"
        );
        let out = rename_record(&out, "Палка", "Сук").unwrap();
        assert!(out.contains("    \"Сук\": (id: \"4c1e\""), "{out}");
        let out = add_record(&out, "Кость", "(tags: [(Hard, 3, 1)])").unwrap();
        assert!(
            out.contains("    \"Кость\": (tags: [(Hard, 3, 1)]),\n}"),
            "{out}"
        );
        let out = remove_record(&out, "Обломок доски").unwrap();
        assert!(!out.contains("Обломок"), "{out}");
        assert!(
            Table::<Material>::from_text(&out).is_err(),
            "Доска's base is gone: Сук"
        );
    }

    #[test]
    fn links_to_records_are_followed_through_options_lists_and_variants() {
        let shape = crate::shape::of::<Recipe>();
        let Shape::Struct(fields) = &shape else {
            panic!("{shape:?}")
        };
        assert_eq!(
            fields[1].1,
            Shape::Option(Box::new(Shape::Record("Material".into())))
        );
        let mut links = Vec::new();
        links_in(
            &shape,
            r#"(steps: [Tag(Hard), Made(("Доска", "9a02")), Made("Смола")], tool: Some("Кремень"))"#,
            "",
            &mut links,
        );
        let found: Vec<(&str, &str)> = links
            .iter()
            .map(|(at, _, t)| (at.as_str(), t.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("steps[1].Made", "(\"Доска\", \"9a02\")"),
                ("steps[2].Made", "\"Смола\""),
                ("tool", "\"Кремень\""),
            ]
        );
    }

    #[test]
    fn a_link_to_no_record_is_found_with_the_nearest_name() {
        let shapes = vec![
            TableShape {
                path: "configs/materials.ron".into(),
                record: "Material".into(),
                shape: crate::shape::of::<Material>(),
            },
            TableShape {
                path: "configs/recipes".into(),
                record: "Recipe".into(),
                shape: crate::shape::of::<Recipe>(),
            },
        ];
        let read = |path: &str| -> Vec<(String, String)> {
            match path {
                "configs/materials.ron" => vec![(path.into(), MATERIALS.into())],
                _ => vec![(
                    "configs/recipes/tools.ron".into(),
                    r#"{
    "Топор": (steps: [Made("Доска"), Made(("Палкa", "4c1e"))], tool: Some("Досак")),
}"#
                    .into(),
                )],
            }
        };
        let problems = link_problems(&shapes, &read);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let (file, line, message) = &problems[0];
        assert_eq!((file.as_str(), *line), ("configs/recipes/tools.ron", 2));
        assert_eq!(
            message,
            "`Топор`: `tool`: no `Material` called `Досак` (did you mean `Доска`?)"
        );
        assert!(shapes[1].holds("configs/recipes/tools.ron"));
        assert!(!shapes[1].holds("configs/recipes.ron"));
    }

    #[test]
    fn theirs_and_ours_merge_by_record_and_field() {
        let base = MATERIALS;
        let ours = set_field(base, "Доска", "note", Some("\"sanded\"")).unwrap();
        let ours = add_record(&ours, "Кость", "(id: \"b0\", tags: [(Hard, 3, 1)])").unwrap();
        let theirs = set_field(base, "Палка", "tags", Some("[(Hard, 1, 3)]")).unwrap();
        let theirs = rename_record(&theirs, "Обломок доски", "Щепа").unwrap();
        let theirs = add_record(&theirs, "Смола", "(id: \"c0\", tags: [(Burns, 3, 2)])").unwrap();
        let merged = merge(base, &ours, &theirs).unwrap();
        assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
        let table = Table::<Material>::from_text(&merged.text).unwrap();
        assert_eq!(table.named("Палка").unwrap().tags, [(Tag::Hard, 1, 3)]);
        assert_eq!(table.named("Доска").unwrap().note, "sanded");
        assert!(table.named("Щепа").is_some() && table.named("Обломок доски").is_none());
        assert!(table.named("Кость").is_some() && table.named("Смола").is_some());
        assert!(merged.text.contains("// A board is a better stick."));

        // Both change one field differently: ours stays, and it is said.
        let theirs = set_field(base, "Доска", "note", Some("\"oiled\"")).unwrap();
        let merged = merge(base, &ours, &theirs).unwrap();
        assert_eq!(merged.conflicts.len(), 1, "{:?}", merged.conflicts);
        assert!(
            merged.conflicts[0].contains("`Доска`: both changed `note`"),
            "{:?}",
            merged.conflicts
        );
        assert!(merged.text.contains("sanded"));

        // Theirs takes a record away that ours left alone: it goes.
        let theirs = remove_record(base, "Обломок доски").unwrap();
        let merged = merge(base, &ours, &theirs).unwrap();
        assert!(!merged.text.contains("Обломок"), "{}", merged.text);

        // A struct of numbers merges by field.
        let merged = merge("(a: 1, b: 2)", "(a: 5, b: 2)", "(a: 1, b: 7)").unwrap();
        assert_eq!(merged.text, "(a: 5, b: 7)");
        assert_eq!(merge("[1]", "[2]", "[3]"), None);
    }

    #[test]
    fn a_folder_is_one_table_and_a_save_reloads_it() {
        let dir = std::env::temp_dir().join(format!("scrap-table-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("wood")).unwrap();
        std::fs::write(
            dir.join("wood/sticks.ron"),
            r#"{ "Палка": (id: "1", tags: [(Hard, 1, 2)]) }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("stone.ron"),
            r#"{ "Кремень": (id: "2", tags: [(Sharp, 2, 1)]) }"#,
        )
        .unwrap();
        let mut table = Table::<Material>::load(&dir).unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(table.reload_if_changed(), None);
        // A save that breaks a rule: said, and the table stays as it was.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            dir.join("stone.ron"),
            r#"{ "Кремень": (id: "2", tags: [(Sharp, 9, 1)]) }"#,
        )
        .unwrap();
        let e = table.reload_if_changed().unwrap().unwrap_err();
        assert!(
            e.contains("stone.ron:line 1: `Кремень`: Sharp has grade 9"),
            "{e}"
        );
        assert_eq!(table.named("Кремень").unwrap().tags[0].1, 2);
        // A new file of the folder is in the table.
        std::fs::write(
            dir.join("stone.ron"),
            r#"{ "Кремень": (id: "2", tags: [(Sharp, 3, 1)]) }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("resin.ron"),
            r#"{ "Смола": (id: "3", tags: [(Burns, 3, 2)]) }"#,
        )
        .unwrap();
        assert_eq!(table.reload_if_changed(), Some(Ok(())));
        assert_eq!(table.len(), 3);
        assert_eq!(table.named("Кремень").unwrap().tags[0].1, 3);
        // One name in two files of the folder: not a table.
        std::fs::write(dir.join("resin.ron"), r#"{ "Палка": (id: "3", tags: []) }"#).unwrap();
        assert!(table
            .reload_if_changed()
            .unwrap()
            .unwrap_err()
            .contains("`Палка` is in the table twice"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_game_s_tables_are_written_for_the_editor_and_checked_for_its_test() {
        let dir = std::env::temp_dir().join(format!("scrap-tables-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("configs")).unwrap();
        std::fs::write(dir.join("configs/materials.ron"), MATERIALS).unwrap();
        std::fs::write(
            dir.join("configs/recipes.ron"),
            r#"{ "Факел": (steps: [Made("Палка")], tool: Some("Смола")) }"#,
        )
        .unwrap();
        let mut tables = Tables::new();
        tables
            .register::<Material>("configs/materials.ron")
            .register::<Recipe>("configs/recipes.ron");
        let problems = tables.problems(&dir);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("`Факел`: `tool`: no `Material` called `Смола`"),
            "{problems:?}"
        );
        tables.write_shapes(dir.join("library/tables.ron")).unwrap();
        let shapes = read_shapes(dir.join("library/tables.ron")).unwrap();
        assert_eq!(shapes, tables.shapes());
        assert_eq!(shapes[1].record, "Recipe");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_record_s_id_from_its_name_is_the_same_everywhere_and_never_zero() {
        assert_eq!(RecordId::from_name("Палка"), RecordId::from_name("Палка"));
        assert_ne!(RecordId::from_name("Палка"), RecordId::from_name("Палки"));
        assert_ne!(RecordId::from_name("").raw(), 0);
        let id = RecordId::from_name("Палка");
        assert_eq!(id.to_string().parse::<RecordId>().unwrap(), id);
        assert_eq!(record_name::<Link<Material>>(), "Link");
        assert_eq!(record_name::<Material>(), "Material");
        let link: Link<Material> = ron::from_str(r#"("Доска", "9a02")"#).unwrap();
        assert_eq!(
            ron::to_string(&link).unwrap(),
            r#"("Доска","0000000000009a02")"#
        );
        let bare: Link<Material> = ron::from_str(r#""Доска""#).unwrap();
        assert_eq!(ron::to_string(&bare).unwrap(), r#""Доска""#);
    }
}
