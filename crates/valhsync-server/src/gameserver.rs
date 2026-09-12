//! Starting and stopping the Valheim dedicated server from ValhSync.
//!
//! Valheim saves the world when it receives Ctrl+C, and only then: killing the
//! process loses everything since the last autosave. So the server is launched
//! in **its own console window**, and [`stop`] sends that console the same
//! Ctrl+C the admin would type into it. ValhSync never terminates the process
//! itself.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The names the dedicated server runs under, on either platform.
const PROCESS_NAMES: &[&str] = &["valheim_server.exe", "valheim_server.x86_64"];

/// Is a Valheim dedicated server process alive on this machine?
pub fn is_running() -> bool {
    pid().is_some()
}

/// The dedicated server's process id, when one is running.
#[must_use]
pub fn pid() -> Option<u32> {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes().iter().find_map(|(pid, p)| {
        let name = p.name().to_string_lossy();
        PROCESS_NAMES
            .iter()
            .any(|k| name.eq_ignore_ascii_case(k))
            .then(|| pid.as_u32())
    })
}

/// Ask the server to shut down the way its own console does: Ctrl+C, which is
/// what makes Valheim write the world to disk before exiting.
///
/// Returns as soon as the signal is delivered. Saving a large world takes a
/// few seconds, during which the process is still alive; callers watch
/// [`is_running`] rather than assume it is gone.
pub fn stop() -> Result<()> {
    let pid = pid().context("no Valheim dedicated server is running on this machine")?;
    interrupt(pid)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn interrupt(pid: u32) -> Result<()> {
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
        SetConsoleCtrlHandler,
    };

    // A Ctrl+C event goes to a console, not to a process, so ValhSync has to
    // join the server's console to send one. `GenerateConsoleCtrlEvent` with
    // group 0 then reaches every process attached to it — ourselves included,
    // hence the handler that ignores it for the duration.
    //
    // SAFETY: none of these take pointers or memory we own. The only argument
    // is a process id, and each call reports failure through its return value,
    // which is checked. The handler is restored and the console released on
    // every path out.
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            let err = std::io::Error::last_os_error();
            AttachConsole(ATTACH_PARENT_PROCESS);
            bail!(
                "cannot reach the server's console window ({err}).                  Press Ctrl+C in it instead: that is what saves the world."
            );
        }
        SetConsoleCtrlHandler(None, 1);
        let sent = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
        let err = std::io::Error::last_os_error();
        FreeConsole();
        SetConsoleCtrlHandler(None, 0);
        // Put our own console back, so the command-line face keeps printing.
        AttachConsole(ATTACH_PARENT_PROCESS);
        if sent == 0 {
            bail!("the server's console refused the stop signal ({err})");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn interrupt(pid: u32) -> Result<()> {
    // SIGINT is the same signal Ctrl+C raises, and `kill` avoids an FFI call
    // for something the system already ships.
    let status = Command::new("kill")
        .arg("-INT")
        .arg(pid.to_string())
        .status()
        .context("cannot run kill(1) to signal the server")?;
    if !status.success() {
        bail!("kill -INT {pid} failed");
    }
    Ok(())
}

/// Type a line into the dedicated server's console, the way the admin would.
///
/// Valheim reads its commands from the console it was started in, so this is
/// the same trick [`stop`] uses for Ctrl+C: join that console and put the
/// keystrokes in its input buffer. It is one-way -- the server answers in its
/// own window and its log, never back to here.
pub fn send_command(line: &str) -> Result<()> {
    let line = one_console_line(line)?;
    let pid = pid().context("no Valheim dedicated server is running on this machine")?;
    type_into_console(pid, &line)
}

/// A command is one line. Newlines are refused rather than passed on: they
/// would run commands the admin did not see themselves type.
fn one_console_line(line: &str) -> Result<String> {
    let line = line.trim();
    if line.is_empty() {
        bail!("there is nothing to send");
    }
    if line.chars().count() > 512 {
        bail!("that is longer than one console line");
    }
    if let Some(c) = line.chars().find(|c| c.is_control()) {
        bail!("a command is a single line; remove the {c:?} in it");
    }
    Ok(line.to_string())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn type_into_console(pid: u32, line: &str) -> Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, FreeConsole, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT,
        KEY_EVENT_RECORD, KEY_EVENT_RECORD_0, WriteConsoleInputW,
    };

    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const VK_RETURN: u16 = 0x0D;

    let key = |ch: u16, vk: u16, down: i32| INPUT_RECORD {
        #[allow(clippy::cast_possible_truncation)] // KEY_EVENT is 0x0001
        EventType: KEY_EVENT as u16,
        Event: INPUT_RECORD_0 {
            KeyEvent: KEY_EVENT_RECORD {
                bKeyDown: down,
                wRepeatCount: 1,
                wVirtualKeyCode: vk,
                wVirtualScanCode: 0,
                uChar: KEY_EVENT_RECORD_0 { UnicodeChar: ch },
                dwControlKeyState: 0,
            },
        },
    };
    let mut records = Vec::new();
    for unit in line.encode_utf16() {
        records.push(key(unit, 0, 1));
        records.push(key(unit, 0, 0));
    }
    // The Enter that submits the line.
    records.push(key(VK_RETURN, VK_RETURN, 1));
    records.push(key(VK_RETURN, VK_RETURN, 0));

    let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();

    // SAFETY: every call is checked, the only pointers handed over are into
    // `name` and `records`, both alive for the duration, and the console is
    // released on every path out -- including the early return below.
    unsafe {
        FreeConsole();
        if AttachConsole(pid) == 0 {
            let err = std::io::Error::last_os_error();
            AttachConsole(ATTACH_PARENT_PROCESS);
            bail!(
                "cannot reach the server's console window ({err}). Type the command in it instead."
            );
        }
        let handle = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            let err = std::io::Error::last_os_error();
            FreeConsole();
            AttachConsole(ATTACH_PARENT_PROCESS);
            bail!("cannot open the server console's input ({err})");
        }
        let mut written = 0u32;
        #[allow(clippy::cast_possible_truncation)]
        let count = records.len() as u32;
        let ok = WriteConsoleInputW(handle, records.as_ptr(), count, &raw mut written);
        let err = std::io::Error::last_os_error();
        CloseHandle(handle);
        FreeConsole();
        AttachConsole(ATTACH_PARENT_PROCESS);
        if ok == 0 {
            bail!("the server's console refused the command ({err})");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn type_into_console(_pid: u32, _line: &str) -> Result<()> {
    // Elsewhere the server's console is a terminal ValhSync does not own, and
    // there is no equivalent of joining it. Saying so beats pretending.
    bail!("sending console commands is only supported on Windows")
}

/// The admin's own start script. Its directory becomes the working
/// directory: the scripts Iron Gate ships call `valheim_server.exe` by bare
/// name and only work from there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch(pub PathBuf);

impl Launch {
    pub fn describe(&self) -> String {
        self.0.display().to_string()
    }

    fn working_dir(&self) -> Option<&Path> {
        self.0.parent()
    }
}

#[cfg(windows)]
fn spawn_in_new_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_CONSOLE: the server gets its own window, so Ctrl+C there
    // reaches it and it saves the world before exiting.
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    cmd.creation_flags(CREATE_NEW_CONSOLE);
}

#[cfg(not(windows))]
fn spawn_in_new_console(_cmd: &mut Command) {}

/// The `cmd.exe` (or `sh`) ValhSync started the script with.
///
/// Worth keeping hold of for one reason: a batch file interrupted by Ctrl+C
/// leaves cmd.exe asking "Terminate batch job (Y/N)?" and its window sitting
/// there until somebody answers. Valheim has already written the world and
/// exited by then, so there is nothing left in that console to protect --
/// only a question nobody wants to be asked.
#[derive(Debug)]
pub struct Shell(std::process::Child);

impl Shell {
    /// Close the leftover shell, once the game itself is gone.
    ///
    /// Does nothing while the dedicated server is still alive: that process
    /// is the one ValhSync never kills, and a shell still running it is not
    /// leftover.
    pub fn close_if_game_gone(&mut self) -> bool {
        if is_running() {
            return false;
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
        true
    }
}

/// Start the dedicated server. Returns once it has been spawned; it keeps
/// running independently of ValhSync.
pub fn start(launch: &Launch) -> Result<Shell> {
    if is_running() {
        bail!("a Valheim dedicated server is already running on this machine");
    }
    let cwd = launch
        .working_dir()
        .context("cannot determine the server's folder")?;

    let path = &launch.0;
    if !path.is_file() {
        bail!("{} does not exist", path.display());
    }
    let is_script = path.extension().is_some_and(|e| {
        ["bat", "cmd", "sh"]
            .iter()
            .any(|k| e.eq_ignore_ascii_case(k))
    });
    if !is_script {
        bail!(
            "{} is not a start script (.bat, .cmd or .sh)",
            path.display()
        );
    }
    // cmd.exe re-parses its command line, so a path holding a quote or an
    // ampersand could change the command. Refuse those rather than escape
    // them: no real start script has such a name.
    if path
        .to_string_lossy()
        .contains(['"', '&', '|', '^', '<', '>'])
    {
        bail!(
            "{} has a name ValhSync will not pass to a shell; rename it",
            path.display()
        );
    }
    // The script is passed as a single argument: nothing ValhSync composed is
    // ever parsed by a shell.
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(path);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.arg(path);
        c
    };

    cmd.current_dir(cwd);
    // The scripts Iron Gate ships call `valheim_server.exe` by bare name, so
    // the folder has to be searched. Relying on the current directory is not
    // enough: `NoDefaultCurrentDirectoryInExePath` switches that off, and
    // plenty of shells and launchers set it. Put the folder on the child's
    // PATH instead, which works either way.
    cmd.env_remove("NoDefaultCurrentDirectoryInExePath");
    let mut search = vec![cwd.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        search.extend(std::env::split_paths(&existing));
    }
    if let Ok(path) = std::env::join_paths(search) {
        cmd.env("PATH", path);
    }
    spawn_in_new_console(&mut cmd);
    let child = cmd
        .spawn()
        .with_context(|| format!("cannot start {}", launch.describe()))?;
    Ok(Shell(child))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_scripts_are_launched() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_script = dir.path().join("valheim_server.exe");
        std::fs::write(&not_a_script, b"x").unwrap();
        if !is_running() {
            let err = start(&Launch(not_a_script)).unwrap_err().to_string();
            assert!(err.contains("not a start script"), "{err}");

            let tricky = dir.path().join("start&calc.bat");
            std::fs::write(&tricky, b"@echo off").unwrap();
            let err = start(&Launch(tricky)).unwrap_err().to_string();
            assert!(err.contains("will not pass to a shell"), "{err}");
        }
    }

    #[test]
    fn missing_targets_are_reported() {
        let launch = Launch(PathBuf::from("/definitely/not/here.bat"));
        assert!(launch.working_dir().is_some());
        if !is_running() {
            let err = start(&launch).unwrap_err().to_string();
            assert!(err.contains("does not exist"), "{err}");
        }
    }
}
