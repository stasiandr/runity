//! Motion matching: instead of a graph of clips and transitions, a
//! database of every frame of motion there is, and each moment the frame
//! whose features — where the feet are, how fast it goes, where it will be
//! a moment on — best match where the character is and where it is asked
//! to go (Clavet, "Motion Matching", GDC 2016). Played on from there until
//! another frame matches much better.
//!
//! The database here is made from the procedural gait at a range of
//! speeds, so it runs with no clips at all; a project's own mocap would
//! be frames the same way. Played through an active ragdoll, what it picks
//! is held to by physics — motion matching with physical constraints.

use glam::Vec3;

use crate::body::{gait, stride, Pose, PARTS, PELVIS, SHIN};

/// One frame of the database: the pose, and what it is matched by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub pose: Pose,
    /// How fast, metres a second: the speed its clip goes.
    pub speed: f32,
    /// Where each foot is, relative to the pelvis, at rest's scale.
    pub feet: [Vec3; 2],
    /// Which clip, and which frame of it.
    pub clip: usize,
    pub index: usize,
}

/// Frames of motion to match against.
#[derive(Debug, Clone)]
pub struct Database {
    pub frames: Vec<Frame>,
    /// Frames a second each clip was cut at.
    pub rate: f32,
    /// Where each clip starts in `frames`, and how many it has.
    clips: Vec<(usize, usize)>,
}

impl Database {
    /// The procedural gait cut into clips at these speeds, `rate` frames a
    /// second, one stride each.
    pub fn from_gait(speeds: &[f32], rate: f32) -> Self {
        let mut frames = Vec::new();
        let mut clips = Vec::new();
        for (clip, &speed) in speeds.iter().enumerate() {
            // A stride takes its length over the speed; standing, a second.
            let seconds = if speed > 0.05 { stride(speed) / speed } else { 1.0 };
            let count = ((seconds * rate).round() as usize).max(2);
            clips.push((frames.len(), count));
            for index in 0..count {
                let phase = index as f32 / count as f32;
                let pose = gait(speed, phase);
                frames.push(Frame { pose, speed, feet: feet_of(&pose), clip, index });
            }
        }
        Self { frames, rate, clips }
    }

    /// The frame after `at` in its clip, going round.
    pub fn next(&self, at: usize) -> usize {
        let f = &self.frames[at];
        let (start, count) = self.clips[f.clip];
        start + (f.index + 1) % count
    }

    /// The cost of playing frame `at` for feet where `feet` are, going at
    /// `speed`: how far the feet would jump, and how far the speed is off.
    pub fn cost(&self, at: usize, feet: &[Vec3; 2], speed: f32) -> f32 {
        let f = &self.frames[at];
        let jump = f.feet[0].distance(feet[0]) + f.feet[1].distance(feet[1]);
        jump * 1.0 + (f.speed - speed).abs() * 0.6
    }

    /// The best frame for feet where `feet` are and the speed asked for.
    pub fn best(&self, feet: &[Vec3; 2], speed: f32) -> usize {
        (0..self.frames.len())
            .min_by(|a, b| self.cost(*a, feet, speed).total_cmp(&self.cost(*b, feet, speed)))
            .unwrap_or(0)
    }
}

/// Where a pose's feet are, relative to its pelvis.
pub fn feet_of(pose: &Pose) -> [Vec3; 2] {
    let world = pose.world(Vec3::ZERO, glam::Quat::IDENTITY, 1.0);
    [0, 1].map(|side| pose.point(&world, SHIN[side], PARTS[SHIN[side]].to, 1.0) - world[PELVIS].1)
}

/// A character playing the database: its frame, and how long it has
/// been on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matcher {
    pub frame: usize,
    owed: f32,
    /// How much better another frame must be to be jumped to.
    pub stickiness: f32,
}

impl Default for Matcher {
    fn default() -> Self {
        Self { frame: 0, owed: 0.0, stickiness: 0.15 }
    }
}

impl Matcher {
    /// On by `seconds`, asked to go at `speed`: the next frame of what it
    /// plays, or a better one if there is one. The pose to show.
    pub fn advance(&mut self, db: &Database, speed: f32, seconds: f32) -> Pose {
        self.owed += seconds.max(0.0);
        let step = 1.0 / db.rate;
        while self.owed >= step {
            self.owed -= step;
            let next = db.next(self.frame.min(db.frames.len() - 1));
            let feet = db.frames[next].feet;
            let best = db.best(&feet, speed);
            // Jump only when it is clearly better: a match that dithers
            // between two frames looks worse than one a little off.
            self.frame = if db.cost(best, &feet, speed) + self.stickiness < db.cost(next, &feet, speed) { best } else { next };
        }
        let this = db.frames[self.frame].pose;
        let next = db.frames[db.next(self.frame)].pose;
        this.blend(&next, self.owed / step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asked_to_speed_up_it_moves_to_the_faster_clip_and_plays_it_through() {
        let db = Database::from_gait(&[0.0, 0.8, 1.6, 2.6], 30.0);
        let mut m = Matcher::default();
        // Standing, asked to stand: it stays in the standing clip.
        for _ in 0..30 {
            m.advance(&db, 0.0, 1.0 / 30.0);
        }
        assert_eq!(db.frames[m.frame].speed, 0.0);
        // Asked to run: within a second it is in the run.
        for _ in 0..30 {
            m.advance(&db, 2.6, 1.0 / 30.0);
        }
        assert_eq!(db.frames[m.frame].speed, 2.6);
        // And stays, playing it on frame after frame.
        let mut stayed = 0;
        for _ in 0..30 {
            let before = m.frame;
            m.advance(&db, 2.6, 1.0 / 30.0);
            if m.frame == db.next(before) {
                stayed += 1;
            }
        }
        assert!(stayed > 25, "plays on: {stayed} of 30");
    }
}
