//! Looking along an axis: a level's plan from above, a wall face on.
//!
//! Unity's Scene view gizmo — click a cone and the view turns to look
//! along that axis, orthographic, with no vanishing point, so the plan of a
//! greybox level is a plan and a row of posts is a row. What a window's
//! axis widget calls, and what `render` takes as `view` for an agent. A
//! view setting like the camera: nothing in the scene.

use runity::glam::Vec3;

use crate::Session;

/// How far back an orthographic view stands from what it looks at: far
/// enough that a tower between the two is not cut off, and nothing about
/// the picture depends on it.
pub(crate) const ORTHO_STAND: f32 = 200.0;

/// Which axes the handles are along: Unity's Global / Local toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Space {
    /// The world's: a move is north, east or up.
    #[default]
    Global,
    /// The entity's own: a turned wall slides along its own length.
    Local,
}

/// Where the handles sit: Unity's Pivot / Center toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pivot {
    /// On the gizmo's entity; the rest turn and stretch about their own.
    #[default]
    Pivot,
    /// In the middle of everything selected, which turns and stretches
    /// about it as one.
    Center,
}

/// Which side of the world to look from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// From above, looking down: the plan. −z is up the image, +x right.
    Top,
    Bottom,
    /// From +z, looking toward −z, as a new scene's view does.
    Front,
    Back,
    /// From +x.
    Right,
    Left,
}

impl Side {
    pub const ALL: [Side; 6] = [
        Side::Top,
        Side::Bottom,
        Side::Front,
        Side::Back,
        Side::Right,
        Side::Left,
    ];

    /// Its name, as `render`'s `view` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Side::Top => "top",
            Side::Bottom => "bottom",
            Side::Front => "front",
            Side::Back => "back",
            Side::Right => "right",
            Side::Left => "left",
        }
    }

    pub fn from_name(name: &str) -> Option<Side> {
        Side::ALL.into_iter().find(|s| s.name() == name)
    }

    /// From the target toward where the camera stands.
    fn towards_eye(self) -> Vec3 {
        match self {
            Side::Top => Vec3::Y,
            Side::Bottom => Vec3::NEG_Y,
            Side::Front => Vec3::Z,
            Side::Back => Vec3::NEG_Z,
            Side::Right => Vec3::X,
            Side::Left => Vec3::NEG_X,
        }
    }
}

/// The up a view along `forward` keeps: the world's, unless it looks
/// straight up or down, where −z (or +z from below) keeps +x to the right.
pub(crate) fn up_for(forward: Vec3) -> Vec3 {
    let forward = forward.normalize_or_zero();
    if forward.y < -0.999 {
        Vec3::NEG_Z
    } else if forward.y > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    }
}

impl Session {
    /// Look at what the view looks at from one side, orthographic, showing
    /// as much as it did.
    pub fn look_from(&mut self, side: Side) {
        let half = self.visible_half_height();
        self.camera.position = self.camera.target + side.towards_eye() * ORTHO_STAND;
        self.camera.up = up_for(-side.towards_eye());
        self.camera.ortho = Some(half);
    }

    /// Orthographic or perspective, keeping how much is seen — Unity's
    /// Persp/Iso toggle.
    pub fn set_orthographic(&mut self, ortho: bool) {
        if ortho == self.is_orthographic() {
            return;
        }
        let back = (self.camera.position - self.camera.target).normalize_or_zero();
        if ortho {
            self.camera.ortho = Some(self.visible_half_height());
            self.camera.position = self.camera.target + back * ORTHO_STAND;
        } else {
            // Stand where the perspective view shows the same.
            let half = self.camera.ortho.take().unwrap_or(1.0);
            let tan = (self.camera.fov_y_degrees.to_radians() * 0.5)
                .tan()
                .max(1e-3);
            self.camera.position = self.camera.target + back * (half / tan);
        }
    }

    /// Turn the view's head where it stands, in degrees: right drag.
    /// Leaves an axis view for perspective, as Unity does.
    pub fn look(&mut self, yaw: f32, pitch: f32) {
        self.set_orthographic(false);
        let offset = self.camera.target - self.camera.position;
        let distance = offset.length().max(1e-3);
        let current_pitch = (offset.y / distance).clamp(-1.0, 1.0).asin();
        let current_yaw = offset.x.atan2(offset.z);
        let pitch = (current_pitch + pitch.to_radians()).clamp(-1.5, 1.5);
        let yaw = current_yaw + yaw.to_radians();
        let direction = Vec3::new(
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        );
        self.camera.target = self.camera.position + direction * distance;
        self.camera.up = Vec3::Y;
    }

    /// Move the view, what it looks at with it, in metres along where it
    /// looks, to its right and up the world.
    pub fn fly(&mut self, forward: f32, right: f32, up: f32) {
        let ahead = (self.camera.target - self.camera.position).normalize_or_zero();
        let side = ahead.cross(self.camera.up).normalize_or_zero();
        let step = ahead * forward + side * right + Vec3::Y * up;
        self.camera.position += step;
        self.camera.target += step;
    }

    /// Metres a second a flythrough goes before shift.
    pub fn fly_speed(&self) -> f32 {
        self.fly_speed
    }

    /// Handles along the world's axes or the selected entity's own.
    pub fn set_space(&mut self, space: Space) {
        self.space = space;
    }

    pub fn space(&self) -> Space {
        self.space
    }

    pub fn set_pivot(&mut self, pivot: Pivot) {
        self.pivot = pivot;
    }

    pub fn pivot(&self) -> Pivot {
        self.pivot
    }

    /// Which way the handles point now: the selected entity's turn in the
    /// world when Local — and always for scale and the rect, which stretch
    /// the entity's own axes whichever way the toggle is, as in Unity.
    pub(crate) fn handle_orientation(&self) -> runity::glam::Quat {
        use runity::gizmo::Tool;
        let local = self.space == Space::Local || matches!(self.tool, Tool::Scale | Tool::Rect);
        match (local, self.selected) {
            (true, Some(id)) => self
                .placed(id)
                .map(|(_, m)| m.to_scale_rotation_translation().1)
                .unwrap_or_default(),
            _ => runity::glam::Quat::IDENTITY,
        }
    }

    /// Move to View: the selection to where the view looks, as one undo
    /// step — Unity's Ctrl Alt F. The gizmo's entity lands on the view's
    /// pivot; the rest keep their places relative to it.
    pub fn move_to_view(&mut self) -> crate::EditResult<bool> {
        let Some(id) = self.selected else {
            return Ok(false);
        };
        let Some(from) = self.world_position(id) else {
            return Ok(false);
        };
        self.translate_selection_world(self.camera.target - from)?;
        Ok(true)
    }

    /// Move every selected root by `offset` in the world, one undo step.
    fn translate_selection_world(&mut self, offset: Vec3) -> crate::EditResult<()> {
        self.refuse_while_playing()?;
        let moves: Vec<(runity::EntityId, Vec3)> = self
            .selection_roots()
            .into_iter()
            .map(|r| (r, self.parent_matrix(r).inverse().transform_vector3(offset)))
            .collect();
        let scene = self.history.edit();
        for (id, local) in moves {
            if let Some(desc) = scene.get_mut(id) {
                desc.transform.position += local;
            }
        }
        self.respawn();
        Ok(())
    }

    /// Align with View: the selected entity stands where the view does and
    /// looks where it looks (+z forward), one undo step — Ctrl Shift F, how
    /// a game camera is placed: fly to the shot, then this.
    pub fn align_with_view(&mut self) -> crate::EditResult<bool> {
        let Some(id) = self.selected else {
            return Ok(false);
        };
        let forward = (self.camera.target - self.camera.position).normalize_or_zero();
        if forward == Vec3::ZERO {
            return Ok(false);
        }
        let up = self.camera.up;
        let right = up.cross(forward).normalize_or_zero();
        let up = forward.cross(right);
        let turn =
            runity::glam::Quat::from_mat3(&runity::glam::Mat3::from_cols(right, up, forward));
        let world = runity::glam::Mat4::from_rotation_translation(turn, self.camera.position);
        let parent = self.parent_matrix(id);
        let (_, rotation, position) = (parent.inverse() * world).to_scale_rotation_translation();
        let Some(mut transform) = self.transform(id) else {
            return Ok(false);
        };
        transform.position = position;
        transform.set_rotation(rotation);
        self.set_transform(id, transform)?;
        Ok(true)
    }

    /// Look through another camera exactly — the game's, lens and all.
    pub fn look_through(&mut self, camera: runity::Camera) {
        self.camera = camera;
    }

    pub fn is_orthographic(&self) -> bool {
        self.camera.ortho.is_some()
    }

    /// Metres from the middle of the view to its top edge, at the target.
    fn visible_half_height(&self) -> f32 {
        self.camera.ortho.unwrap_or_else(|| {
            let distance = (self.camera.position - self.camera.target).length();
            distance * (self.camera.fov_y_degrees.to_radians() * 0.5).tan()
        })
    }
}
