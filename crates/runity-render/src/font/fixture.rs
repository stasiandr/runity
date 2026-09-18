//! Synthetic sfnt files for the font parser's own tests.
//!
//! The embedded Roboto is one font with one shape of every table; these
//! builders make the other shapes — a short `loca`, a `cmap` segment that goes
//! through `glyphIdArray`, a `kern` table at all — and the broken files the
//! parser has to refuse. Nothing here is compiled into the library.

use super::tables::GlyphMetrics;

/// A font assembled table by table.
pub(crate) struct Builder {
    tables: Vec<([u8; 4], Vec<u8>)>,
}

impl Builder {
    /// A font with no tables at all.
    pub(crate) fn empty() -> Self {
        Self { tables: Vec::new() }
    }

    /// Three glyphs, a long `loca`, a format 4 `cmap` over `A`..=`B`, and an
    /// `hmtx` whose last glyph lives in the compressed tail.
    ///
    /// Glyph 1 is twelve bytes of `glyf` and glyph 2 is twenty; the bytes are
    /// not a real outline, because nothing at this layer looks inside them.
    pub(crate) fn simple() -> Self {
        let mut glyf = vec![0u8; 32];
        glyf[0] = 0x00; // glyph 1: numberOfContours = 1
        glyf[1] = 0x01;
        glyf[12] = 0xFF; // glyph 2: numberOfContours = -1, a composite
        glyf[13] = 0xFF;
        Self::empty()
            .with(b"head", head(1000, true))
            .with(b"maxp", maxp(3))
            .with(b"hhea", hhea(800, -200, 100, 2))
            .with(b"hmtx", hmtx(&[(500, 10), (600, 20)], &[-7]))
            .with(b"loca", loca_long(&[0, 0, 12, 32]))
            .with(b"glyf", glyf)
            .with(
                b"cmap",
                cmap_table(&[(3, 1, cmap_format4(&[Seg::Delta(0x41, 0x42, -0x40)]))]),
            )
    }

    /// Replace `glyf`, and the tables whose size has to agree with it: `loca`,
    /// `maxp` and `hmtx`, one advance per glyph.
    ///
    /// Glyph 0 is `.notdef` and is normally passed as an empty entry; the rest
    /// come from [`glyf_simple`] and [`glyf_composite`].
    pub(crate) fn with_glyphs(self, glyphs: &[Vec<u8>]) -> Self {
        let mut glyf = Vec::new();
        let mut offsets = vec![0u32];
        for glyph in glyphs {
            glyf.extend_from_slice(glyph);
            // `loca` offsets of a short table are halved, so keep every glyph
            // on an even boundary whichever format the caller asks for.
            while glyf.len() % 4 != 0 {
                glyf.push(0);
            }
            offsets.push(glyf.len() as u32);
        }
        let metrics: Vec<(u16, i16)> = glyphs.iter().map(|_| (600, 0)).collect();
        self.with(b"glyf", glyf)
            .with(b"loca", loca_long(&offsets))
            .with(b"maxp", maxp(glyphs.len() as u16))
            .with(b"hhea", hhea(800, -200, 100, glyphs.len() as u16))
            .with(b"hmtx", hmtx(&metrics, &[]))
    }

    /// Add or replace a table.
    pub(crate) fn with(mut self, tag: &[u8; 4], data: Vec<u8>) -> Self {
        self.tables.retain(|(t, _)| t != tag);
        self.tables.push((*tag, data));
        self
    }

    /// Drop a table.
    pub(crate) fn without(mut self, tag: &[u8; 4]) -> Self {
        self.tables.retain(|(t, _)| t != tag);
        self
    }

    /// Serialize as a TrueType file.
    pub(crate) fn build(&self) -> Vec<u8> {
        self.build_tagged([0x00, 0x01, 0x00, 0x00], 0)
    }

    /// Serialize with a chosen sfnt version tag — `OTTO` for a CFF font.
    pub(crate) fn build_tagged(&self, sfnt: [u8; 4], base: usize) -> Vec<u8> {
        let mut tables = self.tables.clone();
        tables.sort_by_key(|(tag, _)| *tag);

        let mut out = Vec::new();
        out.extend_from_slice(&sfnt);
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0; 6]); // searchRange, entrySelector, rangeShift

        let mut at = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (i, (tag, data)) in tables.iter().enumerate() {
            out.extend_from_slice(tag);
            out.extend_from_slice(&[0; 4]); // checksum, not verified on the way in
            out.extend_from_slice(&((base + at) as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(data);
            at += data.len();
            // Pad to the next four-byte boundary, but not past the last table:
            // trailing slack would make a truncated file parse.
            if i + 1 < tables.len() {
                while at % 4 != 0 {
                    body.push(0);
                    at += 1;
                }
            }
        }
        out.extend_from_slice(&body);
        out
    }
}

/// Pack several fonts into a `.ttc` collection.
pub(crate) fn collection(fonts: &[Builder]) -> Vec<u8> {
    let header = 12 + 4 * fonts.len();
    let mut bodies = Vec::new();
    let mut offsets = Vec::new();
    for font in fonts {
        offsets.push(header + bodies.len());
        bodies
            .extend_from_slice(&font.build_tagged([0x00, 0x01, 0x00, 0x00], header + bodies.len()));
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&(fonts.len() as u32).to_be_bytes());
    for offset in offsets {
        out.extend_from_slice(&(offset as u32).to_be_bytes());
    }
    out.extend_from_slice(&bodies);
    out
}

/// A `head` table: em size, the magic number, and the `loca` format.
pub(crate) fn head(units_per_em: u16, long_loca: bool) -> Vec<u8> {
    let mut t = vec![0u8; 54];
    t[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    t[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    t[18..20].copy_from_slice(&units_per_em.to_be_bytes());
    t[50..52].copy_from_slice(&(long_loca as i16).to_be_bytes());
    t
}

/// A `maxp` table holding a glyph count.
pub(crate) fn maxp(num_glyphs: u16) -> Vec<u8> {
    let mut t = vec![0u8; 32];
    t[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    t[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    t
}

/// An `hhea` table with the vertical metrics and the `hmtx` array length.
pub(crate) fn hhea(ascender: i16, descender: i16, line_gap: i16, metrics: u16) -> Vec<u8> {
    let mut t = vec![0u8; 36];
    t[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    t[4..6].copy_from_slice(&ascender.to_be_bytes());
    t[6..8].copy_from_slice(&descender.to_be_bytes());
    t[8..10].copy_from_slice(&line_gap.to_be_bytes());
    t[34..36].copy_from_slice(&metrics.to_be_bytes());
    t
}

/// An `OS/2` table carrying typographic metrics.
pub(crate) fn os2(ascender: i16, descender: i16, line_gap: i16) -> Vec<u8> {
    let mut t = vec![0u8; 96];
    t[0..2].copy_from_slice(&4u16.to_be_bytes()); // version
    t[68..70].copy_from_slice(&ascender.to_be_bytes());
    t[70..72].copy_from_slice(&descender.to_be_bytes());
    t[72..74].copy_from_slice(&line_gap.to_be_bytes());
    t
}

/// An `hmtx` table: paired metrics, then bearings for the compressed tail.
pub(crate) fn hmtx(paired: &[(u16, i16)], tail_bearings: &[i16]) -> Vec<u8> {
    let mut t = Vec::new();
    for (advance, bearing) in paired {
        t.extend_from_slice(&advance.to_be_bytes());
        t.extend_from_slice(&bearing.to_be_bytes());
    }
    for bearing in tail_bearings {
        t.extend_from_slice(&bearing.to_be_bytes());
    }
    t
}

/// The metrics a parsed `hmtx` entry is expected to hold.
pub(crate) fn metric(advance_width: u16, left_side_bearing: i16) -> GlyphMetrics {
    GlyphMetrics {
        advance_width,
        left_side_bearing,
    }
}

/// A short `loca`: offsets are stored halved.
pub(crate) fn loca_short(halves: &[u16]) -> Vec<u8> {
    halves.iter().flat_map(|o| o.to_be_bytes()).collect()
}

/// A long `loca`: offsets are stored as they are.
pub(crate) fn loca_long(offsets: &[u32]) -> Vec<u8> {
    offsets.iter().flat_map(|o| o.to_be_bytes()).collect()
}

/// One segment of a format 4 `cmap` subtable.
pub(crate) enum Seg {
    /// A run of code points mapped by adding a delta to each.
    Delta(u16, u16, i16),
    /// A run starting at a code point, mapped through `glyphIdArray`.
    Glyphs(u16, Vec<u16>),
}

/// A format 4 subtable, with the mandatory 0xFFFF terminator appended.
pub(crate) fn cmap_format4(segments: &[Seg]) -> Vec<u8> {
    let seg_count = segments.len() + 1;
    let mut ends = Vec::new();
    let mut starts = Vec::new();
    let mut deltas = Vec::new();
    let mut range_offsets = Vec::new();
    let mut glyph_array: Vec<u16> = Vec::new();

    for (i, seg) in segments.iter().enumerate() {
        match seg {
            Seg::Delta(start, end, delta) => {
                starts.push(*start);
                ends.push(*end);
                deltas.push(*delta as u16);
                range_offsets.push(0u16);
            }
            Seg::Glyphs(start, glyphs) => {
                starts.push(*start);
                ends.push(start + glyphs.len() as u16 - 1);
                deltas.push(0);
                // Distance from this segment's idRangeOffset slot to its glyphs.
                let slot = 16 + 6 * seg_count + 2 * i;
                let at = 16 + 8 * seg_count + 2 * glyph_array.len();
                range_offsets.push((at - slot) as u16);
                glyph_array.extend_from_slice(glyphs);
            }
        }
    }
    starts.push(0xFFFF);
    ends.push(0xFFFF);
    deltas.push(1);
    range_offsets.push(0);

    let length = 16 + 8 * seg_count + 2 * glyph_array.len();
    let mut t = Vec::with_capacity(length);
    t.extend_from_slice(&4u16.to_be_bytes());
    t.extend_from_slice(&(length as u16).to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes()); // language
    t.extend_from_slice(&((seg_count * 2) as u16).to_be_bytes());
    t.extend_from_slice(&[0; 6]); // searchRange, entrySelector, rangeShift
    for e in &ends {
        t.extend_from_slice(&e.to_be_bytes());
    }
    t.extend_from_slice(&[0; 2]); // reservedPad
    for s in &starts {
        t.extend_from_slice(&s.to_be_bytes());
    }
    for d in &deltas {
        t.extend_from_slice(&d.to_be_bytes());
    }
    for r in &range_offsets {
        t.extend_from_slice(&r.to_be_bytes());
    }
    for g in &glyph_array {
        t.extend_from_slice(&g.to_be_bytes());
    }
    t
}

/// A format 12 subtable from `(start, end, first glyph)` groups.
pub(crate) fn cmap_format12(groups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&12u16.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes()); // reserved
    t.extend_from_slice(&((16 + 12 * groups.len()) as u32).to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes()); // language
    t.extend_from_slice(&(groups.len() as u32).to_be_bytes());
    for (start, end, glyph) in groups {
        t.extend_from_slice(&start.to_be_bytes());
        t.extend_from_slice(&end.to_be_bytes());
        t.extend_from_slice(&glyph.to_be_bytes());
    }
    t
}

/// Wrap subtables in a `cmap` table with one encoding record each.
pub(crate) fn cmap_table(encodings: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes()); // version
    t.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    let mut at = 4 + 8 * encodings.len();
    let mut body = Vec::new();
    for (platform, encoding, sub) in encodings {
        t.extend_from_slice(&platform.to_be_bytes());
        t.extend_from_slice(&encoding.to_be_bytes());
        t.extend_from_slice(&(at as u32).to_be_bytes());
        body.extend_from_slice(sub);
        at += sub.len();
    }
    t.extend_from_slice(&body);
    t
}

/// One point of a glyph contour, in font units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pt {
    pub(crate) x: i16,
    pub(crate) y: i16,
    pub(crate) on_curve: bool,
}

impl Pt {
    /// A point the outline passes through.
    pub(crate) fn on(x: i16, y: i16) -> Pt {
        Pt {
            x,
            y,
            on_curve: true,
        }
    }

    /// A quadratic control point.
    pub(crate) fn off(x: i16, y: i16) -> Pt {
        Pt {
            x,
            y,
            on_curve: false,
        }
    }
}

/// A simple glyph with every coordinate spelled out as a signed word.
pub(crate) fn glyf_simple(contours: &[Vec<Pt>]) -> Vec<u8> {
    glyf_encode(contours, false)
}

/// The same glyph in the encoding a real font uses: one-byte deltas, repeated
/// coordinates left out, and runs of equal flags collapsed.
pub(crate) fn glyf_simple_packed(contours: &[Vec<Pt>]) -> Vec<u8> {
    glyf_encode(contours, true)
}

fn glyf_encode(contours: &[Vec<Pt>], packed: bool) -> Vec<u8> {
    let points: Vec<Pt> = contours.iter().flatten().copied().collect();
    let mut t = Vec::new();
    t.extend_from_slice(&(contours.len() as i16).to_be_bytes());
    for extreme in [
        points.iter().map(|p| p.x).min().unwrap_or(0),
        points.iter().map(|p| p.y).min().unwrap_or(0),
        points.iter().map(|p| p.x).max().unwrap_or(0),
        points.iter().map(|p| p.y).max().unwrap_or(0),
    ] {
        t.extend_from_slice(&extreme.to_be_bytes());
    }

    let mut end = 0usize;
    for contour in contours {
        end += contour.len();
        t.extend_from_slice(&((end - 1) as u16).to_be_bytes());
    }
    t.extend_from_slice(&0u16.to_be_bytes()); // instructionLength

    // Coordinates are stored as deltas from the previous point, starting at the
    // origin, and each axis says in the flags how it is encoded.
    let mut deltas = Vec::with_capacity(points.len());
    let (mut x, mut y) = (0i32, 0i32);
    for point in &points {
        deltas.push((point.x as i32 - x, point.y as i32 - y));
        x = point.x as i32;
        y = point.y as i32;
    }

    let mut flags = Vec::with_capacity(points.len());
    for (point, &(dx, dy)) in points.iter().zip(&deltas) {
        let mut flag = if point.on_curve { 0x01u8 } else { 0x00 };
        if packed {
            flag |= axis_flags(dx, 0x02, 0x10);
            flag |= axis_flags(dy, 0x04, 0x20);
        }
        flags.push(flag);
    }

    let mut at = 0;
    while at < flags.len() {
        let flag = flags[at];
        let run = flags[at..]
            .iter()
            .take(256)
            .take_while(|&&f| f == flag)
            .count();
        if packed && run > 1 {
            t.push(flag | 0x08); // REPEAT
            t.push((run - 1) as u8);
        } else {
            t.extend(std::iter::repeat(flag).take(run));
        }
        at += run;
    }

    for axis in 0..2 {
        for (&flag, &(dx, dy)) in flags.iter().zip(&deltas) {
            let (delta, short, same) = if axis == 0 {
                (dx, flag & 0x02 != 0, flag & 0x10 != 0)
            } else {
                (dy, flag & 0x04 != 0, flag & 0x20 != 0)
            };
            if short {
                t.push(delta.unsigned_abs() as u8);
            } else if !same {
                t.extend_from_slice(&(delta as i16).to_be_bytes());
            }
        }
    }
    t
}

/// The short/same pair of flag bits for one axis of one delta.
fn axis_flags(delta: i32, short: u8, same_or_positive: u8) -> u8 {
    if delta == 0 {
        same_or_positive
    } else if delta.abs() <= 255 {
        short | if delta > 0 { same_or_positive } else { 0 }
    } else {
        0
    }
}

/// One component of a composite glyph.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Comp {
    glyph: u16,
    dx: i16,
    dy: i16,
    matrix: Option<[f32; 4]>,
    scaled_offset: bool,
    xy_values: bool,
}

impl Comp {
    /// A component placed by an offset, which is how nearly all of them are.
    pub(crate) fn at(glyph: u16, dx: i16, dy: i16) -> Comp {
        Comp {
            glyph,
            dx,
            dy,
            matrix: None,
            scaled_offset: false,
            xy_values: true,
        }
    }

    /// Scale both axes by the same factor. F2Dot14 holds -2.0 up to 2.0.
    pub(crate) fn scaled(mut self, scale: f32) -> Comp {
        self.matrix = Some([scale, 0.0, 0.0, scale]);
        self
    }

    /// Scale each axis on its own.
    pub(crate) fn scaled_xy(mut self, x: f32, y: f32) -> Comp {
        self.matrix = Some([x, 0.0, 0.0, y]);
        self
    }

    /// A full 2×2 matrix, in the order the font stores it: xx, xy, yx, yy.
    pub(crate) fn matrix(mut self, matrix: [f32; 4]) -> Comp {
        self.matrix = Some(matrix);
        self
    }

    /// Ask for the offset to be scaled by the matrix as well.
    pub(crate) fn scaled_offset(mut self) -> Comp {
        self.scaled_offset = true;
        self
    }

    /// Store the arguments as point numbers rather than as an offset.
    pub(crate) fn point_matched(mut self) -> Comp {
        self.xy_values = false;
        self
    }
}

/// A composite glyph built from components.
pub(crate) fn glyf_composite(components: &[Comp]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&(-1i16).to_be_bytes());
    t.extend_from_slice(&[0; 8]); // bbox, which a composite glyph gets wrong anyway

    for (i, component) in components.iter().enumerate() {
        let words =
            component.dx < -128 || component.dx > 127 || component.dy < -128 || component.dy > 127;
        let mut flags = 0u16;
        if words {
            flags |= 0x0001; // ARG_1_AND_2_ARE_WORDS
        }
        if component.xy_values {
            flags |= 0x0002; // ARGS_ARE_XY_VALUES
        }
        if component.scaled_offset {
            flags |= 0x0800; // SCALED_COMPONENT_OFFSET
        }
        let matrix = component.matrix.map(matrix_fields);
        if let Some((bit, _)) = matrix {
            flags |= bit;
        }
        if i + 1 < components.len() {
            flags |= 0x0020; // MORE_COMPONENTS
        }

        t.extend_from_slice(&flags.to_be_bytes());
        t.extend_from_slice(&component.glyph.to_be_bytes());
        if words {
            t.extend_from_slice(&component.dx.to_be_bytes());
            t.extend_from_slice(&component.dy.to_be_bytes());
        } else {
            t.push(component.dx as i8 as u8);
            t.push(component.dy as i8 as u8);
        }
        for value in matrix.into_iter().flat_map(|(_, values)| values) {
            t.extend_from_slice(&f2dot14(value));
        }
    }
    t
}

/// Which flag a component's matrix needs and which of its values are stored:
/// one factor for a plain scale, two for a per-axis one, four otherwise.
fn matrix_fields(matrix: [f32; 4]) -> (u16, Vec<f32>) {
    let [a, b, c, d] = matrix;
    if b != 0.0 || c != 0.0 {
        (0x0080, vec![a, b, c, d]) // WE_HAVE_A_TWO_BY_TWO
    } else if a == d {
        (0x0008, vec![a]) // WE_HAVE_A_SCALE
    } else {
        (0x0040, vec![a, d]) // WE_HAVE_AN_X_AND_Y_SCALE
    }
}

/// A scale factor as the 2.14 fixed point a component stores.
fn f2dot14(value: f32) -> [u8; 2] {
    (((value * 16384.0).round() as i32) as i16).to_be_bytes()
}

/// A version 0 `kern` table with a single horizontal format 0 subtable.
pub(crate) fn kern(pairs: &[(u16, u16, i16)]) -> Vec<u8> {
    let mut sub = Vec::new();
    sub.extend_from_slice(&0u16.to_be_bytes()); // subtable version
    sub.extend_from_slice(&((14 + 6 * pairs.len()) as u16).to_be_bytes());
    sub.extend_from_slice(&0x0001u16.to_be_bytes()); // horizontal, format 0
    sub.extend_from_slice(&(pairs.len() as u16).to_be_bytes());
    sub.extend_from_slice(&[0; 6]); // searchRange, entrySelector, rangeShift
    for (left, right, value) in pairs {
        sub.extend_from_slice(&left.to_be_bytes());
        sub.extend_from_slice(&right.to_be_bytes());
        sub.extend_from_slice(&value.to_be_bytes());
    }

    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes()); // table version
    t.extend_from_slice(&1u16.to_be_bytes()); // subtable count
    t.extend_from_slice(&sub);
    t
}
