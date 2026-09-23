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
    /// A flat terrain `size` metres a side: writes `assets/<name>.rterrain`,
    /// imports it and places it at the origin, selected — ready for
    /// [`Session::sculpt`]. One undo step (the file stays, as a new asset
    /// does).
    pub fn new_terrain(&mut self, name: &str, size: f32) -> EditResult<EntityId> {
        self.refuse_while_playing()?;
        let project = self.project.clone().ok_or(EditError::NotInProject)?;
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(EditError::Scene(format!(
                "`{name}`: a terrain's name is snake_case, like north_hills"
            )));
        }
        let file = project.assets().join(format!("{name}.rterrain"));
        if file.exists() || self.bounds_of(name).is_some() {
            return Err(EditError::Scene(format!(
                "there is already an asset called `{name}`"
            )));
        }
        let resolution = ((size * 2.0).round() as u32 + 1).clamp(17, 257);
        std::fs::create_dir_all(project.assets())?;
        std::fs::write(
            &file,
            format!(
                "// Flat ground, to be shaped with strokes below.\n(size: ({size:.1}, {size:.1}), resolution: {resolution}, height: 0.0)\n"
            ),
        )?;
        self.reload_assets();
        if self.bounds_of(name).is_none() {
            return Err(EditError::Import(format!(
                "{} did not import",
                file.display()
            )));
        }
        let id = self.add(None, name)?;
        self.update(id, |d| {
            d.body = runity::Body::Static;
        })?;
        self.history.squash(2);
        self.select(Some(id))?;
        Ok(id)
    }

    /// Draw a solid from an outline (see [`PolySource`]: points around its
    /// origin, a height, lying or standing). Writes `assets/<name>.rpoly`,
    /// imports it, and places it at `at` as a static body with a collider
    /// of its own shape — selected, one undo step.
    pub fn poly_shape(
        &mut self,
        name: &str,
        source: &PolySource,
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
        // Refused before anything is written.
        runity_import::poly::build(source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
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

    /// Push one wall of a Poly Shape out by `metres`, or pull it in: both
    /// ends of edge `edge` (from point `edge` to the next) move along the
    /// wall's outward normal, so the room grows on that side and the walls
    /// beside it follow — `push_face` for a shape a box cannot be.
    pub fn push_poly_edge(&mut self, name: &str, edge: usize, metres: f32) -> EditResult<()> {
        let mut source = self.poly(name)?;
        let n = source.points.len();
        if edge >= n {
            return Err(EditError::Scene(format!(
                "{name} has {n} walls, 0 to {}; not {edge}",
                n.saturating_sub(1)
            )));
        }
        if !metres.is_finite() {
            return Err(EditError::Scene(format!(
                "{metres} metres is not a distance"
            )));
        }
        let (a, b) = (source.points[edge], source.points[(edge + 1) % n]);
        let along = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if along < 1e-6 {
            return Err(EditError::Scene(format!(
                "wall {edge} of {name} has no length to push"
            )));
        }
        // Outward, whichever way round the outline was drawn: to the right
        // of a counter-clockwise walk seen from above, the left otherwise.
        let doubled: f32 = (0..n)
            .map(|i| {
                let (p, q) = (source.points[i], source.points[(i + 1) % n]);
                q.0 * p.1 - p.0 * q.1
            })
            .sum();
        let turn = if doubled >= 0.0 { 1.0 } else { -1.0 };
        let out = (
            -(b.1 - a.1) / along * turn * metres,
            (b.0 - a.0) / along * turn * metres,
        );
        for i in [edge, (edge + 1) % n] {
            source.points[i].0 += out.0;
            source.points[i].1 += out.1;
        }
        self.set_poly(name, &source)
    }
}
