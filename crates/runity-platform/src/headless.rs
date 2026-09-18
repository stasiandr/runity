use crate::window::{Event, Window, WindowConfig};
use std::io;

/// A window that isn't one: it keeps the last presented frame in memory.
///
/// This is what makes the engine testable on a build machine with no display,
/// and what the example binaries use when `RUNITY_HEADLESS=1` is set.
#[derive(Debug, Clone)]
pub struct HeadlessWindow {
    width: u32,
    height: u32,
    title: String,
    frame: Vec<u32>,
    frames_presented: u64,
    /// After this many frames, `poll_events` reports `CloseRequested`.
    pub frame_limit: Option<u64>,
    queued: Vec<Event>,
}

impl HeadlessWindow {
    pub fn new(config: &WindowConfig) -> Self {
        Self {
            width: config.width,
            height: config.height,
            title: config.title.clone(),
            frame: vec![0; (config.width * config.height) as usize],
            frames_presented: 0,
            frame_limit: None,
            queued: Vec::new(),
        }
    }

    pub fn with_frame_limit(mut self, frames: u64) -> Self {
        self.frame_limit = Some(frames);
        self
    }

    /// The most recently presented frame, as `0xAARRGGBB` pixels.
    pub fn last_frame(&self) -> &[u32] {
        &self.frame
    }

    pub fn frames_presented(&self) -> u64 {
        self.frames_presented
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Inject an event, so tests can drive the engine without an OS.
    pub fn push_event(&mut self, event: Event) {
        self.queued.push(event);
    }
}

impl Window for HeadlessWindow {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn poll_events(&mut self) -> io::Result<Vec<Event>> {
        let mut events = std::mem::take(&mut self.queued);
        if let Some(limit) = self.frame_limit {
            if self.frames_presented >= limit {
                events.push(Event::CloseRequested);
            }
        }
        Ok(events)
    }

    fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> io::Result<()> {
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.frame.resize((width * height) as usize, 0);
        }
        let n = self.frame.len().min(pixels.len());
        self.frame[..n].copy_from_slice(&pixels[..n]);
        self.frames_presented += 1;
        Ok(())
    }

    fn set_title(&mut self, title: &str) -> io::Result<()> {
        self.title = title.to_string();
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "headless"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_stores_the_frame_and_counts_it() {
        let mut w = HeadlessWindow::new(&WindowConfig::new("t", 2, 2));
        w.present(&[1, 2, 3, 4], 2, 2).unwrap();
        assert_eq!(w.last_frame(), &[1, 2, 3, 4]);
        assert_eq!(w.frames_presented(), 1);
    }

    #[test]
    fn frame_limit_requests_a_close() {
        let mut w = HeadlessWindow::new(&WindowConfig::new("t", 1, 1)).with_frame_limit(2);
        assert!(w.poll_events().unwrap().is_empty());
        w.present(&[0], 1, 1).unwrap();
        w.present(&[0], 1, 1).unwrap();
        assert_eq!(w.poll_events().unwrap(), vec![Event::CloseRequested]);
    }

    #[test]
    fn present_with_a_new_size_resizes_the_stored_frame() {
        let mut w = HeadlessWindow::new(&WindowConfig::new("t", 2, 2));
        w.present(&[7; 9], 3, 3).unwrap();
        assert_eq!(w.size(), (3, 3));
        assert_eq!(w.last_frame().len(), 9);
    }
}
