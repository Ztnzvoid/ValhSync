//! Finding Valheim, telling whether it runs, and starting it through Steam.

use std::path::{Path, PathBuf};
use std::process::Command;

use valsync_core::steam::{self, VALHEIM_APP_ID};

use crate::error::{Result, SyncError};
use crate::settings::Settings;

/// How BepInEx gets loaded on this install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// `winhttp.dll` next to `valheim.exe`, loaded by Windows itself.
    Windows,
    /// Windows build under Proton: needs `WINEDLLOVERRIDES="winhttp=n,b"`.
    LinuxProton,
    /// Native Linux build: needs the `start_game_bepinex.sh` launch option.
    LinuxNative,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameInstall {
    pub root: PathBuf,
    pub steam_root: Option<PathBuf>,
    pub flavor: Flavor,
}

pub fn looks_like_valheim(root: &Path) -> bool {
    root.join("valheim.exe").is_file() || root.join("valheim.x86_64").is_file()
}

#[cfg(windows)]
fn registry_steam_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Valve\\Steam")
        .ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    Some(PathBuf::from(path))
}

#[cfg(not(windows))]
fn registry_steam_path() -> Option<PathBuf> {
    None
}

/// Candidate Steam roots, most authoritative first.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = registry_steam_path().into_iter().collect();
    for r in steam::default_steam_roots() {
        if !roots.contains(&r) {
            roots.push(r);
        }
    }
    roots.retain(|r| r.is_dir());
    roots
}

fn flavor_of(root: &Path) -> Flavor {
    if cfg!(windows) {
        Flavor::Windows
    } else if cfg!(target_os = "linux") {
        if root.join("valheim.x86_64").is_file() {
            Flavor::LinuxNative
        } else {
            Flavor::LinuxProton
        }
    } else {
        Flavor::Other
    }
}

/// The player's Valheim: the manual setting first, then Steam.
pub fn locate(settings: &Settings) -> Result<GameInstall> {
    let steam_roots = steam_roots();
    let steam_root = steam_roots.first().cloned();
    if let Some(root) = &settings.game_root {
        if !looks_like_valheim(root) {
            return Err(SyncError::NotAGameFolder(root.clone()));
        }
        return Ok(GameInstall {
            root: root.clone(),
            steam_root,
            flavor: flavor_of(root),
        });
    }
    let root = steam::find_app(VALHEIM_APP_ID, &steam_roots)
        .filter(|p| looks_like_valheim(p))
        .ok_or(SyncError::GameNotFound)?;
    Ok(GameInstall {
        flavor: flavor_of(&root),
        root,
        steam_root,
    })
}

/// Is a Valheim process alive? Refreshes the process list once.
pub fn is_running() -> bool {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| {
        let name = p.name().to_string_lossy();
        name.eq_ignore_ascii_case("valheim.exe")
            || name.eq_ignore_ascii_case("valheim.x86_64")
            || name.eq_ignore_ascii_case("valheim")
    })
}

/// How the game was started, for the summary line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMethod {
    SteamExe(PathBuf),
    SteamCommand(String),
    SteamUrl(String),
}

fn steam_url(address: &str) -> String {
    // `steam://run/<appid>//<args>/`; the space before +connect is %20.
    format!("steam://run/{VALHEIM_APP_ID}//+connect%20{address}/")
}

/// Ask Steam to start Valheim and connect to `address`. Steam handles the
/// game's own environment (Proton, overlay); ValSync never runs the game
/// binary itself.
pub fn launch(install: &GameInstall, address: &str) -> Result<LaunchMethod> {
    let args = [
        "-applaunch",
        &VALHEIM_APP_ID.to_string(),
        "+connect",
        address,
    ];
    let spawn_err =
        |what: &str, e: std::io::Error| SyncError::io(format!("cannot start {what}"), e);

    if cfg!(windows) {
        if let Some(steam_exe) = install
            .steam_root
            .as_ref()
            .map(|r| r.join("steam.exe"))
            .filter(|p| p.is_file())
        {
            Command::new(&steam_exe)
                .args(args)
                .spawn()
                .map_err(|e| spawn_err("Steam", e))?;
            return Ok(LaunchMethod::SteamExe(steam_exe));
        }
        // `explorer` hands a steam:// URL to the protocol handler and, unlike
        // `cmd /C start`, never interprets shell metacharacters in it.
        let url = steam_url(address);
        Command::new("explorer")
            .arg(&url)
            .spawn()
            .map_err(|e| spawn_err("the steam:// link", e))?;
        return Ok(LaunchMethod::SteamUrl(url));
    }

    if cfg!(target_os = "linux") {
        if Command::new("steam").args(args).spawn().is_ok() {
            return Ok(LaunchMethod::SteamCommand("steam".into()));
        }
        let flatpak = Command::new("flatpak")
            .args(["run", "com.valvesoftware.Steam"])
            .args(args)
            .spawn();
        if flatpak.is_ok() {
            return Ok(LaunchMethod::SteamCommand(
                "flatpak run com.valvesoftware.Steam".into(),
            ));
        }
        let url = steam_url(address);
        Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| spawn_err("xdg-open", e))?;
        return Ok(LaunchMethod::SteamUrl(url));
    }

    let url = steam_url(address);
    Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|_| SyncError::SteamNotFound)?;
    Ok(LaunchMethod::SteamUrl(url))
}

/// Platform-specific note the player must act on for BepInEx to load at all.
/// ValSync does not edit Steam launch options (out of scope for v1).
pub fn bepinex_hint(install: &GameInstall) -> Option<&'static str> {
    match install.flavor {
        Flavor::Windows | Flavor::Other => None,
        Flavor::LinuxProton => Some(
            "Proton: BepInEx only loads if Valheim's Steam launch options contain\n  \
             WINEDLLOVERRIDES=\"winhttp=n,b\" %command%\n  \
             (Steam > Valheim > Properties > Launch options)",
        ),
        Flavor::LinuxNative => Some(
            "Native Linux: set Valheim's Steam launch options to\n  \
             ./start_game_bepinex.sh %command%\n  \
             (Steam > Valheim > Properties > Launch options)",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_root_must_look_like_valheim() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings {
            game_root: Some(tmp.path().to_path_buf()),
            ..Settings::default()
        };
        assert!(matches!(
            locate(&settings),
            Err(SyncError::NotAGameFolder(_))
        ));
        std::fs::write(tmp.path().join("valheim.exe"), b"x").unwrap();
        let install = locate(&settings).unwrap();
        assert_eq!(install.root, tmp.path());
    }

    #[test]
    fn steam_url_shape() {
        assert_eq!(
            steam_url("valheim.example.org:2456"),
            "steam://run/892970//+connect%20valheim.example.org:2456/"
        );
    }
}
