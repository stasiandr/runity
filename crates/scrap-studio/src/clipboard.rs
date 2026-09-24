//! The system clipboard, with a string of its own where there is none —
//! a headless test, a machine without a display server.

/// What copy and paste use: the system's when it answers.
pub struct SystemClipboard {
    board: Option<arboard::Clipboard>,
    local: String,
}

impl SystemClipboard {
    pub fn new() -> Self {
        Self {
            board: arboard::Clipboard::new().ok(),
            local: String::new(),
        }
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl scrap_ui::Clipboard for SystemClipboard {
    fn get(&mut self) -> Option<String> {
        self.board
            .as_mut()
            .and_then(|b| b.get_text().ok())
            .or_else(|| Some(self.local.clone()))
    }

    fn set(&mut self, text: String) {
        if let Some(b) = self.board.as_mut() {
            let _ = b.set_text(text.clone());
        }
        self.local = text;
    }
}
