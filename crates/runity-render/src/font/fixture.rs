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
