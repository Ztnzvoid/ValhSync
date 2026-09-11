//! A panic leaves a file behind, so a window that dies says why.
//!
//! Without this, a windowed program that panics simply vanishes: no console,
//! no message, nothing to send to whoever wrote it. The hook keeps the default
//! behaviour (the message still goes to stderr for anyone running from a
//! terminal) and writes the same thing next to the program's own files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Set once, at start-up: where to write should the worst happen.
static TARGET: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Write panics to `dir/crash.log` from here on, keeping the default hook.
pub fn install_hook(dir: &Path) {
    if let Ok(mut target) = TARGET.lock() {
        *target = Some(dir.join("crash.log"));
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_report(info);
        previous(info);
    }));
}

/// The file a report would be written to, for a window that wants to say so.
#[must_use]
pub fn log_path() -> Option<PathBuf> {
    TARGET.lock().ok()?.clone()
}

fn write_report(info: &std::panic::PanicHookInfo<'_>) {
    let Some(path) = log_path() else { return };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let where_ = info
        .location()
        .map_or_else(|| "unknown".to_string(), ToString::to_string);
    let _ = writeln!(
        file,
        "{} v{}\n{where_}\n{}\n",
        crate::clock::now_rfc3339(),
        env!("CARGO_PKG_VERSION"),
        message(info)
    );
}

/// The payload as text. Panics carry either a `&str` or a `String`.
fn message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panicked with a payload of an unknown type".to_string()
    }
}

/// The same, for a payload caught by `catch_unwind` rather than the hook.
#[must_use]
pub fn describe(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "an unknown error".to_string()
    }
}
