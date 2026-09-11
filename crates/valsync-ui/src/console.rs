//! Console re-attachment for the CLI face of a windowed Windows executable.

/// On Windows, a `windows_subsystem = "windows"` process starts without a
/// console. When launched from a terminal we attach to the parent's so that
/// `println!` and `eprintln!` land where the user is looking. Elsewhere this
/// is a no-op.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn attach_parent() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: `AttachConsole` takes a process id and has no memory-safety
    // preconditions; it fails harmlessly (returns 0) when there is no parent
    // console, which is exactly the double-click case.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
    // The console cursor sits after the shell prompt; start on a fresh line.
    println!();
}

#[cfg(not(windows))]
pub fn attach_parent() {}
