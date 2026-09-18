//! Golden-image testing: the backbone of headless development.
//!
//! A change to the rasterizer either looks the same as before or it does not,
//! and on a build machine with no display that question can only be answered by
//! comparing pixels. This module renders that comparison useful: a tolerant
//! diff, a written-out difference image, and a one-line way to regenerate
//! references when a change is intended.
//!
//! ```no_run
//! # use runity_render::{Framebuffer, golden};
//! # let frame = Framebuffer::new(4, 4);
//! // Fails the test and writes <name>.actual.png plus <name>.diff.png.
//! golden::assert_matches("tests/golden/cube.png", &frame, golden::Tolerance::default());
//! ```
//!
//! Regenerate every reference with `RUNITY_UPDATE_GOLDEN=1 cargo test`.

use crate::color::Color;
use crate::framebuffer::Framebuffer;
use crate::png::{decode_png, save_png, Image};
use std::fmt;
use std::path::{Path, PathBuf};

/// Set this to rewrite reference images instead of comparing against them.
pub const UPDATE_ENV: &str = "RUNITY_UPDATE_GOLDEN";

/// How much difference still counts as a match.
///
/// Exact equality is the wrong bar: `f32` arithmetic is not bit-identical
/// across architectures, so a reference rendered on one machine can differ by a
/// least significant bit on another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Largest per-channel difference (0-255) that still counts as equal.
    pub channel_delta: u8,
    /// Fraction of pixels allowed to exceed `channel_delta`.
    pub differing_fraction: f32,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            channel_delta: 2,
            differing_fraction: 0.001,
        }
    }
}

impl Tolerance {
    /// Every pixel must match exactly.
    pub const EXACT: Tolerance = Tolerance {
        channel_delta: 0,
        differing_fraction: 0.0,
    };

    pub fn new(channel_delta: u8, differing_fraction: f32) -> Self {
        Self {
            channel_delta,
            differing_fraction,
        }
    }
}

/// What the comparison found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDiff {
    pub width: usize,
    pub height: usize,
    /// Pixels differing by more than the tolerance's `channel_delta`.
    pub differing_pixels: usize,
    pub max_channel_delta: u8,
    pub mean_channel_delta: f32,
    /// Where the first differing pixel is, if any.
    pub first_difference: Option<(usize, usize)>,
}

impl ImageDiff {
    pub fn total_pixels(&self) -> usize {
        self.width * self.height
    }

    pub fn differing_fraction(&self) -> f32 {
        if self.total_pixels() == 0 {
            0.0
        } else {
            self.differing_pixels as f32 / self.total_pixels() as f32
        }
    }

    pub fn is_within(&self, tolerance: Tolerance) -> bool {
        self.differing_fraction() <= tolerance.differing_fraction
    }
}

impl fmt::Display for ImageDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} of {} pixels differ ({:.3}%), largest channel delta {}, mean {:.2}",
            self.differing_pixels,
            self.total_pixels(),
            self.differing_fraction() * 100.0,
            self.max_channel_delta,
            self.mean_channel_delta
        )?;
        if let Some((x, y)) = self.first_difference {
            write!(f, ", first at ({x}, {y})")?;
        }
        Ok(())
    }
}

#[inline]
fn channel_deltas(a: u32, b: u32) -> [u8; 4] {
    [
        ((a >> 24) as u8).abs_diff((b >> 24) as u8),
        ((a >> 16) as u8).abs_diff((b >> 16) as u8),
        ((a >> 8) as u8).abs_diff((b >> 8) as u8),
        (a as u8).abs_diff(b as u8),
    ]
}

/// Compare two images of the same size.
pub fn compare(actual: &Framebuffer, expected: &Image, tolerance: Tolerance) -> Option<ImageDiff> {
    if actual.width() != expected.width || actual.height() != expected.height {
        return None;
    }
    let mut diff = ImageDiff {
        width: actual.width(),
        height: actual.height(),
        differing_pixels: 0,
        max_channel_delta: 0,
        mean_channel_delta: 0.0,
        first_difference: None,
    };
    let mut total_delta = 0u64;

    // Compare what would be displayed, not the HDR values behind it: the
    // reference is an 8-bit file, and that is the thing a human looks at.
    let resolved = actual.resolve();
    for (index, (a, b)) in resolved.iter().zip(&expected.pixels).enumerate() {
        // Alpha is ignored: the reference is stored as RGB.
        let deltas = channel_deltas(*a, *b);
        let worst = deltas[1].max(deltas[2]).max(deltas[3]);
        total_delta += (deltas[1] as u64) + (deltas[2] as u64) + (deltas[3] as u64);
        diff.max_channel_delta = diff.max_channel_delta.max(worst);
        if worst > tolerance.channel_delta {
            diff.differing_pixels += 1;
            diff.first_difference
                .get_or_insert((index % actual.width(), index / actual.width()));
        }
    }
    diff.mean_channel_delta = total_delta as f32 / (diff.total_pixels() as f32 * 3.0).max(1.0);
    Some(diff)
}

/// Build a picture of the difference: matching pixels dimmed to grey, differing
/// ones in magenta scaled by how far off they are.
pub fn diff_image(actual: &Framebuffer, expected: &Image) -> Framebuffer {
    // A diff is data, not light: it must survive resolve untouched.
    let mut out = Framebuffer::new_raw(actual.width(), actual.height());
    out.clear(Color::BLACK);
    for y in 0..actual.height() {
        for x in 0..actual.width() {
            let index = y * actual.width() + x;
            let a = actual.resolved_pixel(x, y);
            let color = match expected.pixels.get(index) {
                Some(b) => {
                    let deltas = channel_deltas(a, *b);
                    let worst = deltas[1].max(deltas[2]).max(deltas[3]);
                    if worst == 0 {
                        Color::from_argb8(a).scale_rgb(0.15)
                    } else {
                        let intensity = 0.35 + (worst as f32 / 255.0) * 0.65;
                        Color::rgb(intensity, 0.0, intensity)
                    }
                }
                None => Color::RED,
            };
            out.set_pixel(x, y, color);
        }
    }
    out
}

/// The result of a successful [`check`].
#[derive(Debug, Clone, PartialEq)]
pub enum GoldenOutcome {
    /// The reference did not exist and was written.
    Created(PathBuf),
    /// `RUNITY_UPDATE_GOLDEN` was set, so the reference was rewritten.
    Updated(PathBuf),
    Matched(ImageDiff),
}

/// A render that no longer matches its reference, and where to look.
#[derive(Debug, Clone, PartialEq)]
pub struct Mismatch {
    pub golden: PathBuf,
    /// The frame as it rendered this time.
    pub actual: PathBuf,
    /// The two images subtracted, for eyeballing what moved.
    pub diff: PathBuf,
    pub report: ImageDiff,
}

/// Why a [`check`] failed.
#[derive(Debug)]
pub enum GoldenFailure {
    /// Boxed because it is far larger than the other variants, and this type is
    /// returned by value from every comparison.
    Mismatch(Box<Mismatch>),
    SizeChanged {
        golden: PathBuf,
        expected: (usize, usize),
        actual: (usize, usize),
    },
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl fmt::Display for GoldenFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoldenFailure::Mismatch(m) => write!(
                f,
                "render differs from {golden}: {report}\n  rendered:   {actual}\n  \
                 difference: {diff}\n  if the change is intended, rerun with {UPDATE_ENV}=1",
                golden = m.golden.display(),
                report = m.report,
                actual = m.actual.display(),
                diff = m.diff.display()
            ),
            GoldenFailure::SizeChanged {
                golden,
                expected,
                actual,
            } => write!(
                f,
                "render is {}x{} but {} is {}x{}; rerun with {UPDATE_ENV}=1 to replace it",
                actual.0,
                actual.1,
                golden.display(),
                expected.0,
                expected.1
            ),
            GoldenFailure::Io { path, error } => {
                write!(f, "golden image {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for GoldenFailure {}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{stem}.{suffix}.png"))
}

fn update_requested() -> bool {
    match std::env::var(UPDATE_ENV) {
        Ok(value) => !matches!(value.as_str(), "" | "0" | "false"),
        Err(_) => false,
    }
}

/// Compare a frame against a reference image on disk.
///
/// Writes the reference when it is missing or when [`UPDATE_ENV`] is set, so a
/// new test starts working the first time it runs.
pub fn check(
    golden_path: impl AsRef<Path>,
    frame: &Framebuffer,
    tolerance: Tolerance,
) -> Result<GoldenOutcome, GoldenFailure> {
    let golden_path = golden_path.as_ref();
    let io_error = |path: &Path, error: std::io::Error| GoldenFailure::Io {
        path: path.to_path_buf(),
        error,
    };

    let write_reference = |outcome: fn(PathBuf) -> GoldenOutcome| {
        if let Some(parent) = golden_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_error(golden_path, e))?;
        }
        save_png(golden_path, frame).map_err(|e| io_error(golden_path, e))?;
        Ok(outcome(golden_path.to_path_buf()))
    };

    if update_requested() {
        return write_reference(GoldenOutcome::Updated);
    }
    if !golden_path.exists() {
        return write_reference(GoldenOutcome::Created);
    }

    let bytes = std::fs::read(golden_path).map_err(|e| io_error(golden_path, e))?;
    let expected = decode_png(&bytes)
        .map_err(|e| io_error(golden_path, std::io::Error::other(e.to_string())))?;

    let Some(report) = compare(frame, &expected, tolerance) else {
        return Err(GoldenFailure::SizeChanged {
            golden: golden_path.to_path_buf(),
            expected: (expected.width, expected.height),
            actual: (frame.width(), frame.height()),
        });
    };

    if report.is_within(tolerance) {
        // Leave no stale artifacts from an earlier failure.
        let _ = std::fs::remove_file(sidecar(golden_path, "actual"));
        let _ = std::fs::remove_file(sidecar(golden_path, "diff"));
        return Ok(GoldenOutcome::Matched(report));
    }

    let actual_path = sidecar(golden_path, "actual");
    let diff_path = sidecar(golden_path, "diff");
    save_png(&actual_path, frame).map_err(|e| io_error(&actual_path, e))?;
    save_png(&diff_path, &diff_image(frame, &expected)).map_err(|e| io_error(&diff_path, e))?;

    Err(GoldenFailure::Mismatch(Box::new(Mismatch {
        golden: golden_path.to_path_buf(),
        actual: actual_path,
        diff: diff_path,
        report,
    })))
}

/// [`check`], but panics with the explanation — the form a test wants.
#[track_caller]
pub fn assert_matches(golden_path: impl AsRef<Path>, frame: &Framebuffer, tolerance: Tolerance) {
    match check(golden_path, frame, tolerance) {
        Ok(GoldenOutcome::Created(path)) => {
            eprintln!("note: wrote a new reference image at {}", path.display());
        }
        Ok(GoldenOutcome::Updated(path)) => {
            eprintln!("note: updated the reference image at {}", path.display());
        }
        Ok(GoldenOutcome::Matched(_)) => {}
        Err(failure) => panic!("{failure}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("runity-golden-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// Raw buffers, so a test's values survive resolve unchanged and the
    /// assertions are about the comparison rather than the tone curve.
    fn frame(width: usize, height: usize, f: impl Fn(usize, usize) -> Color) -> Framebuffer {
        let mut fb = Framebuffer::new_raw(width, height);
        for y in 0..height {
            for x in 0..width {
                fb.set_pixel(x, y, f(x, y));
            }
        }
        fb
    }

    #[test]
    fn a_missing_reference_is_created_then_matched() {
        let dir = scratch_dir();
        let path = dir.join("scene.png");
        let fb = frame(8, 4, |x, _| Color::rgb(x as f32 / 8.0, 0.2, 0.8));

        assert!(matches!(
            check(&path, &fb, Tolerance::default()).unwrap(),
            GoldenOutcome::Created(_)
        ));
        assert!(path.exists());
        // Second run compares against what the first one wrote.
        assert!(matches!(
            check(&path, &fb, Tolerance::EXACT).unwrap(),
            GoldenOutcome::Matched(_)
        ));
    }

    #[test]
    fn a_changed_render_fails_and_writes_the_evidence() {
        let dir = scratch_dir();
        let path = dir.join("scene.png");
        let original = frame(16, 16, |_, _| Color::rgb(0.1, 0.2, 0.3));
        check(&path, &original, Tolerance::EXACT).unwrap();

        let changed = frame(16, 16, |x, y| {
            if x == 4 && y == 5 {
                Color::WHITE
            } else {
                Color::rgb(0.1, 0.2, 0.3)
            }
        });
        let failure = check(&path, &changed, Tolerance::EXACT).unwrap_err();
        let GoldenFailure::Mismatch(mismatch) = &failure else {
            panic!("expected a mismatch, got {failure}");
        };
        let (actual, diff) = (&mismatch.actual, &mismatch.diff);
        assert_eq!(mismatch.report.differing_pixels, 1);
        assert_eq!(mismatch.report.first_difference, Some((4, 5)));
        assert!(
            actual.exists() && diff.exists(),
            "the evidence images are written"
        );
        assert!(
            format!("{failure}").contains(UPDATE_ENV),
            "the message says how to accept it"
        );

        // A matching run cleans the artifacts back up.
        check(&path, &original, Tolerance::EXACT).unwrap();
        assert!(!actual.exists() && !diff.exists());
    }

    #[test]
    fn small_differences_pass_within_tolerance() {
        let dir = scratch_dir();
        let path = dir.join("scene.png");
        let base = frame(32, 32, |_, _| Color::rgb(0.5, 0.5, 0.5));
        check(&path, &base, Tolerance::EXACT).unwrap();

        // One pixel off by a single step out of 1024: inside the default budget.
        let nudged = frame(32, 32, |x, y| {
            if (x, y) == (0, 0) {
                Color::rgb(0.5 + 1.0 / 255.0, 0.5, 0.5)
            } else {
                Color::rgb(0.5, 0.5, 0.5)
            }
        });
        assert!(matches!(
            check(&path, &nudged, Tolerance::new(1, 0.0)).unwrap(),
            GoldenOutcome::Matched(_)
        ));
        // ...and outside an exact one.
        assert!(check(&path, &nudged, Tolerance::EXACT).is_err());
    }

    #[test]
    fn a_resized_render_reports_the_size_instead_of_diffing() {
        let dir = scratch_dir();
        let path = dir.join("scene.png");
        check(&path, &frame(8, 8, |_, _| Color::RED), Tolerance::EXACT).unwrap();

        let failure = check(&path, &frame(9, 8, |_, _| Color::RED), Tolerance::EXACT).unwrap_err();
        assert!(
            matches!(failure, GoldenFailure::SizeChanged { .. }),
            "{failure}"
        );
        assert!(format!("{failure}").contains("9x8"));
    }

    #[test]
    fn the_diff_image_highlights_only_what_changed() {
        let a = frame(
            4,
            1,
            |x, _| if x == 2 { Color::WHITE } else { Color::BLACK },
        );
        let b = Image {
            width: 4,
            height: 1,
            pixels: vec![Color::BLACK.to_argb8(); 4],
        };
        let diff = diff_image(&a, &b);
        let highlighted = diff.get_pixel(2, 0);
        assert!(highlighted.r > 0.3 && highlighted.b > 0.3 && highlighted.g == 0.0);
        assert_eq!(
            diff.get_pixel(0, 0),
            Color::BLACK,
            "matching pixels are dimmed"
        );
    }
}
