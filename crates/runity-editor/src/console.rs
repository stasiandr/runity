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

/// Where a line points in the code: a file, relative as it was printed, and
/// a line and column counted from one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl Line {
    /// The place in the game's code this entry is about — what Unity opens
    /// on a double click: a compile error's `--> src/door.rs:12:9`, a
    /// panic's `panicked at src/main.rs:3:5`, the first frame of a trace in
    /// the game's own `src/`. `None` when it names no place.
    pub fn location(&self) -> Option<Location> {
        let parse = |at: &str| -> Option<Location> {
            let at = at.trim().trim_end_matches(':');
            let mut parts = at.rsplitn(3, ':');
            let column = parts.next()?.parse().ok()?;
            let line = parts.next()?.parse().ok()?;
            let file = parts.next()?.to_string();
            (!file.is_empty() && file.ends_with(".rs")).then_some(Location { file, line, column })
        };
        let lines: Vec<&str> = self.text.lines().collect();
        // A compile error says where on a line of its own.
        for line in &lines {
            if let Some(at) = line.trim_start().strip_prefix("--> ") {
                return parse(at);
            }
        }
        for line in &lines {
            if let Some((_, at)) = line.split_once("panicked at ") {
                return parse(at);
            }
        }
        // A trace: the first frame in the game's own code, not the
        // engine's or the standard library's.
        lines
            .iter()
            .filter_map(|l| l.trim_start().strip_prefix("at "))
            .filter_map(parse)
            // Absolute: the registry's, the toolchain's.
            .find(|l| !std::path::Path::new(&l.file).is_absolute())
    }
}

/// The most lines kept; the oldest go first.
const KEEP: usize = 1000;

#[derive(Debug, Default)]
pub(crate) struct Console {
    lines: Vec<Line>,
    /// The line said last — a repeat counts on its old line, so it is not
    /// always the one at the end — and how many times anything was said.
    last: Option<usize>,
    said: u64,
}

impl Console {
    pub(crate) fn say(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        self.said += 1;
        if let Some(i) = self
            .lines
            .iter()
            .position(|l| l.level == level && l.text == text)
        {
            self.lines[i].count += 1;
            self.last = Some(i);
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
        self.last = Some(self.lines.len() - 1);
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
        self.console.last = None;
    }

    /// The line said most recently, a repeat included, and a number that
    /// grows each time anything is said: the status bar shows the one and
    /// knows from the other that it is new again.
    pub fn last_said(&self) -> Option<(&Line, u64)> {
        let line = self.console.lines.get(self.console.last?)?;
        Some((line, self.console.said))
    }

    /// Add a line — for a window or a tool driving the session.
    pub fn say(&mut self, level: Level, text: impl Into<String>) {
        self.console.say(level, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Line {
        Line {
            level: Level::Error,
            text: text.into(),
            count: 1,
        }
    }

    #[test]
    fn the_line_said_last_is_the_repeat_not_the_end() {
        let mut console = Console::default();
        console.say(Level::Warning, "no ground");
        console.say(Level::Info, "saved");
        assert_eq!(console.lines[console.last.unwrap()].text, "saved");
        let said = console.said;
        console.say(Level::Warning, "no ground");
        assert_eq!(console.lines.len(), 2);
        assert_eq!(console.lines[console.last.unwrap()].text, "no ground");
        assert!(console.said > said);
    }

    #[test]
    fn an_entry_says_where_in_the_code_it_is_about() {
        let compile = line(
            "error[E0425]: cannot find value `speed` in this scope\n  --> src/systems/door.rs:12:9\n   |",
        );
        assert_eq!(
            compile.location(),
            Some(Location {
                file: "src/systems/door.rs".into(),
                line: 12,
                column: 9
            })
        );
        let panic = line("thread 'main' panicked at src/main.rs:3:5:\nattempt to divide by zero");
        assert_eq!(panic.location().unwrap().line, 3);
        let trace = line(
            "error: Error: no display\nStack backtrace:\n   0: anyhow::from\n             at /root/.cargo/registry/anyhow/src/backtrace.rs:10:14\n   1: game::main\n             at ./src/main.rs:240:5",
        );
        assert_eq!(
            trace.location(),
            Some(Location {
                file: "./src/main.rs".into(),
                line: 240,
                column: 5
            })
        );
        assert_eq!(line("the door opened").location(), None);
        assert_eq!(line("see: 12:9").location(), None);
    }
}
