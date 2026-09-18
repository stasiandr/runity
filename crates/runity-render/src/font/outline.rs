//! The `glyf` bytes of one glyph, read as closed contours.
//!
//! A TrueType outline is a list of points with a flag each: a point is either
//! on the curve or a quadratic control point off it. Two control points in a
//! row mean an on-curve point the file does not store — the midpoint between
//! them — so the contour that comes out of here is always an alternation the
//! rasterizer can walk without thinking: a start point and then lines and
//! quadratics back to it.
//!
//! Composite glyphs are resolved here too, which is what makes half of Cyrillic
//! work: `Ё` is `Е` plus a dieresis placed by an offset, and `Й` is `И` plus a
//! breve. A component may carry a scale, a per-axis scale or a full 2×2 matrix,
//! and may itself be composite — so the recursion is capped at
//! [`MAX_COMPOSITE_DEPTH`], and a file whose components point at each other
//! comes back as [`FontError::Malformed`] rather than as a hung thread.
//!
//! Everything here is in font units with y pointing up, exactly as the file
//! stores it; the move to pixels belongs to [`super::raster`].

use runity_math::Vec2;

use super::tables::Cursor;
use super::{Font, FontError};

/// How many composite glyphs may be nested before the font is called broken.
///
/// Real fonts stack two, maybe three deep (a letter, an accent, an accent on an
/// accent). The limit exists for files that are not real.
pub const MAX_COMPOSITE_DEPTH: u8 = 5;

// Component flags, from the `glyf` chapter of the OpenType specification.
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;

// Point flags of a simple glyph.
const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME_OR_POSITIVE: u8 = 0x10;
const Y_SAME_OR_POSITIVE: u8 = 0x20;

/// One step along a contour, from wherever the previous step ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// A straight line to this point.
    Line(Vec2),
    /// A quadratic Bézier: an off-curve control point, then the end point.
    Quad(Vec2, Vec2),
}

impl Segment {
    /// Where this segment ends — the start of the next one.
    pub fn end(&self) -> Vec2 {
        match *self {
            Segment::Line(end) => end,
            Segment::Quad(_, end) => end,
        }
    }
}

/// One closed contour: a start point and the segments that return to it.
///
/// The last segment always ends at [`Contour::start`]; closing the loop is this
/// module's job, not the caller's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Contour {
    /// Where the pen starts, in font units.
    pub start: Vec2,
    /// The steps that walk the contour and come back to `start`.
    pub segments: Vec<Segment>,
}

/// A glyph's outline: every contour, in font units, y up, composites resolved.
///
/// An outline with no contours is not an error — a space is exactly that.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outline {
    /// Outer contours and the holes inside them, in the order the font lists
    /// them. Which is which is the fill rule's business, not the reader's.
    pub contours: Vec<Contour>,
}

impl Outline {
    /// True for a glyph that draws nothing, such as a space.
    pub fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.segments.is_empty())
    }

    /// The smallest box holding every point of the outline, control points
    /// included, in font units — `None` for an empty outline.
    ///
    /// Control points bulge outside the curve, so this box can be a shade
    /// larger than the ink. It is a bound, not a measurement.
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        let mut any = false;
        let mut widen = |p: Vec2| {
            min = Vec2::new(min.x.min(p.x), min.y.min(p.y));
            max = Vec2::new(max.x.max(p.x), max.y.max(p.y));
        };
        for contour in &self.contours {
            if contour.segments.is_empty() {
                continue;
            }
            any = true;
            widen(contour.start);
            for segment in &contour.segments {
                match *segment {
                    Segment::Line(end) => widen(end),
                    Segment::Quad(control, end) => {
                        widen(control);
                        widen(end);
                    }
                }
            }
        }
        any.then_some((min, max))
    }
}

/// Read a glyph's outline, following composite components into other glyphs.
pub(crate) fn build(font: &Font, glyph: u16) -> Result<Outline, FontError> {
    let mut outline = Outline::default();
    append(font, glyph, Transform::IDENTITY, 0, &mut outline)?;
    Ok(outline)
}

/// Append one glyph's contours to `outline`, mapped through `transform`.
fn append(
    font: &Font,
    glyph: u16,
    transform: Transform,
    depth: u8,
    outline: &mut Outline,
) -> Result<(), FontError> {
    let data = font.raw_glyph(glyph);
    if data.is_empty() {
        // An empty `loca` entry is a glyph with no ink, not a broken font.
        return Ok(());
    }
    let mut c = Cursor::new(data);
    let number_of_contours = c.i16()?;
    c.skip(8)?; // xMin, yMin, xMax, yMax — recomputed from the points instead

    if number_of_contours >= 0 {
        simple(&mut c, number_of_contours as usize, transform, outline)
    } else {
        if depth >= MAX_COMPOSITE_DEPTH {
            return Err(FontError::Malformed("composite glyphs nested too deep"));
        }
        composite(font, &mut c, transform, depth, outline)
    }
}

// ---------------------------------------------------------------------------
// Simple glyphs
// ---------------------------------------------------------------------------

/// A point as the file stores it: a position and whether it is on the curve.
#[derive(Clone, Copy)]
struct RawPoint {
    at: Vec2,
    on_curve: bool,
}

/// Read a simple glyph: end points, flags, then the two delta arrays.
fn simple(
    c: &mut Cursor<'_>,
    number_of_contours: usize,
    transform: Transform,
    outline: &mut Outline,
) -> Result<(), FontError> {
    let mut ends = Vec::with_capacity(number_of_contours.min(64));
    for _ in 0..number_of_contours {
        let end = c.u16()? as usize;
        if let Some(previous) = ends.last() {
            if end < *previous {
                return Err(FontError::Malformed("glyph contours end out of order"));
            }
        }
        ends.push(end);
    }
    let instructions = c.u16()? as usize;
    c.skip(instructions)?; // hinting bytecode, which this rasterizer does not run

    let num_points = match ends.last() {
        Some(&last) => last + 1,
        None => return Ok(()), // a glyph header with no contours draws nothing
    };

    // Flags come first and are run-length encoded; x deltas and y deltas follow
    // as two separate arrays, each sized by the flags.
    let mut flags = Vec::with_capacity(num_points);
    while flags.len() < num_points {
        let flag = c.bytes(1)?[0];
        flags.push(flag);
        if flag & REPEAT != 0 {
            let times = c.bytes(1)?[0] as usize;
            for _ in 0..times.min(num_points - flags.len()) {
                flags.push(flag);
            }
        }
    }

    let mut points = Vec::with_capacity(num_points);
    let mut x = 0i32;
    for &flag in &flags {
        x += delta(c, flag & X_SHORT != 0, flag & X_SAME_OR_POSITIVE != 0)?;
        points.push(RawPoint {
            at: Vec2::new(x as f32, 0.0),
            on_curve: flag & ON_CURVE != 0,
        });
    }
    let mut y = 0i32;
    for (point, &flag) in points.iter_mut().zip(&flags) {
        y += delta(c, flag & Y_SHORT != 0, flag & Y_SAME_OR_POSITIVE != 0)?;
        point.at.y = y as f32;
        point.at = transform.apply(point.at);
    }

    let mut start = 0usize;
    for &end in &ends {
        // `ends` is sorted and the last entry sized `points`, so the slice is
        // always inside it.
        let contour = contour_of(&points[start..=end]);
        if !contour.segments.is_empty() {
            outline.contours.push(contour);
        }
        start = end + 1;
    }
    Ok(())
}

/// One coordinate delta, in whichever of the three encodings its flags name.
fn delta(c: &mut Cursor<'_>, short: bool, same_or_positive: bool) -> Result<i32, FontError> {
    if short {
        let magnitude = c.bytes(1)?[0] as i32;
        Ok(if same_or_positive {
            magnitude
        } else {
            -magnitude
        })
    } else if same_or_positive {
        Ok(0) // the coordinate repeats the previous one
    } else {
        Ok(c.i16()? as i32)
    }
}

/// Turn one contour's points into a start point and its segments.
///
/// The file may start a contour anywhere, including on a control point, and may
/// leave out the on-curve point between two control points. Both are fixed up
/// here so that every contour begins on the curve.
fn contour_of(points: &[RawPoint]) -> Contour {
    if points.is_empty() {
        return Contour::default();
    }

    // Begin the walk on the curve: at the first point if it is on the curve, at
    // the last one if that is, and otherwise at the midpoint between them,
    // which is where the implied on-curve point would have been. The point the
    // walk starts from is not walked over again.
    let last = points.len() - 1;
    let (start, first_step, steps) = if points[0].on_curve {
        (points[0].at, 1, last)
    } else if points[last].on_curve {
        (points[last].at, 0, last)
    } else {
        (midpoint(points[0].at, points[last].at), 0, points.len())
    };

    let mut contour = Contour {
        start,
        segments: Vec::with_capacity(points.len()),
    };
    let mut control: Option<Vec2> = None;
    for step in 0..steps {
        let point = points[first_step + step];
        match (control, point.on_curve) {
            (None, true) => contour.segments.push(Segment::Line(point.at)),
            (None, false) => control = Some(point.at),
            (Some(previous), true) => {
                contour.segments.push(Segment::Quad(previous, point.at));
                control = None;
            }
            // Two controls in a row: the on-curve point between them is the one
            // the file leaves implied.
            (Some(previous), false) => {
                contour
                    .segments
                    .push(Segment::Quad(previous, midpoint(previous, point.at)));
                control = Some(point.at);
            }
        }
    }
    match control {
        Some(previous) => contour.segments.push(Segment::Quad(previous, start)),
        None => contour.segments.push(Segment::Line(start)),
    }
    contour
}

fn midpoint(a: Vec2, b: Vec2) -> Vec2 {
    (a + b) * 0.5
}

// ---------------------------------------------------------------------------
// Composite glyphs
// ---------------------------------------------------------------------------

/// An affine map from a component's own units into its parent's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform {
    /// The 2×2 part, in the order the font stores it: xx, xy, yx, yy.
    matrix: [f32; 4],
    offset: Vec2,
}

impl Transform {
    const IDENTITY: Transform = Transform {
        matrix: [1.0, 0.0, 0.0, 1.0],
        offset: Vec2::ZERO,
    };

    fn apply(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.matrix[0] * p.x + self.matrix[2] * p.y + self.offset.x,
            self.matrix[1] * p.x + self.matrix[3] * p.y + self.offset.y,
        )
    }

    /// `self` first, then `outer` — the map a component of a component needs.
    fn then(&self, outer: &Transform) -> Transform {
        let [a, b, c, d] = self.matrix;
        let [e, f, g, h] = outer.matrix;
        Transform {
            matrix: [a * e + b * g, a * f + b * h, c * e + d * g, c * f + d * h],
            offset: outer.apply(self.offset),
        }
    }

    /// The 2×2 part alone, for an offset that is itself scaled.
    fn linear(&self) -> Transform {
        Transform {
            matrix: self.matrix,
            offset: Vec2::ZERO,
        }
    }
}

/// Read a composite glyph and append every component's contours.
fn composite(
    font: &Font,
    c: &mut Cursor<'_>,
    transform: Transform,
    depth: u8,
    outline: &mut Outline,
) -> Result<(), FontError> {
    loop {
        let flags = c.u16()?;
        let glyph = c.u16()?;

        let (arg1, arg2) = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            (c.i16()? as f32, c.i16()? as f32)
        } else {
            let b = c.bytes(2)?;
            (b[0] as i8 as f32, b[1] as i8 as f32)
        };

        let matrix = if flags & WE_HAVE_A_SCALE != 0 {
            let scale = f2dot14(c.i16()?);
            [scale, 0.0, 0.0, scale]
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            [f2dot14(c.i16()?), 0.0, 0.0, f2dot14(c.i16()?)]
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            [
                f2dot14(c.i16()?),
                f2dot14(c.i16()?),
                f2dot14(c.i16()?),
                f2dot14(c.i16()?),
            ]
        } else {
            [1.0, 0.0, 0.0, 1.0]
        };

        // Without ARGS_ARE_XY_VALUES the arguments are point numbers: the
        // component is placed by matching one of its points to one of the
        // parent's. That needs the parent's points, which is more machinery
        // than the fonts we ship need, so such a component is placed at the
        // origin rather than refused.
        let offset = if flags & ARGS_ARE_XY_VALUES != 0 {
            Vec2::new(arg1, arg2)
        } else {
            Vec2::ZERO
        };
        let placement = Transform { matrix, offset };
        // Microsoft's default is an offset in the parent's units; the flag asks
        // for one scaled by the component's own matrix (Apple's reading).
        let placement = if flags & SCALED_COMPONENT_OFFSET != 0 {
            Transform {
                offset: placement.linear().apply(offset),
                ..placement
            }
        } else {
            placement
        };

        append(font, glyph, placement.then(&transform), depth + 1, outline)?;

        if flags & MORE_COMPONENTS == 0 {
            return Ok(());
        }
    }
}

/// A component's scale factor: 2.14 fixed point.
fn f2dot14(raw: i16) -> f32 {
    raw as f32 / 16384.0
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{self, Builder, Comp, Pt};
    use super::*;

    /// A font whose glyphs are the ones given, in order from glyph 0.
    fn font_of(glyphs: &[Vec<u8>]) -> Font {
        Font::from_bytes(Builder::simple().with_glyphs(glyphs).build()).expect("a valid fixture")
    }

    fn square(size: i16) -> Vec<u8> {
        fixture::glyf_simple(&[vec![
            Pt::on(0, 0),
            Pt::on(size, 0),
            Pt::on(size, size),
            Pt::on(0, size),
        ]])
    }

    fn ends(contour: &Contour) -> Vec<Vec2> {
        contour.segments.iter().map(|s| s.end()).collect()
    }

    #[test]
    fn a_square_comes_back_as_four_lines_that_close() {
        let font = font_of(&[vec![], square(100)]);
        let outline = build(&font, 1).unwrap();
        assert_eq!(outline.contours.len(), 1);
        let contour = &outline.contours[0];
        assert_eq!(contour.start, Vec2::new(0.0, 0.0));
        assert_eq!(
            ends(contour),
            vec![
                Vec2::new(100.0, 0.0),
                Vec2::new(100.0, 100.0),
                Vec2::new(0.0, 100.0),
                Vec2::new(0.0, 0.0),
            ],
            "the last segment returns to the start"
        );
        assert!(contour
            .segments
            .iter()
            .all(|s| matches!(s, Segment::Line(_))));
        assert_eq!(outline.bounds(), Some((Vec2::ZERO, Vec2::splat(100.0))));
        assert!(!outline.is_empty());
    }

    #[test]
    fn an_empty_glyph_is_an_empty_outline_and_not_an_error() {
        let font = font_of(&[vec![], vec![]]);
        let outline = build(&font, 1).unwrap();
        assert!(outline.is_empty());
        assert_eq!(outline.bounds(), None);
        // And a glyph id the font does not have answers the same way.
        assert!(build(&font, 900).unwrap().is_empty());
    }

    #[test]
    fn a_control_point_between_two_on_curve_points_is_a_quadratic() {
        let font = font_of(&[
            vec![],
            fixture::glyf_simple(&[vec![Pt::on(0, 0), Pt::off(50, 100), Pt::on(100, 0)]]),
        ]);
        let contour = &build(&font, 1).unwrap().contours[0];
        assert_eq!(contour.start, Vec2::new(0.0, 0.0));
        assert_eq!(
            contour.segments,
            vec![
                Segment::Quad(Vec2::new(50.0, 100.0), Vec2::new(100.0, 0.0)),
                Segment::Line(Vec2::new(0.0, 0.0)),
            ]
        );
    }

    #[test]
    fn two_control_points_in_a_row_imply_the_point_between_them() {
        // A diamond stored as four control points and no on-curve point at all:
        // every corner of the walk has to be invented.
        let font = font_of(&[
            vec![],
            fixture::glyf_simple(&[vec![
                Pt::off(0, 100),
                Pt::off(100, 100),
                Pt::off(100, 0),
                Pt::off(0, 0),
            ]]),
        ]);
        let contour = &build(&font, 1).unwrap().contours[0];
        assert_eq!(
            contour.start,
            Vec2::new(0.0, 50.0),
            "between the last control point and the first"
        );
        assert_eq!(
            contour.segments,
            vec![
                Segment::Quad(Vec2::new(0.0, 100.0), Vec2::new(50.0, 100.0)),
                Segment::Quad(Vec2::new(100.0, 100.0), Vec2::new(100.0, 50.0)),
                Segment::Quad(Vec2::new(100.0, 0.0), Vec2::new(50.0, 0.0)),
                Segment::Quad(Vec2::new(0.0, 0.0), Vec2::new(0.0, 50.0)),
            ],
            "four implied midpoints, the last of them the start"
        );
    }

    #[test]
    fn a_contour_starting_off_the_curve_is_rotated_onto_it() {
        let font = font_of(&[
            vec![],
            fixture::glyf_simple(&[vec![Pt::off(50, 100), Pt::on(100, 0), Pt::on(0, 0)]]),
        ]);
        let contour = &build(&font, 1).unwrap().contours[0];
        assert_eq!(
            contour.start,
            Vec2::new(0.0, 0.0),
            "the last on-curve point"
        );
        assert_eq!(
            contour.segments,
            vec![
                Segment::Quad(Vec2::new(50.0, 100.0), Vec2::new(100.0, 0.0)),
                Segment::Line(Vec2::new(0.0, 0.0)),
            ]
        );
    }

    #[test]
    fn packed_coordinates_read_the_same_as_plain_ones() {
        // Same points, one glyph with short deltas and repeated flags, one
        // spelling every delta out as a signed word.
        let points = vec![
            Pt::on(10, 10),
            Pt::on(60, 10),
            Pt::on(60, 60),
            Pt::on(10, 60),
            Pt::off(-200, 400),
        ];
        let contours = std::slice::from_ref(&points);
        let plain = font_of(&[vec![], fixture::glyf_simple(contours)]);
        let packed = font_of(&[vec![], fixture::glyf_simple_packed(contours)]);
        assert!(
            packed.raw_glyph(1).len() < plain.raw_glyph(1).len(),
            "the packed encoding is the smaller one"
        );
        assert_eq!(build(&packed, 1).unwrap(), build(&plain, 1).unwrap());
    }

    #[test]
    fn several_contours_come_back_separately() {
        let font = font_of(&[
            vec![],
            fixture::glyf_simple(&[
                vec![Pt::on(0, 0), Pt::on(100, 0), Pt::on(100, 100)],
                vec![Pt::on(20, 20), Pt::on(60, 20), Pt::on(60, 60)],
            ]),
        ]);
        let outline = build(&font, 1).unwrap();
        assert_eq!(outline.contours.len(), 2);
        assert_eq!(outline.contours[0].start, Vec2::ZERO);
        assert_eq!(outline.contours[1].start, Vec2::new(20.0, 20.0));
    }

    #[test]
    fn a_component_is_placed_by_its_offset() {
        let font = font_of(&[
            vec![],
            square(100),
            fixture::glyf_composite(&[Comp::at(1, 10, -20), Comp::at(1, 300, 0)]),
        ]);
        let outline = build(&font, 2).unwrap();
        assert_eq!(outline.contours.len(), 2, "one contour per component");
        assert_eq!(outline.contours[0].start, Vec2::new(10.0, -20.0));
        assert_eq!(outline.contours[1].start, Vec2::new(300.0, 0.0));
        assert_eq!(
            outline.bounds(),
            Some((Vec2::new(10.0, -20.0), Vec2::new(400.0, 100.0)))
        );
    }

    #[test]
    fn a_component_can_be_scaled_on_one_axis_or_both() {
        let font = font_of(&[
            vec![],
            square(100),
            fixture::glyf_composite(&[Comp::at(1, 0, 0).scaled(0.5)]),
            fixture::glyf_composite(&[Comp::at(1, 0, 0).scaled_xy(0.5, 1.75)]),
        ]);
        assert_eq!(
            build(&font, 2).unwrap().bounds(),
            Some((Vec2::ZERO, Vec2::splat(50.0)))
        );
        assert_eq!(
            build(&font, 3).unwrap().bounds(),
            Some((Vec2::ZERO, Vec2::new(50.0, 175.0)))
        );
    }

    #[test]
    fn a_point_matched_component_lands_at_the_origin() {
        // Placement by matching two points is not implemented; the component
        // still has to come back as contours rather than as an error.
        let font = font_of(&[
            vec![],
            square(100),
            fixture::glyf_composite(&[Comp::at(1, 3, 4).point_matched()]),
        ]);
        assert_eq!(build(&font, 2).unwrap().contours[0].start, Vec2::ZERO);
    }

    #[test]
    fn a_component_can_carry_a_full_two_by_two_matrix() {
        // A quarter turn: x becomes y, y becomes -x.
        let font = font_of(&[
            vec![],
            square(100),
            fixture::glyf_composite(&[Comp::at(1, 5, 5).matrix([0.0, 1.0, -1.0, 0.0])]),
        ]);
        let outline = build(&font, 2).unwrap();
        assert_eq!(
            ends(&outline.contours[0]),
            vec![
                Vec2::new(5.0, 105.0),
                Vec2::new(-95.0, 105.0),
                Vec2::new(-95.0, 5.0),
                Vec2::new(5.0, 5.0),
            ]
        );
    }

    #[test]
    fn a_scaled_component_offset_is_scaled_too() {
        let font = font_of(&[
            vec![],
            square(100),
            fixture::glyf_composite(&[Comp::at(1, 40, 40).scaled(0.5)]),
            fixture::glyf_composite(&[Comp::at(1, 40, 40).scaled(0.5).scaled_offset()]),
        ]);
        assert_eq!(
            build(&font, 2).unwrap().contours[0].start,
            Vec2::splat(40.0)
        );
        assert_eq!(
            build(&font, 3).unwrap().contours[0].start,
            Vec2::splat(20.0)
        );
    }

    #[test]
    fn a_component_of_a_component_composes_both_transforms() {
        let font = font_of(&[
            vec![],
            square(100),
            fixture::glyf_composite(&[Comp::at(1, 10, 0).scaled(0.5)]),
            fixture::glyf_composite(&[Comp::at(2, 0, 7).scaled(1.5)]),
        ]);
        let outline = build(&font, 3).unwrap();
        // The inner square is halved and moved by 10, then scaled by 1.5 and
        // moved by 7 upwards: 75 units across, starting at (15, 7).
        assert_eq!(
            outline.bounds(),
            Some((Vec2::new(15.0, 7.0), Vec2::new(90.0, 82.0)))
        );
    }

    #[test]
    fn a_composite_glyph_that_points_at_itself_is_an_error_and_not_a_hang() {
        let font = font_of(&[vec![], fixture::glyf_composite(&[Comp::at(1, 1, 1)])]);
        assert_eq!(
            build(&font, 1).unwrap_err(),
            FontError::Malformed("composite glyphs nested too deep")
        );

        // And so is a pair that point at each other.
        let font = font_of(&[
            vec![],
            fixture::glyf_composite(&[Comp::at(2, 0, 0)]),
            fixture::glyf_composite(&[Comp::at(1, 0, 0)]),
        ]);
        assert!(build(&font, 1).is_err());

        // A chain shorter than the limit still resolves.
        let mut glyphs = vec![vec![], square(10)];
        for parent in 1..MAX_COMPOSITE_DEPTH as u16 {
            glyphs.push(fixture::glyf_composite(&[Comp::at(parent, 1, 0)]));
        }
        let font = font_of(&glyphs);
        let deepest = glyphs.len() as u16 - 1;
        assert_eq!(build(&font, deepest).unwrap().contours.len(), 1);
    }

    #[test]
    fn truncating_a_glyph_anywhere_gives_an_error_and_not_a_panic() {
        let full = fixture::glyf_simple_packed(&[vec![
            Pt::on(0, 0),
            Pt::off(50, 100),
            Pt::on(100, 0),
            Pt::on(-10, -10),
        ]]);
        let composite = fixture::glyf_composite(&[
            Comp::at(1, 10, 10).matrix([1.0, 0.25, 0.0, 1.0]),
            Comp::at(1, 0, 0),
        ]);
        for glyph in [full, composite] {
            for end in 1..glyph.len() {
                let font = font_of(&[vec![], glyph[..end].to_vec()]);
                // Some prefixes are legal glyphs with fewer points; the point is
                // that every one of them returns rather than panics.
                let _ = build(&font, 1);
            }
        }
    }

    #[test]
    fn a_contour_list_that_runs_backwards_is_refused() {
        let mut glyph = fixture::glyf_simple(&[
            vec![Pt::on(0, 0), Pt::on(10, 0), Pt::on(10, 10)],
            vec![Pt::on(20, 20), Pt::on(30, 20), Pt::on(30, 30)],
        ]);
        // endPtsOfContours: swap 2 and 5 so the contours end out of order.
        glyph[10..12].copy_from_slice(&5u16.to_be_bytes());
        glyph[12..14].copy_from_slice(&2u16.to_be_bytes());
        let font = font_of(&[vec![], glyph]);
        assert_eq!(
            build(&font, 1).unwrap_err(),
            FontError::Malformed("glyph contours end out of order")
        );
    }

    #[test]
    fn roboto_glyphs_come_out_with_contours_where_they_should_be() {
        let font = Font::embedded();
        let em = font.units_per_em() as f32;

        let o = build(&font, font.glyph_index('o').unwrap()).unwrap();
        assert_eq!(o.contours.len(), 2, "an 'o' is a ring: outside and inside");
        assert!(
            o.contours
                .iter()
                .all(|c| c.segments.iter().any(|s| matches!(s, Segment::Quad(..)))),
            "both contours are curved"
        );

        let (min, max) = o.bounds().unwrap();
        assert!(
            min.y > -0.1 * em && max.y < 0.8 * em,
            "'o' is an x-height letter"
        );

        // Ё is Е plus a dieresis, so it is taller and has three contours where
        // Е has one.
        let e = build(&font, font.glyph_index('Е').unwrap()).unwrap();
        let yo = build(&font, font.glyph_index('Ё').unwrap()).unwrap();
        assert_eq!(e.contours.len(), 1);
        assert_eq!(yo.contours.len(), 3, "the letter and two dots");
        assert!(yo.bounds().unwrap().1.y > e.bounds().unwrap().1.y);
    }

    #[test]
    fn every_glyph_of_the_embedded_font_reads_without_an_error() {
        let font = Font::embedded();
        let mut with_contours = 0;
        for glyph in 0..font.num_glyphs() {
            let outline = build(&font, glyph).unwrap_or_else(|e| panic!("glyph {glyph}: {e}"));
            if !outline.is_empty() {
                with_contours += 1;
            }
        }
        assert!(with_contours > 1000, "Roboto is not a font of empty boxes");
    }
}
