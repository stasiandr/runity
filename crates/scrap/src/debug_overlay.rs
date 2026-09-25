//! What a thing in the running game is doing, written over it: Unreal's
//! Gameplay Debugger, lite.
//!
//! On (the `debug` action, apostrophe in the template's `input.ron`, as in
//! Unreal), the entity nearest the middle of the view — what the crosshair
//! is on — gets its state beside it: name and ID, where it is, its body,
//! who owns it, its animator's state, and its saved components as the
//! editor's Inspector shows them in `game.*` fields ([`describe`] — the
//! same capture). Its collider is outlined over everything. Nothing is
//! drawn, and nothing costs, while it is off.

use glam::{Vec2, Vec3, Vec4};

use crate::components::Components;
use crate::gpu::Gpu;
use crate::render::{Camera, Frame, MeshHandle, Renderer};
use crate::ui::{Quad, TextRun, Ui};
use crate::world::{LineName, NetId, Owned, Physics, Replica, SceneId, Shape, WorldTransform};

/// How far from the middle of the view, as a share of its shorter side,
/// a thing can be and still be the one looked at.
const REACH: f32 = 0.25;

/// A component's value longer than this is cut: the overlay is a glance.
const LONGEST: usize = 72;

/// The overlay's switch, and the line mesh it draws colliders with.
#[derive(Debug, Default)]
pub struct DebugOverlay {
    on: bool,
    arm: Option<MeshHandle>,
}

/// Where an entity is in the world, as drawn.
fn position(world: &hecs::World, entity: hecs::Entity) -> Option<Vec3> {
    if let Ok(placed) = world.get::<&WorldTransform>(entity) {
        return Some(placed.0.w_axis.truncate());
    }
    world
        .get::<&crate::Transform>(entity)
        .ok()
        .map(|t| t.position)
}

/// The ID the editor knows it by: its scene line's, or the one it was
/// spawned under.
fn id_of(world: &hecs::World, entity: hecs::Entity) -> Option<crate::EntityId> {
    if let Ok(id) = world.get::<&SceneId>(entity) {
        return Some(id.0);
    }
    world.get::<&NetId>(entity).ok().map(|id| id.0)
}

/// The thing looked at: of those the editor can name, the one drawn
/// nearest the middle of a `size`-pixel view, within a quarter of it.
pub fn target(world: &hecs::World, camera: &Camera, size: Vec2) -> Option<hecs::Entity> {
    let middle = size * 0.5;
    let reach = size.x.min(size.y) * REACH;
    world
        .iter()
        .map(|e| e.entity())
        .filter(|&e| id_of(world, e).is_some() && crate::world::is_active(world, e))
        .filter_map(|e| {
            let at = position(world, e)?;
            if at.distance(camera.position) < 0.1 {
                return None;
            }
            let on_screen = camera.screen_point(at, size)?;
            let off = on_screen.distance(middle);
            (off <= reach).then_some((e, off))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)
}

/// An entity's state in lines: what the overlay writes beside it.
pub fn describe(world: &hecs::World, components: &Components, entity: hecs::Entity) -> Vec<String> {
    let mut lines = Vec::new();
    let name = world
        .get::<&LineName>(entity)
        .map(|n| n.0.clone())
        .unwrap_or_default();
    let id = id_of(world, entity)
        .map(|id| id.to_string())
        .unwrap_or_default();
    lines.push(format!("{name} {id}").trim().to_string());
    if let Some(at) = position(world, entity) {
        lines.push(format!("at ({:.2}, {:.2}, {:.2})", at.x, at.y, at.z));
    }
    if let Ok(body) = world.get::<&Physics>(entity) {
        lines.push(format!("body {:?}", body.0));
    }
    if world.get::<&Owned>(entity).is_ok() {
        lines.push("owned here".into());
    } else if world.get::<&Replica>(entity).is_ok() {
        lines.push("a replica: another player owns it".into());
    }
    #[cfg(feature = "animation")]
    {
        let state = crate::animgraph::animator_state(world, entity);
        if !state.is_empty() {
            lines.push(format!("animator {state}"));
        }
    }
    for (component, value) in components.write_saved(world, entity) {
        let mut value = value;
        if value.chars().count() > LONGEST {
            value = value.chars().take(LONGEST).collect::<String>() + "…";
        }
        lines.push(format!("{component}: {value}"));
    }
    lines
}

impl DebugOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self) {
        self.on = !self.on;
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// While on: the thing looked at, its state written beside it in `ui`
    /// and its collider outlined in `frame`. Call after the frame is built,
    /// with the camera it was built from; `size` is the screen's pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        world: &hecs::World,
        components: &Components,
        camera: &Camera,
        size: Vec2,
        ui: &mut Ui,
        frame: &mut Frame,
        gpu: &Gpu,
        renderer: &mut Renderer,
    ) {
        if !self.on {
            return;
        }
        let text = 15.0;
        let Some(entity) = target(world, camera, size) else {
            ui.text(TextRun::new(
                size.x * 0.5 + 12.0,
                size.y * 0.5,
                text,
                Vec4::new(1.0, 1.0, 1.0, 0.6),
                "nothing looked at",
            ));
            return;
        };
        let lines = describe(world, components, entity);
        let at = position(world, entity)
            .and_then(|p| camera.screen_point(p, size))
            .unwrap_or(size * 0.5);
        let row = text * 1.35;
        let wide = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
        let (x, y) = (at.x + 16.0, at.y - row);
        ui.quad(
            Quad::new(
                x - 6.0,
                y - 4.0,
                wide * text * 0.58 + 12.0,
                row * lines.len() as f32 + 8.0,
                Vec4::new(0.0, 0.0, 0.0, 0.6),
            )
            .rounded(4.0),
        );
        for (i, line) in lines.into_iter().enumerate() {
            let color = if i == 0 {
                Vec4::new(1.0, 0.85, 0.4, 1.0)
            } else {
                Vec4::ONE
            };
            ui.text(TextRun::new(x, y + row * i as f32, text, color, line));
        }
        // Its collider, over everything.
        let (Ok(shape), Ok(placed)) = (
            world.get::<&Shape>(entity),
            world.get::<&WorldTransform>(entity),
        ) else {
            return;
        };
        let body = world
            .get::<&Physics>(entity)
            .map(|b| b.0)
            .unwrap_or(crate::scene::Body::None);
        let arm = *self
            .arm
            .get_or_insert_with(|| renderer.upload_mesh_owned(gpu, &crate::builtin::cube(1.0)));
        frame.overlay_draws.extend(crate::gizmo::collider_draws(
            arm,
            shape.0,
            placed.0,
            0.02,
            crate::gizmo::collider_color(body),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Health(u32);

    fn camera() -> Camera {
        Camera {
            position: Vec3::new(0.0, 0.0, 10.0),
            target: Vec3::ZERO,
            ..Camera::default()
        }
    }

    #[test]
    fn the_thing_looked_at_is_described_as_the_inspector_has_it() {
        let mut components = Components::new();
        components.register_saved::<Health>("health");
        let mut world = hecs::World::new();
        let id = crate::EntityId::fresh();
        let near = world.spawn((
            SceneId(id),
            LineName("wolf".into()),
            WorldTransform(glam::Mat4::from_translation(Vec3::new(0.2, 0.0, 0.0))),
            Physics(crate::scene::Body::Dynamic),
            Health(7),
        ));
        world.spawn((
            SceneId(crate::EntityId::fresh()),
            WorldTransform(glam::Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0))),
        ));
        // Not the editor's to name: never the target.
        world.spawn((WorldTransform(glam::Mat4::IDENTITY),));
        let size = Vec2::new(800.0, 600.0);
        assert_eq!(target(&world, &camera(), size), Some(near));

        let lines = describe(&world, &components, near);
        assert_eq!(lines[0], format!("wolf {id}"));
        assert_eq!(lines[1], "at (0.20, 0.00, 0.00)");
        assert!(lines.contains(&"body Dynamic".to_string()), "{lines:?}");
        assert!(lines.contains(&"health: (7)".to_string()), "{lines:?}");

        // Looking away: nothing.
        let away = Camera {
            target: Vec3::new(0.0, 50.0, 10.0),
            ..camera()
        };
        assert_eq!(target(&world, &away, size), None);
    }
}
