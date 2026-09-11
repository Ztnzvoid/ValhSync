//! Starting the Valheim dedicated server from ValSync.
//!
//! Deliberately one-way: ValSync can start the game server, never stop it.
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

/// How ValSync should start the game server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Run the admin's own start script. Its directory becomes the working
    /// directory, because the scripts Iron Gate ships call
    /// `valheim_server.exe` by bare name and only work from there.
    Script(PathBuf),
    /// Run the server binary directly with explicit arguments.
    Binary { exe: PathBuf, args: Vec<String> },
}

impl Launch {
    pub fn describe(&self) -> String {
        match self {
            Self::Script(p) => p.display().to_string(),
            Self::Binary { exe, .. } => exe.display().to_string(),
        }
    }

    fn working_dir(&self) -> Option<&Path> {
        match self {
            Self::Script(p) | Self::Binary { exe: p, .. } => p.parent(),
        }
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
/// running independently of ValSync.
pub fn start(launch: &Launch) -> Result<()> {
    if is_running() {
        bail!("a Valheim dedicated server is already running on this machine");
    }
    let cwd = launch
        .working_dir()
        .context("cannot determine the server's folder")?;

    let mut cmd = match launch {
        Launch::Script(path) => {
            if !path.is_file() {
                bail!("{} does not exist", path.display());
            }
            if cfg!(windows) {
                let mut c = Command::new("cmd");
                // /C plus the script as a single argument: no shell parsing of
                // anything ValSync composed, and the admin's script is run
                // verbatim from its own directory.
                c.arg("/C").arg(path);
                c
            } else {
                let mut c = Command::new("/bin/sh");
                c.arg(path);
                c
            }
        }
        Launch::Binary { exe, args } => {
            if !exe.is_file() {
                bail!("{} does not exist", exe.display());
            }
            let mut c = Command::new(exe);
            c.args(args);
            // The stock scripts set this; without it Steam networking refuses
            // to initialise.
            c.env(
                "SteamAppId",
                valsync_core::steam::VALHEIM_APP_ID.to_string(),
            );
            c
        }
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
        let launch = Launch::Script(PathBuf::from("/definitely/not/here.bat"));
        assert!(launch.working_dir().is_some());
        if !is_running() {
            let err = start(&launch).unwrap_err().to_string();
            assert!(err.contains("does not exist"), "{err}");
        }
    }
}
