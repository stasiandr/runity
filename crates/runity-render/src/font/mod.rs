//! Reading TrueType fonts — the sfnt table soup — without `rusttype`.
//!
//! A [`Font`] reads its own tables ([`tables`]), turns a glyph's `glyf` bytes
//! into contours ([`outline`]) and those contours into 8-bit coverage
//! ([`raster`]). What comes out is a [`GlyphBitmap`]: pixels, a placement
//! relative to the pen, and an advance. Putting those pixels on a surface is
//! the caller's business, and so is deciding what a line of text is.
//!
//! ```no_run
//! use runity_render::font::Font;
//!
//! let font = Font::embedded();
//! let glyph = font.glyph_index('Ж').expect("Roboto covers Cyrillic");
//! let bitmap = font.rasterize(glyph, 16.0).unwrap();
//! println!("{}x{} pixels, {} across", bitmap.width, bitmap.height, bitmap.advance);
//! ```
//!
//! Rasterizing the same glyph at the same size twice costs once: a [`Font`]
//! keeps its bitmaps in a [`RefCell`]-guarded cache, which is why a `Font` is
//! usable from one thread only. The renderer around it is single-threaded by
//! design, so that costs nothing here; [`Font::clear_cache`] and
//! [`Font::cache_bytes`] are there to keep the memory it holds visible.
//!
//! What is supported: TrueType outlines (`glyf`/`loca`), simple and composite
//! glyphs, `.ttc` collections by index, `cmap` formats 4 and 12, and format 0
//! `kern`. What is not: CFF (`.otf`) outlines, which are refused with
//! [`FontError::Unsupported`] rather than parsed into an empty font, and
//! hinting, which this rasterizer answers with a contrast curve instead.

mod outline;
mod raster;
mod tables;
mod text;

#[cfg(test)]
mod fixture;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

pub use outline::{Contour, Outline, Segment, MAX_COMPOSITE_DEPTH};
pub use raster::GlyphBitmap;
pub use tables::GlyphMetrics;
pub use text::{Align, TextSize, TextStyle};

use tables::{Cmap, Directory, KernPair};

/// The largest pixel size a glyph is rasterized at.
///
/// Well past any text on a screen; it is here so that a size out of a
/// configuration file cannot ask for a gigabyte of coverage.
const MAX_SIZE: f32 = 512.0;

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
/// Coordinates and metrics are in font units; the `*_units` accessors give
/// them as the file stores them, and the ones taking a size in pixels — such as
/// [`Font::ascent`] — have already divided by [`Font::units_per_em`].
///
/// A `Font` is not `Sync`: the glyph cache behind [`Font::rasterize`] is a
/// [`RefCell`]. Share one per thread.
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
    /// Rasterized glyphs, keyed by glyph id and size in 64ths of a pixel.
    ///
    /// No atlas image: an atlas saves texture switches on a GPU, and this
    /// blitter walks memory. Bitmaps are handed out as [`Rc`] so that a cache
    /// hit copies a pointer and not a glyph.
    cache: RefCell<HashMap<(u16, u32), Rc<GlyphBitmap>>>,
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
            cache: RefCell::new(HashMap::new()),
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
    pub fn ascender_units(&self) -> i16 {
        self.ascender
    }

    /// Depth below the baseline, in font units — negative in every real font.
    pub fn descender_units(&self) -> i16 {
        self.descender
    }

    /// Extra space the designer wants between two lines, in font units.
    pub fn line_gap_units(&self) -> i16 {
        self.line_gap
    }

    /// Baseline-to-baseline distance in font units: ascent, descent and gap.
    pub fn line_height_units(&self) -> i32 {
        self.ascender as i32 - self.descender as i32 + self.line_gap as i32
    }

    /// Pixels per font unit at a given size — the number every metric here is
    /// multiplied by.
    pub fn scale(&self, size: f32) -> f32 {
        size / self.units_per_em as f32
    }

    /// Height above the baseline at a size in pixels.
    ///
    /// From the typographic fields of `OS/2` where the font has them, falling
    /// back to `hhea`: those are the numbers the designer meant for setting
    /// lines, rather than whatever the tallest glyph happens to reach.
    pub fn ascent(&self, size: f32) -> f32 {
        self.ascender as f32 * self.scale(size)
    }

    /// Depth below the baseline at a size in pixels — negative, going down.
    pub fn descent(&self, size: f32) -> f32 {
        self.descender as f32 * self.scale(size)
    }

    /// Space to leave between one line's descent and the next line's ascent,
    /// in pixels.
    pub fn line_gap(&self, size: f32) -> f32 {
        self.line_gap as f32 * self.scale(size)
    }

    /// Baseline-to-baseline distance in pixels: ascent, descent and gap.
    pub fn line_height(&self, size: f32) -> f32 {
        self.line_height_units() as f32 * self.scale(size)
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

    /// A glyph's contours in font units, with composite components resolved.
    ///
    /// An empty outline is the normal answer for a space or for a glyph id the
    /// font does not have. The errors are about the file: a `glyf` entry that
    /// ends mid-point is [`FontError::Truncated`], and components nested deeper
    /// than [`MAX_COMPOSITE_DEPTH`] — which no honest font is — are
    /// [`FontError::Malformed`].
    pub fn outline(&self, glyph: u16) -> Result<Outline, FontError> {
        outline::build(self, glyph)
    }

    /// Rasterize a glyph at a size in pixels, through the cache.
    ///
    /// The bitmap is 8-bit coverage plus the offsets that place it against the
    /// pen; see [`GlyphBitmap`]. The second call for the same glyph at the same
    /// size does no work at all — it hands back the same [`Rc`] — so a caller
    /// laying out text does not need a cache of its own.
    ///
    /// Sizes are rounded to a 64th of a pixel before they become a cache key,
    /// so a size that wobbles in its last bits does not fill the cache with
    /// copies of one glyph. A size at or below zero, or one past 512 pixels, is
    /// clamped rather than refused.
    pub fn rasterize(&self, glyph: u16, size: f32) -> Result<Rc<GlyphBitmap>, FontError> {
        let size = quantize(size);
        let key = (glyph, (size * 64.0) as u32);
        if let Some(cached) = self.cache.borrow().get(&key) {
            return Ok(Rc::clone(cached));
        }

        let outline = self.outline(glyph)?;
        let scale = self.scale(size);
        let advance = self.glyph_metrics(glyph).unwrap_or_default().advance_width as f32 * scale;
        let bitmap = Rc::new(raster::rasterize(&outline, scale, advance));
        self.cache.borrow_mut().insert(key, Rc::clone(&bitmap));
        Ok(bitmap)
    }

    /// Throw away every rasterized glyph held for this font.
    ///
    /// Bitmaps a caller still holds an [`Rc`] to stay alive until it drops them.
    pub fn clear_cache(&self) {
        self.cache.borrow_mut().clear();
    }

    /// How many glyph bitmaps the cache is holding.
    pub fn cached_glyphs(&self) -> usize {
        self.cache.borrow().len()
    }

    /// Roughly how much memory those bitmaps take, in bytes.
    ///
    /// Coverage plus the header of each bitmap and its key; near enough to
    /// watch a cache grow, not an allocator's own accounting.
    pub fn cache_bytes(&self) -> usize {
        let entry = std::mem::size_of::<(u16, u32)>() + std::mem::size_of::<Rc<GlyphBitmap>>();
        self.cache
            .borrow()
            .values()
            .map(|bitmap| bitmap.bytes() + entry)
            .sum()
    }
}

/// A size in pixels, clamped to what this rasterizer will draw and rounded to
/// the 64th of a pixel the cache is keyed by.
fn quantize(size: f32) -> f32 {
    if !size.is_finite() {
        return 0.0;
    }
    (size.clamp(0.0, MAX_SIZE) * 64.0).round() / 64.0
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
            .field("cached_glyphs", &self.cached_glyphs())
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
        assert!(font.ascender_units() > 0 && font.descender_units() < 0);
        assert!(font.line_height_units() > font.units_per_em() as i32 / 2);
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
                with_os2.ascender_units(),
                with_os2.descender_units(),
                with_os2.line_gap_units()
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
                empty_os2.ascender_units(),
                empty_os2.descender_units(),
                empty_os2.line_gap_units()
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
        assert_eq!(font.ascender_units(), 1000);
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

    /// Total coverage of a bitmap, in whole pixels of ink.
    fn ink(bitmap: &GlyphBitmap) -> f32 {
        bitmap.coverage.iter().map(|&c| c as f32).sum::<f32>() / 255.0
    }

    #[test]
    fn a_rasterized_glyph_is_placed_against_the_pen_and_carries_its_advance() {
        let font = Font::embedded();
        let glyph = font.glyph_index('H').expect("Roboto has an H");
        let bitmap = font.rasterize(glyph, 40.0).unwrap();

        assert!(
            bitmap.width > 10 && bitmap.height > 20,
            "{bitmap:?} is tiny"
        );
        assert_eq!(bitmap.coverage.len(), bitmap.width * bitmap.height);
        // An H sits on the baseline and reaches the cap height above it.
        assert!(bitmap.top > 0 && (bitmap.top as f32) < font.ascent(40.0));
        assert_eq!(bitmap.height as i32, bitmap.top, "an H has nothing below");
        assert!(bitmap.left >= 0);
        let advance = font.glyph_metrics(glyph).unwrap().advance_width as f32;
        assert!((bitmap.advance - advance * font.scale(40.0)).abs() < 1e-3);

        // The middle of a stem is solid and the space beside the letter is not.
        let middle = bitmap.height / 2;
        assert_eq!(bitmap.coverage_at(1, middle), 255, "the left stem");
        assert_eq!(
            bitmap.coverage_at(bitmap.width / 2, 1),
            0,
            "between the stems, above the crossbar"
        );
    }

    #[test]
    fn a_space_rasterizes_to_no_pixels_and_still_moves_the_pen() {
        let font = Font::embedded();
        let space = font.glyph_index(' ').unwrap();
        let bitmap = font.rasterize(space, 24.0).unwrap();
        assert!(bitmap.is_empty());
        assert!(bitmap.advance > 3.0, "a space is not zero-width");
    }

    #[test]
    fn the_second_call_for_a_glyph_comes_out_of_the_cache() {
        let font = Font::embedded();
        let glyph = font.glyph_index('g').unwrap();
        assert_eq!(font.cache_bytes(), 0, "nothing is rasterized up front");

        let first = font.rasterize(glyph, 18.0).unwrap();
        let filled = font.cache_bytes();
        assert!(filled > first.coverage.len(), "the cache holds the bitmap");

        let again = font.rasterize(glyph, 18.0).unwrap();
        assert!(Rc::ptr_eq(&first, &again), "the same bitmap, not a copy");
        assert_eq!(font.cache_bytes(), filled, "and no second entry");
        assert_eq!(font.cached_glyphs(), 1);

        // A size that differs by less than a 64th of a pixel is the same key;
        // a different size is a different glyph to rasterize.
        let nudged = font.rasterize(glyph, 18.001).unwrap();
        assert!(Rc::ptr_eq(&first, &nudged));
        let bigger = font.rasterize(glyph, 19.0).unwrap();
        assert!(!Rc::ptr_eq(&first, &bigger));
        assert_eq!(font.cached_glyphs(), 2);
        assert!(font.cache_bytes() > filled);

        font.clear_cache();
        assert_eq!(font.cache_bytes(), 0);
        assert_eq!(font.cached_glyphs(), 0);
        // The bitmap handed out earlier outlives the cache it came from.
        assert!(!first.coverage.is_empty());
    }

    #[test]
    fn the_same_glyph_twice_the_size_carries_four_times_the_ink() {
        let font = Font::embedded();
        for ch in ['o', 'M', '8', 'Щ'] {
            let glyph = font.glyph_index(ch).unwrap();
            let small = ink(&font.rasterize(glyph, 24.0).unwrap());
            let large = ink(&font.rasterize(glyph, 48.0).unwrap());
            let ratio = large / small;
            assert!(
                (3.5..4.5).contains(&ratio),
                "{ch:?} went from {small} to {large} pixels of ink, a factor of {ratio}"
            );
        }
    }

    #[test]
    fn the_inside_of_a_glyph_is_solid_and_the_outside_is_empty() {
        // A capital O at a size where the ring is several pixels thick: the
        // middle of the ring is ink, the counter inside it is not, and neither
        // are the corners of the bitmap the round letter never reaches.
        let font = Font::embedded();
        let bitmap = font
            .rasterize(font.glyph_index('O').unwrap(), 64.0)
            .unwrap();
        let (w, h) = (bitmap.width, bitmap.height);
        assert_eq!(bitmap.coverage_at(w / 2, h / 2), 0, "the counter is empty");
        assert_eq!(bitmap.coverage_at(w / 2, 1), 255, "the top of the ring");
        assert_eq!(bitmap.coverage_at(w / 2, h - 2), 255, "and the bottom");
        assert_eq!(bitmap.coverage_at(1, h / 2), 255, "the left of the ring");
        assert_eq!(
            bitmap.coverage_at(0, 0),
            0,
            "the corner it curves away from"
        );
        assert_eq!(bitmap.coverage_at(w - 1, h - 1), 0);
    }

    #[test]
    fn the_composite_yo_is_the_letter_e_with_two_dots_over_it() {
        // Ё is a composite glyph: Е placed at the origin and a dieresis placed
        // above it. If the component transform were wrong the letter underneath
        // would be shifted, so it is compared with a plain Е pixel by pixel.
        let font = Font::embedded();
        let size = 48.0;
        let e = font
            .rasterize(font.glyph_index('Е').unwrap(), size)
            .unwrap();
        let yo = font
            .rasterize(font.glyph_index('Ё').unwrap(), size)
            .unwrap();

        assert_eq!(e.left, yo.left, "the same left side bearing");
        assert_eq!(e.advance, yo.advance);
        assert!(yo.top > e.top, "the dots reach above the cap height");
        assert!(yo.height > e.height);
        assert_eq!(e.width, yo.width, "and no wider than the letter");

        // Rows of Ё below the dots line up with Е once both are hung off the
        // baseline, which is what `top` measures.
        let offset = (yo.top - e.top) as usize;
        let mut differing = 0;
        for y in 0..e.height {
            for x in 0..e.width {
                if e.coverage_at(x, y) != yo.coverage_at(x, y + offset) {
                    differing += 1;
                }
            }
        }
        assert_eq!(differing, 0, "the Е inside Ё sits exactly where Е does");

        // And the dots themselves are ink, in the rows above the letter.
        let dots: u32 = (0..offset)
            .flat_map(|y| (0..yo.width).map(move |x| (x, y)))
            .map(|(x, y)| yo.coverage_at(x, y) as u32)
            .sum();
        assert!(dots > 0, "the dieresis rasterized to nothing");
    }

    #[test]
    fn scaled_vertical_metrics_follow_the_size() {
        let font = Font::embedded();
        let em = font.units_per_em() as f32;
        assert_eq!(
            font.scale(em),
            1.0,
            "a size of one em is one unit per pixel"
        );

        assert!((font.ascent(16.0) - font.ascender_units() as f32 * 16.0 / em).abs() < 1e-4);
        assert!(font.ascent(16.0) > 0.0);
        assert!(font.descent(16.0) < 0.0, "descent goes below the baseline");
        assert!(font.line_gap(16.0) >= 0.0);
        assert!(
            (font.line_height(16.0)
                - (font.ascent(16.0) - font.descent(16.0) + font.line_gap(16.0)))
            .abs()
                < 1e-4
        );
        assert!(font.line_height(16.0) > 16.0, "lines must not overlap");

        // Twice the size is twice everything.
        assert!((font.ascent(32.0) - 2.0 * font.ascent(16.0)).abs() < 1e-4);
        assert!((font.line_height(32.0) - 2.0 * font.line_height(16.0)).abs() < 1e-4);
    }

    #[test]
    fn a_size_that_makes_no_sense_gives_an_empty_glyph_and_not_a_panic() {
        let font = Font::embedded();
        let glyph = font.glyph_index('A').unwrap();
        for size in [0.0, -12.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let bitmap = font.rasterize(glyph, size).unwrap();
            assert!(bitmap.is_empty(), "size {size} drew something");
        }
        // Past the cap, a glyph still comes out, at the cap.
        let huge = font.rasterize(glyph, 10_000.0).unwrap();
        let capped = font.rasterize(glyph, MAX_SIZE).unwrap();
        assert!(Rc::ptr_eq(&huge, &capped));
        assert!(huge.height < 1000);
    }

    #[test]
    fn rasterizing_a_broken_glyph_is_an_error_and_not_a_panic() {
        let font = Font::from_bytes(
            Builder::simple()
                .with_glyphs(&[
                    vec![],
                    fixture::glyf_composite(&[fixture::Comp::at(1, 0, 0)]),
                ])
                .build(),
        )
        .unwrap();
        assert_eq!(
            font.rasterize(1, 20.0).unwrap_err(),
            FontError::Malformed("composite glyphs nested too deep")
        );
        assert_eq!(font.cached_glyphs(), 0, "a failure is not cached");
    }

    #[test]
    fn every_glyph_of_the_embedded_font_rasterizes() {
        // Not the whole font at a readable size — that is a second of work; a
        // stride through it at a small size still walks every code path.
        let font = Font::embedded();
        for glyph in (0..font.num_glyphs()).step_by(7) {
            let bitmap = font
                .rasterize(glyph, 11.0)
                .unwrap_or_else(|e| panic!("glyph {glyph}: {e}"));
            assert_eq!(bitmap.coverage.len(), bitmap.width * bitmap.height);
        }
        assert!(font.cache_bytes() > 0);
    }

    #[test]
    fn debug_does_not_print_half_a_megabyte() {
        let text = format!("{:?}", Font::embedded());
        assert!(text.contains("units_per_em: 2048"));
        assert!(text.len() < 200, "Debug prints the numbers, not the bytes");
    }
}
