//! Reading TrueType fonts — the sfnt table soup — without `rusttype`.
//!
//! This module only reads numbers. A [`Font`] knows how big its em is, which
//! glyph a character maps to, how wide that glyph is, and where its outline
//! lives in the `glyf` table; it does not know what the outline *means*. Turning
//! [`Font::raw_glyph`]'s bytes into contours and then into pixels is the next
//! layer's job.
//!
//! ```no_run
//! use runity_render::font::Font;
//!
//! let font = Font::embedded();
//! let glyph = font.glyph_index('Ж').expect("Roboto covers Cyrillic");
//! let metrics = font.glyph_metrics(glyph).unwrap();
//! println!("{} units wide of {}", metrics.advance_width, font.units_per_em());
//! ```
//!
//! What is supported: TrueType outlines (`glyf`/`loca`), `.ttc` collections by
//! index, `cmap` formats 4 and 12, and format 0 `kern`. What is not: CFF
//! (`.otf`) outlines, which are refused with
//! [`FontError::Unsupported`] rather than parsed into an empty font.

mod tables;

#[cfg(test)]
mod fixture;

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::ops::Range;
use std::path::Path;

pub use tables::GlyphMetrics;

use tables::{Cmap, Directory, KernPair};

/// Roboto Regular, Apache-2.0, as shipped in `assets/` — the font
/// [`Font::embedded`] parses. Exposed so a caller can keep the bytes without
/// going through the file system.
pub const ROBOTO_REGULAR: &[u8] = include_bytes!("../../assets/Roboto-Regular.ttf");

/// Everything that can go wrong while reading a font file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// The file does not start with a recognized sfnt version tag.
    NotAFont,
    /// A table, or a structure inside one, runs past the end of the file.
    Truncated,
    /// A table the parser cannot work without is absent.
    MissingTable([u8; 4]),
    /// The bytes are a font's shape but not a font's contents.
    Malformed(&'static str),
    /// A font this parser deliberately does not read.
    Unsupported(&'static str),
    /// A collection was indexed past its last font.
    NoSuchFont { index: u32, count: u32 },
    /// The file could not be read at all.
    Io(io::ErrorKind),
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontError::NotAFont => write!(f, "not a TrueType font"),
            FontError::Truncated => write!(f, "font file ends mid-table"),
            FontError::MissingTable(tag) => {
                write!(f, "font has no {} table", String::from_utf8_lossy(tag))
            }
            FontError::Malformed(what) => write!(f, "malformed font: {what}"),
            FontError::Unsupported(what) => write!(f, "unsupported font: {what}"),
            FontError::NoSuchFont { index, count } => {
                write!(f, "font {index} of a collection that holds {count}")
            }
            FontError::Io(kind) => write!(f, "cannot read font file: {kind:?}"),
        }
    }
}

impl std::error::Error for FontError {}

/// A parsed TrueType font: its tables, resolved.
///
/// The file's bytes are kept — borrowed for [`Font::embedded`], owned for
/// [`Font::load`] — because `glyf` is handed out as slices of them. Everything
/// else (`loca`, `hmtx`, `kern`) is parsed up front, so a lookup is an index or
/// a binary search and never a parse.
///
/// Coordinates and metrics are in font units; divide by [`Font::units_per_em`]
/// to get ems.
pub struct Font {
    data: Cow<'static, [u8]>,
    units_per_em: u16,
    num_glyphs: u16,
    ascender: i16,
    descender: i16,
    line_gap: i16,
    glyf: Range<usize>,
    /// `num_glyphs + 1` byte offsets into `glyf`, checked against its length.
    loca: Vec<u32>,
    /// One entry per glyph: the compressed `hmtx` tail is expanded.
    hmtx: Vec<GlyphMetrics>,
    cmap: Option<Cmap>,
    kern: Vec<KernPair>,
}

impl Font {
    /// The Roboto Regular compiled into the binary.
    ///
    /// The asset is ours and is parsed by the test suite on every run, so this
    /// does not hand back a `Result`. It panics only if the bytes in
    /// [`ROBOTO_REGULAR`] stop being a font, which no runtime input can cause.
    pub fn embedded() -> Font {
        Font::from_bytes(ROBOTO_REGULAR).expect("the embedded Roboto is a valid TrueType font")
    }

    /// Read a `.ttf` from disk; for a `.ttc`, this opens the first font.
    pub fn load(path: impl AsRef<Path>) -> Result<Font, FontError> {
        Font::load_indexed(path, 0)
    }

    /// Read font number `index` of a file — 0 for a plain `.ttf`, any font of
    /// a `.ttc` collection.
    pub fn load_indexed(path: impl AsRef<Path>, index: u32) -> Result<Font, FontError> {
        let bytes = std::fs::read(path).map_err(|e| FontError::Io(e.kind()))?;
        Font::from_bytes_indexed(bytes, index)
    }

    /// Parse a font already in memory; for a `.ttc`, the first font.
    pub fn from_bytes(data: impl Into<Cow<'static, [u8]>>) -> Result<Font, FontError> {
        Font::from_bytes_indexed(data, 0)
    }

    /// Parse font number `index` of bytes already in memory.
    pub fn from_bytes_indexed(
        data: impl Into<Cow<'static, [u8]>>,
        index: u32,
    ) -> Result<Font, FontError> {
        let data = data.into();
        let directory = Directory::parse(&data, index)?;

        // An `.otf` usually says so in its version tag, but a CFF table under a
        // TrueType tag exists too, and would otherwise look like a font with no
        // glyphs at all.
        if directory.range(b"glyf").is_none()
            && (directory.range(b"CFF ").is_some() || directory.range(b"CFF2").is_some())
        {
            return Err(FontError::Unsupported("CFF outlines are not supported"));
        }

        let head = tables::parse_head(&data[directory.require(b"head")?])?;
        let num_glyphs = tables::parse_maxp(&data[directory.require(b"maxp")?])?;
        let hhea = tables::parse_hhea(&data[directory.require(b"hhea")?])?;
        let hmtx = tables::parse_hmtx(
            &data[directory.require(b"hmtx")?],
            num_glyphs,
            hhea.number_of_h_metrics,
        )?;

        let glyf = directory.require(b"glyf")?;
        let loca = tables::parse_loca(
            &data[directory.require(b"loca")?],
            num_glyphs,
            head.long_loca,
            glyf.len(),
        )?;

        let cmap = match directory.range(b"cmap") {
            Some(range) => Cmap::parse(&data, range)?,
            None => None,
        };

        // OS/2 carries the metrics the type designer intended for line layout;
        // hhea's are what the font's own bounding box came out to.
        let (ascender, descender, line_gap) = directory
            .range(b"OS/2")
            .and_then(|range| tables::parse_os2_metrics(&data[range]))
            .filter(|&(a, d, _)| a != 0 || d != 0)
            .unwrap_or((hhea.ascender, hhea.descender, hhea.line_gap));

        let kern = match directory.range(b"kern") {
            Some(range) => tables::parse_kern(&data[range])?,
            None => Vec::new(),
        };

        Ok(Font {
            data,
            units_per_em: head.units_per_em,
            num_glyphs,
            ascender,
            descender,
            line_gap,
            glyf,
            loca,
            hmtx,
            cmap,
            kern,
        })
    }

    /// How many fonts a file holds: the count of a `.ttc`, or 1 for a `.ttf`.
    pub fn collection_count(data: &[u8]) -> Result<u32, FontError> {
        Directory::count(data)
    }

    /// Font design units per em — the unit every other number here is in.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// How many glyphs the font has; glyph ids run `0..num_glyphs`.
    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    /// Height of the tallest glyph above the baseline, in font units.
    pub fn ascender(&self) -> i16 {
        self.ascender
    }

    /// Depth below the baseline, in font units — negative in every real font.
    pub fn descender(&self) -> i16 {
        self.descender
    }

    /// Extra space the designer wants between two lines, in font units.
    pub fn line_gap(&self) -> i16 {
        self.line_gap
    }

    /// Baseline-to-baseline distance in font units: ascent, descent and gap.
    pub fn line_height(&self) -> i32 {
        self.ascender as i32 - self.descender as i32 + self.line_gap as i32
    }

    /// The glyph a character maps to, or `None` if the font has no glyph for it.
    ///
    /// `.notdef` (glyph 0) is reported as `None`: a font that answers with the
    /// empty box is a font that does not have the character.
    pub fn glyph_index(&self, ch: char) -> Option<u16> {
        let cmap = self.cmap.as_ref()?;
        match cmap.lookup(&self.data, ch as u32) {
            0 => None,
            glyph => Some(glyph),
        }
    }

    /// Advance width and left side bearing of a glyph, in font units.
    ///
    /// `None` for a glyph id the font does not have.
    pub fn glyph_metrics(&self, glyph: u16) -> Option<GlyphMetrics> {
        self.hmtx.get(glyph as usize).copied()
    }

    /// How much to move the pen between two glyphs, in font units.
    ///
    /// Reads the old `kern` table only; fonts that keep their kerning in `GPOS`
    /// (most modern ones, Roboto included) return 0 here.
    pub fn kerning(&self, left: u16, right: u16) -> i16 {
        tables::kern_lookup(&self.kern, left, right)
    }

    /// The unparsed `glyf` entry for a glyph — the boundary with the layer that
    /// builds outlines.
    ///
    /// What is promised: the slice is exactly the bytes `loca` assigns to this
    /// glyph, inside `glyf`, with both offsets already checked against the real
    /// size of the table. An empty slice means an empty glyph (a space, and any
    /// glyph whose `loca` entry has zero length) — that is normal, not an error.
    /// Out-of-range glyph ids also give an empty slice.
    ///
    /// What is *not* promised: nothing inside the slice has been looked at. The
    /// caller reads the 10-byte glyph header itself — `numberOfContours` first,
    /// negative for a composite glyph — and is responsible for its own bounds
    /// checking from there. Composites are not resolved here; this module never
    /// follows a component reference.
    pub fn raw_glyph(&self, glyph: u16) -> &[u8] {
        let index = glyph as usize;
        let (Some(&start), Some(&end)) = (self.loca.get(index), self.loca.get(index + 1)) else {
            return &[];
        };
        let start = self.glyf.start + start as usize;
        let end = self.glyf.start + end as usize;
        &self.data[start..end]
    }
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("units_per_em", &self.units_per_em)
            .field("num_glyphs", &self.num_glyphs)
            .field("ascender", &self.ascender)
            .field("descender", &self.descender)
            .field("line_gap", &self.line_gap)
            .field("bytes", &self.data.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{self, Builder};
    use super::*;

    #[test]
    fn embedded_roboto_parses() {
        let font = Font::embedded();
        assert_eq!(font.units_per_em(), 2048);
        assert!(font.num_glyphs() > 1000, "Roboto is not a five-glyph font");
        assert!(font.ascender() > 0 && font.descender() < 0);
        assert!(font.line_height() > font.units_per_em() as i32 / 2);
    }

    #[test]
    fn embedded_roboto_maps_characters_to_glyphs_with_widths() {
        let font = Font::embedded();
        let a = font.glyph_index('A').expect("has 'A'");
        let space = font.glyph_index(' ').expect("has a space");
        assert_ne!(a, space);
        assert!(font.glyph_metrics(a).unwrap().advance_width > 0);
        assert!(
            font.glyph_metrics(space).unwrap().advance_width > 0,
            "a space is empty but not zero-width"
        );
        assert!(!font.raw_glyph(a).is_empty(), "'A' has an outline");
        assert!(font.raw_glyph(space).is_empty(), "a space has none");
        assert!(font.glyph_index('\u{10FFFF}').is_none());
    }

    #[test]
    fn raw_glyph_slices_stay_inside_glyf() {
        let font = Font::embedded();
        for glyph in 0..font.num_glyphs() {
            let bytes = font.raw_glyph(glyph);
            if !bytes.is_empty() {
                assert!(
                    bytes.len() >= 10,
                    "glyph {glyph} is too short to hold a header"
                );
            }
        }
        assert!(font.raw_glyph(font.num_glyphs()).is_empty(), "past the end");
        assert!(font.raw_glyph(u16::MAX).is_empty());
    }

    #[test]
    fn glyf_bytes_match_what_loca_says() {
        let font = Font::from_bytes(Builder::simple().build()).unwrap();
        assert_eq!(font.raw_glyph(0).len(), 0, "glyph 0 is empty");
        assert_eq!(font.raw_glyph(1).len(), 12);
        assert_eq!(font.raw_glyph(2).len(), 20);
        assert_eq!(&font.raw_glyph(1)[..2], &[0x00, 0x01]);
        assert_eq!(&font.raw_glyph(2)[..2], &[0xFF, 0xFF], "a composite glyph");
    }

    #[test]
    fn a_short_loca_names_the_same_glyphs_as_a_long_one() {
        let long = Font::from_bytes(Builder::simple().build()).unwrap();
        let short = Font::from_bytes(
            Builder::simple()
                .with(b"head", fixture::head(1000, false))
                .with(b"loca", fixture::loca_short(&[0, 0, 6, 16]))
                .build(),
        )
        .unwrap();
        assert_eq!(short.raw_glyph(1), long.raw_glyph(1));
        assert_eq!(short.raw_glyph(2), long.raw_glyph(2));
    }

    #[test]
    fn os2_typo_metrics_win_over_hhea() {
        let with_os2 = Font::from_bytes(
            Builder::simple()
                .with(b"OS/2", fixture::os2(1500, -400, 60))
                .build(),
        )
        .unwrap();
        assert_eq!(
            (
                with_os2.ascender(),
                with_os2.descender(),
                with_os2.line_gap()
            ),
            (1500, -400, 60)
        );

        // An OS/2 with empty typo fields falls back to hhea rather than
        // flattening the font to zero height.
        let empty_os2 = Font::from_bytes(
            Builder::simple()
                .with(b"OS/2", fixture::os2(0, 0, 0))
                .build(),
        )
        .unwrap();
        assert_eq!(
            (
                empty_os2.ascender(),
                empty_os2.descender(),
                empty_os2.line_gap()
            ),
            (800, -200, 100)
        );
    }

    #[test]
    fn kerning_comes_from_a_kern_table_when_there_is_one() {
        let font = Font::from_bytes(
            Builder::simple()
                .with(b"kern", fixture::kern(&[(1, 2, -80)]))
                .build(),
        )
        .unwrap();
        assert_eq!(font.kerning(1, 2), -80);
        assert_eq!(font.kerning(2, 1), 0);
        // Roboto keeps its kerning in GPOS, so it answers zero and says so.
        assert_eq!(Font::embedded().kerning(1, 2), 0);
    }

    #[test]
    fn a_collection_is_opened_by_index() {
        let second = Builder::simple()
            .with(b"head", fixture::head(2048, true))
            .with(b"hhea", fixture::hhea(1000, -300, 0, 2));
        let ttc = fixture::collection(&[Builder::simple(), second]);

        assert_eq!(Font::collection_count(&ttc), Ok(2));
        assert_eq!(
            Font::from_bytes_indexed(ttc.clone(), 0)
                .unwrap()
                .units_per_em(),
            1000
        );
        let font = Font::from_bytes_indexed(ttc.clone(), 1).unwrap();
        assert_eq!(font.units_per_em(), 2048);
        assert_eq!(font.ascender(), 1000);
        assert_eq!(font.raw_glyph(1).len(), 12, "the second font's own glyf");

        assert_eq!(
            Font::from_bytes_indexed(ttc, 2).unwrap_err(),
            FontError::NoSuchFont { index: 2, count: 2 }
        );
        // A plain font holds exactly one, and index 1 of it does not exist.
        let ttf = Builder::simple().build();
        assert_eq!(Font::collection_count(&ttf), Ok(1));
        assert_eq!(
            Font::from_bytes_indexed(ttf, 1).unwrap_err(),
            FontError::NoSuchFont { index: 1, count: 1 }
        );
    }

    #[test]
    fn a_ttc_on_disk_opens_by_index() {
        let dir = std::env::temp_dir().join("runity-font-ttc");
        std::fs::create_dir_all(&dir).expect("temp dir");
        // Named after the process, so two suites running at once do not share
        // a file.
        let path = dir.join(format!("pair-{}.ttc", std::process::id()));
        let second = Builder::simple().with(b"head", fixture::head(2048, true));
        std::fs::write(&path, fixture::collection(&[Builder::simple(), second])).expect("write");

        assert_eq!(Font::load(&path).unwrap().units_per_em(), 1000);
        assert_eq!(Font::load_indexed(&path, 1).unwrap().units_per_em(), 2048);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_is_an_error_and_not_a_panic() {
        let err = Font::load("no/such/font.ttf").unwrap_err();
        assert_eq!(err, FontError::Io(io::ErrorKind::NotFound));
    }

    #[test]
    fn cff_fonts_are_refused_by_name() {
        // An `.otf` as it comes: the OTTO version tag.
        let otto = Builder::simple()
            .without(b"glyf")
            .without(b"loca")
            .with(b"CFF ", vec![0; 64])
            .build_tagged(*b"OTTO", 0);
        assert_eq!(
            Font::from_bytes(otto).unwrap_err(),
            FontError::Unsupported("CFF outlines are not supported")
        );

        // And a CFF table hiding under a TrueType version tag.
        let disguised = Builder::simple()
            .without(b"glyf")
            .without(b"loca")
            .with(b"CFF ", vec![0; 64])
            .build();
        assert_eq!(
            Font::from_bytes(disguised).unwrap_err(),
            FontError::Unsupported("CFF outlines are not supported")
        );
    }

    #[test]
    fn files_that_are_not_fonts_are_refused() {
        assert_eq!(
            Font::from_bytes(&b""[..]).unwrap_err(),
            FontError::Truncated
        );
        assert_eq!(
            Font::from_bytes(&b"not a font at all, just some bytes"[..]).unwrap_err(),
            FontError::NotAFont
        );
        // A PNG, which is the other file this crate reads.
        assert_eq!(
            Font::from_bytes(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a][..]).unwrap_err(),
            FontError::NotAFont
        );
    }

    #[test]
    fn a_font_without_the_tables_it_needs_is_refused() {
        for tag in [b"head", b"maxp", b"hhea", b"hmtx", b"loca", b"glyf"] {
            let font = Builder::simple().without(tag).build();
            assert_eq!(
                Font::from_bytes(font).unwrap_err(),
                FontError::MissingTable(*tag),
                "a font without {} must say so",
                String::from_utf8_lossy(tag)
            );
        }
        // A font without cmap is readable, it just maps nothing.
        let font = Font::from_bytes(Builder::simple().without(b"cmap").build()).unwrap();
        assert!(font.glyph_index('A').is_none());
        assert_eq!(font.raw_glyph(1).len(), 12);
    }

    #[test]
    fn truncating_a_font_anywhere_gives_an_error_and_not_a_panic() {
        let font = Builder::simple().build();
        for end in 0..font.len() {
            let result = Font::from_bytes(font[..end].to_vec());
            assert!(result.is_err(), "{end} bytes of a font must not parse");
        }
        assert!(Font::from_bytes(font).is_ok(), "all of it still parses");
    }

    #[test]
    fn truncating_the_embedded_roboto_gives_an_error_and_not_a_panic() {
        // Every length is too slow for a half-megabyte file; a coarse sweep
        // plus the ends of every table still walks every parser in the module.
        let mut lengths: Vec<usize> = (0..ROBOTO_REGULAR.len()).step_by(4093).collect();
        lengths.extend([1, 2, 3, 11, 12, 13, 27, 44, 300, 410, 540, 20_875, 44_629]);
        for end in lengths {
            let result = Font::from_bytes(ROBOTO_REGULAR[..end].to_vec());
            assert!(result.is_err(), "{end} bytes of Roboto must not parse");
        }
    }

    #[test]
    fn a_font_with_its_offsets_stirred_gives_an_error_and_not_a_panic() {
        let font = Builder::simple().build();
        // Reverse every four-byte field of every directory record: offsets and
        // lengths come out enormous or misaligned, tags stop being tags.
        for record in 0..(font.len() - 12) / 16 {
            for field in 0..4 {
                let mut broken = font.clone();
                let at = 12 + record * 16 + field * 4;
                broken[at..at + 4].reverse();
                // Not an assertion about which error: the point is that some
                // Result comes back at all, from any of these.
                let _ = Font::from_bytes(broken);
            }
        }

        // Named cases, where the error is worth pinning down.
        let mut lying_length = font.clone();
        lying_length[12 + 12..12 + 16].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        assert_eq!(
            Font::from_bytes(lying_length).unwrap_err(),
            FontError::Truncated
        );

        let mut lying_offset = font.clone();
        lying_offset[12 + 8..12 + 12].copy_from_slice(&0xFFFF_0000u32.to_be_bytes());
        assert_eq!(
            Font::from_bytes(lying_offset).unwrap_err(),
            FontError::Truncated
        );
    }

    #[test]
    fn a_loca_that_points_outside_glyf_is_an_error() {
        let font = Builder::simple()
            .with(b"loca", fixture::loca_long(&[0, 0, 12, 4096]))
            .build();
        assert_eq!(
            Font::from_bytes(font).unwrap_err(),
            FontError::Malformed("loca offset points past glyf")
        );
    }

    #[test]
    fn a_maxp_that_claims_more_glyphs_than_loca_has_is_an_error() {
        let font = Builder::simple().with(b"maxp", fixture::maxp(4000)).build();
        assert_eq!(Font::from_bytes(font).unwrap_err(), FontError::Truncated);
    }

    #[test]
    fn every_byte_of_a_cmap_flipped_still_only_returns() {
        // The lookup path is the one that trusts offsets inside a table, so it
        // gets its own sweep: a glyph id or nothing, never a panic.
        let base = Builder::simple().build();
        let font = Font::from_bytes(base).unwrap();
        for ch in ['A', 'B', 'C', '\0', '\u{FFFF}', '\u{10FFFF}'] {
            let _ = font.glyph_index(ch);
        }

        let sub = fixture::cmap_format4(&[fixture::Seg::Glyphs(0x41, vec![1, 2])]);
        for byte in 0..sub.len() {
            let mut broken = sub.clone();
            broken[byte] ^= 0xFF;
            let table = fixture::cmap_table(&[(3, 1, broken)]);
            if let Ok(font) = Font::from_bytes(Builder::simple().with(b"cmap", table).build()) {
                for ch in ['A', 'B', 'C', 'Ж', '\u{10FFFF}'] {
                    let _ = font.glyph_index(ch);
                }
            }
        }
    }

    #[test]
    fn debug_does_not_print_half_a_megabyte() {
        let text = format!("{:?}", Font::embedded());
        assert!(text.contains("units_per_em: 2048"));
        assert!(text.len() < 200, "Debug prints the numbers, not the bytes");
    }
}
