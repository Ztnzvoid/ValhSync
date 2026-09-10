//! `valsync-invite.txt` next to the executable: the admin ships it with the
//! launcher, and the player never has to paste anything.

use std::path::PathBuf;

use valsync_core::Invite;

use crate::error::Result;
use crate::paths::AppPaths;
use crate::servers::{JoinOutcome, KnownServer, ServerBook};

/// Where the sidecar would be, if the executable's location is known.
pub fn sidecar_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(crate::INVITE_FILE_NAME))
}

/// Import the sidecar if present and not already known. Returns the server it
/// added or updated, or `None` when there was nothing to do. Errors in the
/// file are reported, never fatal: the launcher must still start.
pub fn import_if_present(paths: &AppPaths) -> Result<Option<(KnownServer, JoinOutcome)>> {
    let Some(path) = sidecar_path().filter(|p| p.is_file()) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| crate::SyncError::io(format!("cannot read {}", path.display()), e))?;
    let Some(line) = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("valsync1:"))
    else {
        return Err(crate::SyncError::Other(format!(
            "{} does not contain an invite code (a line starting with valsync1:)",
            path.display()
        )));
    };
    let invite = Invite::parse(line)?;
    let mut book = ServerBook::load(paths)?;
    let outcome = book.join(&invite, false)?;
    if outcome == JoinOutcome::AlreadyKnown {
        return Ok(None);
    }
    book.save(paths)?;
    let server = book.resolve(Some(&invite.name))?.clone();
    Ok(Some((server, outcome)))
}
