use crate::color::Color;

/// A CPU render target: packed `0xAARRGGBB` color plus an f32 depth buffer.
///
/// Depth follows the projection convention in `runity-math`: 0 at the near
/// plane, 1 at the far plane, smaller is closer.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    width: usize,
    height: usize,
    color: Vec<u32>,
    depth: Vec<f32>,
    /// Per-pixel write counter, allocated only while overdraw tracking is on.
    overdraw: Option<Vec<u32>>,
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
            color: vec![0; width * height],
            depth: vec![f32::INFINITY; width * height],
            overdraw: None,
        }
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
    pub fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height as f32
    }

    /// Raw pixels, row-major, top row first — ready to hand to the OS.
    #[inline]
    pub fn pixels(&self) -> &[u32] {
        &self.color
    }

    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.color
    }

    #[inline]
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    /// Count how many times each pixel is written, for [`crate::debug::overdraw_view`].
    ///
    /// Off by default: it costs an allocation and a branch per written
    /// fragment, which is worth paying only while you are looking at it.
    pub fn track_overdraw(&mut self, enabled: bool) {
        self.overdraw = enabled.then(|| vec![0; self.width * self.height]);
    }

    /// Write counts from the last clear, if tracking is on.
    #[inline]
    pub fn overdraw(&self) -> Option<&[u32]> {
        self.overdraw.as_deref()
    }

    /// Write a depth value directly. Useful for tools and tests; the rasterizer
    /// has its own path.
    #[inline]
    pub fn set_depth_at(&mut self, x: usize, y: usize, depth: f32) {
        if x < self.width && y < self.height {
            self.depth[y * self.width + x] = depth;
        }
    }

    /// Read a depth value.
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
        self.color.resize(width * height, 0);
        self.depth.clear();
        self.depth.resize(width * height, f32::INFINITY);
        if self.overdraw.is_some() {
            self.overdraw = Some(vec![0; width * height]);
        }
    }

    pub fn clear(&mut self, color: Color) {
        let packed = color.to_argb8();
        self.color.fill(packed);
        self.depth.fill(f32::INFINITY);
        if let Some(overdraw) = &mut self.overdraw {
            overdraw.fill(0);
        }
    }

    pub fn clear_color(&mut self, color: Color) {
        self.color.fill(color.to_argb8());
    }

    pub fn clear_depth(&mut self) {
        self.depth.fill(f32::INFINITY);
    }

    /// Write a pixel without bounds checking of the caller's coordinates
    /// (they are checked here; out-of-range writes are dropped).
    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x < self.width && y < self.height {
            self.color[y * self.width + x] = color.to_argb8();
        }
    }

    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> Color {
        Color::from_argb8(self.color[y * self.width + x])
    }

    /// Alpha-blend a solid rectangle, top-left at `(x, y)`, `width` by `height`.
    ///
    /// Clipped to the framebuffer before the pixel loop runs, so a rectangle
    /// far outside it costs nothing. `color.a` is the blend strength — 1.0
    /// replaces the covered pixels outright, 0.0 draws nothing.
    pub fn fill_rect(&mut self, x: i32, y: i32, width: usize, height: usize, color: Color) {
        let alpha = color.a.clamp(0.0, 1.0);
        if width == 0 || height == 0 || alpha <= 0.0 {
            return;
        }
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = (x.saturating_add(width as i32)).clamp(0, self.width as i32) as usize;
        let y1 = (y.saturating_add(height as i32)).clamp(0, self.height as i32) as usize;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for py in y0..y1 {
            for px in x0..x1 {
                let blended = self.get_pixel(px, py).blend_over(color, alpha);
                self.set_pixel(px, py, blended);
            }
        }
    }

    /// Alpha-blend a rectangle's border, `thickness` pixels wide, drawn inward
    /// from `(x, y)` and `width` by `height` — the frame around a panel rather
    /// than a filled one.
    ///
    /// A `thickness` that would make the two opposite edges of the border meet
    /// or overlap just fills the whole rectangle, so a caller cannot double up
    /// the blend at the corners by asking for a heavier border than fits.
    pub fn stroke_rect(
        &mut self,
        x: i32,
        y: i32,
        width: usize,
        height: usize,
        thickness: usize,
        color: Color,
    ) {
        if width == 0 || height == 0 || thickness == 0 {
            return;
        }
        if thickness.saturating_mul(2) >= width || thickness.saturating_mul(2) >= height {
            self.fill_rect(x, y, width, height, color);
            return;
        }
        let t = thickness as i32;
        self.fill_rect(x, y, width, thickness, color); // top
        self.fill_rect(x, y + height as i32 - t, width, thickness, color); // bottom
        let middle_height = height - 2 * thickness;
        self.fill_rect(x, y + t, thickness, middle_height, color); // left
        self.fill_rect(x + width as i32 - t, y + t, thickness, middle_height, color);
        // right
    }

    #[inline]
    pub(crate) fn set_depth(&mut self, index: usize, z: f32) {
        self.depth[index] = z;
    }

    #[inline]
    pub(crate) fn write_packed(&mut self, index: usize, packed: u32) {
        self.color[index] = packed;
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
        assert!(fb.pixels().iter().all(|p| *p == 0xff_ff_00_00));
        assert!(fb.depth().iter().all(|d| *d == f32::INFINITY));
    }

    #[test]
    fn resize_discards_and_reallocates() {
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(Color::WHITE);
        fb.resize(8, 2);
        assert_eq!(fb.width(), 8);
        assert_eq!(fb.pixels().len(), 16);
        assert!(fb.pixels().iter().all(|p| *p == 0));
    }

    #[test]
    fn out_of_bounds_writes_are_dropped() {
        let mut fb = Framebuffer::new(2, 2);
        fb.set_pixel(9, 9, Color::WHITE);
        assert!(fb.pixels().iter().all(|p| *p == 0));
    }

    #[test]
    fn overdraw_tracking_is_opt_in_and_resets_on_clear() {
        let mut fb = Framebuffer::new(2, 2);
        assert!(fb.overdraw().is_none(), "off by default");
        fb.track_overdraw(true);
        fb.write_packed(0, 0xffff_ffff);
        fb.write_packed(0, 0xff00_0000);
        assert_eq!(fb.overdraw().unwrap()[0], 2);
        assert_eq!(fb.overdraw().unwrap()[1], 0);
        fb.clear(Color::BLACK);
        assert_eq!(fb.overdraw().unwrap()[0], 0);
    }

    #[test]
    fn fill_rect_is_opaque_at_full_alpha_and_clipped_to_the_frame() {
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(Color::BLACK);
        fb.fill_rect(1, 1, 2, 2, Color::WHITE);
        for y in 0..4 {
            for x in 0..4 {
                let expected = if (1..3).contains(&x) && (1..3).contains(&y) {
                    Color::WHITE
                } else {
                    Color::BLACK
                };
                assert_eq!(fb.get_pixel(x, y), expected, "at ({x}, {y})");
            }
        }

        // Straddling the edges: only the in-bounds part is touched, and no
        // iteration wraps or panics.
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(Color::BLACK);
        fb.fill_rect(-1, -1, 3, 3, Color::WHITE);
        assert_eq!(fb.get_pixel(1, 1), Color::WHITE);
        assert_eq!(fb.get_pixel(2, 2), Color::BLACK);

        // Entirely outside: a no-op, not a crash.
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(Color::BLACK);
        fb.fill_rect(100, 100, 5, 5, Color::WHITE);
        fb.fill_rect(-100, -100, 5, 5, Color::WHITE);
        assert!(fb.pixels().iter().all(|p| *p == Color::BLACK.to_argb8()));
    }

    #[test]
    fn fill_rect_blends_by_the_same_formula_as_blend_alpha() {
        let mut fb = Framebuffer::new(1, 1);
        let base = Color::rgb(0.2, 0.4, 0.6);
        fb.clear(base);
        let src = Color::rgba(1.0, 0.0, 0.0, 0.4);
        fb.fill_rect(0, 0, 1, 1, src);
        // Both `base` and the blended result round-trip through the packed
        // u32 store, so the expectation goes through the same round trips.
        let stored_base = Color::from_argb8(base.to_argb8());
        let expected = Color::from_argb8(stored_base.blend_over(src, src.a).to_argb8());
        assert_eq!(fb.get_pixel(0, 0), expected);
    }

    #[test]
    fn stroke_rect_lights_the_border_and_leaves_the_middle_alone() {
        let mut fb = Framebuffer::new(6, 6);
        fb.clear(Color::BLACK);
        fb.stroke_rect(1, 1, 4, 4, 1, Color::WHITE);
        for y in 1..5 {
            for x in 1..5 {
                let on_border = x == 1 || x == 4 || y == 1 || y == 4;
                let expected = if on_border {
                    Color::WHITE
                } else {
                    Color::BLACK
                };
                assert_eq!(fb.get_pixel(x, y), expected, "at ({x}, {y})");
            }
        }
        assert_eq!(fb.get_pixel(2, 2), Color::BLACK, "the middle is untouched");
    }

    #[test]
    fn stroke_rect_thicker_than_the_rectangle_just_fills_it() {
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(Color::BLACK);
        fb.stroke_rect(0, 0, 4, 4, 10, Color::WHITE);
        assert!(fb.pixels().iter().all(|p| *p == Color::WHITE.to_argb8()));
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
