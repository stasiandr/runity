//! The player in the editor (docs/player.md): its size and jump drawn in
//! the Scene view, and Play from Here.
//!
//! The engine does not know how the game's player walks — that is the
//! game's code. It knows what `scrap.ron` says about the player
//! ([`PlayerMetrics`]) and which lines the scene marks `player_start`; the
//! reference is drawn from the first, and Play from Here hands the game a
//! place for the second.

use scrap::glam::Vec3;
use scrap::player::{PlayerMetrics, Start};

use crate::console::Level;
use crate::{EditError, EditResult, Session};

impl Session {
    /// The project's player, as `scrap.ron` says it now — read from the
    /// file each time, so a change saved in Project Settings shows at
    /// once. The default player outside a project, or when the file does
    /// not read (`scrap check` says why).
    pub fn player_metrics(&self) -> PlayerMetrics {
        let Some(project) = self.project.as_ref() else {
            return PlayerMetrics::default();
        };
        std::fs::read_to_string(project.root().join("scrap.ron"))
            .ok()
            .and_then(|text| scrap::ron::from_str::<scrap::project::Manifest>(&text).ok())
            .map(|m| m.game.player)
            .unwrap_or_default()
    }

    /// The walker navigation shows and paths are found for: the project's
    /// player.
    pub fn walker(&self) -> scrap::navigation::NavSettings {
        scrap::navigation::NavSettings::for_player(&self.player_metrics())
    }

    /// Draw the player — a capsule its height and width, and the arc of a
    /// running jump — where the pointer is over the view, or where Play
    /// from Here would start when it is not (what `render` shows the
    /// agent). A view setting, not an edit.
    pub fn set_show_player(&mut self, show: bool) {
        self.show_player = show;
    }

    pub fn show_player(&self) -> bool {
        self.show_player
    }

    /// Where the reference stands: on what the pointer is over, looking
    /// the view's way; else where Play from Here would start.
    pub fn player_reference(&mut self) -> Option<Start> {
        let facing = self.level_facing();
        if let Some((x, y)) = self.hover_at {
            let (from, direction) = self.ray(x, y);
            if let Some((point, _)) = self.cast(from, direction) {
                return Some(Start::looking(point, facing));
            }
        }
        self.start_here().ok()
    }

    /// Where Play from Here puts the player: on the ground the middle of
    /// the view looks at, or — when it looks at the sky, a wall or a slope
    /// too steep to stand on — on the ground under the camera; facing the
    /// way the view looks. Refused when there is neither.
    pub fn start_here(&mut self) -> EditResult<Start> {
        let facing = self.level_facing();
        let standable = (self.player_metrics().slope.to_radians()).cos();
        let (w, h) = self.size();
        let (from, direction) = self.ray(w / 2, h / 2);
        let ahead = self.cast(from, direction);
        let below = self.cast(self.camera.position, Vec3::NEG_Y);
        [ahead, below]
            .into_iter()
            .flatten()
            .find(|(_, normal)| normal.y >= standable)
            .map(|(point, _)| Start::looking(point, facing))
            .ok_or_else(|| {
                EditError::Scene(
                    "nothing to stand on here: look at the ground, or fly over it".into(),
                )
            })
    }

    /// Play from Here: [`Session::start_game`] with the game told to start
    /// its player where [`Session::start_here`] says (`SCRAP_START`). The
    /// game moves the lines its scene marks `player_start` there; a scene
    /// with none is said in the Console, and the game starts as it would.
    pub fn start_game_from_here(&mut self) -> EditResult<Start> {
        let start = self.start_here()?;
        if scrap::player::player_starts(self.history.scene()).is_empty() {
            self.say(
                Level::Warning,
                "nothing in the scene is marked player_start: the game is told where to \
                 start, and starts where its own code puts the player",
            );
        }
        self.start_game_at(Some(start))?;
        let p = start.position;
        self.say(
            Level::Info,
            format!(
                "playing from ({:.2}, {:.2}, {:.2}), facing {:.0}°",
                p.x, p.y, p.z, start.yaw_deg
            ),
        );
        Ok(start)
    }

    /// What the reference draws: the capsule, and the arc its feet take in
    /// a running jump.
    pub(crate) fn player_draws(&mut self) -> Vec<scrap::Draw> {
        let Some(start) = self.player_reference() else {
            return Vec::new();
        };
        let metrics = self.player_metrics();
        let arm = self.gizmo_arm_mesh();
        let thickness = self.line_width(start.position, 0.0015);
        let color = scrap::gizmo::player_color();
        let mut draws = scrap::gizmo::capsule_draws(
            arm,
            start.position,
            metrics.height,
            metrics.radius,
            start.facing(),
            thickness,
            color,
        );
        let arc = metrics.jump_arc(start.position, start.facing(), 24);
        draws.extend(scrap::gizmo::polyline_draws(arm, &arc, thickness, color));
        draws
    }

    /// The way the view looks, level; along −z when it looks straight
    /// down or up (a top view).
    fn level_facing(&self) -> Vec3 {
        let look = self.camera.target - self.camera.position;
        let level = Vec3::new(look.x, 0.0, look.z);
        if level.length_squared() > 1e-6 {
            return level.normalize();
        }
        let up = Vec3::new(self.camera.up.x, 0.0, self.camera.up.z);
        up.try_normalize().unwrap_or(Vec3::NEG_Z)
    }

    /// The first solid a ray hits, and the surface's normal there.
    fn cast(&mut self, from: Vec3, direction: Vec3) -> Option<(Vec3, Vec3)> {
        let revision = self.history.revision();
        if self.solids.as_ref().is_none_or(|(r, _)| *r != revision) {
            self.solids = Some((revision, self.solid_without(&[])));
        }
        let far = self.camera.far;
        let (_, world) = self.solids.as_ref()?;
        world
            .cast_ray_with_normal(from, direction, far, false)
            .map(|(point, normal, _)| (point, normal))
    }
}
