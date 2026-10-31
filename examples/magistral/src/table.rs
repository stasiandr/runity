//! The screen and the UI tree it lives in, and the clicks on it turned
//! into presses: the window and the tests both go through here.

use std::path::Path;

use scrap::html::Screen;
use scrap::scrap_ui::Ui;

use crate::front::{Front, Press};

pub struct Table {
    pub screen: Screen,
    pub ui: Ui,
}

impl Table {
    pub fn load(root: &Path) -> Result<Table, String> {
        let path = root.join("content/magistral/ui/screens/game.screen.html");
        let mut screen = Screen::load(&path).map_err(|p| p.to_string())?;
        let mut ui = Ui::new();
        let parent = ui.root();
        screen.mount(&mut ui, parent);
        Ok(Table { screen, ui })
    }

    /// What was clicked since the last frame, as presses.
    pub fn presses(&mut self) -> Vec<Press> {
        let done = self.screen.read(&self.ui.events());
        let mut out = Vec::new();
        if let Some(r) = done.clicked_row("cell") {
            out.push(Press::Cell(r as u8));
        }
        if let Some(r) = done.clicked_row("card") {
            out.push(Press::Card(r));
        }
        if let Some(r) = done.clicked_row("train") {
            out.push(Press::Train(r));
        }
        if let Some(r) = done.clicked_row("offer") {
            out.push(Press::Offer(Some(r)));
        }
        if let Some(r) = done.clicked_row("remove") {
            out.push(Press::Remove(Some(r)));
        }
        let plain = [
            ("end", Press::EndTurn),
            ("hint", Press::Hint),
            ("hint2", Press::Hint),
            ("auto", Press::Auto),
            ("auto2", Press::Auto),
            ("cancel", Press::Cancel),
            ("again", Press::NewRun),
            ("again2", Press::NewRun),
        ];
        for (id, press) in plain {
            if done.clicked(id) {
                out.push(press);
            }
        }
        if done.clicked("skip") {
            out.push(Press::Offer(None));
            out.push(Press::Remove(None));
        }
        out
    }

    /// Read the clicks, do them, and show the run as it now is.
    pub fn frame(&mut self, front: &mut Front, delta: f32) {
        for press in self.presses() {
            // "Nothing" means one thing at a time: take none, or keep all.
            let phase_before = front.state.phase.clone();
            front.press(press);
            if front.state.phase != phase_before {
                break;
            }
        }
        if let Some(Err(problems)) = self.screen.poll(&mut self.ui, delta) {
            eprintln!("{problems}");
        }
        self.show(front);
    }

    pub fn show(&mut self, front: &Front) {
        self.screen.set("v", front.view());
        self.screen.update(&mut self.ui);
    }
}
