//! The sync itself: fetch, plan, download, back up, apply, record.
//!
//! Order of operations is what makes a failed sync harmless:
//!
//! 1. Everything is downloaded to `_valhsync_tmp/` inside the game folder and
//!    verified **before** anything is touched.
//! 2. A backup journal is opened. Each change is stashed, then done, then
//!    journaled, in that order.
//! 3. The installed state is written last. If anything fails in between,
//!    the journal is replayed backwards on the spot.

use std::path::{Path, PathBuf};

use valhsync_core::path::{ensure_within_root, to_os_path};
use valhsync_core::plan::{PlanCounts, PlannedFile};
use valhsync_core::{
    Action, InstalledState, Invite, Limits, Manifest, QUARANTINE_ROOT, SyncPlan, clock, plan,
};

use crate::backup::{Backup, Done};
use crate::error::{Result, SyncError};
use crate::game::{self, GameInstall};
use crate::http::Client;
use crate::paths::{
    AppPaths, copy_atomic, move_file, read_json, remove_empty_parents, write_json_atomic,
};
use crate::servers::{KnownServer, ServerBook};
use crate::settings::Settings;

/// Everything a sync needs to know about this machine.
#[derive(Debug)]
pub struct Context {
    pub paths: AppPaths,
    pub settings: Settings,
    pub client: Client,
    /// Tests run against a fake game folder while a real Valheim may be
    /// open on the developer's machine; they turn the process check off.
    pub skip_process_check: bool,
    /// Accept a manifest older than the last one applied (admin rolled back).
    pub allow_older: bool,
    /// Replace every file that differs, configuration included.
    pub repair: bool,
}

impl Context {
    pub fn discover() -> Result<Self> {
        let paths = AppPaths::discover()?;
        let settings = Settings::load(&paths)?;
        Ok(Self {
            paths,
            settings,
            client: Client::new()?,
            skip_process_check: false,
            allow_older: false,
            repair: false,
        })
    }

    pub fn with(paths: AppPaths, settings: Settings) -> Result<Self> {
        Ok(Self {
            paths,
            settings,
            client: Client::new()?,
            skip_process_check: false,
            allow_older: false,
            repair: false,
        })
    }

    fn game_running(&self) -> bool {
        !self.skip_process_check && game::is_running()
    }

    pub fn installed(&self) -> Result<Option<InstalledState>> {
        read_json(&self.paths.installed_file())
    }
}

/// Progress events, for the CLI's console output and the window's bar.
#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    Fetching {
        url: &'a str,
    },
    Planned(&'a SyncPlan),
    Downloading {
        index: usize,
        count: usize,
        path: &'a str,
        size: u64,
    },
    /// Overall bytes downloaded so far, out of the plan's total.
    Progress {
        done: u64,
        total: u64,
    },
    Applying {
        changes: usize,
    },
    Done,
}

pub trait Progress {
    fn on(&mut self, event: Event<'_>);
}

impl<F: FnMut(Event<'_>)> Progress for F {
    fn on(&mut self, event: Event<'_>) {
        self(event);
    }
}

/// Discards progress.
#[derive(Debug, Default, Clone, Copy)]
pub struct Silent;

impl Progress for Silent {
    fn on(&mut self, _: Event<'_>) {}
}

/// A verified manifest and the plan to reach it. Nothing has been changed yet.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub server: KnownServer,
    pub install: GameInstall,
    pub manifest: Manifest,
    pub plan: SyncPlan,
    pub previous: Option<InstalledState>,
    /// First sync ever, or first sync with this server: the plan should be
    /// shown and confirmed.
    pub needs_confirmation: bool,
}

impl Prepared {
    pub fn is_up_to_date(&self) -> bool {
        self.plan.is_noop()
    }
}

/// What a finished sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub counts: PlanCounts,
    pub downloaded_bytes: u64,
    pub backup_stamp: Option<String>,
    pub quarantine_dir: Option<String>,
}

/// What an address turned out to be, before the player accepts it.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub invite: Invite,
    /// Fingerprint of the key the server published, for the player to compare
    /// with what the admin announced.
    pub fingerprint: String,
    pub files: usize,
}

/// Ask the server at `address` who it is: fetch its key, then check that the
/// key really signs its manifest. The player still confirms the fingerprint.
pub fn discover(ctx: &Context, address: &str) -> Result<Discovered> {
    let url = valhsync_core::invite::address_to_url(address)
        .ok_or_else(|| SyncError::Other(format!("{address:?} is not a server address")))?;
    let key = ctx.client.fetch_key(&url)?;
    let (manifest, _) = ctx.client.fetch_manifest(
        &url,
        &key,
        &ctx.settings.allowed_roots(),
        &Limits::default(),
    )?;
    Ok(Discovered {
        fingerprint: key.fingerprint(),
        files: manifest.files.len(),
        invite: Invite::new(url, &key, manifest.server_name.trim()),
    })
}

/// Could the server not be reached at all? A different failure means it
/// answered, and the problem is elsewhere.
pub fn is_unreachable(error: &SyncError) -> bool {
    matches!(
        error,
        SyncError::Unreachable { .. } | SyncError::HttpStatus { .. }
    )
}

/// Fetch and verify the manifest, locate the game, compute the plan.
pub fn prepare(
    ctx: &Context,
    server: &KnownServer,
    progress: &mut dyn Progress,
) -> Result<Prepared> {
    let install = game::locate(&ctx.settings)?;
    let key = server.public_key()?;
    progress.on(Event::Fetching { url: &server.url });
    let (manifest, _) = ctx
        .client
        .fetch_manifest(
            &server.url,
            &key,
            &ctx.settings.allowed_roots(),
            &Limits::default(),
        )
        .map_err(|e| match e {
            // A bad signature on a URL we trust means the key changed (or an impostor).
            SyncError::Core(valhsync_core::CoreError::BadSignature) => SyncError::KeyMismatch {
                name: server.name.clone(),
                pinned: server.fingerprint(),
                received: "a different key".into(),
            },
            other => other,
        })?;
    // Replay protection: a valid signature does not prove freshness. Someone
    // on the network path could serve yesterday's manifest to reinstall a mod
    // version the admin has since replaced.
    if !ctx.allow_older
        && let Some(seen) = server.last_generated_at.as_deref()
        && let (Some(seen_at), Some(got_at)) = (
            clock::parse_rfc3339(seen),
            clock::parse_rfc3339(&manifest.generated_at),
        )
        && got_at < seen_at
    {
        return Err(SyncError::OlderManifest {
            name: server.name.clone(),
            seen: seen.to_string(),
            received: manifest.generated_at.clone(),
        });
    }
    let previous = ctx.installed()?;
    let plan = plan::compute_with(&manifest, &install.root, previous.as_ref(), ctx.repair)?;
    progress.on(Event::Planned(&plan));
    let needs_confirmation = previous.as_ref().is_none_or(|p| p.server_id != server.id);
    Ok(Prepared {
        server: server.clone(),
        install,
        manifest,
        plan,
        previous,
        needs_confirmation,
    })
}

/// Apply a prepared plan. Refuses while the game runs.
pub fn apply(ctx: &Context, prepared: &Prepared, progress: &mut dyn Progress) -> Result<Applied> {
    let root = &prepared.install.root;
    if ctx.game_running() {
        return Err(SyncError::GameRunning);
    }
    let counts = prepared.plan.counts();
    if prepared.plan.is_noop() {
        finish_bookkeeping(ctx, prepared)?;
        progress.on(Event::Done);
        return Ok(Applied {
            counts,
            downloaded_bytes: 0,
            backup_stamp: None,
            quarantine_dir: None,
        });
    }

    let tmp = root.join(crate::TMP_DIR_NAME);
    let _ = std::fs::remove_dir_all(&tmp);
    SyncError::at(&tmp, "cannot create", std::fs::create_dir_all(&tmp))?;

    let downloaded_bytes = match download_all(ctx, prepared, &tmp, progress) {
        Ok(bytes) => bytes,
        Err(e) => {
            // Nothing was touched yet; just drop the partial downloads.
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };

    let stamp = clock::dir_stamp();
    let quarantine_rel = format!("{QUARANTINE_ROOT}/{stamp}");
    let mut backup = Backup::create(
        &ctx.paths,
        &prepared.server,
        &prepared.manifest.pack_id,
        root,
        prepared.previous.clone(),
    )?;
    let changes = prepared
        .plan
        .items
        .iter()
        .filter(|i| i.action.changes_disk())
        .count();
    progress.on(Event::Applying { changes });

    let outcome = apply_changes(&prepared.plan, root, &tmp, &quarantine_rel, &mut backup)
        .and_then(|()| finish_bookkeeping(ctx, prepared));

    let _ = std::fs::remove_dir_all(&tmp);

    if let Err(reason) = outcome {
        let stamp = backup.record.stamp.clone();
        return Err(match backup.rollback(&ctx.paths.installed_file()) {
            Ok(_) => SyncError::RolledBack {
                reason: reason.to_string(),
                stamp,
            },
            Err(rb) => SyncError::RollbackFailed {
                reason: reason.to_string(),
                stamp,
                rollback_error: rb.to_string(),
            },
        });
    }
    backup.mark_completed()?;
    let _ = Backup::prune(&ctx.paths, ctx.settings.keep_backups);

    progress.on(Event::Done);
    Ok(Applied {
        counts,
        downloaded_bytes,
        backup_stamp: Some(backup.record.stamp.clone()),
        quarantine_dir: (counts.quarantine > 0).then_some(quarantine_rel),
    })
}

fn download_all(
    ctx: &Context,
    prepared: &Prepared,
    tmp: &Path,
    progress: &mut dyn Progress,
) -> Result<u64> {
    let entries = prepared.manifest.by_lower_path();
    let to_get: Vec<&PlannedFile> = prepared
        .plan
        .items
        .iter()
        .filter(|i| i.action.needs_download())
        .collect();
    let total = prepared.plan.download_bytes;
    let mut done_before: u64 = 0;
    for (index, item) in to_get.iter().enumerate() {
        let entry = entries
            .get(&item.path.to_lowercase())
            .ok_or_else(|| SyncError::Other(format!("{} vanished from the manifest", item.path)))?;
        progress.on(Event::Downloading {
            index: index + 1,
            count: to_get.len(),
            path: &entry.path,
            size: entry.size,
        });
        let dest = tmp.join(&entry.blake3);
        // Same content planned twice (two paths, one hash): download once.
        if !dest.is_file() {
            let mut report = |received: u64| {
                progress.on(Event::Progress {
                    done: done_before + received,
                    total,
                });
            };
            ctx.client
                .download(&prepared.server.url, entry, &dest, &mut report)?;
        }
        done_before += entry.size;
        progress.on(Event::Progress {
            done: done_before,
            total,
        });
    }
    Ok(done_before)
}

/// Replay the plan against the disk, journaling as it goes. Quarantine first
/// (so a foreign DLL never coexists with the new one), then removals, then
/// installs.
fn apply_changes(
    plan: &SyncPlan,
    root: &Path,
    tmp: &Path,
    quarantine_rel: &str,
    backup: &mut Backup,
) -> Result<()> {
    for item in plan.with_action(Action::Quarantine) {
        let from = ensure_within_root(root, &item.path)?;
        let to_rel = format!("{quarantine_rel}/{}", item.path);
        let to = ensure_within_root(root, &to_rel)?;
        move_file(&from, &to)?;
        backup.record_done(Done::Quarantined {
            path: item.path.clone(),
            to: to_rel,
        })?;
    }
    for item in plan.with_action(Action::Remove) {
        let target = ensure_within_root(root, &item.path)?;
        move_file(&target, &backup.stash_path(&item.path))?;
        backup.record_done(Done::Removed {
            path: item.path.clone(),
        })?;
        remove_empty_parents(root, &target);
    }
    for item in plan.items.iter().filter(|i| i.action.needs_download()) {
        let hash = item
            .blake3
            .as_deref()
            .ok_or_else(|| SyncError::Other(format!("{}: no digest in plan", item.path)))?;
        let staged = tmp.join(hash);
        let target = ensure_within_root(root, &item.path)?;
        if item.action == Action::Replace {
            copy_atomic(&target, &backup.stash_path(&item.path))?;
        }
        if let Some(parent) = target.parent() {
            SyncError::at(parent, "cannot create", std::fs::create_dir_all(parent))?;
        }
        // Several plan entries may share one staged blob: copy, don't move,
        // unless this is the last user. Copy is simplest and still atomic.
        copy_atomic(&staged, &target)?;
        backup.record_done(match item.action {
            Action::Replace => Done::Replaced {
                path: item.path.clone(),
            },
            _ => Done::Added {
                path: item.path.clone(),
            },
        })?;
    }
    Ok(())
}

/// Record the new installed state, note the pack on the server entry, and
/// clean up a leftover `winhttp.dll.off` from "play without mods".
fn finish_bookkeeping(ctx: &Context, prepared: &Prepared) -> Result<()> {
    let state = InstalledState::from_manifest(&prepared.manifest, &prepared.server.id);
    write_json_atomic(&ctx.paths.installed_file(), &state)?;

    let root = &prepared.install.root;
    let off = to_os_path(root, "winhttp.dll.off");
    if off.is_file() && to_os_path(root, "winhttp.dll").is_file() {
        let _ = std::fs::remove_file(off);
    }

    let mut book = ServerBook::load(&ctx.paths)?;
    book.note_pack(
        &prepared.server.id,
        &prepared.manifest.pack_id,
        &prepared.manifest.generated_at,
    );
    book.save(&ctx.paths)
}

/// Undo the most recent sync.
pub fn rollback(ctx: &Context) -> Result<(String, crate::backup::RollbackReport)> {
    if ctx.game_running() {
        return Err(SyncError::GameRunning);
    }
    let mut backup = Backup::latest_restorable(&ctx.paths)?.ok_or(SyncError::NoBackup)?;
    let report = backup.rollback(&ctx.paths.installed_file())?;
    Ok((backup.record.stamp.clone(), report))
}

/// Path of the quarantine folder inside a game root, for display.
pub fn quarantine_dir(root: &Path) -> PathBuf {
    to_os_path(root, QUARANTINE_ROOT)
}
