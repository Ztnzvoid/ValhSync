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
