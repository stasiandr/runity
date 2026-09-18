//! Saying why, when there is no renderer to say it with.
//!
//! A GPU that will not start is not a panic message in a terminal: the app was
//! very likely double-clicked, and a process that vanishes with a non-zero
//! status has told the user nothing. So the reason goes three places — stderr
//! for whoever ran it from a shell, an `NSAlert` for whoever did not, and the
//! exit status for whatever launched it.

use super::objc::*;

/// Show the reason and leave with a non-zero status.
pub fn fail(title: &str, detail: &str) -> ! {
    eprintln!("runity: {title}\n{detail}");
    // SAFETY: AppKit on the main thread. `sharedApplication` is what makes a
    // modal panel possible at all in a process that never called `run`.
    unsafe {
        let _pool = Pool::push();
        let app = msg_id(class(b"NSApplication\0"), sel(b"sharedApplication\0"));
        if !app.is_null() {
            let alert = msg_id(
                msg_id(class(b"NSAlert\0"), sel(b"alloc\0")),
                sel(b"init\0"),
            );
            if !alert.is_null() {
                let mut heading = nsstring(title);
                let mut body = nsstring(detail);
                msg_with_id(alert, sel(b"setMessageText:\0"), heading);
                msg_with_id(alert, sel(b"setInformativeText:\0"), body);
                msg(alert, sel(b"runModal\0"));
                release(&mut heading);
                release(&mut body);
                let mut alert = alert;
                release(&mut alert);
            }
        }
    }
    std::process::exit(1)
}
