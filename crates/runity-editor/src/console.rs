//! The Console: what the editor has to say, in one place.
//!
//! Unity's Console window, as data a window draws and an agent reads:
//! what opening a scene skipped, what an import warned about, a source
//! that would not rebuild, an edit the Scene view refused. Each line has a
//! level; the same line again counts up rather than repeating — Unity's
//! Collapse, always on, because a reload that fails every quarter second
//! is one problem, not four hundred.

use crate::Session;

/// How much a line matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// One line of the Console.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub level: Level,
    pub text: String,
    /// How many times it was said.
    pub count: u32,
}

/// The most lines kept; the oldest go first.
const KEEP: usize = 1000;

#[derive(Debug, Default)]
pub(crate) struct Console {
    lines: Vec<Line>,
}

impl Console {
    pub(crate) fn say(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        if let Some(line) = self
            .lines
            .iter_mut()
            .find(|l| l.level == level && l.text == text)
        {
            line.count += 1;
            return;
        }
        if self.lines.len() == KEEP {
            self.lines.remove(0);
        }
        self.lines.push(Line {
            level,
            text,
            count: 1,
        });
    }
}

impl Session {
    /// Everything said so far, oldest first.
    pub fn console(&self) -> &[Line] {
        &self.console.lines
    }

    /// How many lines of each level: the counters on the Console's bar.
    pub fn console_counts(&self) -> (usize, usize, usize) {
        let count = |level| {
            self.console
                .lines
                .iter()
                .filter(|l| l.level == level)
                .count()
        };
        (
            count(Level::Info),
            count(Level::Warning),
            count(Level::Error),
        )
    }

    pub fn clear_console(&mut self) {
        self.console.lines.clear();
    }

    /// Add a line — for a window or a tool driving the session.
    pub fn say(&mut self, level: Level, text: impl Into<String>) {
        self.console.say(level, text);
    }
}
