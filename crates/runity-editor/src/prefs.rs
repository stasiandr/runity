//! What the editor remembers for the person using it: where the view was
//! in each scene, which scene was open last, the snap settings.
//!
//! Unity keeps this in `Library/`; here it is `.runity/editor.ron` in the
//! project, ignored by git — it is one person's, and a scene whose file
//! changed each time someone looked at it from somewhere else would be a
//! diff about nothing. The scene's own `view` stays the shared first look.

use std::collections::BTreeMap;
use std::path::PathBuf;

use runity::glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{Session, Snap};

/// Where the file is, inside a project.
pub const FILE: &str = ".runity/editor.ron";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Prefs {
    /// The scene open last, relative to the project.
    last_scene: String,
    /// Per scene, relative: where the view stood and looked.
    views: BTreeMap<String, SavedView>,
    snap: (f32, f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct SavedView {
    position: Vec3,
    target: Vec3,
    #[serde(default)]
    ortho: Option<f32>,
}

impl Session {
    fn prefs_path(&self) -> Option<PathBuf> {
        Some(self.project.as_ref()?.root().join(FILE))
    }

    fn read_prefs(&self) -> Prefs {
        self.prefs_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| runity::ron::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Write down where the view is in the open scene, and that it is the
    /// one open. Quietly nothing outside a project: it is a convenience.
    pub fn remember_view(&self) {
        let (Some(path), Some(project), Some(scene)) = (
            self.prefs_path(),
            self.project.as_ref(),
            self.scene_path.as_ref(),
        ) else {
            return;
        };
        let Some(name) = project.relative(scene) else {
            return;
        };
        let mut prefs = self.read_prefs();
        prefs.views.insert(
            name.clone(),
            SavedView {
                position: self.camera.position,
                target: self.camera.target,
                ortho: self.camera.ortho,
            },
        );
        prefs.last_scene = name;
        prefs.snap = (self.snap.meters, self.snap.degrees, self.snap.scale);
        if let Ok(text) = runity::ron::ser::to_string_pretty(&prefs, Default::default()) {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = std::fs::write(&path, text + "\n");
        }
    }

    /// Put the view back where this person left it in the open scene, and
    /// their snap settings. `false` when there is nothing remembered.
    pub(crate) fn restore_view(&mut self) -> bool {
        let prefs = self.read_prefs();
        let (m, d, s) = prefs.snap;
        self.set_snap(Snap {
            meters: m,
            degrees: d,
            scale: s,
        });
        let name = match (self.project.as_ref(), self.scene_path.as_ref()) {
            (Some(project), Some(scene)) => project.relative(scene),
            _ => None,
        };
        let Some(view) = name.and_then(|n| prefs.views.get(&n).copied()) else {
            return false;
        };
        self.set_camera(view.position, view.target);
        self.camera.ortho = view.ortho;
        true
    }

    /// The scene this person had open last in a project, to open on start.
    pub fn last_scene(project: &runity::Project) -> Option<PathBuf> {
        let text = std::fs::read_to_string(project.root().join(FILE)).ok()?;
        let prefs: Prefs = runity::ron::from_str(&text).ok()?;
        let path = project.root().join(&prefs.last_scene);
        (!prefs.last_scene.is_empty() && path.is_file()).then_some(path)
    }
}
