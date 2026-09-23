//! The editor. See the crate's documentation for what it is and is not.

#[cfg(target_os = "macos")]
fn main() {
    runity_studio::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "the editor's window is macOS-only today: how the engine's frame reaches \
         a GPUI window is settled there and nowhere else (docs/DNA.md, open \
         question 1). The editor's own work — open, select, drag, undo, import, \
         play — is in runity-editor and runs everywhere, and `runity-mcp` drives \
         it without a window."
    );
    std::process::exit(1);
}
