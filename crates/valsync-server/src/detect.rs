//! Guessing where the dedicated server is, how it is started, and with which
//! arguments. Everything here is best effort: the admin can always override
//! what we found.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use valsync_core::steam::{self, VALHEIM_SERVER_APP_ID};

pub fn looks_like_server_root(p: &Path) -> bool {
    p.join("valheim_server.exe").is_file() || p.join("valheim_server.x86_64").is_file()
}

pub fn has_bepinex(p: &Path) -> bool {
    p.join("BepInEx").is_dir()
}

fn common_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        out.push(PathBuf::from(r"C:\valheim_server"));
        out.push(PathBuf::from(r"C:\Games\valheim_server"));
    } else if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        out.push(home.join("valheim_server"));
        out.push(home.join("valheim"));
        out.push(home.join("Steam/steamapps/common/Valheim dedicated server"));
        out.push(PathBuf::from("/home/steam/valheim"));
        out.push(PathBuf::from("/opt/valheim"));
    }
    out
}

/// Steam's own install of app 896660 first, then the usual manual locations.
pub fn detect_server_root() -> Option<PathBuf> {
    steam::find_app(VALHEIM_SERVER_APP_ID, &[])
        .filter(|p| looks_like_server_root(p))
        .or_else(|| {
            common_candidates()
                .into_iter()
                .find(|p| looks_like_server_root(p))
        })
}

/// The executable that actually runs the game server, inside `root`.
pub fn server_binary(root: &Path) -> Option<PathBuf> {
    for name in ["valheim_server.exe", "valheim_server.x86_64"] {
        let p = root.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// A start script found next to the server binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartScript {
    pub path: PathBuf,
    pub args: ServerArgs,
    /// True for the file Steam overwrites on every update.
    pub is_stock: bool,
}

/// What a start script (or a command line) says about the server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerArgs {
    pub name: Option<String>,
    pub world: Option<String>,
    pub port: Option<u16>,
    pub public: Option<bool>,
    pub crossplay: bool,
    /// A password was found. The value is deliberately not kept: ValSync has
    /// no use for it and must never store or publish it.
    pub has_password: bool,
}

const STOCK_SCRIPTS: &[&str] = &[
    "start_headless_server.bat",
    "start_server.sh",
    "start_server_bepinex.sh",
    "start_game_bepinex.sh",
];

/// Scripts in `root` that launch the dedicated server, custom copies first
/// (the stock ones get overwritten by Steam, so admins copy them).
pub fn find_start_scripts(root: &Path) -> Vec<StartScript> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_lowercase();
        let is_script = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("sh"));
        if !is_script {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains("valheim_server") {
            continue;
        }
        out.push(StartScript {
            args: parse_start_script(&text),
            is_stock: STOCK_SCRIPTS.contains(&lower.as_str()),
            path,
        });
    }
    out.sort_by_key(|s| (s.is_stock, s.path.clone()));
    out
}

/// Read the launch arguments out of a `.bat` or `.sh`.
///
/// Handles the shape Iron Gate ships and that admins copy: variables assigned
/// first (`set "PORT=2456"` or `PORT=2456`) and referenced later (`%PORT%`,
/// `$PORT`), with the command split over several lines by `^` or `\`.
pub fn parse_start_script(text: &str) -> ServerArgs {
    let mut vars: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let rest = line
            .strip_prefix("set ")
            .or_else(|| line.strip_prefix("SET "))
            .unwrap_or(line);
        let rest = rest.trim().trim_matches('"');
        if let Some((key, value)) = rest.split_once('=') {
            let key = key.trim().trim_matches('"');
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                vars.insert(
                    key.to_uppercase(),
                    value.trim().trim_matches('"').to_string(),
                );
            }
        }
    }

    // Join the continued command line into one string.
    let mut joined = String::new();
    let mut continuing = false;
    for line in text.lines() {
        let trimmed = line.trim();
        let starts = trimmed.contains("valheim_server");
        if !(continuing || starts) {
            continue;
        }
        if trimmed.starts_with("REM ") || trimmed.starts_with('#') {
            continue;
        }
        let body = trimmed.trim_end();
        let (body, more) = match body.strip_suffix('^').or_else(|| body.strip_suffix('\\')) {
            Some(b) => (b, true),
            None => (body, false),
        };
        joined.push(' ');
        joined.push_str(body.trim_end());
        continuing = more;
        if !more {
            break;
        }
    }

    let expand = |token: &str| -> String {
        let t = token.trim().trim_matches('"');
        let key = t
            .strip_prefix('%')
            .and_then(|k| k.strip_suffix('%'))
            .or_else(|| t.strip_prefix('$'))
            .map(str::to_uppercase);
        match key.and_then(|k| vars.get(&k).cloned()) {
            Some(v) => v,
            None => t.to_string(),
        }
    };

    let tokens = split_args(&joined);
    let mut args = ServerArgs::default();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i].as_str();
        let next = |i: &mut usize| -> Option<String> {
            *i += 1;
            tokens.get(*i).map(|t| expand(t))
        };
        match tok {
            "-name" => args.name = next(&mut i),
            "-world" => args.world = next(&mut i),
            "-port" => args.port = next(&mut i).and_then(|v| v.parse().ok()),
            "-public" => args.public = next(&mut i).map(|v| v.trim() != "0"),
            "-password" => {
                let _ = next(&mut i);
                args.has_password = true;
            }
            "-crossplay" => args.crossplay = true,
            _ => {}
        }
        i += 1;
    }
    args
}

/// Split a command line on whitespace, keeping quoted runs together.
fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAT: &str = r#"@echo off
title Valheim Dedicated Server
setlocal
REM  -port 9999 dans un commentaire ne doit pas compter
set "PASSWORD=secret123"
set "SERVERNAME=Northwatch"
set "WORLD=Northwatch"
set "PORT=2456"
set "PUBLIC=0"
set SteamAppId=892970

valheim_server.exe -nographics -batchmode ^
 -name "%SERVERNAME%" ^
 -port %PORT% ^
 -world "%WORLD%" ^
 -password "%PASSWORD%" ^
 -public %PUBLIC% ^
 -crossplay ^
 -saveinterval 1800
echo done
"#;

    const SH: &str = r#"#!/bin/bash
export templdpath=$LD_LIBRARY_PATH
./valheim_server.x86_64 -name "My server" -port 2457 -world "Dedicated" -public 1
"#;

    #[test]
    fn reads_a_windows_start_script() {
        let a = parse_start_script(BAT);
        assert_eq!(a.name.as_deref(), Some("Northwatch"));
        assert_eq!(a.world.as_deref(), Some("Northwatch"));
        assert_eq!(a.port, Some(2456));
        assert_eq!(a.public, Some(false));
        assert!(a.crossplay);
        assert!(a.has_password);
    }

    #[test]
    fn reads_a_linux_start_script() {
        let a = parse_start_script(SH);
        assert_eq!(a.name.as_deref(), Some("My server"));
        assert_eq!(a.port, Some(2457));
        assert_eq!(a.public, Some(true));
        assert!(!a.crossplay);
        assert!(!a.has_password);
    }

    #[test]
    fn custom_scripts_come_before_stock_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("start_headless_server.bat"), BAT).unwrap();
        std::fs::write(dir.path().join("my_server.bat"), BAT).unwrap();
        std::fs::write(dir.path().join("notes.txt"), BAT).unwrap();
        let found = find_start_scripts(dir.path());
        assert_eq!(found.len(), 2);
        assert!(found[0].path.ends_with("my_server.bat"));
        assert!(!found[0].is_stock);
        assert!(found[1].is_stock);
    }

    #[test]
    fn password_value_is_never_kept() {
        let a = parse_start_script(BAT);
        assert!(a.has_password);
        let debug = format!("{a:?}");
        assert!(!debug.contains("secret123"), "{debug}");
    }
}
