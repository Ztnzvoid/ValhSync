//! Replacing the launcher with a newer build of itself.
//!
//! The decision of *whether* to replace it is made in `valhsync-core`, against
//! a signed offer; this module only carries it out on disk. Two rules shape
//! everything here. A running executable on Windows cannot be overwritten, but
//! it can be renamed -- Windows locks a file's contents, not its directory
//! entry -- so the install is two renames rather than a copy. And the player
//! must never be left with a folder that has no launcher in it: if the second
//! rename fails, the first one is undone before anyone hears about the error.

use std::path::{Path, PathBuf};

use valhsync_core::UpdateOffer;

use crate::error::{Result, SyncError};

/// Where a verified build waits until it is installed.
const NEW_SUFFIX: &str = ".new";

/// Where the launcher that was running gets moved to.
const OLD_SUFFIX: &str = ".old";

/// The target triple this launcher was built for, as `build.rs` recorded it.
/// An offer naming any other triple is a build for someone else's machine.
pub fn current_target() -> &'static str {
    env!("VALHSYNC_TARGET")
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Is this offer worth acting on? Both halves matter: a newer build for
/// another platform is a brick, and the same version again is not an update.
pub fn wanted(offer: &UpdateOffer) -> bool {
    offer.runs_on(current_target()) && offer.is_newer_than(current_version())
}

/// Where to download a new build to: beside the running executable, so
/// installing it is a rename within one directory -- same volume, atomic, and
/// nothing to interrupt halfway.
pub fn staging_path() -> Result<PathBuf> {
    Ok(with_suffix(&current_exe()?, NEW_SUFFIX))
}

/// Put `staged` in the place of the running executable and return the path to
/// run. The launcher keeps the name it was started under, so shortcuts, batch
/// files and Steam entries still point at a working launcher afterwards.
pub fn install(staged: &Path) -> Result<PathBuf> {
    swap(&current_exe()?, staged)
}

/// Start the new launcher and return; the caller exits immediately after.
///
/// Nothing is waited on: this process has to be gone before Windows will
/// release the old executable, and the child outlives it.
pub fn relaunch(exe: &Path) -> Result<()> {
    let mut command = std::process::Command::new(exe);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    command
        .spawn()
        .map_err(|e| SyncError::io(format!("cannot start {}", exe.display()), e))?;
    Ok(())
}

/// Delete the launcher the last update replaced.
///
/// Not at the end of the swap: at that moment the old file is still the image
/// of the running process and Windows refuses to unlink it. One start later it
/// is an ordinary file. Failures are ignored -- a leftover megabyte is not
/// worth a message, and the next start will try again.
pub fn clean_stale() {
    if let Ok(exe) = std::env::current_exe() {
        clean_stale_at(&exe);
    }
}

fn clean_stale_at(current: &Path) {
    let _ = std::fs::remove_file(with_suffix(current, OLD_SUFFIX));
}

fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().map_err(|e| SyncError::io("cannot locate the launcher itself", e))
}

/// `valhsync.exe` -> `valhsync.exe.new`. Appended rather than replacing the
/// extension, so the two files can never collide with a real `valhsync.new`
/// and neither looks runnable to the shell.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// The swap itself, against an explicit path so it can be tested against a
/// temporary directory instead of the running process.
fn swap(current: &Path, staged: &Path) -> Result<PathBuf> {
    let old = with_suffix(current, OLD_SUFFIX);
    // A leftover from a previous update would make the rename fail: on Windows
    // renaming onto an existing file is refused.
    let _ = std::fs::remove_file(&old);
    SyncError::at(current, "cannot move aside", std::fs::rename(current, &old))?;

    let Err(e) = std::fs::rename(staged, current) else {
        return Ok(current.to_path_buf());
    };
    // We are between two launchers. Put the working one back before saying
    // anything, and if even that fails, say exactly which file to rename.
    Err(match std::fs::rename(&old, current) {
        Ok(()) => SyncError::io(format!("cannot install {}", staged.display()), e),
        Err(back) => SyncError::Other(format!(
            "cannot install {} ({e}), and putting the previous launcher back failed too ({back}). \
             It is still there: rename {} to {}",
            staged.display(),
            old.display(),
            current.display()
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use valhsync_core::update::{UPDATE_FORMAT, parse_version};

    fn offer(version: &str, target: &str) -> UpdateOffer {
        UpdateOffer {
            format: UPDATE_FORMAT,
            version: version.into(),
            target: target.into(),
            exe: "valhsync.exe".into(),
            size: 4096,
            blake3: "a".repeat(64),
        }
    }

    #[test]
    fn an_update_is_wanted_only_when_it_is_newer_and_for_this_machine() {
        let (major, _, _) = parse_version(current_version()).expect("our own version parses");
        let newer = format!("{}.0.0", major + 1);

        assert!(wanted(&offer(&newer, current_target())));
        assert!(
            !wanted(&offer(current_version(), current_target())),
            "the build already running is not an update"
        );
        assert!(
            !wanted(&offer(&newer, "sparc64-unknown-netbsd")),
            "a newer build for another platform is a brick, not an update"
        );
    }

    #[test]
    fn the_new_launcher_takes_the_name_the_old_one_was_started_under() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("valhsync.exe");
        let staged = with_suffix(&current, NEW_SUFFIX);
        std::fs::write(&current, b"launcher 1").unwrap();
        std::fs::write(&staged, b"launcher 2").unwrap();

        let installed = swap(&current, &staged).unwrap();

        assert_eq!(installed, current);
        assert_eq!(std::fs::read(&current).unwrap(), b"launcher 2");
        assert!(!staged.exists(), "the staged file was moved, not copied");
        assert_eq!(
            std::fs::read(with_suffix(&current, OLD_SUFFIX)).unwrap(),
            b"launcher 1",
            "the previous launcher is kept until the next start can delete it"
        );
    }

    /// The window where the player has no launcher at all must close again.
    #[test]
    fn a_failed_install_puts_the_previous_launcher_back() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("valhsync.exe");
        std::fs::write(&current, b"launcher 1").unwrap();
        let missing = tmp.path().join("valhsync.exe.new");

        let err = swap(&current, &missing).unwrap_err();

        assert!(current.is_file(), "the player must still have a launcher");
        assert_eq!(std::fs::read(&current).unwrap(), b"launcher 1");
        assert!(
            !with_suffix(&current, OLD_SUFFIX).exists(),
            "nothing should be left aside once it is back in place"
        );
        assert!(err.to_string().contains("cannot install"), "{err}");
    }

    #[test]
    fn cleaning_up_touches_only_the_launcher_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("valhsync.exe");
        for name in [
            "valhsync.exe",
            "valhsync.exe.old",
            "valhsync.exe.new",
            "valhsync.old",
            "servers.json",
        ] {
            std::fs::write(tmp.path().join(name), b"x").unwrap();
        }

        clean_stale_at(&current);

        assert!(!tmp.path().join("valhsync.exe.old").exists());
        for kept in [
            "valhsync.exe",
            "valhsync.exe.new",
            "valhsync.old",
            "servers.json",
        ] {
            assert!(tmp.path().join(kept).is_file(), "{kept} should survive");
        }
    }

    /// Called on every start, usually with nothing to do.
    #[test]
    fn cleaning_up_with_nothing_to_clean_is_quiet() {
        let tmp = tempfile::tempdir().unwrap();
        clean_stale_at(&tmp.path().join("valhsync.exe"));
    }
}
