//! The shell on Android, where the Activity owns the window (DNA, rule 1):
//! the game is a library the Activity loads, and `android_main` is where
//! it is handed the app. [`start`] is the first thing it calls; then the
//! game's own `main` runs as on the desktop, and [`crate::shell::run`]
//! builds its loop on the app it was handed.
//!
//! * What the game prints — every `eprintln!` — goes to logcat, under the
//!   game's name: an app's stdout and stderr go nowhere.
//! * The project's data (scenes, prefabs, the library) is packed in the
//!   APK's `assets/data/`, with `assets/data/files.txt` listing it —
//!   Android's assets cannot list a folder's folders. It is copied into
//!   the app's own files the first time a build runs, and the game reads
//!   it from there with `std::fs`, as everywhere else
//!   ([`scrap_core::project::data_file`]). An APK built without its data
//!   reads what was put in that folder by hand (`adb push` and `run-as …
//!   cp`, as the game's script does): a quick iteration, a small APK.
//! * The player's own files go to the app's internal files.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use winit::platform::android::activity::{self, AndroidApp};

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// The app the Activity started, once [`start`] has been handed it.
pub(crate) fn app() -> Option<AndroidApp> {
    APP.get().cloned()
}

/// Called first from the game's `android_main`: logging, the data, the
/// player's folder. `name` is what logcat tags the game's lines with.
pub fn start(app: AndroidApp, name: &str) {
    to_logcat(name);
    environment();
    // wgpu's and winit's warnings: on a desktop nobody installs a logger
    // either, but there a validation error's panic says enough.
    let level = if std::env::var_os("SCRAP_LOG_INFO").is_some() { log::LevelFilter::Info } else { log::LevelFilter::Warn };
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(level);
    }
    let internal = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    std::env::set_var(scrap_core::player_prefs::USER_DIR_VAR, internal.join("user"));
    let data = internal.join(scrap_core::project::DATA);
    let started = std::time::Instant::now();
    match unpack(&app, &data) {
        Ok(0) => {}
        Ok(n) => eprintln!("{n} files unpacked to {} in {:.1?}", data.display(), started.elapsed()),
        // An APK built without its data: what was put there by hand is read.
        Err(e) => eprintln!("the data was not unpacked ({e}); reading {}", data.display()),
    }
    scrap_core::project::set_data_dir(data);
    let _ = APP.set(app);
}

/// An app is started with no environment of its own: the variables a
/// game reads (`SCRAP_SCENE`, `SCRAP_LOOP_TIMES`…) come from the system
/// property `debug.scrap.env` instead, as `NAME=value` separated by
/// spaces: `adb shell setprop debug.scrap.env "SCRAP_SCENE=HandsSandbox"`.
fn environment() {
    let mut value = [0u8; 92]; // PROP_VALUE_MAX
    // SAFETY: a NUL-terminated name and a buffer of the size the call writes at most.
    let n = unsafe { libc::__system_property_get(c"debug.scrap.env".as_ptr(), value.as_mut_ptr().cast()) };
    let text = String::from_utf8_lossy(&value[..n.max(0) as usize]).into_owned();
    for pair in text.split_whitespace() {
        if let Some((key, value)) = pair.split_once('=') {
            eprintln!("{key}={value} (debug.scrap.env)");
            std::env::set_var(key, value);
        }
    }
}

/// The list of what the APK carries, under `assets/data/`.
const LIST: &str = "files.txt";

/// Copy `assets/data/` into `dir`, unless the copy there is of this very
/// list (the list's first line is the build's stamp). Returns how many
/// files it copied.
fn unpack(app: &AndroidApp, dir: &Path) -> std::io::Result<usize> {
    let assets = app.asset_manager();
    let open = |relative: &str| {
        let name = std::ffi::CString::new(format!("data/{relative}")).map_err(std::io::Error::other)?;
        assets
            .open(&name)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no asset data/{relative}")))
    };
    let mut list = String::new();
    open(LIST)?.read_to_string(&mut list)?;
    let done = dir.join(LIST);
    if std::fs::read_to_string(&done).is_ok_and(|had| had == list) {
        return Ok(0);
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    let mut count = 0;
    let mut buffer = vec![0u8; 1 << 20];
    for relative in list.lines().skip(1).filter(|l| !l.is_empty()) {
        let to = dir.join(relative);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut from = open(relative)?;
        let mut out = std::io::BufWriter::new(std::fs::File::create(&to)?);
        loop {
            let n = from.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            out.write_all(&buffer[..n])?;
        }
        out.flush()?;
        count += 1;
    }
    // Last, so an unpacking cut short is done again next time.
    std::fs::write(done, list)?;
    Ok(count)
}

/// `log`'s records, to stderr and so to logcat.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
    }
    fn flush(&self) {}
}

/// stdout and stderr, line by line, to logcat.
fn to_logcat(name: &str) {
    let mut pipe = [0; 2];
    // SAFETY: plain libc calls on descriptors this process owns.
    unsafe {
        if libc::pipe(pipe.as_mut_ptr()) != 0 {
            return;
        }
        libc::dup2(pipe[1], libc::STDOUT_FILENO);
        libc::dup2(pipe[1], libc::STDERR_FILENO);
        libc::close(pipe[1]);
    }
    let tag = std::ffi::CString::new(name).unwrap_or_default();
    // SAFETY: the read end is ours alone from here.
    let read = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(pipe[0]) };
    let _ = std::thread::Builder::new().name("logcat".into()).spawn(move || {
        for line in std::io::BufReader::new(read).lines().map_while(Result::ok) {
            let Ok(text) = std::ffi::CString::new(line) else { continue };
            // SAFETY: two NUL-terminated strings that outlive the call.
            unsafe {
                activity::ndk_sys::__android_log_write(activity::ndk_sys::android_LogPriority::ANDROID_LOG_INFO.0 as i32, tag.as_ptr(), text.as_ptr());
            }
        }
    });
}
