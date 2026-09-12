//! The world, copied aside before anything is allowed to change the pack.
//!
//! Everything else ValhSync touches can be fetched again: a mod that goes
//! wrong is re-downloaded, and the launcher's own backup puts the old files
//! back (see `valhsync::backup`). The world cannot. It exists on one disk, it
//! holds every hour the players have spent, and adding or updating a mod --
//! one that writes into the save, or one that is removed again afterwards --
//! can leave it unreadable with nothing to restore from. So before a pack
//! changes, the world is copied somewhere ValhSync owns.
//!
//! A world is **two files**, and Iron Gate's manual is plain about it: the
//! world itself is `<name>.db` and its metadata is `<name>.fwl`. Valheim will
//! not load one without the other, so a backup holding only the `.db` is not a
//! backup at all -- [`take`] refuses rather than produce one. Valheim also
//! keeps its own `<name>.db.old` and `<name>.fwl.old` beside them, the
//! previous save it rotates out on each write; those are Valheim's business
//! and are never copied here.
//!
//! Worlds live in `<savedir>/worlds_local/`. The copies deliberately do not:
//! Valheim lists every world it finds in that folder as one a player can join,
//! so a backup left there would show up in the server's own world list and
//! could be started by mistake. [`dir_for`] puts them in a sibling folder with
//! ValhSync's name on it instead.
//!
//! # The copy is only as still as the server
//!
//! Valheim writes the world periodically while it runs, and again on shutdown.
//! Reading a `.db` in the middle of one of those writes gives a torn file that
//! looks fine on disk and fails to load. Nothing here can prevent that -- the
//! dedicated server offers no "freeze" and no hook to wait on -- so this
//! module does not pretend otherwise: [`trust_now`] says whether a server
//! process is alive on this machine, so the window can tell the admin to stop
//! it first, and every backup taken while one was running carries a note
//! inside it saying so.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};

use valhsync_core::clock;

use crate::detect::ServerArgs;

/// The folder the copies go in, beside `worlds_local` rather than inside it.
///
/// Named after the tool that writes it so an admin looking at their save path
/// can tell at a glance that Valheim did not put it there.
pub const DIR_NAME: &str = "valhsync-world-backups";

/// Where a copy is assembled before it is given its real name.
///
/// One fixed name rather than one per backup: a copy interrupted by a crash or
/// a full disk leaves this folder behind, and the next [`take`] clears it
/// instead of letting gigabytes of half-copied world accumulate forever.
const INCOMING: &str = ".incoming.valhsync-tmp";

/// Left in every backup, for whoever opens the folder a year from now with no
/// idea what it is or whether they can trust it.
const NOTE: &str = "valhsync-backup.txt";

/// A folder holding one copy of the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// The folder itself, holding `<name>.db`, `<name>.fwl` and the note.
    pub path: PathBuf,
    /// When the folder was written, from the filesystem.
    pub at: SystemTime,
    /// What it occupies on disk, so a list of backups can say what deleting
    /// one would win back.
    pub bytes: u64,
}

/// Whether a copy taken right now can be trusted.
///
/// This is the whole of the torn-copy problem, reduced to the one thing that
/// can actually be checked from outside the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// No dedicated server is running on this machine, so the world files are
    /// not moving and a copy of them is exact.
    Cold,
    /// A dedicated server is running. It may write the world at any moment,
    /// and a copy taken across one of those writes is torn.
    Live,
}

impl Trust {
    /// What to put in front of the admin before they take a backup, or `None`
    /// when there is nothing to warn about.
    #[must_use]
    pub fn warning(self) -> Option<&'static str> {
        match self {
            Trust::Cold => None,
            Trust::Live => Some(LIVE_WARNING),
        }
    }
}

/// Said whenever a backup is taken, or offered, while the server is up.
///
/// It stops short of refusing: a torn copy of a world is still better than no
/// copy at all when the alternative is kicking the players off, and the admin
/// is the one who knows which it is.
pub const LIVE_WARNING: &str = concat!(
    "The dedicated server is running. It writes the world while it plays, so a ",
    "copy taken now can be torn and fail to load. Stop the server first for a ",
    "backup you can trust.",
);

/// Is the game server running right now?
#[must_use]
pub fn trust_now() -> Trust {
    if crate::gameserver::is_running() {
        Trust::Live
    } else {
        Trust::Cold
    }
}

/// Where this installation's copies belong.
///
/// The same save path Valheim uses -- `-savedir` when the start script sets
/// one, the platform's own folder otherwise -- which is what
/// [`crate::players::lists_dir`] already works out, with ValhSync's own folder
/// under it. `None` when the platform has no home directory to speak of, the
/// same case in which the rest of ValhSync cannot find the worlds either.
#[must_use]
pub fn dir_for(server_root: &Path, args: Option<&ServerArgs>) -> Option<PathBuf> {
    Some(crate::players::lists_dir(server_root, args)?.join(DIR_NAME))
}

/// How much room a backup of this world will want.
///
/// Free space cannot be read without reaching past the standard library, so
/// the honest thing is to hand the caller the size and let it be shown next to
/// whatever the admin already knows about their disk. [`take`] itself fails
/// with a plain message when the room runs out mid-copy.
pub fn bytes_needed(world_db: &Path) -> Result<u64> {
    let (db, fwl) = pair(world_db)?;
    Ok(size_of(&db) + size_of(&fwl))
}

/// Copy the world aside. `why` is a short label that goes in the folder name.
///
/// The copy is assembled under a temporary name and renamed into place only
/// once both files are there, so an interrupted backup is never mistaken for a
/// usable one -- [`list`] will not show it and a restore cannot reach for it.
pub fn take(world_db: &Path, into: &Path, why: &str) -> Result<Backup> {
    let (db, fwl) = pair(world_db)?;
    refuse_worlds_local(into)?;
    let why = tag(why);
    let trust = trust_now();

    fs::create_dir_all(into).with_context(|| format!("cannot create {}", into.display()))?;
    let tmp = into.join(INCOMING);
    if tmp.exists() {
        // Whatever a previous run left behind. It is either finished and
        // renamed, or it is rubbish.
        fs::remove_dir_all(&tmp)
            .with_context(|| format!("cannot clear {}", tmp.display()))
            .or_else(|e| if tmp.exists() { Err(e) } else { Ok(()) })?;
    }
    fs::create_dir(&tmp).with_context(|| format!("cannot create {}", tmp.display()))?;

    let filled = fill(&tmp, &db, &fwl, &why, trust);
    let bytes = match filled {
        Ok(bytes) => bytes,
        Err(e) => {
            // Refuse rather than half-copy: nothing of a failed backup is left
            // where it could later be taken for a good one.
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };

    let path = free_name(into, &why);
    fs::rename(&tmp, &path)
        .with_context(|| format!("cannot put the copy in place at {}", path.display()))?;
    Ok(Backup {
        path,
        at: SystemTime::now(),
        bytes,
    })
}

/// What is already there, newest first.
///
/// Only finished backups: a folder is one when its name is a stamp this
/// module wrote and it holds a world file. Anything else an admin has dropped
/// in there is theirs and is neither listed nor, in [`prune`], deleted.
pub fn list(into: &Path) -> Result<Vec<Backup>> {
    let entries = match fs::read_dir(into) {
        Ok(e) => e,
        // Nothing has ever been backed up. That is an empty list, not a fault.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", into.display())),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !stamped(name) || !entry.path().is_dir() || !holds_a_world(&entry.path()) {
            continue;
        }
        // Built as `into` plus one file name, which is what makes `prune`'s
        // check that the parent is still `into` mean something.
        let path = into.join(name);
        let at = fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let bytes = folder_bytes(&path);
        out.push(Backup { path, at, bytes });
    }
    // By name, which is the stamp, which sorts chronologically as text. File
    // times would not: copying a backup folder about changes them.
    out.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
    Ok(out)
}

/// Keep the newest `keep`, delete the rest. Returns how many went.
///
/// Never all of them, whatever `keep` says, matching the launcher's own
/// pruning: the last copy of a world is the one thing in this program that
/// cannot be fetched again, and a caller that passes zero has made a mistake
/// that should not cost somebody their save.
pub fn prune(into: &Path, keep: usize) -> Result<usize> {
    let all = list(into)?;
    let mut removed = 0;
    for old in all.iter().skip(keep.max(1)) {
        // Provably inside `into`: `list` builds every path as `into` joined
        // with a single file name, and this refuses anything that is not, so
        // no `..`, no absolute path and no symlinked name can send a delete
        // somewhere else.
        if old.path.parent() != Some(into) {
            continue;
        }
        fs::remove_dir_all(&old.path)
            .with_context(|| format!("cannot delete {}", old.path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

/// The two files that make up a world, or the reason there is no backup to be
/// had.
///
/// The `.fwl` is the half that is easy to forget and fatal to miss, so a world
/// missing it is refused here rather than copied anyway and discovered to be
/// useless on the day it is needed.
fn pair(world_db: &Path) -> Result<(PathBuf, PathBuf)> {
    if world_db.extension().is_none_or(|e| e != "db") {
        bail!(
            "{} is not a world file. ValhSync wants the world's .db -- not its .db.old, which is \
             the copy Valheim rotates out for itself",
            world_db.display()
        );
    }
    if !world_db.is_file() {
        bail!("there is no world at {}", world_db.display());
    }
    let fwl = world_db.with_extension("fwl");
    if !fwl.is_file() {
        bail!(
            "{} has no {} beside it. A Valheim world is both files and will not load without its \
             .fwl, so there is nothing worth backing up here",
            world_db.display(),
            fwl.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    Ok((world_db.to_path_buf(), fwl))
}

/// Put both files and the note in the folder. Returns what they weigh.
fn fill(tmp: &Path, db: &Path, fwl: &Path, why: &str, trust: Trust) -> Result<u64> {
    for from in [db, fwl] {
        let name = from.file_name().unwrap_or_default();
        copy(from, &tmp.join(name))?;
    }
    let world = db.file_stem().unwrap_or_default().to_string_lossy();
    let mut note = format!(
        "ValhSync copied this world aside.\n\nworld: {world}\ntaken: {}\nreason: {why}\n",
        clock::now_rfc3339()
    );
    note.push_str(match trust {
        Trust::Cold => {
            "state: the dedicated server was not running, so this copy is exact.\n\nTo restore \
             it, stop the server and put both files back in worlds_local.\n"
        }
        Trust::Live => {
            "state: the dedicated server WAS RUNNING. It writes the world as it plays, so this \
             copy may have been taken mid-write and may not load. Treat it as a last resort, and \
             take another with the server stopped.\n\nTo restore it, stop the server and put both \
             files back in worlds_local.\n"
        }
    });
    fs::write(tmp.join(NOTE), note.as_bytes())
        .with_context(|| format!("cannot write {}", tmp.join(NOTE).display()))?;
    Ok(folder_bytes(tmp))
}

/// One file copied, with the out-of-room case answered properly.
///
/// A world is the biggest thing ValhSync ever writes, and "Access is denied.
/// (os error 112)" in front of somebody whose disk is full helps nobody.
fn copy(from: &Path, to: &Path) -> Result<u64> {
    match fs::copy(from, to) {
        Ok(n) => Ok(n),
        Err(e) if out_of_room(&e) => bail!(
            "ran out of room copying {}: the drive holding {} needs {} free for this world, and \
             has not got it",
            from.display(),
            to.display(),
            mib(size_of(from))
        ),
        Err(e) => {
            Err(e).with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))
        }
    }
}

/// A full disk, however this platform spells it.
fn out_of_room(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded
    ) || matches!(
        e.raw_os_error(),
        // ENOSPC, and Windows' two ways of saying the same thing.
        Some(28 | 112 | 39)
    )
}

/// Sizes as an admin reads them, without a float anywhere near the arithmetic.
fn mib(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    format!("{}.{} MiB", bytes / MIB, bytes % MIB * 10 / MIB)
}

fn size_of(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}

fn folder_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(std::fs::Metadata::is_file)
        .map(|m| m.len())
        .sum()
}

/// The caller's reason, reduced to something that can only ever be one folder
/// name.
///
/// Whatever comes in -- a command line argument, a mod's name, a line typed
/// into the window -- leaves as letters, digits and `-`. That is what makes
/// `..`, `/`, `\`, a drive letter and a trailing space impossible by
/// construction rather than by a list of things to look for. Everything else
/// becomes a separator rather than being dropped, so `a/b` cannot quietly
/// close up into one word that means something different.
fn tag(why: &str) -> String {
    let mut out = String::new();
    for c in why.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            // One separator, however many characters produced it.
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "backup".to_string()
    } else {
        out.to_string()
    }
}

/// `20260912-051622-before-update`, and the next free one when a second
/// backup lands in the same second.
///
/// The stamp leads so the names sort chronologically as text, which is what
/// [`list`] and [`prune`] rely on, and is the same shape the launcher gives
/// its own backup folders.
fn free_name(into: &Path, why: &str) -> PathBuf {
    let stamp = clock::dir_stamp();
    let first = into.join(format!("{stamp}-{why}"));
    if !first.exists() {
        return first;
    }
    // The counter goes on the end, where it cannot break the ordering the
    // stamp gives.
    for n in 2..1000 {
        let next = into.join(format!("{stamp}-{why}-{n}"));
        if !next.exists() {
            return next;
        }
    }
    first
}

/// Does this folder name look like one this module wrote?
///
/// `20260912-051622` and then anything: eight digits, a dash, six digits. It
/// is what keeps [`prune`] off folders that belong to somebody else.
fn stamped(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 15
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..15].iter().all(u8::is_ascii_digit)
}

/// Is there actually a world in there? An empty folder with the right name is
/// not a backup, and deleting one to make room for another would be a bad
/// trade.
fn holds_a_world(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
    })
}

/// Refuse to put backups where Valheim would offer them as worlds to join.
fn refuse_worlds_local(into: &Path) -> Result<()> {
    if into
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("worlds_local"))
    {
        bail!(
            "{} is inside worlds_local, where Valheim lists everything it finds as a world a \
             player can join. Backups go beside it, in {DIR_NAME}",
            into.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world as Valheim leaves it: both files, plus the `.old` pair it
    /// rotates out for itself.
    fn world(dir: &Path, name: &str) -> PathBuf {
        let worlds = dir.join("worlds_local");
        fs::create_dir_all(&worlds).unwrap();
        let db = worlds.join(format!("{name}.db"));
        fs::write(&db, b"world bytes").unwrap();
        fs::write(worlds.join(format!("{name}.fwl")), b"metadata").unwrap();
        fs::write(worlds.join(format!("{name}.db.old")), b"stale world").unwrap();
        fs::write(worlds.join(format!("{name}.fwl.old")), b"stale metadata").unwrap();
        db
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_world_without_its_fwl_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        fs::remove_file(db.with_extension("fwl")).unwrap();
        let err = take(&db, &home.path().join(DIR_NAME), "test")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Midgard.fwl"), "{err}");
        // And nothing was left behind pretending to be a backup.
        assert!(list(&home.path().join(DIR_NAME)).unwrap().is_empty());
    }

    #[test]
    fn both_halves_of_the_world_are_copied_and_valheims_own_old_files_are_not() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let into = home.path().join(DIR_NAME);
        let backup = take(&db, &into, "before update").unwrap();
        assert_eq!(
            names(&backup.path),
            ["Midgard.db", "Midgard.fwl", NOTE],
            "the .old pair is Valheim's and is not ours to copy"
        );
        assert_eq!(
            fs::read(backup.path.join("Midgard.db")).unwrap(),
            b"world bytes"
        );
        assert_eq!(backup.bytes, folder_bytes(&backup.path));
    }

    #[test]
    fn the_folder_name_starts_with_a_sortable_stamp_and_carries_the_reason() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let backup = take(&db, &home.path().join(DIR_NAME), "Before Update").unwrap();
        let name = backup
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(stamped(&name), "{name}");
        assert!(name.ends_with("-before-update"), "{name}");
    }

    #[test]
    fn a_reason_cannot_escape_the_backup_folder() {
        // Every one of these would be a path if it survived as written.
        for why in [
            "../../etc",
            "C:\\Windows",
            "..",
            "a/b",
            "  ",
            "",
            "a very long reason that somebody pasted in from a changelog entry",
        ] {
            let name = tag(why);
            assert!(!name.is_empty(), "{why:?}");
            assert!(
                name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{why:?} became {name:?}"
            );
            assert_eq!(Path::new(&name).components().count(), 1, "{why:?}");
            assert!(name.len() <= 40, "{why:?} became {name:?}");
        }
        assert_eq!(tag(".."), "backup");
        assert_eq!(tag("a/b"), "a-b");
    }

    #[test]
    fn two_backups_in_the_same_second_do_not_overwrite_each_other() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let into = home.path().join(DIR_NAME);
        let first = take(&db, &into, "same").unwrap();
        let second = take(&db, &into, "same").unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(list(&into).unwrap().len(), 2);
    }

    #[test]
    fn backups_are_listed_newest_first() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let into = home.path().join(DIR_NAME);
        for _ in 0..3 {
            take(&db, &into, "why").unwrap();
        }
        let listed = list(&into).unwrap();
        assert_eq!(listed.len(), 3);
        let names: Vec<_> = listed.iter().map(|b| b.path.file_name().unwrap()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.reverse();
        assert_eq!(names, sorted);
    }

    #[test]
    fn a_missing_backup_folder_is_an_empty_list() {
        let home = tempfile::tempdir().unwrap();
        assert!(list(&home.path().join("never-used")).unwrap().is_empty());
    }

    #[test]
    fn prune_deletes_backups_and_provably_nothing_else() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let into = home.path().join(DIR_NAME);
        for _ in 0..4 {
            take(&db, &into, "why").unwrap();
        }
        // Things that are not ValhSync's, in and around the backup folder.
        fs::create_dir_all(into.join("my own notes")).unwrap();
        fs::write(into.join("my own notes/keep.txt"), b"mine").unwrap();
        fs::write(into.join("readme.txt"), b"mine").unwrap();
        // A folder with the right shape of name but no world in it.
        fs::create_dir_all(into.join("20200101-000000-empty")).unwrap();

        assert_eq!(prune(&into, 2).unwrap(), 2);
        assert_eq!(list(&into).unwrap().len(), 2);
        assert!(into.join("my own notes/keep.txt").is_file());
        assert!(into.join("readme.txt").is_file());
        assert!(into.join("20200101-000000-empty").is_dir());
        // The world itself, one folder up, is untouched.
        assert!(db.is_file());
        assert!(db.with_extension("fwl").is_file());
    }

    #[test]
    fn prune_never_deletes_the_last_copy_of_a_world() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let into = home.path().join(DIR_NAME);
        take(&db, &into, "only").unwrap();
        assert_eq!(prune(&into, 0).unwrap(), 0);
        assert_eq!(list(&into).unwrap().len(), 1);
    }

    #[test]
    fn backups_are_refused_inside_worlds_local() {
        // Valheim offers everything in there as a world to join, so a backup
        // living there would turn up in the server's own world list.
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let err = take(&db, &home.path().join("worlds_local/backups"), "why")
            .unwrap_err()
            .to_string();
        assert!(err.contains("worlds_local"), "{err}");
    }

    #[test]
    fn a_half_finished_copy_is_never_listed_and_is_cleared_by_the_next_one() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let into = home.path().join(DIR_NAME);
        fs::create_dir_all(into.join(INCOMING)).unwrap();
        fs::write(into.join(INCOMING).join("Midgard.db"), b"torn").unwrap();
        assert!(
            list(&into).unwrap().is_empty(),
            "an unfinished copy is not a backup"
        );

        take(&db, &into, "why").unwrap();
        assert!(!into.join(INCOMING).exists(), "the leftovers were cleared");
        assert_eq!(list(&into).unwrap().len(), 1);
    }

    #[test]
    fn valheims_own_old_file_is_not_taken_for_a_world() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let old = db.with_extension("db.old");
        let err = take(&old, &home.path().join(DIR_NAME), "why")
            .unwrap_err()
            .to_string();
        assert!(err.contains(".db.old"), "{err}");
    }

    #[test]
    fn the_size_of_a_backup_is_known_before_it_is_taken() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let want = bytes_needed(&db).unwrap();
        assert_eq!(want, "world bytes".len() as u64 + "metadata".len() as u64);
        let backup = take(&db, &home.path().join(DIR_NAME), "why").unwrap();
        assert!(backup.bytes >= want, "the note is in there too");
    }

    #[test]
    fn a_full_disk_is_reported_as_a_full_disk() {
        // The message an admin sees instead of "os error 112".
        let full = std::io::Error::from_raw_os_error(if cfg!(windows) { 112 } else { 28 });
        assert!(out_of_room(&full));
        assert!(!out_of_room(&std::io::Error::from(
            std::io::ErrorKind::NotFound
        )));
        assert_eq!(mib(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn a_backup_taken_while_the_server_ran_says_so_in_the_folder() {
        let home = tempfile::tempdir().unwrap();
        let db = world(home.path(), "Midgard");
        let tmp = home.path().join("note");
        fs::create_dir_all(&tmp).unwrap();
        fill(&tmp, &db, &db.with_extension("fwl"), "why", Trust::Live).unwrap();
        let note = fs::read_to_string(tmp.join(NOTE)).unwrap();
        assert!(note.contains("WAS RUNNING"), "{note}");
        assert_eq!(Trust::Live.warning(), Some(LIVE_WARNING));
        assert_eq!(Trust::Cold.warning(), None);
    }
}
