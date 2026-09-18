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
    }

    pub fn clear(&mut self, color: Color) {
        let packed = color.to_argb8();
        self.color.fill(packed);
        self.depth.fill(f32::INFINITY);
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

    #[inline]
    pub(crate) fn set_depth(&mut self, index: usize, z: f32) {
        self.depth[index] = z;
    }

    #[inline]
    pub(crate) fn write_packed(&mut self, index: usize, packed: u32) {
        self.color[index] = packed;
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
}
