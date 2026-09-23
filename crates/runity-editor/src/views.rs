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
