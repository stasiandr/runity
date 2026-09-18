use crate::color::Color;
use crate::gbuffer::GBuffer;
use crate::tonemap::ToneMap;

/// A CPU render target: linear HDR color, a depth buffer, and optionally the
/// G-buffer attachments the deferred passes need.
///
/// Color is **linear light with no upper bound**, not display values. Nothing
/// clamps until [`Framebuffer::resolve`] applies exposure, the tone curve and
/// the sRGB transfer — that is what lets a highlight be ten times white and
/// still roll off smoothly instead of turning into a flat blob.
///
/// Depth follows the projection convention in `runity-math`: 0 at the near
/// plane, 1 at the far plane, smaller is closer.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    width: usize,
    height: usize,
    color: Vec<Color>,
    depth: Vec<f32>,
    /// Per-pixel write counter, allocated only while overdraw tracking is on.
    overdraw: Option<Vec<u32>>,
    /// Geometry attachments, allocated only for deferred rendering.
    gbuffer: Option<GBuffer>,
    /// How [`Framebuffer::resolve`] turns this buffer into 8-bit pixels.
    pub tone_map: ToneMap,
    /// Linear multiplier applied before the tone curve.
    pub exposure: f32,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        assert!(
            width > 0 && height > 0,
            "framebuffer must have a non-zero size"
        );
        Self {
            width,
            height,
            color: vec![Color::BLACK; width * height],
            depth: vec![f32::INFINITY; width * height],
            overdraw: None,
            gbuffer: None,
            tone_map: ToneMap::default(),
            exposure: 1.0,
        }
    }

    /// A buffer holding display values rather than light: no exposure, no tone
    /// curve, no gamma. Debug views and masks use this.
    pub fn new_raw(width: usize, height: usize) -> Self {
        let mut fb = Self::new(width, height);
        fb.tone_map = ToneMap::Raw;
        fb
    }

    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.color.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.color.is_empty()
    }

    #[inline]
    pub fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height as f32
    }

    /// Linear HDR pixels, row-major, top row first.
    #[inline]
    pub fn colors(&self) -> &[Color] {
        &self.color
    }

    #[inline]
    pub fn colors_mut(&mut self) -> &mut [Color] {
        &mut self.color
    }

    #[inline]
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    /// Tone-map the frame into packed `0xAARRGGBB` pixels ready for a window or
    /// a PNG.
    pub fn resolve(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.color.len());
        self.resolve_into(&mut out);
        out
    }

    /// As [`Framebuffer::resolve`], reusing a buffer instead of allocating —
    /// the main loop calls this every frame.
    pub fn resolve_into(&self, out: &mut Vec<u32>) {
        out.clear();
        out.reserve(self.color.len());
        let (map, exposure) = (self.tone_map, self.exposure);
        out.extend(
            self.color
                .iter()
                .map(|c| map.apply(*c, exposure).to_argb8()),
        );
    }

    /// Count how many times each pixel is written, for [`crate::debug::overdraw_view`].
    ///
    /// Off by default: it costs an allocation and a branch per written
    /// fragment, which is worth paying only while you are looking at it.
    pub fn track_overdraw(&mut self, enabled: bool) {
        self.overdraw = enabled.then(|| vec![0; self.width * self.height]);
    }

    /// Write counts since the last clear, if tracking is on.
    #[inline]
    pub fn overdraw(&self) -> Option<&[u32]> {
        self.overdraw.as_deref()
    }

    /// Allocate (or drop) the deferred attachments: normals, material
    /// parameters and world positions alongside the color and depth.
    pub fn enable_gbuffer(&mut self, enabled: bool) {
        self.gbuffer = enabled.then(|| GBuffer::new(self.width, self.height));
    }

    #[inline]
    pub fn gbuffer(&self) -> Option<&GBuffer> {
        self.gbuffer.as_ref()
    }

    #[inline]
    pub fn gbuffer_mut(&mut self) -> Option<&mut GBuffer> {
        self.gbuffer.as_mut()
    }

    /// Detach the G-buffer so a pass can read it while writing color, and hand
    /// it back with [`Framebuffer::put_gbuffer`] when done.
    #[inline]
    pub fn take_gbuffer(&mut self) -> Option<GBuffer> {
        self.gbuffer.take()
    }

    #[inline]
    pub fn put_gbuffer(&mut self, gbuffer: GBuffer) {
        self.gbuffer = Some(gbuffer);
    }

    /// Write a depth value directly. Useful for tools and tests; the rasterizer
    /// has its own path.
    #[inline]
    pub fn set_depth_at(&mut self, x: usize, y: usize, depth: f32) {
        if x < self.width && y < self.height {
            self.depth[y * self.width + x] = depth;
        }
    }

    #[inline]
    pub fn depth_at(&self, x: usize, y: usize) -> f32 {
        self.depth[y * self.width + x]
    }

    /// Resize in place, discarding contents. No-op if the size is unchanged.
    pub fn resize(&mut self, width: usize, height: usize) {
        if width == self.width && height == self.height {
            return;
        }
        assert!(
            width > 0 && height > 0,
            "framebuffer must have a non-zero size"
        );
        self.width = width;
        self.height = height;
        self.color.clear();
        self.color.resize(width * height, Color::BLACK);
        self.depth.clear();
        self.depth.resize(width * height, f32::INFINITY);
        if self.overdraw.is_some() {
            self.overdraw = Some(vec![0; width * height]);
        }
        if self.gbuffer.is_some() {
            self.gbuffer = Some(GBuffer::new(width, height));
        }
    }

    /// Clear color, depth and every attachment.
    pub fn clear(&mut self, color: Color) {
        self.color.fill(color);
        self.depth.fill(f32::INFINITY);
        if let Some(overdraw) = &mut self.overdraw {
            overdraw.fill(0);
        }
        if let Some(gbuffer) = &mut self.gbuffer {
            gbuffer.clear();
        }
    }

    pub fn clear_color(&mut self, color: Color) {
        self.color.fill(color);
    }

    pub fn clear_depth(&mut self) {
        self.depth.fill(f32::INFINITY);
    }

    /// Write a linear color. Out-of-range coordinates are dropped.
    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x < self.width && y < self.height {
            self.color[y * self.width + x] = color;
        }
    }

    /// Read a linear color.
    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> Color {
        self.color[y * self.width + x]
    }

    /// The resolved, displayable value of one pixel.
    #[inline]
    pub fn resolved_pixel(&self, x: usize, y: usize) -> u32 {
        self.tone_map
            .apply(self.get_pixel(x, y), self.exposure)
            .to_argb8()
    }

    #[inline]
    pub(crate) fn set_depth(&mut self, index: usize, z: f32) {
        self.depth[index] = z;
    }

    #[inline]
    pub(crate) fn write_color(&mut self, index: usize, color: Color) {
        self.color[index] = color;
        if let Some(overdraw) = &mut self.overdraw {
            overdraw[index] += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_fills_color_and_depth() {
        let mut fb = Framebuffer::new(4, 3);
        fb.clear(Color::RED);
        assert!(fb.colors().iter().all(|c| *c == Color::RED));
        assert!(fb.depth().iter().all(|d| *d == f32::INFINITY));
    }

    #[test]
    fn color_is_stored_in_linear_light_without_clamping() {
        let mut fb = Framebuffer::new(2, 1);
        // Ten times white: a value that only means anything before tone mapping.
        fb.set_pixel(0, 0, Color::rgb(10.0, 10.0, 10.0));
        assert_eq!(fb.get_pixel(0, 0).r, 10.0, "the buffer must not clamp");

        // Two values well above white must still resolve to different pixels.
        fb.set_pixel(1, 0, Color::rgb(2.0, 2.0, 2.0));
        let resolved = fb.resolve();
        assert_ne!(
            resolved[0], resolved[1],
            "the tone curve must not clip them together"
        );
    }

    #[test]
    fn resolve_applies_the_tone_curve_and_gamma() {
        let mut fb = Framebuffer::new(1, 1);
        fb.tone_map = ToneMap::Raw;
        fb.set_pixel(0, 0, Color::rgb(0.5, 0.5, 0.5));
        let raw = fb.resolve()[0] & 0xff;

        fb.tone_map = ToneMap::AcesFilmic;
        let mapped = fb.resolve()[0] & 0xff;
        assert_ne!(
            raw, mapped,
            "the curve and the sRGB transfer must both apply"
        );
        assert!(mapped > raw, "sRGB encoding lifts mid grey");
    }

    #[test]
    fn resolve_into_reuses_its_buffer() {
        let fb = Framebuffer::new(4, 4);
        let mut out = Vec::new();
        fb.resolve_into(&mut out);
        let capacity = out.capacity();
        fb.resolve_into(&mut out);
        assert_eq!(out.len(), 16);
        assert_eq!(
            out.capacity(),
            capacity,
            "no reallocation on the second frame"
        );
    }

    #[test]
    fn resize_discards_and_reallocates() {
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(Color::WHITE);
        fb.resize(8, 2);
        assert_eq!(fb.width(), 8);
        assert_eq!(fb.len(), 16);
        assert!(fb.colors().iter().all(|c| *c == Color::BLACK));
    }

    #[test]
    fn out_of_bounds_writes_are_dropped() {
        let mut fb = Framebuffer::new(2, 2);
        fb.set_pixel(9, 9, Color::WHITE);
        assert!(fb.colors().iter().all(|c| *c == Color::BLACK));
    }

    #[test]
    fn overdraw_tracking_is_opt_in_and_resets_on_clear() {
        let mut fb = Framebuffer::new(2, 2);
        assert!(fb.overdraw().is_none(), "off by default");
        fb.track_overdraw(true);
        fb.write_color(0, Color::WHITE);
        fb.write_color(0, Color::BLACK);
        assert_eq!(fb.overdraw().unwrap()[0], 2);
        assert_eq!(fb.overdraw().unwrap()[1], 0);
        fb.clear(Color::BLACK);
        assert_eq!(fb.overdraw().unwrap()[0], 0);
    }

    #[test]
    fn the_gbuffer_is_opt_in_and_follows_resizes() {
        let mut fb = Framebuffer::new(4, 4);
        assert!(fb.gbuffer().is_none());
        fb.enable_gbuffer(true);
        assert_eq!(fb.gbuffer().unwrap().len(), 16);
        fb.resize(2, 3);
        assert_eq!(fb.gbuffer().unwrap().len(), 6);
        fb.enable_gbuffer(false);
        assert!(fb.gbuffer().is_none());
    }

    #[test]
    fn depth_can_be_read_and_written_directly() {
        let mut fb = Framebuffer::new(2, 2);
        assert_eq!(fb.depth_at(1, 1), f32::INFINITY);
        fb.set_depth_at(1, 1, 0.5);
        assert_eq!(fb.depth_at(1, 1), 0.5);
        fb.set_depth_at(9, 9, 0.1); // out of bounds, dropped
    }
}
