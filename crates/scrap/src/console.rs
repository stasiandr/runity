//! The in-game console: Unreal's `~`, SRDebugger's console for Unity.
//!
//! The commands are the core's ([`scrap_core::console`]): a game registers
//! its cheats as functions of the world, the engine brings `help`, `get`
//! and `set` (which writes `tuning/` so the game's `Tuned` reloads it).
//! This is the line to type them into over the game — opened by the
//! `console` action (backtick in the template's `input.ron`), Enter runs,
//! Up and Down walk the lines typed before, Tab completes a name, Escape
//! or the action again closes — and the door the editor's Console and an
//! agent come in by: [`Console::frame`] runs what arrived over the embed
//! socket (`Context::commands`) as if typed here.
//!
//! Every line run and its answer is printed, `> line` then the answer
//! indented, so the editor's Console shows it as one entry and an agent
//! finds it there.
//!
//! **Only in development by default.** A release build ships no cheats
//! unless asked: [`Console::for_build`] turns it on in a debug build, and
//! in any build run with `SCRAP_CONSOLE=1` (a playtester's). Off, it draws
//! nothing, takes no keys and answers the editor that it is off.

pub use scrap_core::console::*;

use crate::input::{Input, Key};
use crate::ui::{Quad, TextRun, Ui};

/// The variable that turns the console on in a build that has it off.
pub const CONSOLE_VAR: &str = "SCRAP_CONSOLE";

/// How many answer lines are kept to show.
const KEEP: usize = 200;

/// The console over the game: what is typed, what was typed, what came
/// back.
#[derive(Debug, Clone)]
pub struct Console {
    pub commands: Commands,
    enabled: bool,
    open: bool,
    /// Whether it had the keyboard this frame, open or closing.
    keyboard: bool,
    line: String,
    typed: Vec<String>,
    /// Which of `typed` Up has gone back to.
    back: Option<usize>,
    output: Vec<String>,
}

impl Console {
    /// A console with `commands`, on or off.
    pub fn new(commands: Commands, enabled: bool) -> Self {
        Self {
            commands,
            enabled,
            open: false,
            keyboard: false,
            line: String::new(),
            typed: Vec::new(),
            back: None,
            output: Vec::new(),
        }
    }

    /// On in a debug build (`cfg!(debug_assertions)` of the game, passed
    /// in) and wherever [`CONSOLE_VAR`] is set; off in a release build
    /// otherwise.
    pub fn for_build(commands: Commands, debug: bool) -> Self {
        let asked = std::env::var_os(CONSOLE_VAR).is_some_and(|v| v != "0");
        Self::new(commands, debug || asked)
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether the keyboard was the console's this frame — open, or
    /// closed by this frame's Escape: the game should not read its keys
    /// as its own (Escape quitting, letters jumping).
    pub fn has_keyboard(&self) -> bool {
        self.keyboard
    }

    /// What it has answered, oldest first.
    pub fn output(&self) -> &[String] {
        &self.output
    }

    /// Run a line as if typed, print it and its answer, and keep both to
    /// show. The answer, `Err` for why it could not.
    pub fn run(&mut self, world: &mut hecs::World, line: &str) -> Result<String, String> {
        let line = line.trim();
        let answer = if self.enabled {
            self.commands.run(world, line)
        } else {
            Err(format!(
                "the console is off in this build ({CONSOLE_VAR}=1 turns it on)"
            ))
        };
        let mut said = vec![format!("> {line}")];
        match &answer {
            Ok(text) => said.extend(text.lines().map(|l| format!("  {l}"))),
            Err(why) => said.push(format!("  error: {why}")),
        }
        // One write, so nothing the game prints lands in the middle.
        eprintln!("{}", said.join("\n"));
        self.output.extend(said);
        if self.output.len() > KEEP {
            self.output.drain(..self.output.len() - KEEP);
        }
        if !line.is_empty() && self.typed.last().map(String::as_str) != Some(line) {
            self.typed.push(line.to_string());
        }
        answer
    }

    /// One frame: open or close on `toggled` (the `console` action
    /// pressed), take the keyboard while open, run what the editor sent
    /// (`from_editor`, `Context::commands`) and draw over the top of a
    /// `size`-pixel screen.
    pub fn frame(
        &mut self,
        world: &mut hecs::World,
        input: &Input,
        toggled: bool,
        from_editor: &[String],
        ui: &mut Ui,
        size: glam::Vec2,
    ) {
        for line in from_editor {
            let _ = self.run(world, line);
        }
        let was_open = self.open;
        if self.enabled && toggled {
            self.open = !self.open;
        }
        self.keyboard = was_open || self.open;
        if !self.open {
            return;
        }
        // The key that opened it types a character too: not into the line.
        if was_open && !toggled {
            self.type_keys(world, input);
        }
        if self.open {
            self.draw(ui, size);
        }
    }

    fn type_keys(&mut self, world: &mut hecs::World, input: &Input) {
        self.line
            .extend(input.text().chars().filter(|c| !c.is_control()));
        if input.pressed(Key::Backspace) {
            self.line.pop();
        }
        if input.pressed(Key::Escape) {
            self.open = false;
            return;
        }
        if input.pressed(Key::Up) && !self.typed.is_empty() {
            let at = self
                .back
                .map_or(self.typed.len() - 1, |i| i.saturating_sub(1));
            self.back = Some(at);
            self.line = self.typed[at].clone();
        }
        if input.pressed(Key::Down) {
            match self.back {
                Some(i) if i + 1 < self.typed.len() => {
                    self.back = Some(i + 1);
                    self.line = self.typed[i + 1].clone();
                }
                _ => {
                    self.back = None;
                    self.line.clear();
                }
            }
        }
        if input.pressed(Key::Tab) && !self.line.contains(' ') {
            let found: Vec<String> = self
                .commands
                .complete(&self.line)
                .into_iter()
                .map(String::from)
                .collect();
            match found.as_slice() {
                [one] => self.line = format!("{one} "),
                [] => {}
                many => self.output.push(format!("  {}", many.join("  "))),
            }
        }
        if input.pressed(Key::Enter) {
            let line = std::mem::take(&mut self.line);
            self.back = None;
            let _ = self.run(world, &line);
        }
    }

    fn draw(&self, ui: &mut Ui, size: glam::Vec2) {
        let text = 16.0;
        let row = text * 1.4;
        let height = (size.y * 0.4).max(row * 3.0);
        ui.quad(Quad::new(
            0.0,
            0.0,
            size.x,
            height,
            glam::Vec4::new(0.03, 0.03, 0.05, 0.85),
        ));
        let pad = 12.0;
        let input_y = height - row - pad * 0.5;
        ui.text(TextRun::new(
            pad,
            input_y,
            text,
            glam::Vec4::ONE,
            format!("> {}|", self.line),
        ));
        let fits = ((input_y - pad) / row).max(0.0) as usize;
        let shown = &self.output[self.output.len().saturating_sub(fits)..];
        for (i, line) in shown.iter().enumerate() {
            let y = input_y - row * (shown.len() - i) as f32;
            let color = if line.starts_with("  error:") {
                glam::Vec4::new(1.0, 0.45, 0.4, 1.0)
            } else if line.starts_with('>') {
                glam::Vec4::new(0.6, 0.8, 1.0, 1.0)
            } else {
                glam::Vec4::new(0.85, 0.85, 0.85, 1.0)
            };
            ui.text(TextRun::new(pad, y, text, color, line.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputEvent;

    fn give(world: &mut hecs::World, words: &[&str]) -> Result<String, String> {
        let n: u32 = words.first().and_then(|w| w.parse().ok()).ok_or("give N")?;
        world.spawn((n,));
        Ok(format!("{n} given"))
    }

    fn commands() -> Commands {
        let mut commands = Commands::new();
        commands.add("give", "give N things", give);
        commands
    }

    /// A frame of keys pressed and text typed.
    fn keys(input: &mut Input, keys: &[Key], text: &str) {
        input.begin_frame();
        for key in keys {
            input.handle(&InputEvent::KeyDown(*key));
            input.handle(&InputEvent::KeyUp(*key));
        }
        if !text.is_empty() {
            input.handle(&InputEvent::Text(text.into()));
        }
    }

    #[test]
    fn typed_in_the_console_a_command_runs_and_up_brings_it_back() {
        let mut console = Console::new(commands(), true);
        let (mut world, mut input, mut ui) = (hecs::World::new(), Input::new(), Ui::new());
        let size = glam::Vec2::new(800.0, 600.0);
        // Opened by the action; the backtick it typed stays out of the line.
        keys(&mut input, &[Key::Backquote], "`");
        console.frame(&mut world, &input, true, &[], &mut ui, size);
        assert!(console.is_open() && console.has_keyboard());
        keys(&mut input, &[], "gi");
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        keys(&mut input, &[Key::Tab], "");
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        keys(&mut input, &[], "3");
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        keys(&mut input, &[Key::Enter], "");
        ui.clear();
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        assert_eq!(world.len(), 1);
        assert_eq!(console.output(), ["> give 3", "  3 given"]);
        assert!(ui.texts.iter().any(|t| t.text == "  3 given"), "drawn");

        keys(&mut input, &[Key::Up], "");
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        keys(&mut input, &[Key::Enter], "");
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        assert_eq!(world.len(), 2, "the line typed before, again");

        keys(&mut input, &[Key::Escape], "");
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        assert!(!console.is_open());
        assert!(
            console.has_keyboard(),
            "this frame's Escape was the console's"
        );
        keys(&mut input, &[Key::Escape], "");
        console.frame(&mut world, &input, false, &[], &mut ui, size);
        assert!(!console.has_keyboard());
    }

    #[test]
    fn a_line_from_the_editor_runs_and_an_off_console_says_so() {
        let mut console = Console::new(commands(), true);
        let (mut world, input, mut ui) = (hecs::World::new(), Input::new(), Ui::new());
        let size = glam::Vec2::new(800.0, 600.0);
        console.frame(&mut world, &input, false, &["give x".into()], &mut ui, size);
        assert_eq!(console.output(), ["> give x", "  error: give N"]);
        assert!(ui.texts.is_empty(), "closed, it draws nothing");

        let mut off = Console::new(commands(), false);
        off.frame(&mut world, &input, true, &["give 1".into()], &mut ui, size);
        assert!(!off.is_open(), "off, the action does not open it");
        assert_eq!(world.len(), 0);
        assert!(off.output()[1].contains("off in this build"));
    }
}
