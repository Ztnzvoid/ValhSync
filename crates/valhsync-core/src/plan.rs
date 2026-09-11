//! Computing what a sync has to do, without doing it.
//!
//! The plan compares three things: the manifest (what the server wants), the
//! game folder (what is there) and the previous installed state (what ValhSync
//! itself put there). Nothing is written; the launcher applies the plan later.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::error::{CoreError, Result};
use crate::hash;
use crate::manifest::{FileEntry, Manifest, Policy};
use crate::path;
use crate::state::{InstalledFile, InstalledState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Not present locally: download and install.
    Add,
    /// Present with a different hash and policy `enforce`: download and replace.
    Replace,
    /// Installed by ValhSync earlier, no longer in the manifest: move to backup.
    Remove,
    /// Found in a managed root, unknown to the manifest: move to quarantine.
    Quarantine,
    /// Present with a different hash but policy `seed`: left alone.
    SeedKept,
    /// Already identical.
    Keep,
}

impl Action {
    pub fn needs_download(self) -> bool {
        matches!(self, Self::Add | Self::Replace)
    }

    pub fn changes_disk(self) -> bool {
        matches!(
            self,
            Self::Add | Self::Replace | Self::Remove | Self::Quarantine
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFile {
    pub path: String,
    pub action: Action,
    /// Manifest size for downloads, on-disk size otherwise.
    pub size: u64,
    /// Manifest digest for Add/Replace/Keep/SeedKept.
    pub blake3: Option<String>,
    pub policy: Option<Policy>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPlan {
    pub pack_id: String,
    pub items: Vec<PlannedFile>,
    pub download_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanCounts {
    pub add: usize,
    pub replace: usize,
    pub remove: usize,
    pub quarantine: usize,
    pub seed_kept: usize,
    pub keep: usize,
}

impl SyncPlan {
    /// True when applying the plan would not touch the disk.
    pub fn is_noop(&self) -> bool {
        self.items.iter().all(|i| !i.action.changes_disk())
    }

    pub fn counts(&self) -> PlanCounts {
        let mut c = PlanCounts::default();
        for i in &self.items {
            match i.action {
                Action::Add => c.add += 1,
                Action::Replace => c.replace += 1,
                Action::Remove => c.remove += 1,
                Action::Quarantine => c.quarantine += 1,
                Action::SeedKept => c.seed_kept += 1,
                Action::Keep => c.keep += 1,
            }
        }
        c
    }

    pub fn with_action(&self, action: Action) -> impl Iterator<Item = &PlannedFile> {
        self.items.iter().filter(move |i| i.action == action)
    }
}

/// Compare `manifest` against `game_root` and `previous`, and decide.
///
/// Unmanaged files inside `manifest.managed_roots` are quarantined with one
/// deliberate nuance: mods write their own data (translations, caches) next to
/// their DLL at runtime. So inside a folder that the manifest manages, only
/// foreign `.dll` files are quarantined; other files are left alone. A folder
/// the manifest knows nothing about is quarantined whole, and so is any loose
/// `.dll` directly in a managed root.
pub fn compute(
    manifest: &Manifest,
    game_root: &Path,
    previous: Option<&InstalledState>,
) -> Result<SyncPlan> {
    compute_with(manifest, game_root, previous, false)
}

/// As [`compute`], but `repair` ignores the `seed` policy: every file that
/// differs from the manifest is replaced, configuration included. It is what
/// a player asks for when their installation is in a state they cannot
/// explain.
pub fn compute_with(
    manifest: &Manifest,
    game_root: &Path,
    previous: Option<&InstalledState>,
    repair: bool,
) -> Result<SyncPlan> {
    let wanted = manifest.by_lower_path();
    let installed: HashMap<String, _> = previous
        .map(InstalledState::by_lower_path)
        .unwrap_or_default();
    let mut items = Vec::with_capacity(manifest.files.len());

    // 1. Manifest entries: add, replace, keep, or seed-kept.
    for entry in &manifest.files {
        let os_path = path::ensure_within_root(game_root, &entry.path)?;
        let action = match std::fs::symlink_metadata(&os_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Action::Add,
            Err(e) => return Err(CoreError::io(os_path, e)),
            Ok(md) if md.is_dir() => return Err(CoreError::Obstructed(os_path)),
            Ok(_) => {
                let local = hash::to_hex(&hash::hash_file(&os_path)?);
                if local == entry.blake3 {
                    Action::Keep
                } else if entry.policy == Policy::Seed && !repair {
                    Action::SeedKept
                } else {
                    Action::Replace
                }
            }
        };
        items.push(PlannedFile {
            path: entry.path.clone(),
            action,
            size: entry.size,
            blake3: Some(entry.blake3.clone()),
            policy: Some(entry.policy),
        });
    }

    // 2. Files we installed before that the server no longer ships.
    for (lower, file) in &installed {
        if wanted.contains_key(lower) {
            continue;
        }
        let os_path = path::ensure_within_root(game_root, &file.path)?;
        if let Ok(md) = std::fs::symlink_metadata(&os_path)
            && md.is_file()
        {
            items.push(PlannedFile {
                path: file.path.clone(),
                action: Action::Remove,
                size: md.len(),
                blake3: None,
                policy: Some(file.policy),
            });
        }
    }

    // 3. Unmanaged files in managed roots.
    quarantine_unmanaged(manifest, game_root, &wanted, &installed, &mut items)?;

    items.sort_by(|a, b| a.action.cmp(&b.action).then_with(|| a.path.cmp(&b.path)));
    let download_bytes = items
        .iter()
        .filter(|i| i.action.needs_download())
        .map(|i| i.size)
        .sum();
    Ok(SyncPlan {
        pack_id: manifest.pack_id.clone(),
        items,
        download_bytes,
    })
}

/// Step 3 of [`compute`]: walk each managed root and list what has to go to
/// quarantine. See the heuristic described on [`compute`].
fn quarantine_unmanaged(
    manifest: &Manifest,
    game_root: &Path,
    wanted: &HashMap<String, &FileEntry>,
    installed: &HashMap<String, &InstalledFile>,
    items: &mut Vec<PlannedFile>,
) -> Result<()> {
    let quarantine_lower = crate::QUARANTINE_ROOT.to_lowercase();
    for root in &manifest.managed_roots {
        let root_lower = root.to_lowercase();
        let root_os = path::ensure_within_root(game_root, root)?;
        if !root_os.is_dir() {
            continue;
        }
        // Top-level folders under this root that contain at least one managed file.
        let managed_dirs: HashSet<String> = wanted
            .keys()
            .filter_map(|k| k.strip_prefix(&format!("{root_lower}/")))
            .filter_map(|rest| rest.split_once('/').map(|(first, _)| first.to_string()))
            .collect();

        for entry in WalkDir::new(&root_os).follow_links(false).min_depth(1) {
            let entry = entry.map_err(|e| {
                let p = e.path().map_or_else(|| root_os.clone(), Path::to_path_buf);
                CoreError::io(p, e.into())
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(rel_in_root) = path::to_rel_path(&root_os, entry.path()) else {
                continue;
            };
            let rel = format!("{root}/{rel_in_root}");
            let lower = rel.to_lowercase();
            if wanted.contains_key(&lower)
                || installed.contains_key(&lower)
                || lower.starts_with(&quarantine_lower)
            {
                continue;
            }
            let is_dll = Path::new(&rel)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("dll"));
            let quarantine = match rel_in_root.split_once('/') {
                None => is_dll,
                Some((first, _)) => is_dll || !managed_dirs.contains(&first.to_lowercase()),
            };
            if quarantine {
                items.push(PlannedFile {
                    path: rel,
                    action: Action::Quarantine,
                    size: entry.metadata().map_or(0, |m| m.len()),
                    blake3: None,
                    policy: None,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::entry;
    use std::fs;

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let p = path::to_os_path(root, rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn manifest() -> Manifest {
        Manifest::new(
            "S",
            "h:2456",
            vec!["BepInEx/plugins".into(), "BepInEx/patchers".into()],
            vec![
                entry("winhttp.dll", b"doorstop", Policy::Enforce),
                entry("BepInEx/plugins/Azu/Azu.dll", b"azu v2", Policy::Enforce),
                entry("BepInEx/plugins/Azu/Azu.cfg", b"azu cfg", Policy::Enforce),
                entry("BepInEx/config/Azu.cfg", b"seeded", Policy::Seed),
                entry("BepInEx/core/BepInEx.dll", b"core", Policy::Enforce),
            ],
        )
    }

    fn action_of(plan: &SyncPlan, p: &str) -> Option<Action> {
        plan.items.iter().find(|i| i.path == p).map(|i| i.action)
    }

    #[test]
    fn fresh_install_adds_everything() {
        let game = tempfile::tempdir().unwrap();
        write(game.path(), "valheim.exe", b"game");
        let plan = compute(&manifest(), game.path(), None).unwrap();
        assert_eq!(plan.counts().add, 5);
        assert_eq!(plan.download_bytes, manifest().total_bytes());
        assert!(!plan.is_noop());
    }

    #[test]
    fn second_run_is_noop() {
        let game = tempfile::tempdir().unwrap();
        let g = game.path();
        write(g, "winhttp.dll", b"doorstop");
        write(g, "BepInEx/plugins/Azu/Azu.dll", b"azu v2");
        write(g, "BepInEx/plugins/Azu/Azu.cfg", b"azu cfg");
        write(g, "BepInEx/config/Azu.cfg", b"seeded");
        write(g, "BepInEx/core/BepInEx.dll", b"core");
        let m = manifest();
        let state = InstalledState::from_manifest(&m, "srv");
        let plan = compute(&m, g, Some(&state)).unwrap();
        assert!(plan.is_noop(), "{plan:?}");
        assert_eq!(plan.counts().keep, 5);
        assert_eq!(plan.download_bytes, 0);
    }

    #[test]
    fn replace_seed_remove_and_quarantine() {
        let game = tempfile::tempdir().unwrap();
        let g = game.path();
        write(g, "winhttp.dll", b"doorstop");
        write(g, "BepInEx/core/BepInEx.dll", b"core");
        write(g, "BepInEx/plugins/Azu/Azu.dll", b"azu v1"); // outdated -> Replace
        write(g, "BepInEx/plugins/Azu/Azu.cfg", b"azu cfg");
        write(
            g,
            "BepInEx/plugins/Azu/Azu.translations.yml",
            b"generated by mod",
        ); // left alone
        write(
            g,
            "BepInEx/plugins/Azu/Sneaky.dll",
            b"foreign dll in managed folder",
        ); // Quarantine
        write(g, "BepInEx/config/Azu.cfg", b"player changed keybind"); // Seed -> kept
        write(
            g,
            "BepInEx/plugins/EquipmentAndQuickSlots/EAQS.dll",
            b"leftover",
        ); // Quarantine
        write(
            g,
            "BepInEx/plugins/EquipmentAndQuickSlots/readme.txt",
            b"leftover",
        ); // Quarantine
        write(g, "BepInEx/plugins/LooseOld.dll", b"loose"); // Quarantine
        write(g, "BepInEx/plugins/notes.txt", b"loose non-dll"); // left alone
        write(
            g,
            "BepInEx/plugins/OldMod/OldMod.dll",
            b"we installed this before",
        ); // Remove
        write(
            g,
            "BepInEx/_valhsync_quarantine/20260101-000000/BepInEx/plugins/x.dll",
            b"q",
        );

        let m = manifest();
        let mut prev = InstalledState::from_manifest(&m, "srv");
        prev.files.push(crate::state::InstalledFile {
            path: "BepInEx/plugins/OldMod/OldMod.dll".into(),
            blake3: "0".repeat(64),
            policy: Policy::Enforce,
        });

        let plan = compute(&m, g, Some(&prev)).unwrap();
        assert_eq!(action_of(&plan, "winhttp.dll"), Some(Action::Keep));
        assert_eq!(
            action_of(&plan, "BepInEx/plugins/Azu/Azu.dll"),
            Some(Action::Replace)
        );
        assert_eq!(
            action_of(&plan, "BepInEx/config/Azu.cfg"),
            Some(Action::SeedKept)
        );
        assert_eq!(
            action_of(&plan, "BepInEx/plugins/OldMod/OldMod.dll"),
            Some(Action::Remove)
        );
        assert_eq!(
            action_of(&plan, "BepInEx/plugins/Azu/Sneaky.dll"),
            Some(Action::Quarantine)
        );
        assert_eq!(
            action_of(&plan, "BepInEx/plugins/EquipmentAndQuickSlots/EAQS.dll"),
            Some(Action::Quarantine)
        );
        assert_eq!(
            action_of(&plan, "BepInEx/plugins/EquipmentAndQuickSlots/readme.txt"),
            Some(Action::Quarantine)
        );
        assert_eq!(
            action_of(&plan, "BepInEx/plugins/LooseOld.dll"),
            Some(Action::Quarantine)
        );
        assert_eq!(
            action_of(&plan, "BepInEx/plugins/Azu/Azu.translations.yml"),
            None
        );
        assert_eq!(action_of(&plan, "BepInEx/plugins/notes.txt"), None);
        assert!(
            plan.items
                .iter()
                .all(|i| !i.path.contains("_valhsync_quarantine"))
        );
        assert_eq!(plan.download_bytes, b"azu v2".len() as u64);

        let c = plan.counts();
        assert_eq!(
            (c.replace, c.remove, c.quarantine, c.seed_kept),
            (1, 1, 4, 1)
        );
    }

    #[test]
    fn repair_puts_configs_back() {
        let game = tempfile::tempdir().unwrap();
        let g = game.path();
        write(g, "winhttp.dll", b"doorstop");
        write(g, "BepInEx/core/BepInEx.dll", b"core");
        write(g, "BepInEx/plugins/Azu/Azu.dll", b"azu v2");
        write(g, "BepInEx/plugins/Azu/Azu.cfg", b"azu cfg");
        write(g, "BepInEx/config/Azu.cfg", b"player changed this");
        let m = manifest();

        let normal = compute(&m, g, None).unwrap();
        assert_eq!(
            action_of(&normal, "BepInEx/config/Azu.cfg"),
            Some(Action::SeedKept)
        );

        let repair = compute_with(&m, g, None, true).unwrap();
        assert_eq!(
            action_of(&repair, "BepInEx/config/Azu.cfg"),
            Some(Action::Replace)
        );
    }

    #[test]
    fn directory_in_the_way_is_an_error() {
        let game = tempfile::tempdir().unwrap();
        fs::create_dir_all(game.path().join("winhttp.dll")).unwrap();
        let err = compute(&manifest(), game.path(), None).unwrap_err();
        assert!(matches!(err, CoreError::Obstructed(_)));
    }

    #[test]
    fn case_only_differences_count_as_same_file() {
        let game = tempfile::tempdir().unwrap();
        let g = game.path();
        write(g, "BepInEx/plugins/azu/AZU.DLL", b"azu v2");
        let m = manifest();
        let plan = compute(&m, g, None).unwrap();
        // On a case-insensitive FS the manifest path resolves to the same
        // file, so it is Keep; on Linux it is Add. Either way the existing
        // file must not be quarantined.
        assert!(
            plan.items.iter().all(|i| i.action != Action::Quarantine),
            "{plan:?}"
        );
    }
}
