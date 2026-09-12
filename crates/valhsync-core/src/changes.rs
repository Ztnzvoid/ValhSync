//! What a sync will change, told in mods rather than in files.
//!
//! A plan is a list of files, which is the right shape to apply and the wrong
//! shape to read: forty entries under `BepInEx/plugins/Seasonality/` are one
//! thing happening, not forty. Grouping them back into mods is what lets the
//! launcher say "Seasonality added, PlantEverything updated" before asking
//! somebody to agree to it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::plan::{Action, SyncPlan};

/// Where a mod lives. Everything under one folder here is one mod.
const PLUGINS: &str = "BepInEx/plugins/";

/// What is happening to one mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Not installed before.
    Added,
    /// Installed, and at least one of its files is being replaced.
    Updated,
    /// Installed by ValhSync earlier, gone from the manifest.
    Removed,
}

/// One line of a changelog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModChange {
    pub name: String,
    pub kind: ChangeKind,
}

/// The mods a plan touches, in the order they should be read: what arrives,
/// what moves, what goes.
///
/// Files outside `BepInEx/plugins/` are deliberately left out. BepInEx itself,
/// `winhttp.dll` and the doorstop files change for reasons that mean nothing
/// to a player, and listing them would bury the one line that does.
#[must_use]
pub fn mods_touched(plan: &SyncPlan) -> Vec<ModChange> {
    let mut seen: BTreeMap<&str, ChangeKind> = BTreeMap::new();
    for item in &plan.items {
        let Some(rest) = item.path.strip_prefix(PLUGINS) else {
            continue;
        };
        let name = rest.split_once('/').map_or(rest, |(folder, _)| folder);
        let kind = match item.action {
            Action::Add => ChangeKind::Added,
            Action::Replace => ChangeKind::Updated,
            Action::Remove => ChangeKind::Removed,
            // Quarantine is about a file the player put there themselves, and
            // the rest are not changes at all.
            _ => continue,
        };
        seen.entry(name)
            .and_modify(|current| *current = merge(*current, kind))
            .or_insert(kind);
    }
    let mut out: Vec<ModChange> = seen
        .into_iter()
        .map(|(name, kind)| ModChange {
            name: name.to_string(),
            kind,
        })
        .collect();
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    out
}

/// A mod whose files are partly new and partly replaced is an update, not an
/// arrival: some of it was already there. One that is both losing files and
/// gaining them is also an update -- it is being reshaped, not removed.
fn merge(a: ChangeKind, b: ChangeKind) -> ChangeKind {
    if a == b { a } else { ChangeKind::Updated }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlannedFile;

    fn item(path: &str, action: Action) -> PlannedFile {
        PlannedFile {
            path: path.to_string(),
            action,
            size: 0,
            blake3: None,
            policy: None,
        }
    }

    fn plan(items: Vec<PlannedFile>) -> SyncPlan {
        SyncPlan {
            pack_id: "p".into(),
            items,
            download_bytes: 0,
        }
    }

    #[test]
    fn many_files_under_one_mod_are_one_line() {
        let p = plan(vec![
            item("BepInEx/plugins/Seasonality/Seasonality.dll", Action::Add),
            item("BepInEx/plugins/Seasonality/assets/a.bundle", Action::Add),
            item("BepInEx/plugins/Seasonality/assets/b.bundle", Action::Add),
        ]);
        assert_eq!(
            mods_touched(&p),
            vec![ModChange {
                name: "Seasonality".into(),
                kind: ChangeKind::Added
            }]
        );
    }

    #[test]
    fn a_mod_partly_new_and_partly_replaced_is_an_update() {
        let p = plan(vec![
            item("BepInEx/plugins/Mod/old.dll", Action::Replace),
            item("BepInEx/plugins/Mod/new.dll", Action::Add),
        ]);
        assert_eq!(mods_touched(&p)[0].kind, ChangeKind::Updated);
    }

    #[test]
    fn unchanged_mods_and_quarantine_say_nothing() {
        let p = plan(vec![
            item("BepInEx/plugins/Kept/a.dll", Action::Keep),
            item("BepInEx/plugins/Seeded/b.cfg", Action::SeedKept),
            item("BepInEx/plugins/Stranger/c.dll", Action::Quarantine),
        ]);
        assert!(mods_touched(&p).is_empty());
    }

    /// BepInEx and the doorstop files change for reasons that mean nothing to
    /// a player; listing them would bury the line that does.
    #[test]
    fn only_mods_are_reported() {
        let p = plan(vec![
            item("winhttp.dll", Action::Replace),
            item("doorstop_config.ini", Action::Replace),
            item("BepInEx/core/BepInEx.dll", Action::Replace),
            item("BepInEx/plugins/Real/real.dll", Action::Add),
        ]);
        let out = mods_touched(&p);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].name, "Real");
    }

    #[test]
    fn arrivals_come_before_departures() {
        let p = plan(vec![
            item("BepInEx/plugins/Gone/x.dll", Action::Remove),
            item("BepInEx/plugins/Zeta/z.dll", Action::Add),
            item("BepInEx/plugins/Alpha/a.dll", Action::Add),
            item("BepInEx/plugins/Moved/m.dll", Action::Replace),
        ]);
        let out: Vec<_> = mods_touched(&p)
            .into_iter()
            .map(|c| (c.name, c.kind))
            .collect();
        assert_eq!(
            out,
            vec![
                ("Alpha".to_string(), ChangeKind::Added),
                ("Zeta".to_string(), ChangeKind::Added),
                ("Moved".to_string(), ChangeKind::Updated),
                ("Gone".to_string(), ChangeKind::Removed),
            ]
        );
    }
}
