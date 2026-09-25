//! Poly Shape: a floor plan drawn as points, pulled up into a solid; and
//! brushes: boxes, ramps and cylinders added and cut out in order.
//!
//! ProBuilder's New Poly Shape, as a source file rather than a mesh baked
//! into the scene: the outline lives in `assets/<name>.scrpoly` (see
//! [`scrap_import::poly`]), a line of text per shape, and the scene places
//! it like any model. Changing a point is changing the file — a diff
//! someone can read — and every scene that places it, and the running game,
//! get the new floor.
//!
//! Brushes are the same idea for Hammer's and TrenchBroom's blockout:
//! `assets/<name>.scrbrush` (see [`scrap_import::brush`]) is a line a
//! brush, added or cut out in order, and [`Session::carve`] turns grey
//! shapes already in the scene into one — the first cut by the others.

use scrap::glam::{EulerRot, Mat4, Vec3};
use scrap::id::EntityId;
use scrap_import::brush::{Brush, BrushSource, Op, Shape};
use scrap_import::poly::PolySource;

use crate::{EditError, EditResult, Session};

impl Session {
    /// A flat terrain `size` metres a side: writes `assets/<name>.scrterrain`,
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
        let file = project.assets().join(format!("{name}.scrterrain"));
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
            d.set_part(&scrap::Body::Static);
        })?;
        self.history.squash(2);
        self.select(Some(id))?;
        Ok(id)
    }

    /// Draw a solid from an outline (see [`PolySource`]: points around its
    /// origin, a height, lying or standing). Writes `assets/<name>.scrpoly`,
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
        let file = project.assets().join(format!("{name}.scrpoly"));
        if file.exists() || self.bounds_of(name).is_some() {
            return Err(EditError::Scene(format!(
                "there is already a model named `{name}`; pick another name, or change its points with set_poly"
            )));
        }
        // Refused before anything is written.
        scrap_import::poly::build(source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        std::fs::create_dir_all(project.assets())
            .and_then(|()| std::fs::write(&file, source.to_text()))
            .map_err(|e| EditError::Io(format!("{}: {e}", file.display())))?;
        self.import(&file)?;
        let id = self.add_entity(
            None,
            scrap::EntityDesc {
                name: name.to_string(),
                transform: scrap::scene::Transform {
                    position: at,
                    ..Default::default()
                },
                ..Default::default()
            }
            .with(scrap::scene::ModelRef(name.to_string().into()))
            .with(scrap::Body::Static)
            .with(scrap::scene::Collider::Model),
        )?;
        self.select(Some(id))?;
        Ok(id)
    }

    /// What a Poly Shape's file says: its outline and height.
    pub fn poly(&self, name: &str) -> EditResult<PolySource> {
        let project = self.project.as_ref().ok_or(EditError::NotInProject)?;
        let file = project.assets().join(format!("{name}.scrpoly"));
        let text = std::fs::read_to_string(&file)
            .map_err(|e| EditError::Io(format!("{}: {e}", file.display())))?;
        scrap::ron::from_str(&text)
            .map_err(|e| EditError::Scene(format!("{}: {e}", file.display())))
    }

    /// Give a Poly Shape a new outline or height: the file is rewritten and
    /// rebuilt, and everything placing it changes with it. An outline that
    /// cannot be a floor is refused and the file left as it was.
    pub fn set_poly(&mut self, name: &str, source: &PolySource) -> EditResult<()> {
        self.refuse_while_playing()?;
        let project = self.project.clone().ok_or(EditError::NotInProject)?;
        let file = project.assets().join(format!("{name}.scrpoly"));
        if !file.is_file() {
            return Err(EditError::Scene(format!(
                "no {name}.scrpoly in assets/ — make one with poly_shape"
            )));
        }
        scrap_import::poly::build(source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
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

/// A name a person might type, as an asset's: `Back Wall` → `back_wall`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "brush".into()
    } else {
        out
    }
}

fn snake_case(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A number as a person would have typed it: no float noise, no `-0.0`.
fn tidy(v: Vec3, per_unit: f32) -> Vec3 {
    // Divided rather than multiplied by a step: 45000 / 1000 is 45 exactly.
    let r = (v * per_unit).round() / per_unit;
    Vec3::new(r.x + 0.0, r.y + 0.0, r.z + 0.0)
}

impl Session {
    fn brush_file(&self, name: &str) -> EditResult<std::path::PathBuf> {
        let project = self.project.as_ref().ok_or(EditError::NotInProject)?;
        Ok(project.assets().join(format!("{name}.scrbrush")))
    }

    /// A solid made of brushes (see [`BrushSource`]: boxes, ramps,
    /// cylinders, stairs, added or cut out in order). Writes
    /// `assets/<name>.scrbrush`, imports it, and places it at `at` as a
    /// static body with a collider of its own shape — selected, one undo
    /// step.
    pub fn brush_shape(
        &mut self,
        name: &str,
        source: &BrushSource,
        at: Vec3,
    ) -> EditResult<EntityId> {
        self.refuse_while_playing()?;
        if !snake_case(name) {
            return Err(EditError::Scene(format!(
                "`{name}`: a brush solid's name is snake_case, like back_wall"
            )));
        }
        let file = self.brush_file(name)?;
        if file.exists() || self.bounds_of(name).is_some() {
            return Err(EditError::Scene(format!(
                "there is already a model named `{name}`; pick another name, or add to it with brush_add"
            )));
        }
        scrap_import::brush::build(source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        self.write_brushes(&file, &source.to_text())?;
        let id = self.add_entity(
            None,
            scrap::EntityDesc {
                name: name.to_string(),
                transform: scrap::scene::Transform {
                    position: at,
                    ..Default::default()
                },
                ..Default::default()
            }
            .with(scrap::scene::ModelRef(name.to_string().into()))
            .with(scrap::Body::Static)
            .with(scrap::scene::Collider::Model),
        )?;
        self.select(Some(id))?;
        Ok(id)
    }

    fn write_brushes(&mut self, file: &std::path::Path, text: &str) -> EditResult<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| EditError::Io(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(file, text)
            .map_err(|e| EditError::Io(format!("{}: {e}", file.display())))?;
        self.import(file)?;
        Ok(())
    }

    /// What a brush solid's file says: its brushes, in order.
    pub fn brushes(&self, name: &str) -> EditResult<BrushSource> {
        let file = self.brush_file(name)?;
        let text = std::fs::read_to_string(&file).map_err(|_| {
            EditError::Scene(format!(
                "no {name}.scrbrush in assets/ — make one with brush_shape or carve"
            ))
        })?;
        scrap::ron::from_str(&text)
            .map_err(|e| EditError::Scene(format!("{}: {e}", file.display())))
    }

    /// Give a brush solid a whole new list of brushes: the file is
    /// rewritten and rebuilt, and everything placing it changes. Brushes
    /// that leave nothing are refused and the file left as it was.
    pub fn set_brushes(&mut self, name: &str, source: &BrushSource) -> EditResult<()> {
        self.refuse_while_playing()?;
        self.brushes(name)?;
        scrap_import::brush::build(source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        let file = self.brush_file(name)?;
        self.write_brushes(&file, &source.to_text())?;
        self.respawn();
        Ok(())
    }

    /// One more brush at the end of a brush solid — added, or cut out of
    /// everything before it — in the solid's own space. Written as one
    /// line; the rest of the file stays as it was.
    pub fn add_brush(&mut self, name: &str, brush: &Brush) -> EditResult<()> {
        self.refuse_while_playing()?;
        let mut source = self.brushes(name)?;
        source.brushes.push(brush.clone());
        scrap_import::brush::build(&source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        let file = self.brush_file(name)?;
        let text = std::fs::read_to_string(&file)?;
        let text = crate::add_to_list(&text, "brushes", &brush.to_line());
        self.write_brushes(&file, &text)?;
        self.respawn();
        Ok(())
    }

    /// Cut `cutters` out of `target` — Hammer's carve. Each cutter, a grey
    /// `builtin:` cube, ramp, cylinder or stairs however it is placed,
    /// becomes a subtract brush and leaves the scene. A `builtin:` target
    /// becomes a new brush solid, `assets/<name>.scrbrush` (its own name
    /// in snake_case when `name` is not given), in its place: its id,
    /// name, material and place kept, its scale moved into the brush, a
    /// static body with a collider of its shape. A target that already is
    /// a brush solid gets the cuts added to its file. One undo step for
    /// the scene; the file stays, as a new asset does. Returns the solid's
    /// name.
    pub fn carve(
        &mut self,
        target: EntityId,
        cutters: &[EntityId],
        name: Option<&str>,
    ) -> EditResult<String> {
        self.refuse_while_playing()?;
        let project = self.project.clone().ok_or(EditError::NotInProject)?;
        if cutters.is_empty() {
            return Err(EditError::Scene(
                "carve needs something to cut with: select the solid first, then the shapes to cut out of it".into(),
            ));
        }
        let line = self
            .history
            .scene()
            .get(target)
            .ok_or(EditError::NoEntity(target))?
            .clone();
        let model = line
            .part::<scrap::scene::ModelRef>()
            .map(|m| m.0.as_str().to_string())
            .unwrap_or_default();
        let world = self
            .world_matrix(target)
            .ok_or(EditError::NoEntity(target))?;
        // The brushes' space: a new solid keeps the target's place and
        // turn, and its scale goes into its first brush; a brush solid's
        // space is its model's.
        let (asset, frame, first) = if let Some(shape) = Shape::of_builtin(&model) {
            let asset = match name {
                Some(n) if !snake_case(n) => {
                    return Err(EditError::Scene(format!(
                        "`{n}`: a brush solid's name is snake_case, like back_wall"
                    )))
                }
                Some(n) => {
                    if self.brush_file(n)?.exists() || self.bounds_of(n).is_some() {
                        return Err(EditError::Scene(format!(
                            "there is already a model named `{n}`; pick another name"
                        )));
                    }
                    n.to_string()
                }
                None => {
                    let base = snake(&line.name);
                    let mut n = base.clone();
                    let mut k = 2;
                    while self.brush_file(&n)?.exists() || self.bounds_of(&n).is_some() {
                        n = format!("{base}_{k}");
                        k += 1;
                    }
                    n
                }
            };
            let scale = line.transform.scale;
            if scale.min_element() <= 0.0 {
                return Err(EditError::Scene(format!(
                    "`{}` is mirrored or flat (scale {:?}): give it a scale above zero first",
                    line.name,
                    scale.to_array()
                )));
            }
            let first = Brush {
                op: Op::Add,
                shape,
                position: Vec3::ZERO,
                rotation_deg: Vec3::ZERO,
                scale: tidy(scale, 1e4),
                sides: scrap_import::brush::CYLINDER_SIDES,
            };
            (asset, world * Mat4::from_scale(scale.recip()), Some(first))
        } else if !model.is_empty() && self.brush_file(&model)?.is_file() {
            (model.clone(), world, None)
        } else {
            return Err(EditError::Scene(format!(
                "`{}` is {}: carve cuts a builtin cube, ramp, cylinder or stairs, or a brush solid",
                line.name,
                if model.is_empty() {
                    "not a model".to_string()
                } else {
                    format!("`{model}`")
                }
            )));
        };
        let into = frame.inverse();
        let mut cuts = Vec::with_capacity(cutters.len());
        for &cutter in cutters {
            let desc = self
                .history
                .scene()
                .get(cutter)
                .ok_or(EditError::NoEntity(cutter))?;
            if cutter == target {
                return Err(EditError::Scene(format!(
                    "`{}` cannot cut itself",
                    desc.name
                )));
            }
            if desc.flatten().iter().any(|(d, _)| d.id == target) {
                return Err(EditError::Scene(format!(
                    "`{}` holds `{}` as a child: taking it away would take the solid too",
                    desc.name, line.name
                )));
            }
            let cutter_model = desc
                .part::<scrap::scene::ModelRef>()
                .map(|m| m.0.as_str().to_string())
                .unwrap_or_default();
            let shape = Shape::of_builtin(&cutter_model).ok_or_else(|| {
                EditError::Scene(format!(
                    "`{}` cuts only as a builtin cube, ramp, cylinder or stairs, not {}",
                    desc.name,
                    if cutter_model.is_empty() {
                        "nothing".to_string()
                    } else {
                        format!("`{cutter_model}`")
                    }
                ))
            })?;
            let name = desc.name.clone();
            let at = into
                * self
                    .world_matrix(cutter)
                    .ok_or(EditError::NoEntity(cutter))?;
            let (scale, turn, position) = at.to_scale_rotation_translation();
            let again = Mat4::from_scale_rotation_translation(scale, turn, position);
            if !again.abs_diff_eq(at, 1e3) || scale.min_element() <= 0.0 {
                return Err(EditError::Scene(format!(
                    "`{name}` is skewed against `{}` (a turned shape inside something stretched unevenly): a brush is a shape moved, turned and scaled",
                    line.name
                )));
            }
            let (y, x, z) = turn.to_euler(EulerRot::YXZ);
            cuts.push(Brush {
                op: Op::Subtract,
                shape,
                position: tidy(position, 1e4),
                rotation_deg: tidy(Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI), 1e3),
                scale: tidy(scale, 1e4),
                sides: scrap_import::brush::CYLINDER_SIDES,
            });
        }

        // Refused before anything is written.
        let file = project.assets().join(format!("{asset}.scrbrush"));
        let (mut source, text) = match &first {
            Some(first) => (
                BrushSource {
                    brushes: vec![first.clone()],
                },
                None,
            ),
            None => (self.brushes(&asset)?, Some(std::fs::read_to_string(&file)?)),
        };
        source.brushes.extend(cuts.iter().cloned());
        scrap_import::brush::build(&source).map_err(|e| EditError::Scene(format!("{e:#}")))?;
        let text = match text {
            None => source.to_text(),
            Some(text) => cuts.iter().fold(text, |text, cut| {
                crate::add_to_list(&text, "brushes", &cut.to_line())
            }),
        };
        self.write_brushes(&file, &text)?;

        let scene = self.history.edit();
        if let Some(desc) = scene.get_mut(target) {
            if first.is_some() {
                desc.set_part(&scrap::scene::ModelRef(asset.clone().into()));
                desc.transform.scale = Vec3::ONE;
            }
            if desc.part::<scrap::Body>().is_none() {
                desc.set_part(&scrap::Body::Static);
            }
            desc.set_part(&scrap::scene::Collider::Model);
        }
        for &cutter in cutters {
            scrap::edit::remove(scene, cutter);
        }
        self.respawn();
        self.after_structural_change();
        self.select(Some(target))?;
        Ok(asset)
    }
}
