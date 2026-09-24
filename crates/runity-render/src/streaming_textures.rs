//! Mip streaming: a texture asset keeps on the GPU only the levels the
//! screen needs of it.
//!
//! Each screen frame, every draw in view says how many texels of its maps
//! it covers across — its height on the screen in pixels, times how often
//! its material tiles them — and each texture keeps the finest level any
//! draw asked for, a level finer than that so nothing is soft. Between
//! frames [`Renderer::stream_textures`](crate::Renderer) uploads it again
//! from the level wanted down, from the library's copy: at once when a
//! finer one is wanted, a level at a time when a coarser one will do,
//! and down to a picture of 64 texels when it has not been seen for two
//! seconds. A texture never streamed stays whole, as uploaded.

use crate::asset::AssetId;

/// Levels a stream remembers the size of.
const MOST_LEVELS: usize = 16;
/// Frames unseen before a texture gives up all but its small levels.
const COLD: u32 = 120;
/// The side a cold texture keeps.
const COLD_SIDE: u32 = 64;

/// One texture asset's resident levels, and what the frames need.
#[derive(Debug, Clone, Copy)]
pub struct TextureStream {
    pub id: AssetId,
    sizes: [(u32, u32); MOST_LEVELS],
    levels: u32,
    /// The finest level on the GPU now.
    pub first: u32,
    /// The finest level the last frame that saw it needed.
    need: u32,
    /// The finest level this frame has asked for so far.
    asked: u32,
    unseen: u32,
}

impl TextureStream {
    pub fn new(id: AssetId, sizes: &[(u32, u32)]) -> Self {
        let mut out = [(1, 1); MOST_LEVELS];
        for (o, s) in out.iter_mut().zip(sizes) {
            *o = *s;
        }
        Self {
            id,
            sizes: out,
            levels: sizes.len().clamp(1, MOST_LEVELS) as u32,
            first: 0,
            need: 0,
            asked: u32::MAX,
            unseen: 0,
        }
    }

    /// A draw this frame shows `texels` of it across the screen.
    pub fn see(&mut self, texels: f32) {
        let side = self.sizes[0].0.max(self.sizes[0].1) as f32;
        let fits = (side / texels.max(1.0)).log2().floor() as i64 - 1;
        let level = fits.clamp(0, self.levels as i64 - 1) as u32;
        self.asked = self.asked.min(level);
    }

    /// The frame is over: what it asked for is what is needed.
    pub fn end_frame(&mut self) {
        if self.asked == u32::MAX {
            self.unseen = self.unseen.saturating_add(1);
        } else {
            self.need = self.asked;
            self.unseen = 0;
        }
        self.asked = u32::MAX;
    }

    /// The level to have on the GPU.
    pub fn wanted(&self) -> u32 {
        let target = if self.unseen > COLD {
            // Long unseen: only what fits in a small picture.
            (0..self.levels)
                .find(|&l| self.sizes[l as usize].0.max(self.sizes[l as usize].1) <= COLD_SIDE)
                .unwrap_or(self.levels - 1)
        } else {
            self.need
        };
        if target < self.first {
            target
        } else if target > self.first + 1 {
            // Coarser a level at a time: a thing walked away from does not
            // blur the moment it is.
            self.first + 1
        } else {
            self.first
        }
    }

    /// Bytes of the levels from `first` down.
    pub fn bytes_from(&self, first: u32) -> u64 {
        (first..self.levels)
            .map(|l| {
                let (w, h) = self.sizes[l as usize];
                w as u64 * h as u64 * 4
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_texture_wants_its_fine_levels_when_near_and_gives_them_up_when_far_or_gone() {
        let sizes: Vec<(u32, u32)> = (0..11).map(|l| (1024 >> l, 1024 >> l)).collect();
        let mut s = TextureStream::new(AssetId(1), &sizes);
        // Near: 800 texels across wants the whole picture.
        s.see(800.0);
        s.end_frame();
        assert_eq!(s.wanted(), 0);
        // Far: 20 texels — a level coarser each time it is asked.
        s.see(20.0);
        s.end_frame();
        assert_eq!(s.wanted(), 1);
        s.first = 1;
        assert_eq!(s.wanted(), 2);
        // Near again: straight back.
        s.see(1000.0);
        s.end_frame();
        assert_eq!(s.wanted(), 0);
        // Unseen for long: down to 64 texels, a level at a time.
        s.first = 0;
        for _ in 0..=COLD + 1 {
            s.end_frame();
        }
        assert_eq!(s.wanted(), 1);
        assert!(s.bytes_from(4) < s.bytes_from(0) / 100);
    }
}
