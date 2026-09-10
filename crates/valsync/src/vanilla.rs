//! "Play without mods": BepInEx loads through `winhttp.dll`; renaming it to
//! `winhttp.dll.off` disables the whole mod stack without deleting anything.
//! The next sync restores `winhttp.dll` from the manifest and removes the
//! `.off` copy.

use std::path::Path;

use valsync_core::path::to_os_path;

use crate::error::{Result, SyncError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModsState {
    /// `winhttp.dll` present: BepInEx loads.
    On,
    /// Only `winhttp.dll.off` present: vanilla game.
    Off,
    /// Neither: BepInEx is not installed.
    NotInstalled,
}

pub fn state(root: &Path) -> ModsState {
    let on = to_os_path(root, "winhttp.dll").is_file();
    let off = to_os_path(root, "winhttp.dll.off").is_file();
    match (on, off) {
        (true, _) => ModsState::On,
        (false, true) => ModsState::Off,
        (false, false) => ModsState::NotInstalled,
    }
}

/// Enable or disable mods. Returns the resulting state.
pub fn set(root: &Path, mods_on: bool) -> Result<ModsState> {
    let on = to_os_path(root, "winhttp.dll");
    let off = to_os_path(root, "winhttp.dll.off");
    match (mods_on, state(root)) {
        (true, ModsState::Off) => {
            SyncError::at(&on, "cannot restore", std::fs::rename(&off, &on))?;
        }
        (false, ModsState::On) => {
            let _ = std::fs::remove_file(&off);
            SyncError::at(
                &off,
                "cannot rename winhttp.dll to",
                std::fs::rename(&on, &off),
            )?;
        }
        _ => {}
    }
    Ok(state(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(state(tmp.path()), ModsState::NotInstalled);
        std::fs::write(tmp.path().join("winhttp.dll"), b"d").unwrap();
        assert_eq!(set(tmp.path(), false).unwrap(), ModsState::Off);
        assert!(tmp.path().join("winhttp.dll.off").is_file());
        assert_eq!(set(tmp.path(), false).unwrap(), ModsState::Off);
        assert_eq!(set(tmp.path(), true).unwrap(), ModsState::On);
        assert!(!tmp.path().join("winhttp.dll.off").exists());
    }
}
