//! The sfnt tables a TrueType file is made of, and the bounds-checked cursor
//! that reads them.
//!
//! Everything here reads foreign bytes, so nothing here is allowed to panic or
//! to loop without an upper bound: every read goes through [`Cursor`], every
//! table offset is checked against the length of its own table, and every
//! `loca` offset is checked against the size of `glyf`. A file that does not
//! make sense comes back as [`FontError`], never as garbage.

use super::FontError;
use std::ops::Range;

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

/// A forward-only reader over a byte slice that cannot run off the end.
///
/// Every method returns [`FontError::Truncated`] instead of reading out of
/// bounds, so a caller can parse a whole table with `?` and never a bounds
/// check of its own.
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    /// Start reading `data` from its first byte.
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    /// Move to an absolute position; positions past the end are refused.
    pub(crate) fn seek(&mut self, to: usize) -> Result<(), FontError> {
        if to > self.data.len() {
            return Err(FontError::Truncated);
        }
        self.at = to;
        Ok(())
    }

    /// Skip `n` bytes.
    pub(crate) fn skip(&mut self, n: usize) -> Result<(), FontError> {
        let to = self.at.checked_add(n).ok_or(FontError::Truncated)?;
        self.seek(to)
    }

    /// Take the next `n` bytes.
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], FontError> {
        let end = self.at.checked_add(n).ok_or(FontError::Truncated)?;
        let slice = self.data.get(self.at..end).ok_or(FontError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    /// Read a big-endian `u16`.
    pub(crate) fn u16(&mut self) -> Result<u16, FontError> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    /// Read a big-endian `i16` (FWord, or a signed field of `head`).
    pub(crate) fn i16(&mut self) -> Result<i16, FontError> {
        Ok(self.u16()? as i16)
    }

    /// Read a big-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32, FontError> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read a four-byte table tag.
    pub(crate) fn tag(&mut self) -> Result<[u8; 4], FontError> {
        let b = self.bytes(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }
}

/// Read a big-endian `u16` at `at`, or zero if that is past the end.
///
/// Only for lookups inside a table whose arrays were already checked at parse
/// time: `cmap` offsets can point anywhere, and a missing glyph id is a legal
/// answer to that, not a parse error.
fn u16_at(data: &[u8], at: usize) -> u16 {
    match data.get(at..).and_then(|rest| rest.get(..2)) {
        Some(b) => u16::from_be_bytes([b[0], b[1]]),
        None => 0,
    }
}

/// Read a big-endian `u32` at `at`, or zero if that is past the end.
fn u32_at(data: &[u8], at: usize) -> u32 {
    match data.get(at..).and_then(|rest| rest.get(..4)) {
        Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// Table directory
// ---------------------------------------------------------------------------

/// Where each table of one font lives in the file.
///
/// Ranges are validated against the length of the file when the directory is
/// parsed, so slicing the file with one of them cannot panic.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Directory {
    tables: Vec<([u8; 4], Range<usize>)>,
}

impl Directory {
    /// Parse the offset table of font `index`.
    ///
    /// A plain `.ttf`/`.otf` holds a single font and accepts only index 0; a
    /// `.ttc` collection is indexed by its font list. An `OTTO` (CFF) file is
    /// refused here rather than parsed into a font with no outlines.
    pub(crate) fn parse(data: &[u8], index: u32) -> Result<Self, FontError> {
        let mut c = Cursor::new(data);
        let tag = c.tag()?;
        if &tag == b"ttcf" {
            c.skip(4)?; // version
            let count = c.u32()?;
            if index >= count {
                return Err(FontError::NoSuchFont { index, count });
            }
            c.skip(index as usize * 4)?;
            let offset = c.u32()? as usize;
            c.seek(offset)?;
            let inner = c.tag()?;
            if &inner == b"ttcf" {
                return Err(FontError::Malformed("a collection inside a collection"));
            }
            check_sfnt(&inner)?;
        } else {
            if index != 0 {
                return Err(FontError::NoSuchFont { index, count: 1 });
            }
            check_sfnt(&tag)?;
        }

        let num_tables = c.u16()? as usize;
        c.skip(6)?; // searchRange, entrySelector, rangeShift
        let mut tables = Vec::with_capacity(num_tables.min(64));
        for _ in 0..num_tables {
            let tag = c.tag()?;
            c.skip(4)?; // checksum, not verified: fonts in the wild get it wrong
            let start = c.u32()? as usize;
            let length = c.u32()? as usize;
            let end = start.checked_add(length).ok_or(FontError::Truncated)?;
            if end > data.len() {
                return Err(FontError::Truncated);
            }
            tables.push((tag, start..end));
        }
        Ok(Self { tables })
    }

    /// How many fonts a file holds: the collection count, or 1 for a plain font.
    pub(crate) fn count(data: &[u8]) -> Result<u32, FontError> {
        let mut c = Cursor::new(data);
        let tag = c.tag()?;
        if &tag == b"ttcf" {
            c.skip(4)?;
            c.u32()
        } else {
            check_sfnt(&tag)?;
            Ok(1)
        }
    }

    /// The byte range of a table, if the font has it.
    pub(crate) fn range(&self, tag: &[u8; 4]) -> Option<Range<usize>> {
        self.tables
            .iter()
            .find(|(t, _)| t == tag)
            .map(|(_, r)| r.clone())
    }

    /// The byte range of a table the parser cannot do without.
    pub(crate) fn require(&self, tag: &[u8; 4]) -> Result<Range<usize>, FontError> {
        self.range(tag).ok_or(FontError::MissingTable(*tag))
    }
}

/// Accept the sfnt version tags that mean "TrueType outlines follow".
fn check_sfnt(tag: &[u8; 4]) -> Result<(), FontError> {
    match tag {
        [0x00, 0x01, 0x00, 0x00] | b"true" => Ok(()),
        b"OTTO" => Err(FontError::Unsupported("CFF outlines are not supported")),
        _ => Err(FontError::NotAFont),
    }
}

// ---------------------------------------------------------------------------
// head, maxp, hhea
// ---------------------------------------------------------------------------

/// The fields of `head` the rest of the parser needs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Head {
    /// Font design units per em; all glyph coordinates are in these.
    pub(crate) units_per_em: u16,
    /// 0 for a short (u16, halved) `loca`, 1 for a long (u32) one.
    pub(crate) long_loca: bool,
}

/// Parse `head`: the magic number, the em size and the `loca` format.
pub(crate) fn parse_head(table: &[u8]) -> Result<Head, FontError> {
    let mut c = Cursor::new(table);
    c.skip(12)?; // version, fontRevision, checkSumAdjustment
    if c.u32()? != 0x5F0F_3CF5 {
        return Err(FontError::Malformed("head magic number is wrong"));
    }
    c.skip(2)?; // flags
    let units_per_em = c.u16()?;
    if !(16..=16384).contains(&units_per_em) {
        return Err(FontError::Malformed("unitsPerEm out of range"));
    }
    c.seek(50)?; // created, modified, bbox, macStyle, lowestRecPPEM, fontDirectionHint
    let index_to_loc_format = c.i16()?;
    let long_loca = match index_to_loc_format {
        0 => false,
        1 => true,
        _ => return Err(FontError::Malformed("unknown indexToLocFormat")),
    };
    Ok(Head {
        units_per_em,
        long_loca,
    })
}

/// Parse `maxp` and return the glyph count.
pub(crate) fn parse_maxp(table: &[u8]) -> Result<u16, FontError> {
    let mut c = Cursor::new(table);
    c.skip(4)?; // version
    c.u16()
}

/// The fields of `hhea` the rest of the parser needs.
pub(crate) struct Hhea {
    pub(crate) ascender: i16,
    pub(crate) descender: i16,
    pub(crate) line_gap: i16,
    /// How many glyphs carry their own advance width in `hmtx`.
    pub(crate) number_of_h_metrics: u16,
}

/// Parse `hhea`: vertical metrics and the length of the `hmtx` advance array.
pub(crate) fn parse_hhea(table: &[u8]) -> Result<Hhea, FontError> {
    let mut c = Cursor::new(table);
    c.skip(4)?; // version
    let ascender = c.i16()?;
    let descender = c.i16()?;
    let line_gap = c.i16()?;
    c.seek(34)?; // advanceWidthMax .. metricDataFormat
    let number_of_h_metrics = c.u16()?;
    if number_of_h_metrics == 0 {
        return Err(FontError::Malformed("hhea claims no horizontal metrics"));
    }
    Ok(Hhea {
        ascender,
        descender,
        line_gap,
        number_of_h_metrics,
    })
}

/// Typographic ascender, descender and line gap from `OS/2`, if it has them.
///
/// Returns `None` for a table too short to hold the typo fields (version 0
/// tables cut off after the panose block are rare but do exist); the caller
/// falls back to `hhea`.
pub(crate) fn parse_os2_metrics(table: &[u8]) -> Option<(i16, i16, i16)> {
    let mut c = Cursor::new(table);
    c.seek(68).ok()?;
    let ascender = c.i16().ok()?;
    let descender = c.i16().ok()?;
    let line_gap = c.i16().ok()?;
    Some((ascender, descender, line_gap))
}

// ---------------------------------------------------------------------------
// hmtx
// ---------------------------------------------------------------------------

/// One glyph's horizontal metrics, in font units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlyphMetrics {
    /// How far the pen moves after drawing this glyph.
    pub advance_width: u16,
    /// Distance from the pen to the left edge of the glyph's outline.
    pub left_side_bearing: i16,
}

/// Parse `hmtx` into one entry per glyph.
///
/// `hmtx` is stored compressed: only the first `number_of_h_metrics` glyphs
/// carry an advance, and every glyph after that repeats the last one while
/// keeping its own left side bearing in a trailing array. Monospaced fonts use
/// that to store a single advance for thousands of glyphs; here the tail is
/// expanded so a lookup is an index.
pub(crate) fn parse_hmtx(
    table: &[u8],
    num_glyphs: u16,
    number_of_h_metrics: u16,
) -> Result<Vec<GlyphMetrics>, FontError> {
    if number_of_h_metrics > num_glyphs {
        return Err(FontError::Malformed("hhea has more metrics than glyphs"));
    }
    let paired = number_of_h_metrics as usize;
    let mut c = Cursor::new(table);
    let mut metrics = Vec::with_capacity(num_glyphs as usize);
    for _ in 0..paired {
        let advance_width = c.u16()?;
        let left_side_bearing = c.i16()?;
        metrics.push(GlyphMetrics {
            advance_width,
            left_side_bearing,
        });
    }
    // The tail: the last advance, plus a bearing each — and fonts do get cut
    // off right here, so a missing bearing is read as zero rather than refused.
    let last = metrics.last().copied().unwrap_or_default().advance_width;
    for i in paired..num_glyphs as usize {
        let at = paired * 4 + (i - paired) * 2;
        metrics.push(GlyphMetrics {
            advance_width: last,
            left_side_bearing: u16_at(table, at) as i16,
        });
    }
    Ok(metrics)
}

// ---------------------------------------------------------------------------
// loca
// ---------------------------------------------------------------------------

/// Parse `loca` into `num_glyphs + 1` offsets into `glyf`.
///
/// Both formats are stored here as byte offsets: the short format keeps them
/// halved, so every entry is doubled on the way in. Offsets are checked against
/// the real size of `glyf` and against each other, which is what makes
/// [`super::Font::raw_glyph`] a plain slice with no further checking.
pub(crate) fn parse_loca(
    table: &[u8],
    num_glyphs: u16,
    long: bool,
    glyf_len: usize,
) -> Result<Vec<u32>, FontError> {
    let count = num_glyphs as usize + 1;
    let mut c = Cursor::new(table);
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = if long { c.u32()? } else { c.u16()? as u32 * 2 };
        if offset as usize > glyf_len {
            return Err(FontError::Malformed("loca offset points past glyf"));
        }
        if let Some(previous) = offsets.last() {
            if offset < *previous {
                return Err(FontError::Malformed("loca offsets go backwards"));
            }
        }
        offsets.push(offset);
    }
    Ok(offsets)
}

// ---------------------------------------------------------------------------
// cmap
// ---------------------------------------------------------------------------

/// The one `cmap` subtable this font is read through.
///
/// `sub` is a range into the whole file, covering the subtable and everything
/// after it inside `cmap` (subtables lie about their own length often enough
/// that the end of the table is the safer bound). `count` is the segment count
/// for format 4 and the group count for format 12; both were checked to fit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Cmap {
    format: u16,
    count: usize,
    sub: Range<usize>,
}

impl Cmap {
    /// Pick a subtable in the order (3,10), (3,1), (0,\*) and validate it.
    ///
    /// Windows full-repertoire (format 12) first, then Windows BMP, then any
    /// Unicode platform record. Encodings in a format we do not read are passed
    /// over rather than refused: a font is unusable only if none is left.
    pub(crate) fn parse(file: &[u8], table: Range<usize>) -> Result<Option<Self>, FontError> {
        let data = &file[table.clone()];
        let mut c = Cursor::new(data);
        c.skip(2)?; // version
        let num_records = c.u16()? as usize;
        let mut records = Vec::with_capacity(num_records.min(32));
        for _ in 0..num_records {
            let platform = c.u16()?;
            let encoding = c.u16()?;
            let offset = c.u32()? as usize;
            records.push((platform, encoding, offset));
        }

        let ranked = |(platform, encoding): (u16, u16)| match (platform, encoding) {
            (3, 10) => Some(0),
            (3, 1) => Some(1),
            (0, _) => Some(2),
            _ => None,
        };
        let mut candidates: Vec<(u8, usize)> = records
            .iter()
            .filter_map(|&(p, e, offset)| ranked((p, e)).map(|rank| (rank, offset)))
            .collect();
        candidates.sort_by_key(|&(rank, _)| rank);

        for (_, offset) in candidates {
            if offset >= data.len() {
                continue;
            }
            let start = table.start + offset;
            match Self::parse_subtable(file, start..table.end) {
                Ok(Some(cmap)) => return Ok(Some(cmap)),
                Ok(None) => continue, // a format we do not read
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// Validate one subtable's arrays; `Ok(None)` for an unread format.
    fn parse_subtable(file: &[u8], sub: Range<usize>) -> Result<Option<Self>, FontError> {
        let data = &file[sub.clone()];
        let mut c = Cursor::new(data);
        let format = c.u16()?;
        match format {
            4 => {
                c.skip(4)?; // length, language
                let seg_count_x2 = c.u16()? as usize;
                if seg_count_x2 == 0 || seg_count_x2 % 2 != 0 {
                    return Err(FontError::Malformed("cmap format 4 segment count"));
                }
                let count = seg_count_x2 / 2;
                // header, endCode[], pad, startCode[], idDelta[], idRangeOffset[]
                let needed = 16 + 8 * count;
                if data.len() < needed {
                    return Err(FontError::Truncated);
                }
                Ok(Some(Self { format, count, sub }))
            }
            12 => {
                c.skip(10)?; // reserved, length, language
                let count = c.u32()? as usize;
                let needed = count
                    .checked_mul(12)
                    .and_then(|n| n.checked_add(16))
                    .ok_or(FontError::Truncated)?;
                if data.len() < needed {
                    return Err(FontError::Truncated);
                }
                Ok(Some(Self { format, count, sub }))
            }
            _ => Ok(None),
        }
    }

    /// The glyph id for a code point, or 0 (`.notdef`) if the font has none.
    pub(crate) fn lookup(&self, file: &[u8], code: u32) -> u16 {
        let data = &file[self.sub.clone()];
        match self.format {
            4 => self.lookup_format4(data, code),
            12 => self.lookup_format12(data, code),
            _ => 0,
        }
    }

    /// Binary search of the segment arrays of a format 4 subtable.
    fn lookup_format4(&self, data: &[u8], code: u32) -> u16 {
        if code > 0xFFFF {
            return 0;
        }
        let code = code as u16;
        let ends = 14;
        let starts = ends + self.count * 2 + 2;
        let deltas = starts + self.count * 2;
        let range_offsets = deltas + self.count * 2;

        // The first segment whose endCode is not below `code`.
        let (mut lo, mut hi) = (0usize, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if u16_at(data, ends + mid * 2) < code {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == self.count {
            return 0;
        }
        let start = u16_at(data, starts + lo * 2);
        if code < start {
            return 0; // inside a gap between two segments
        }
        let delta = u16_at(data, deltas + lo * 2);
        let range_offset = u16_at(data, range_offsets + lo * 2) as usize;
        if range_offset == 0 {
            return code.wrapping_add(delta);
        }
        // idRangeOffset is a byte offset from its own slot into glyphIdArray.
        let at = range_offsets + lo * 2 + range_offset + (code - start) as usize * 2;
        let glyph = u16_at(data, at);
        if glyph == 0 {
            0
        } else {
            glyph.wrapping_add(delta)
        }
    }

    /// Binary search of the group array of a format 12 subtable.
    fn lookup_format12(&self, data: &[u8], code: u32) -> u16 {
        let groups = 16;
        let (mut lo, mut hi) = (0usize, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let at = groups + mid * 12;
            if u32_at(data, at + 4) < code {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == self.count {
            return 0;
        }
        let at = groups + lo * 12;
        let start = u32_at(data, at);
        if code < start {
            return 0;
        }
        let glyph = u32_at(data, at + 8) + (code - start);
        if glyph > u16::MAX as u32 {
            0
        } else {
            glyph as u16
        }
    }
}

// ---------------------------------------------------------------------------
// kern
// ---------------------------------------------------------------------------

/// One kerning pair: how much to move the pen between two glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KernPair {
    /// `(left << 16) | right`, so a pair sorts and compares as one integer.
    pub(crate) key: u32,
    pub(crate) value: i16,
}

/// Parse the format 0 subtables of a `kern` table.
///
/// Only the original (version 0) table with horizontal, non-cross-stream,
/// non-minimum format 0 subtables is read; anything else — Apple's version 1
/// table, state-machine formats — yields no pairs rather than an error, since a
/// font without kerning is still a perfectly good font.
pub(crate) fn parse_kern(table: &[u8]) -> Result<Vec<KernPair>, FontError> {
    let mut c = Cursor::new(table);
    if c.u16()? != 0 {
        return Ok(Vec::new());
    }
    let num_subtables = c.u16()? as usize;
    let mut pairs: Vec<KernPair> = Vec::new();
    let mut at = 4;
    for _ in 0..num_subtables {
        c.seek(at)?;
        c.skip(2)?; // subtable version
        let length = c.u16()? as usize;
        let coverage = c.u16()?;
        if length < 6 {
            break; // a zero length would loop forever
        }
        let horizontal = coverage & 0x1 != 0;
        let minimum = coverage & 0x2 != 0;
        let cross_stream = coverage & 0x4 != 0;
        let format = coverage >> 8;
        if format == 0 && horizontal && !minimum && !cross_stream {
            let num_pairs = c.u16()? as usize;
            c.skip(6)?; // searchRange, entrySelector, rangeShift
            for _ in 0..num_pairs {
                let left = c.u16()? as u32;
                let right = c.u16()? as u32;
                let value = c.i16()?;
                pairs.push(KernPair {
                    key: (left << 16) | right,
                    value,
                });
            }
        }
        at = at.checked_add(length).ok_or(FontError::Truncated)?;
        if at >= table.len() {
            break;
        }
    }
    pairs.sort_by_key(|p| p.key);
    Ok(pairs)
}

/// Look a pair up in the sorted table; 0 when the pair is not kerned.
pub(crate) fn kern_lookup(pairs: &[KernPair], left: u16, right: u16) -> i16 {
    let key = ((left as u32) << 16) | right as u32;
    match pairs.binary_search_by_key(&key, |p| p.key) {
        Ok(i) => pairs[i].value,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::fixture::{self, Builder};

    #[test]
    fn cursor_refuses_to_read_past_the_end() {
        let mut c = Cursor::new(&[0x00, 0x01, 0x02]);
        assert_eq!(c.u16(), Ok(1));
        assert_eq!(c.u16(), Err(FontError::Truncated));
        assert_eq!(c.seek(3), Ok(()));
        assert_eq!(c.seek(4), Err(FontError::Truncated));
        assert_eq!(c.skip(usize::MAX), Err(FontError::Truncated));
    }

    #[test]
    fn loca_reads_both_formats_the_same_way() {
        let short = fixture::loca_short(&[0, 2, 2, 7]);
        let long = fixture::loca_long(&[0, 4, 4, 14]);
        // The short format stores halves, so the same picture needs half the
        // numbers — after parsing both must be byte offsets.
        assert_eq!(
            parse_loca(&short, 3, false, 20).unwrap(),
            vec![0, 4, 4, 14],
            "short offsets are doubled"
        );
        assert_eq!(parse_loca(&long, 3, true, 20).unwrap(), vec![0, 4, 4, 14]);
    }

    #[test]
    fn loca_is_checked_against_the_size_of_glyf() {
        let loca = fixture::loca_long(&[0, 4, 40]);
        assert_eq!(
            parse_loca(&loca, 2, true, 20),
            Err(FontError::Malformed("loca offset points past glyf"))
        );
        let backwards = fixture::loca_long(&[0, 8, 4]);
        assert_eq!(
            parse_loca(&backwards, 2, true, 20),
            Err(FontError::Malformed("loca offsets go backwards"))
        );
        // One offset short of `num_glyphs + 1`.
        assert_eq!(
            parse_loca(&fixture::loca_long(&[0, 4]), 2, true, 20),
            Err(FontError::Truncated)
        );
    }

    #[test]
    fn hmtx_repeats_the_last_advance_over_the_compressed_tail() {
        // Three glyphs, two of them with their own advance; the third takes the
        // second's advance and keeps a bearing of its own.
        let hmtx = fixture::hmtx(&[(500, 10), (600, 20)], &[-7]);
        let metrics = parse_hmtx(&hmtx, 3, 2).unwrap();
        assert_eq!(metrics[0], fixture::metric(500, 10));
        assert_eq!(metrics[1], fixture::metric(600, 20));
        assert_eq!(
            metrics[2],
            fixture::metric(600, -7),
            "the last glyph inherits the advance and keeps its bearing"
        );
    }

    #[test]
    fn hmtx_tail_without_bearings_is_not_an_error() {
        let hmtx = fixture::hmtx(&[(500, 10)], &[]);
        let metrics = parse_hmtx(&hmtx, 3, 1).unwrap();
        assert_eq!(metrics.len(), 3);
        assert_eq!(metrics[2], fixture::metric(500, 0));
        // More metrics than glyphs is a contradiction, though.
        assert!(parse_hmtx(&hmtx, 1, 2).is_err());
    }

    #[test]
    fn cmap_format4_hits_both_ends_of_every_segment() {
        // Two delta segments and one that goes through glyphIdArray.
        let sub = fixture::cmap_format4(&[
            fixture::Seg::Delta(0x0041, 0x005A, 10),  // 'A'..'Z' -> 75..
            fixture::Seg::Delta(0x0410, 0x042F, 100), // 'А'..'Я'
            fixture::Seg::Glyphs(0x2200, vec![7, 0, 9]),
        ]);
        let file = fixture::cmap_table(&[(3, 1, sub)]);
        let cmap = Cmap::parse(&file, 0..file.len()).unwrap().unwrap();

        assert_eq!(cmap.lookup(&file, 0x0041), 0x0041 + 10, "first of segment");
        assert_eq!(cmap.lookup(&file, 0x005A), 0x005A + 10, "last of segment");
        assert_eq!(cmap.lookup(&file, 0x0040), 0, "one below the first");
        assert_eq!(cmap.lookup(&file, 0x005B), 0, "one above the last");
        assert_eq!(cmap.lookup(&file, 0x0410), 0x0410 + 100);
        assert_eq!(cmap.lookup(&file, 0x042F), 0x042F + 100);
        assert_eq!(cmap.lookup(&file, 0x2200), 7, "through glyphIdArray");
        assert_eq!(cmap.lookup(&file, 0x2201), 0, "a hole in glyphIdArray");
        assert_eq!(cmap.lookup(&file, 0x2202), 9);
        assert_eq!(cmap.lookup(&file, 0xFFFF), 0, "the terminator segment");
        assert_eq!(cmap.lookup(&file, 0x1_0000), 0, "above the BMP");
    }

    #[test]
    fn cmap_format12_covers_code_points_above_the_bmp() {
        let sub = fixture::cmap_format12(&[(0x0041, 0x005A, 1), (0x1_F600, 0x1_F60F, 200)]);
        let file = fixture::cmap_table(&[(3, 10, sub)]);
        let cmap = Cmap::parse(&file, 0..file.len()).unwrap().unwrap();
        assert_eq!(cmap.lookup(&file, 0x0041), 1);
        assert_eq!(cmap.lookup(&file, 0x005A), 26);
        assert_eq!(cmap.lookup(&file, 0x005B), 0);
        assert_eq!(cmap.lookup(&file, 0x1_F600), 200);
        assert_eq!(cmap.lookup(&file, 0x1_F60F), 215);
        assert_eq!(cmap.lookup(&file, 0x1_F610), 0);
    }

    #[test]
    fn cmap_prefers_3_10_then_3_1_then_unicode() {
        let bmp = fixture::cmap_format4(&[fixture::Seg::Delta(0x0041, 0x0041, 0)]);
        let full = fixture::cmap_format12(&[(0x0041, 0x0041, 900)]);
        let unicode = fixture::cmap_format4(&[fixture::Seg::Delta(0x0041, 0x0041, 100)]);

        let all =
            fixture::cmap_table(&[(0, 3, unicode.clone()), (3, 1, bmp.clone()), (3, 10, full)]);
        let cmap = Cmap::parse(&all, 0..all.len()).unwrap().unwrap();
        assert_eq!(cmap.lookup(&all, 0x41), 900, "(3,10) wins");

        let no_full = fixture::cmap_table(&[(0, 3, unicode.clone()), (3, 1, bmp)]);
        let cmap = Cmap::parse(&no_full, 0..no_full.len()).unwrap().unwrap();
        assert_eq!(cmap.lookup(&no_full, 0x41), 0x41, "(3,1) beats (0,*)");

        let only_unicode = fixture::cmap_table(&[(0, 3, unicode)]);
        let cmap = Cmap::parse(&only_unicode, 0..only_unicode.len())
            .unwrap()
            .unwrap();
        assert_eq!(cmap.lookup(&only_unicode, 0x41), 0x41 + 100);
    }

    #[test]
    fn cmap_skips_records_in_formats_we_do_not_read() {
        // A Macintosh-style format 6 subtable under a (0,*) record, plus a real
        // format 4 one: the unreadable record must not hide the readable font.
        let mut format6 = vec![0x00, 0x06, 0x00, 0x0a, 0x00, 0x00];
        format6.extend_from_slice(&[0x00, 0x41, 0x00, 0x00]);
        let four = fixture::cmap_format4(&[fixture::Seg::Delta(0x0041, 0x0041, 5)]);
        let file = fixture::cmap_table(&[(0, 0, format6), (3, 1, four)]);
        let cmap = Cmap::parse(&file, 0..file.len()).unwrap().unwrap();
        assert_eq!(cmap.lookup(&file, 0x41), 0x46);

        // And a font whose only Unicode record is unreadable maps nothing.
        let mut only6 = vec![0x00, 0x06, 0x00, 0x0a, 0x00, 0x00];
        only6.extend_from_slice(&[0x00, 0x41, 0x00, 0x00]);
        let file = fixture::cmap_table(&[(0, 0, only6)]);
        assert!(Cmap::parse(&file, 0..file.len()).unwrap().is_none());
    }

    #[test]
    fn cmap_with_a_lying_segment_count_is_an_error() {
        let mut sub = fixture::cmap_format4(&[fixture::Seg::Delta(0x41, 0x41, 0)]);
        // Claim sixty-four segments in a subtable that holds two.
        sub[6] = 0x00;
        sub[7] = 0x80;
        let file = fixture::cmap_table(&[(3, 1, sub)]);
        assert_eq!(Cmap::parse(&file, 0..file.len()), Err(FontError::Truncated));
    }

    #[test]
    fn kern_format0_pairs_are_found_by_binary_search() {
        let pairs = parse_kern(&fixture::kern(&[(5, 9, -40), (1, 2, -15), (5, 1, 20)])).unwrap();
        assert_eq!(kern_lookup(&pairs, 1, 2), -15);
        assert_eq!(kern_lookup(&pairs, 5, 1), 20);
        assert_eq!(kern_lookup(&pairs, 5, 9), -40);
        assert_eq!(kern_lookup(&pairs, 9, 5), 0, "the pair is ordered");
        assert_eq!(kern_lookup(&pairs, 3, 3), 0);
    }

    #[test]
    fn kern_of_an_unknown_version_yields_no_pairs() {
        let mut table = fixture::kern(&[(1, 2, -15)]);
        table[1] = 1; // Apple's version 1 table, which we do not read
        assert!(parse_kern(&table).unwrap().is_empty());
    }

    #[test]
    fn head_rejects_a_file_that_only_looks_like_one() {
        let mut head = fixture::head(2048, true);
        head[12] = 0x00; // break the magic number
        assert_eq!(
            parse_head(&head),
            Err(FontError::Malformed("head magic number is wrong"))
        );

        let mut head = fixture::head(2048, true);
        head[18] = 0x00; // unitsPerEm = 0
        head[19] = 0x00;
        assert_eq!(
            parse_head(&head),
            Err(FontError::Malformed("unitsPerEm out of range"))
        );

        let mut head = fixture::head(2048, true);
        head[51] = 0x07; // indexToLocFormat = 7
        assert_eq!(
            parse_head(&head),
            Err(FontError::Malformed("unknown indexToLocFormat"))
        );
    }

    #[test]
    fn a_table_that_runs_past_the_end_of_the_file_is_refused() {
        let font = Builder::simple().build();
        let mut broken = font.clone();
        // The first directory record's length field, made absurd.
        let record = 12 + 12;
        broken[record..record + 4].copy_from_slice(&0x0010_0000u32.to_be_bytes());
        assert_eq!(Directory::parse(&broken, 0), Err(FontError::Truncated));
        assert!(Directory::parse(&font, 0).is_ok());
    }
}
