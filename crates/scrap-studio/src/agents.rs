//! Agents in the window: the studio serves `scrap-mcp`'s protocol on its
//! own session (docs/live-mcp.md, DNA postulate 5).
//!
//! While a project is open, [`Agents`] keeps a [`scrap_mcp::live::Listener`]
//! on that project's socket, and the window's frame loop hands it the
//! session before each frame: every message an agent sent since is
//! answered there, on the document the person is looking at, and the frame
//! that follows shows what it did. A message arriving wakes the loop, so an
//! agent does not wait for the idle frame. Another project opened in the
//! window moves the socket with it; none open (the Projects screen), none.
//! `SCRAP_STUDIO_AGENTS=0` turns it off.

use std::path::PathBuf;
use std::sync::Arc;

use scrap_editor::console::Level;
use scrap_editor::Session;

/// The id of the wake-up sent through the window's user events: no menu
/// line has it.
pub const WAKE: &str = "scrap.agents.wake";

pub struct Agents {
    #[cfg(unix)]
    listener: Option<scrap_mcp::live::Listener>,
    /// The project root the session had when last looked at, as it names it.
    seen: Option<PathBuf>,
    /// The loop's wake-up, for a message that arrives while it waits.
    wake: Arc<dyn Fn() + Send + Sync>,
    on: bool,
    /// While an agent is connected, macOS is told the app is at work, so
    /// App Nap does not hold a window in the background asleep for seconds
    /// between its messages.
    #[cfg(target_os = "macos")]
    awake: Option<Awake>,
}

impl Agents {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            #[cfg(unix)]
            listener: None,
            seen: None,
            wake: Arc::new(wake),
            on: std::env::var("SCRAP_STUDIO_AGENTS").map_or(true, |v| v != "0"),
            #[cfg(target_os = "macos")]
            awake: None,
        }
    }

    /// Listen on the session's project (again, when it changed), and
    /// answer every message waiting, on `session`. Whether any was.
    pub fn serve(&mut self, session: &mut Session) -> bool {
        if !self.on {
            return false;
        }
        let root = session.project().map(|p| p.root().to_path_buf());
        if root != self.seen {
            self.seen = root.clone();
            self.follow(session, root);
        }
        #[cfg(unix)]
        if let Some(listener) = &mut self.listener {
            #[cfg(target_os = "macos")]
            {
                let wanted = listener.connections() > 0;
                if wanted != self.awake.is_some() {
                    self.awake = wanted.then(Awake::begin);
                }
            }
            return listener.serve(session) > 0;
        }
        false
    }

    #[cfg_attr(not(unix), allow(unused_variables))]
    fn follow(&mut self, session: &mut Session, root: Option<PathBuf>) {
        #[cfg(unix)]
        {
            // The old project's socket goes before the new one is made.
            self.listener = None;
            #[cfg(target_os = "macos")]
            {
                self.awake = None;
            }
            let (Some(root), Some(dir)) = (root, scrap_cli::hub::config_dir()) else {
                return;
            };
            let wake = self.wake.clone();
            match scrap_mcp::live::Listener::open(&dir, &root, move || wake()) {
                Ok(listener) => {
                    session.say(
                        Level::Info,
                        format!(
                            "agents: scrap-mcp started in this project drives this window ({})",
                            listener.socket().display()
                        ),
                    );
                    self.listener = Some(listener);
                }
                Err(problem) => session.say(Level::Warning, format!("agents: {problem}")),
            }
        }
    }
}

/// An activity macOS is told of: the app is not napped while it lasts.
#[cfg(target_os = "macos")]
struct Awake(
    objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>,
);

#[cfg(target_os = "macos")]
impl Awake {
    fn begin() -> Self {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        Self(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &NSString::from_str("an agent is driving the editor"),
            ),
        )
    }
}

#[cfg(target_os = "macos")]
impl Drop for Awake {
    fn drop(&mut self) {
        // Safety: the token is the one beginActivity returned.
        unsafe { objc2_foundation::NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}
