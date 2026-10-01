//! Making and changing clips — Unity's Animation window, under the panel.
//!
//! A selected entity's clips are of three kinds, and the window treats them
//! alike — tracks of keys over time:
//!
//! * **Things** — a `*.clip.ron` its `animator` graph plays, moving the
//!   entity and what is under it by path: a door, a platform, a bell.
//!   Made here from nothing, as Unity makes a clip on a GameObject.
//! * **Rig** — a clip of the bones of the model it draws, made here from
//!   the rest pose (`base: (model: …)` with no clip; see
//!   [`scrap::overlay`]).
//! * **Model** — a clip that came with the model. Changing one writes an
//!   edit over it (`base: (model: …, clip: …)`) with only the tracks that
//!   changed: it is still the model's, and the model imported again still
//!   comes through under the edit.
//!
//! Showing a clip at a time poses the entity *in the world*, not in the
//! document, as [`Session::preview_clip`] plays one: nothing is an undo
//! step, and a respawn lets go of it. What is written is the clip's file,
//! changed only where it changed ([`scrap::overlay::write`]).

#[allow(unused_imports)]
use scrap::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use scrap::animation::Clip;
use scrap::layout::Kind;
use scrap::motion::{Motion, Motions, Property, Track};
use scrap::overlay::{self, Base};
use scrap::{Animator, AssetLink, EntityId, SceneId};

use crate::{EditError, EditResult, Session};

/// Where a clip comes from, and so what changing it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClipKind {
    /// Moves the entity and what is under it.
    Things,
    /// The model's bones, made here from the rest pose.
    Rig,
    /// Came with the model; changed by an edit over it.
    Model,
}

/// A clip as the Animation window has it open: every track that plays,
/// and what its file says.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipDoc {
    pub name: String,
    pub kind: ClipKind,
    /// Its file; `None` for a model's clip nobody has changed yet.
    pub file: Option<PathBuf>,
    /// The file's contents: for a model's clip, only the edit.
    pub motion: Motion,
    /// Every track that plays, the edit's in place of the model's: what
    /// the window lists and draws.
    pub tracks: Vec<Track>,
    /// Seconds.
    pub length: f32,
    /// The model a rig's or a model's clip is of.
    pub model: Option<String>,
}

impl ClipDoc {
    /// Whether the file keys this track — on a model's clip, whether it is
    /// changed from the model's.
    pub fn is_written(&self, path: &str, what: Property) -> bool {
        self.motion.tracks.iter().any(|t| t.path == path && t.what == what)
    }

    /// Put a track in, in place of the one of its path and property, or
    /// as a new one; an empty track goes. On a model's clip the track
    /// becomes part of the edit over it.
    pub fn set_track(&mut self, track: Track) {
        let same = |t: &Track| t.path == track.path && t.what == track.what;
        for list in [&mut self.tracks, &mut self.motion.tracks] {
            match list.iter().position(same) {
                Some(i) if track.keys.is_empty() => {
                    list.remove(i);
                }
                Some(i) => list[i] = track.clone(),
                None if !track.keys.is_empty() => list.push(track.clone()),
                None => {}
            }
        }
        if self.kind == ClipKind::Model && self.motion.base.is_none() {
            self.motion.base = Some(Base {
                model: AssetLink::named(self.model.clone().unwrap_or_default()),
                clip: self.name.clone(),
            });
        }
    }

    pub fn track(&self, path: &str, what: Property) -> Option<&Track> {
        self.tracks.iter().find(|t| t.path == path && t.what == what)
    }

    /// Change the length: the clip ends there, its keys where they are.
    pub fn set_length(&mut self, seconds: f32) {
        self.length = seconds.max(1.0 / 60.0);
        self.motion.length = self.length;
    }
}

impl Session {
    /// The project's graphs and clips, read now.
    fn motions(&self) -> Motions {
        self.project
            .as_ref()
            .map(|p| Motions::load(p.root()).0)
            .unwrap_or_default()
    }

    /// The skin of the model an entity draws, as the library has it: its
    /// clips without the project's edits.
    fn library_skin(&self, id: EntityId) -> Option<scrap::asset::MeshSkin> {
        let model = self.entity_model(id)?;
        self.library.as_ref()?.mesh_by_name(&model)?.skin_owned()
    }

    /// The hecs entity of a document's entity in the world.
    pub(crate) fn world_entity(&self, id: EntityId) -> Option<hecs::Entity> {
        self.world
            .query::<(hecs::Entity, &SceneId)>()
            .iter()
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
    }

    /// The clips an entity can show, by name and kind: its model's, the
    /// project's clips of that model's rig, and the clips its `animator`
    /// graph moves things with.
    pub fn clip_list(&self, id: EntityId) -> Vec<(String, ClipKind)> {
        let motions = self.motions();
        let mut out: Vec<(String, ClipKind)> = Vec::new();
        if let (Some(skin), Some(model)) = (self.library_skin(id), self.entity_model(id)) {
            out.extend(skin.clips.iter().map(|c| (c.name.clone(), ClipKind::Model)));
            let mut rig: Vec<(String, ClipKind)> = motions
                .clips
                .iter()
                .filter_map(|(name, m)| {
                    let base = m.base.as_ref()?;
                    (base.model.name == model && !out.iter().any(|(n, _)| n == name)).then(|| {
                        let kind = if base.clip.is_empty() { ClipKind::Rig } else { ClipKind::Model };
                        (name.clone(), kind)
                    })
                })
                .collect();
            rig.sort();
            out.extend(rig);
        }
        if let Some(graph) = self.line(id).map(|l| l.animator()).filter(|a| !a.is_empty()) {
            if let Some(graph) = motions.graphs.get(&graph) {
                for clip in graph.clips() {
                    let things = motions.clips.get(&clip).is_some_and(|m| m.base.is_none());
                    if things && !out.iter().any(|(n, _)| *n == clip) {
                        out.push((clip, ClipKind::Things));
                    }
                }
            }
        }
        out
    }

    /// A clip of an entity, open for editing: its tracks and its file.
    pub fn open_clip(&self, id: EntityId, name: &str) -> EditResult<ClipDoc> {
        let motions = self.motions();
        let file = self.project.as_ref().and_then(|p| p.file(Kind::Clip, name));
        let written = motions.clips.get(name).cloned();
        let model = self.entity_model(id);
        let skin = self.library_skin(id);
        let from_model = skin.as_ref().and_then(|s| s.clips.iter().find(|c| c.name == name));
        let missing = || EditError::Scene(format!("no clip `{name}` here"));
        match (written, from_model) {
            // Things, or a rig's own.
            (Some(motion), None) => {
                let kind = if motion.base.is_some() { ClipKind::Rig } else { ClipKind::Things };
                let base = motion.base.as_ref().filter(|b| !b.clip.is_empty());
                if let (Some(base), Some(skin)) = (base, &skin) {
                    // An edit of another name over a model's clip.
                    let clip = skin.clips.iter().find(|c| c.name == base.clip).ok_or_else(missing)?;
                    return Ok(edited(name, clip, &skin.skeleton, motion, file, model));
                }
                Ok(ClipDoc {
                    name: name.to_string(),
                    kind,
                    length: motion.length,
                    tracks: motion.tracks.clone(),
                    motion,
                    file,
                    model,
                })
            }
            (written, Some(clip)) => {
                let skin = skin.as_ref().expect("found in it");
                let motion = written.unwrap_or(Motion { length: clip.duration, ..Default::default() });
                Ok(edited(name, clip, &skin.skeleton, motion, file, model))
            }
            (None, None) => Err(missing()),
        }
    }

    /// Write an open clip to its file — making the file for a model's clip
    /// changed for the first time — changed only where it changed.
    pub fn save_clip(&mut self, doc: &mut ClipDoc) -> EditResult<PathBuf> {
        let project = self.project.as_ref().ok_or_else(|| EditError::Scene("no project open".into()))?;
        let path = doc.file.clone().unwrap_or_else(|| project.new_file(Kind::Clip, &doc.name));
        let old = scrap::files::read_to_string(&path).unwrap_or_default();
        let text = overlay::write(&old, &doc.motion);
        if text != old {
            if let Some(dir) = path.parent() {
                let _ = scrap::files::create_dir_all(dir);
            }
            scrap::files::write(&path, text).map_err(|e| EditError::Scene(format!("{}: {e}", path.display())))?;
        }
        doc.file = Some(path.clone());
        Ok(path)
    }

    /// Make a clip for an entity: of its things, or with `rig` of its
    /// model's bones. Its graph gets a state that plays it, and an entity
    /// with no `animator` gets a graph of its own — Unity's Create New
    /// Clip, which makes the controller with it. Refuses a name a clip
    /// already has.
    pub fn new_clip(&mut self, id: EntityId, name: &str, rig: bool) -> EditResult<ClipDoc> {
        let name = name.trim();
        if name.is_empty() || name.contains(['/', '\\', '"']) {
            return Err(EditError::Scene(format!("`{name}` cannot be a clip's name")));
        }
        let project = self.project.clone().ok_or_else(|| EditError::Scene("no project open".into()))?;
        if project.file(Kind::Clip, name).is_some() || self.clip_list(id).iter().any(|(n, _)| n == name) {
            return Err(EditError::Scene(format!("there is a clip `{name}` already")));
        }
        let model = self.entity_model(id);
        let base = if rig {
            let model = model.clone().filter(|_| self.library_skin(id).is_some()).ok_or_else(|| {
                EditError::Scene("its model has no bones: make a clip of its things".into())
            })?;
            Some(Base { model: AssetLink::named(model), clip: String::new() })
        } else {
            None
        };
        let mut doc = ClipDoc {
            name: name.to_string(),
            kind: if rig { ClipKind::Rig } else { ClipKind::Things },
            file: None,
            motion: Motion { base, length: 1.0, tracks: Vec::new() },
            tracks: Vec::new(),
            length: 1.0,
            model,
        };
        self.save_clip(&mut doc)?;
        self.add_to_graph(id, name)?;
        Ok(doc)
    }

    /// A state that plays `clip` in the entity's graph, or a graph of the
    /// entity's own that starts with it.
    fn add_to_graph(&mut self, id: EntityId, clip: &str) -> EditResult<()> {
        let project = self.project.clone().ok_or_else(|| EditError::Scene("no project open".into()))?;
        let state: scrap::animgraph::State = scrap::ron::from_str(&format!("(clip: {clip:?})")).expect("a state");
        let graph = self.line(id).map(|l| l.animator()).unwrap_or_default();
        if let Some(path) = (!graph.is_empty()).then(|| project.file(Kind::Animator, &graph)).flatten() {
            let old = scrap::files::read_to_string(&path).map_err(|e| EditError::Scene(e.to_string()))?;
            let mut g: scrap::animgraph::Graph =
                scrap::ron::from_str(&old).map_err(|e| EditError::Scene(format!("{}: {e}", path.display())))?;
            if g.states.contains_key(clip) {
                return Ok(());
            }
            g.states.insert(clip.to_string(), state);
            if g.start.is_empty() {
                g.start = clip.to_string();
            }
            let text = scrap::graph_text::write(&old, &g);
            return scrap::files::write(&path, text).map_err(|e| EditError::Scene(e.to_string()));
        }
        // A graph named after the entity, a number after it if that is taken.
        let entity = self.entity_name(id).unwrap_or_else(|| "clips".into());
        let stem: String = entity.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        let name = (0..)
            .map(|n| if n == 0 { stem.clone() } else { format!("{stem}-{n}") })
            .find(|n| project.file(Kind::Animator, n).is_none())
            .expect("a free name");
        let path = project.new_file(Kind::Animator, &name);
        if let Some(dir) = path.parent() {
            let _ = scrap::files::create_dir_all(dir);
        }
        let text = format!("(\n    start: {clip:?},\n    states: {{\n        {clip:?}: (clip: {clip:?}),\n    }},\n)\n");
        scrap::files::write(&path, text).map_err(|e| EditError::Scene(e.to_string()))?;
        self.set_field(id, "animator", &name)
    }

    /// Show an entity as an open clip has it at `time` seconds: its things
    /// moved, or its bones posed. In the world only.
    pub fn pose_clip(&mut self, id: EntityId, doc: &ClipDoc, time: f32) -> EditResult<()> {
        let entity = self.world_entity(id).ok_or(EditError::NoEntity(id))?;
        match doc.kind {
            ClipKind::Things => {
                let line = self.line(id).ok_or(EditError::NoEntity(id))?.clone();
                let parts = scrap::motion::animates(&line).parts;
                let mut by_path: Vec<(&str, Vec<&Track>)> = Vec::new();
                for t in &doc.tracks {
                    match by_path.iter_mut().find(|(p, _)| *p == t.path) {
                        Some((_, list)) => list.push(t),
                        None => by_path.push((&t.path, vec![t])),
                    }
                }
                for (path, tracks) in by_path {
                    let Some((_, part)) = parts.iter().find(|(p, _)| p == path) else { continue };
                    let (Some(target), Some(desc)) = (self.world_entity(*part), self.expanded().get(*part).cloned()) else {
                        continue;
                    };
                    let mut place = desc.transform;
                    for t in tracks {
                        let Some(v) = t.sample(time) else { continue };
                        match t.what {
                            Property::X => place.position.x = v,
                            Property::Y => place.position.y = v,
                            Property::Z => place.position.z = v,
                            Property::TurnX => place.rotation_deg.x = v,
                            Property::TurnY => place.rotation_deg.y = v,
                            Property::TurnZ => place.rotation_deg.z = v,
                            Property::ScaleX => place.scale.x = v,
                            Property::ScaleY => place.scale.y = v,
                            Property::ScaleZ => place.scale.z = v,
                            Property::Active => scrap::world::set_active(&mut self.world, target, v > 0.5),
                            Property::Volume | Property::ParticleRate => {}
                        }
                    }
                    if let Ok(mut t) = self.world.get::<&mut scrap::Transform>(target) {
                        *t = place;
                    }
                }
                scrap::world::apply_hierarchy(&mut self.world);
            }
            ClipKind::Rig | ClipKind::Model => {
                let skin = self.library_skin(id).ok_or_else(|| EditError::Scene("its model has no bones".into()))?;
                let clip = baked(doc, &skin);
                let length = clip.duration.max(1e-4);
                let mut animator = Animator::new(Arc::new(skin.skeleton), Arc::new(vec![clip]));
                animator.play(0, 0.0);
                animator.set_speed(0.0);
                animator.set_fraction((time / length).clamp(0.0, 1.0));
                let _ = self.world.insert_one(entity, animator);
            }
        }
        if !self.previewing.contains(&id) {
            self.previewing.push(id);
        }
        Ok(())
    }

    /// Let go of a clip shown: the entity as the document has it.
    pub fn stop_posing(&mut self) {
        if !self.previewing.is_empty() {
            self.respawn();
        }
    }

    /// Where everything under an entity is in the document, by path: what
    /// recording compares to see what was moved.
    pub fn part_places(&self, id: EntityId) -> Vec<(String, scrap::Transform)> {
        let Some(line) = self.line(id) else { return Vec::new() };
        scrap::motion::animates(line)
            .parts
            .into_iter()
            .filter_map(|(p, part)| Some((p, self.expanded().get(part)?.transform)))
            .collect()
    }

    /// The paths of what an entity's clip can move — `""` itself, then
    /// what is under it — for adding a track.
    pub fn animatable_paths(&self, id: EntityId) -> Vec<String> {
        self.line(id).map(|l| scrap::motion::animates(l).parts.into_iter().map(|(p, _)| p).collect()).unwrap_or_default()
    }

    /// What a track's property would be without the track: the scene's
    /// for a thing, the rest pose's for a rig's bone, the model's own clip's
    /// for a model's — what the tween slider's Default blends toward.
    pub fn default_value(&self, id: EntityId, doc: &ClipDoc, path: &str, what: Property, time: f32) -> f32 {
        let mut bare = doc.clone();
        let other = |t: &Track| !(t.path == path && t.what == what);
        bare.tracks.retain(other);
        bare.motion.tracks.retain(other);
        self.current_value(id, &bare, path, what, time)
    }

    /// What a track's property is now, where the document has it: the
    /// first key a new track starts from. A bone's, from the clip shown at
    /// `time`.
    pub fn current_value(&self, id: EntityId, doc: &ClipDoc, path: &str, what: Property, time: f32) -> f32 {
        if let Some(t) = doc.track(path, what) {
            if let Some(v) = t.sample(time) {
                return v;
            }
        }
        match doc.kind {
            ClipKind::Things => {
                let line = self.line(id);
                let part = line.and_then(|l| scrap::motion::animates(l).parts.into_iter().find(|(p, _)| p == path));
                let Some(desc) = part.and_then(|(_, id)| self.expanded().get(id)) else { return 0.0 };
                let t = &desc.transform;
                match what {
                    Property::X => t.position.x,
                    Property::Y => t.position.y,
                    Property::Z => t.position.z,
                    Property::TurnX => t.rotation_deg.x,
                    Property::TurnY => t.rotation_deg.y,
                    Property::TurnZ => t.rotation_deg.z,
                    Property::ScaleX => t.scale.x,
                    Property::ScaleY => t.scale.y,
                    Property::ScaleZ => t.scale.z,
                    Property::Active => if desc.inactive { 0.0 } else { 1.0 },
                    Property::Volume | Property::ParticleRate => 1.0,
                }
            }
            ClipKind::Rig | ClipKind::Model => {
                let Some(skin) = self.library_skin(id) else { return 0.0 };
                let clip = baked(doc, &skin);
                let Some(joint) = overlay::joint_named(&skin.skeleton, path) else { return 0.0 };
                let pose = clip.sample(&skin.skeleton, time, false);
                let p = pose[joint];
                let Some((part, axis)) = overlay::part_of(what) else { return 0.0 };
                match part {
                    scrap::animation::Path::Translation => p.translation[axis],
                    scrap::animation::Path::Scale => p.scale[axis],
                    scrap::animation::Path::Rotation => overlay::degrees(scrap::glam::Quat::from_array(p.rotation))[axis],
                }
            }
        }
    }
}

impl Session {
    /// The entity whose clips a selection is shown in, and the selection's
    /// path under it: the nearest line at or above it with an `animator`
    /// — Unity's Animation window keeps to the Animator above what is
    /// selected — or the selection itself, at `""`.
    pub fn clip_root(&self, id: EntityId) -> (EntityId, String) {
        fn chain(list: &[scrap::scene::EntityDesc], id: EntityId, out: &mut Vec<scrap::scene::EntityDesc>) -> bool {
            for e in list {
                out.push(e.clone());
                if e.id == id || chain(&e.children, id, out) {
                    return true;
                }
                out.pop();
            }
            false
        }
        let mut above = Vec::new();
        if !chain(&self.expanded().entities, id, &mut above) {
            return (id, String::new());
        }
        let Some(at) = above.iter().rposition(|e| !e.animator().is_empty()) else {
            return (id, String::new());
        };
        let path: Vec<&str> = above[at + 1..].iter().map(|e| e.name.as_str()).collect();
        (above[at].id, path.join("/"))
    }
}

/// A model's clip with an edit over it, open.
fn edited(name: &str, clip: &Clip, skeleton: &scrap::animation::Skeleton, motion: Motion, file: Option<PathBuf>, model: Option<String>) -> ClipDoc {
    let mut tracks = overlay::tracks_of(clip, skeleton);
    for mine in &motion.tracks {
        let same = |t: &Track| overlay::joint_named(skeleton, &t.path) == overlay::joint_named(skeleton, &mine.path) && t.what == mine.what;
        match tracks.iter().position(same) {
            Some(i) => tracks[i] = mine.clone(),
            None => tracks.push(mine.clone()),
        }
    }
    let length = if motion.length > 0.0 { motion.length } else { clip.duration };
    ClipDoc { name: name.to_string(), kind: ClipKind::Model, file, motion, tracks, length, model }
}

/// The clip an open rig's or model's clip plays, as the game bakes it.
fn baked(doc: &ClipDoc, skin: &scrap::asset::MeshSkin) -> Clip {
    let base_name = doc.motion.base.as_ref().map(|b| b.clip.as_str()).unwrap_or(doc.name.as_str());
    let base = skin
        .clips
        .iter()
        .find(|c| c.name == base_name)
        .cloned()
        .unwrap_or_else(|| Clip { name: doc.name.clone(), duration: doc.length, channels: Vec::new() });
    let mut motion = doc.motion.clone();
    motion.length = doc.length;
    if motion.tracks.is_empty() {
        return Clip { duration: doc.length, ..base };
    }
    overlay::bake(&doc.name, &motion, &base, &skin.skeleton, &mut Vec::new())
}
