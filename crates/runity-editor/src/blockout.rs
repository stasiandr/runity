//! Poly Shape: a floor plan drawn as points, pulled up into a solid.
//!
//! ProBuilder's New Poly Shape, as a source file rather than a mesh baked
//! into the scene: the outline lives in `assets/<name>.rpoly` (see
//! [`runity_import::poly`]), a line of text per shape, and the scene places
//! it like any model. Changing a point is changing the file — a diff
//! someone can read — and every scene that places it, and the running game,
//! get the new floor.

use runity::glam::Vec3;
use runity::id::EntityId;
use runity_import::poly::PolySource;

use crate::{EditError, EditResult, Session};

impl Session {
    /// Draw a solid from an outline: `points` are x and z in metres around
    /// its origin, `height` how far up it goes. Writes
    /// `assets/<name>.rpoly`, imports it, and places it at `at` as a static
    /// body with a collider of its own shape — selected, one undo step.
    pub fn poly_shape(
        &mut self,
        name: &str,
        points: &[(f32, f32)],
        height: f32,
        at: Vec3,
    ) -> EditResult<EntityId> {
        self.refuse_while_playing()?;
        let project = self.project.clone().ok_or(EditError::NotInProject)?;
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(EditError::Scene(format!(
                "`{name}`: a shape's name is snake_case, like hall_floor"
            )));
        }
        let file = project.assets().join(format!("{name}.rpoly"));
        if file.exists() || self.bounds_of(name).is_some() {
            return Err(EditError::Scene(format!(
                "there is already a model named `{name}`; pick another name, or change its points with set_poly"
            )));
        }
        let source = PolySource {
            points: points.to_vec(),
            height,
        };
        // Refused before anything is written.
        runity_import::poly::build(&source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        std::fs::create_dir_all(project.assets())
            .and_then(|()| std::fs::write(&file, source.to_text()))
            .map_err(|e| EditError::Io(format!("{}: {e}", file.display())))?;
        self.import(&file)?;
        let id = self.add_entity(
            None,
            runity::EntityDesc {
                name: name.to_string(),
                model: name.to_string(),
                body: runity::Body::Static,
                collider: runity::scene::Collider::Model,
                transform: runity::scene::Transform {
                    position: at,
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        self.select(Some(id))?;
        Ok(id)
    }

    /// What a Poly Shape's file says: its outline and height.
    pub fn poly(&self, name: &str) -> EditResult<PolySource> {
        let project = self.project.as_ref().ok_or(EditError::NotInProject)?;
        let file = project.assets().join(format!("{name}.rpoly"));
        let text = std::fs::read_to_string(&file)
            .map_err(|e| EditError::Io(format!("{}: {e}", file.display())))?;
        runity::ron::from_str(&text)
            .map_err(|e| EditError::Scene(format!("{}: {e}", file.display())))
    }

    /// Give a Poly Shape a new outline or height: the file is rewritten and
    /// rebuilt, and everything placing it changes with it. An outline that
    /// cannot be a floor is refused and the file left as it was.
    pub fn set_poly(&mut self, name: &str, source: &PolySource) -> EditResult<()> {
        self.refuse_while_playing()?;
        let project = self.project.clone().ok_or(EditError::NotInProject)?;
        let file = project.assets().join(format!("{name}.rpoly"));
        if !file.is_file() {
            return Err(EditError::Scene(format!(
                "no {name}.rpoly in assets/ — make one with poly_shape"
            )));
        }
        runity_import::poly::build(source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        std::fs::write(&file, source.to_text())
            .map_err(|e| EditError::Io(format!("{}: {e}", file.display())))?;
        self.import(&file)?;
        self.respawn();
        Ok(())
    }
}
