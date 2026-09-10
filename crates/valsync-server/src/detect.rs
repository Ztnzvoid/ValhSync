//! Guessing where the dedicated server is installed, for `init`.

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
