//! What a running game answers a request from outside it — the control
//! socket's (`scrap::shell::control`) and the world control's
//! (`scrap::world_control`): shared here so a game's own commands, the
//! engine's world commands and the shell's speak one word.

pub use serde_json;
pub use serde_json::Value;

/// An answer to one request.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Done(Value),
    Failed(String),
    /// Not yet: ask again next frame (a wait, a walk under way).
    Pending,
    /// Not a command of whoever was asked: ask the next.
    Unknown,
}

impl Answer {
    /// `Ok` as done, `Err` as failed.
    pub fn of(result: Result<Value, String>) -> Self {
        match result {
            Ok(v) => Answer::Done(v),
            Err(e) => Answer::Failed(e),
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Answer::Unknown)
    }
}
