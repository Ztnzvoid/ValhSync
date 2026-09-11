//! Starting the Valheim dedicated server from ValhSync.
//!
//! Deliberately one-way: ValhSync can start the game server, never stop it.
//! Valheim saves the world when it receives Ctrl+C in its own console; killing
//! the process would lose everything since the last autosave (30 minutes by
//! default). So the server is launched in **its own console window**, which the
//! admin closes with Ctrl+C exactly as before.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Is a Valheim dedicated server process alive on this machine?
pub fn is_running() -> bool {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| {
        let name = p.name().to_string_lossy();
        name.eq_ignore_ascii_case("valheim_server.exe")
            || name.eq_ignore_ascii_case("valheim_server.x86_64")
    })
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

/// Start the dedicated server. Returns once it has been spawned; it keeps
/// running independently of ValhSync.
pub fn start(launch: &Launch) -> Result<()> {
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
    spawn_in_new_console(&mut cmd);
    cmd.spawn()
        .with_context(|| format!("cannot start {}", launch.describe()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
