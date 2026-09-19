//! Parser for the valley scene format: a line-based, space-separated text
//! file describing what stands where in the "first ten minutes" scene. See
//! card #33 part 3 for the format's rationale — no JSON, no TOML, a file
//! meant to be hand-edited and diffed in git.
//!
//! ```text
//! model log     models/log.obj      textures/nature.png
//! box   shelter  2.4 1.8 2.6
//! place fire   0.0  0.0  0.0     0      1.0
//! scatter spruce  seed 7  count 240  ring 18 45
//! settler Кора   pose standing  at  1.2 0.0 -2.8   height 1.68
//! ```
//!
//! `#` starts a comment that runs to the end of the line; blank lines are
//! ignored. [`parse`] never panics — every malformed line becomes a
//! [`SceneError`] carrying the 1-based line number and a message.

use runity_math::Vec3;
use std::path::PathBuf;
use std::str::{FromStr, SplitWhitespace};

/// One parsed `model` line: a named asset built from an OBJ mesh and a
/// texture atlas, referenced by name from `place` and `scatter` lines.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDef {
    pub name: String,
    pub obj_path: PathBuf,
    pub atlas_path: PathBuf,
}

/// One parsed `box` line: a placeholder cuboid standing in for an asset
/// that doesn't exist yet, sized in meters. `role` both names it (for
/// `place` lines to refer to) and describes what it stands in for.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxDef {
    pub role: String,
    pub size: Vec3,
}

/// One parsed `place` line: a single instance of a named `model` or `box`
/// at a position, with a yaw rotation in degrees and a uniform scale.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceDef {
    pub name: String,
    pub position: Vec3,
    pub rotation_deg: f32,
    pub scale: f32,
}

/// One parsed `scatter` line: many instances of a named `model`, seeded
/// deterministically and scattered in a ring around the world origin.
#[derive(Debug, Clone, PartialEq)]
pub struct ScatterDef {
    pub name: String,
    pub seed: u64,
    pub count: u32,
    pub ring_inner: f32,
    pub ring_outer: f32,
}

/// One parsed `settler` line: a named settler with a pose, a position, and
/// a height in meters.
#[derive(Debug, Clone, PartialEq)]
pub struct SettlerDef {
    pub name: String,
    pub pose: String,
    pub position: Vec3,
    pub height: f32,
}

/// One line of a parsed scene, tagged by the row kind that produced it.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneLine {
    Model(ModelDef),
    Box(BoxDef),
    Place(PlaceDef),
    Scatter(ScatterDef),
    Settler(SettlerDef),
}

/// A parsed scene file: every non-comment, non-blank line, in file order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub lines: Vec<SceneLine>,
}

/// A scene file line that failed to parse: which line, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "scene parse error on line {}: {}",
            self.line, self.message
        )
    }
}

impl std::error::Error for SceneError {}

/// Parse a whole scene file. Never panics: any malformed line is reported
/// as an [`SceneError`] with its 1-based line number instead.
pub fn parse(text: &str) -> Result<Scene, SceneError> {
    let mut scene = Scene::default();
    for (line_no, raw) in text.lines().enumerate() {
        let trimmed = raw.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut fields = Fields {
            line: line_no + 1,
            inner: trimmed.split_whitespace(),
        };
        let tag = fields.token("a line kind (model/box/place/scatter/settler)")?;
        let item = match tag {
            "model" => SceneLine::Model(parse_model(&mut fields)?),
            "box" => SceneLine::Box(parse_box(&mut fields)?),
            "place" => SceneLine::Place(parse_place(&mut fields)?),
            "scatter" => SceneLine::Scatter(parse_scatter(&mut fields)?),
            "settler" => SceneLine::Settler(parse_settler(&mut fields)?),
            other => return Err(fields.err(format!("unknown line kind `{other}`"))),
        };
        fields.finish()?;
        scene.lines.push(item);
    }
    Ok(scene)
}

fn parse_model(fields: &mut Fields) -> Result<ModelDef, SceneError> {
    let name = fields.text("model name")?;
    let obj_path = PathBuf::from(fields.text("model obj path")?);
    let atlas_path = PathBuf::from(fields.text("model atlas path")?);
    Ok(ModelDef {
        name,
        obj_path,
        atlas_path,
    })
}

fn parse_box(fields: &mut Fields) -> Result<BoxDef, SceneError> {
    let role = fields.text("box role")?;
    let size = fields.vec3("box size")?;
    Ok(BoxDef { role, size })
}

fn parse_place(fields: &mut Fields) -> Result<PlaceDef, SceneError> {
    let name = fields.text("place name")?;
    let position = fields.vec3("place position")?;
    let rotation_deg = fields.number("place rotation")?;
    let scale = fields.number("place scale")?;
    Ok(PlaceDef {
        name,
        position,
        rotation_deg,
        scale,
    })
}

fn parse_scatter(fields: &mut Fields) -> Result<ScatterDef, SceneError> {
    let name = fields.text("scatter name")?;
    fields.keyword("seed")?;
    let seed = fields.number("scatter seed")?;
    fields.keyword("count")?;
    let count = fields.number("scatter count")?;
    fields.keyword("ring")?;
    let ring_inner = fields.number("scatter ring inner radius")?;
    let ring_outer = fields.number("scatter ring outer radius")?;
    Ok(ScatterDef {
        name,
        seed,
        count,
        ring_inner,
        ring_outer,
    })
}

fn parse_settler(fields: &mut Fields) -> Result<SettlerDef, SceneError> {
    let name = fields.text("settler name")?;
    fields.keyword("pose")?;
    let pose = fields.text("settler pose")?;
    fields.keyword("at")?;
    let position = fields.vec3("settler position")?;
    fields.keyword("height")?;
    let height = fields.number("settler height")?;
    Ok(SettlerDef {
        name,
        pose,
        position,
        height,
    })
}

/// The remaining whitespace-separated tokens of one scene line, with a
/// shared line number for error reporting.
struct Fields<'a> {
    line: usize,
    inner: SplitWhitespace<'a>,
}

impl<'a> Fields<'a> {
    fn err(&self, message: impl Into<String>) -> SceneError {
        SceneError {
            line: self.line,
            message: message.into(),
        }
    }

    fn token(&mut self, what: &str) -> Result<&'a str, SceneError> {
        self.inner
            .next()
            .ok_or_else(|| self.err(format!("missing {what}")))
    }

    fn text(&mut self, what: &str) -> Result<String, SceneError> {
        Ok(self.token(what)?.to_string())
    }

    fn keyword(&mut self, kw: &str) -> Result<(), SceneError> {
        let found = self.token(&format!("`{kw}`"))?;
        if found == kw {
            Ok(())
        } else {
            Err(self.err(format!("expected `{kw}`, found `{found}`")))
        }
    }

    fn number<T: FromStr>(&mut self, what: &str) -> Result<T, SceneError> {
        let token = self.token(what)?;
        token
            .parse()
            .map_err(|_| self.err(format!("{what} `{token}` is not a number")))
    }

    fn vec3(&mut self, what: &str) -> Result<Vec3, SceneError> {
        let x = self.number(&format!("{what} x"))?;
        let y = self.number(&format!("{what} y"))?;
        let z = self.number(&format!("{what} z"))?;
        Ok(Vec3::new(x, y, z))
    }

    fn finish(&mut self) -> Result<(), SceneError> {
        match self.inner.next() {
            Some(extra) => Err(self.err(format!("unexpected extra field `{extra}`"))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact example from card #33 part 3, plus the `settler` line from
    /// part 5 of the same design — the format's own worked example.
    const CARD_EXAMPLE: &str = "\
# вещь        модель              атлас
model log     models/log.obj      textures/nature.png
model spruce  models/spruce.obj   textures/nature.png

# заглушка, пока нужного ассета нет: размер в метрах и роль
box   shelter  2.4 1.8 2.6

# что       где (x y z)      поворот  масштаб
place fire   0.0  0.0  0.0     0      1.0
place log   -2.4  0.0  1.1    37      1.0
scatter spruce  seed 7  count 240  ring 18 45

settler Кора   pose standing  at  1.2 0.0 -2.8   height 1.68
";

    #[test]
    fn card_example_parses_into_the_expected_structure() {
        let scene = parse(CARD_EXAMPLE).expect("the card's own example must parse");
        assert_eq!(scene.lines.len(), 7);
        assert_eq!(
            scene.lines[0],
            SceneLine::Model(ModelDef {
                name: "log".to_string(),
                obj_path: PathBuf::from("models/log.obj"),
                atlas_path: PathBuf::from("textures/nature.png"),
            })
        );
        assert_eq!(
            scene.lines[2],
            SceneLine::Box(BoxDef {
                role: "shelter".to_string(),
                size: Vec3::new(2.4, 1.8, 2.6),
            })
        );
        assert_eq!(
            scene.lines[3],
            SceneLine::Place(PlaceDef {
                name: "fire".to_string(),
                position: Vec3::new(0.0, 0.0, 0.0),
                rotation_deg: 0.0,
                scale: 1.0,
            })
        );
        assert_eq!(
            scene.lines[4],
            SceneLine::Place(PlaceDef {
                name: "log".to_string(),
                position: Vec3::new(-2.4, 0.0, 1.1),
                rotation_deg: 37.0,
                scale: 1.0,
            })
        );
        assert_eq!(
            scene.lines[5],
            SceneLine::Scatter(ScatterDef {
                name: "spruce".to_string(),
                seed: 7,
                count: 240,
                ring_inner: 18.0,
                ring_outer: 45.0,
            })
        );
        assert_eq!(
            scene.lines[6],
            SceneLine::Settler(SettlerDef {
                name: "Кора".to_string(),
                pose: "standing".to_string(),
                position: Vec3::new(1.2, 0.0, -2.8),
                height: 1.68,
            })
        );
    }

    #[test]
    fn settler_line_parses() {
        let scene = parse("settler Кора pose standing at 1.2 0.0 -2.8 height 1.68").unwrap();
        assert_eq!(
            scene.lines[0],
            SceneLine::Settler(SettlerDef {
                name: "Кора".to_string(),
                pose: "standing".to_string(),
                position: Vec3::new(1.2, 0.0, -2.8),
                height: 1.68,
            })
        );
    }

    #[test]
    fn blank_and_comment_only_lines_are_ignored() {
        let scene = parse("\n   \n# just a comment\n   # indented comment\n").unwrap();
        assert!(scene.lines.is_empty());
    }

    #[test]
    fn trailing_comment_on_a_real_line_is_stripped() {
        let scene =
            parse("model log models/log.obj textures/nature.png # placeholder wood").unwrap();
        assert_eq!(scene.lines.len(), 1);
    }

    #[test]
    fn missing_field_reports_its_line_without_panicking() {
        let err = parse("# comment on line 1\nmodel log models/log.obj\n")
            .expect_err("missing atlas path must error");
        assert_eq!(err.line, 2);
        assert!(
            err.message.contains("atlas path"),
            "message should name the missing field: {}",
            err.message
        );
    }

    #[test]
    fn non_numeric_field_reports_its_line_without_panicking() {
        let err =
            parse("place fire 0.0 not-a-number 0.0 0 1.0").expect_err("bad number must error");
        assert_eq!(err.line, 1);
        assert!(err.message.contains("not-a-number"), "{}", err.message);
    }

    #[test]
    fn unknown_line_kind_is_a_clean_error_not_a_panic() {
        let err = parse("teleport home").expect_err("unknown tag must error");
        assert_eq!(err.line, 1);
        assert!(err.message.contains("teleport"), "{}", err.message);
    }

    #[test]
    fn scatter_missing_keyword_reports_what_was_expected() {
        let err =
            parse("scatter spruce 7 count 240 ring 18 45").expect_err("missing `seed` must error");
        assert_eq!(err.line, 1);
        assert!(err.message.contains("seed"), "{}", err.message);
    }

    #[test]
    fn extra_trailing_field_is_an_error() {
        let err =
            parse("place fire 0.0 0.0 0.0 0 1.0 extra").expect_err("trailing token must error");
        assert_eq!(err.line, 1);
        assert!(err.message.contains("extra"), "{}", err.message);
    }
}
