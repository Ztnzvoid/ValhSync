//! What a sync will change, told in mods rather than in files.
//!
//! A plan is a list of files, which is the right shape to apply and the wrong
//! shape to read: forty entries under `BepInEx/plugins/Seasonality/` are one
//! thing happening, not forty. Grouping them back into mods is what lets the
//! launcher say "Seasonality added, PlantEverything updated" rather than
//! listing forty files nobody reads.

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
    pair_renames(&mut out);
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Fold "BetterArchery gone, BetterArchery-2.0.0 arrived" into one update.
///
/// Mods are grouped by the folder they sit in, and admins put the version in
/// that folder's name -- Thunderstore's packages are named that way, so a
/// dropped update almost always lands beside its predecessor under a
/// different name. What a player then reads is the same mod both removed and
/// added, which looks alarming and says nothing true: nothing was removed,
/// something was updated.
///
/// Two names belong to one mod when they are equal once a trailing version is
/// taken off. That is a guess, and a small one: two different mods sharing a
/// name with only a version between them would be the same mod anyway.
fn pair_renames(changes: &mut Vec<ModChange>) {
    let stems: Vec<String> = changes.iter().map(|c| stem(&c.name).to_string()).collect();
    let mut drop = Vec::new();
    for (i, change) in changes.iter().enumerate() {
        if change.kind != ChangeKind::Removed {
            continue;
        }
        let paired = changes
            .iter()
            .enumerate()
            .find(|(j, other)| *j != i && other.kind == ChangeKind::Added && stems[*j] == stems[i]);
        if paired.is_some() {
            drop.push(i);
        }
    }
    for (j, other) in changes.iter_mut().enumerate() {
        if other.kind == ChangeKind::Added && drop.iter().any(|i| stems[*i] == stems[j]) {
            other.kind = ChangeKind::Updated;
        }
    }
    let mut i = 0;
    changes.retain(|_| {
        let keep = !drop.contains(&i);
        i += 1;
        keep
    });
}

/// A folder name with its trailing version taken off, if it has one.
///
/// `Seasonality-3.8.1` and `EpicLoot-0.14.4` become `Seasonality` and
/// `EpicLoot`; `Advize_PlantEverything` and `BalrondShipyard` are left alone,
/// because a name is not a version just for having a dash in it.
fn stem(name: &str) -> &str {
    let Some((head, tail)) = name.rsplit_once('-') else {
        return name;
    };
    let looks_like_a_version = !tail.is_empty()
        && tail.starts_with(|c: char| c.is_ascii_digit())
        && tail.chars().all(|c| c.is_ascii_digit() || c == '.');
    if looks_like_a_version && !head.is_empty() {
        head
    } else {
        name
    }
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
    fn a_mod_that_gained_a_version_in_its_folder_name_is_one_update() {
        // What an admin does when they drop a Thunderstore package beside a
        // folder somebody had named by hand. Reading "BetterArchery removed"
        // would frighten a player over nothing.
        let changes = mods_touched(&plan(vec![
            item(
                "BepInEx/plugins/BetterArchery/BetterArchery.dll",
                Action::Remove,
            ),
            item(
                "BepInEx/plugins/BetterArchery-2.0.0/BetterArchery.dll",
                Action::Add,
            ),
        ]));
        assert_eq!(
            changes,
            vec![ModChange {
                name: "BetterArchery-2.0.0".into(),
                kind: ChangeKind::Updated,
            }]
        );
    }

    #[test]
    fn a_version_bump_in_the_folder_name_is_one_update_too() {
        let changes = mods_touched(&plan(vec![
            item(
                "BepInEx/plugins/EpicLoot-0.14.3/EpicLoot.dll",
                Action::Remove,
            ),
            item("BepInEx/plugins/EpicLoot-0.14.4/EpicLoot.dll", Action::Add),
        ]));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].kind, ChangeKind::Updated);
        assert_eq!(changes[0].name, "EpicLoot-0.14.4");
    }

    #[test]
    fn a_mod_that_really_went_still_reads_as_removed() {
        let changes = mods_touched(&plan(vec![
            item(
                "BepInEx/plugins/Seasonality/Seasonality.dll",
                Action::Remove,
            ),
            item("BepInEx/plugins/EpicLoot-0.14.4/EpicLoot.dll", Action::Add),
        ]));
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(
            changes
                .iter()
                .any(|c| c.name == "Seasonality" && c.kind == ChangeKind::Removed)
        );
    }

    #[test]
    fn a_dash_is_not_a_version() {
        // Plenty of mods carry one in the name itself.
        assert_eq!(stem("Advize_PlantEverything"), "Advize_PlantEverything");
        assert_eq!(stem("Server_devcommands-1.113.0"), "Server_devcommands");
        assert_eq!(stem("OdinPlus-OdinHorse"), "OdinPlus-OdinHorse");
        assert_eq!(stem("Smoothbrain-Mining-1.1.7"), "Smoothbrain-Mining");
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
