//! What changed, kept so it can be read after the fact.
//!
//! The plan says what the next sync will do; once it has been done, that plan
//! is gone. A player who pressed Play, watched a progress bar and went to make
//! coffee has no way back to "what did I just install". This is the answer:
//! one entry per sync that changed something, written after it succeeded.
//!
//! It is a local record of the player's own machine, not a feed from the
//! server. Nothing here is trusted for anything -- it decides what a panel
//! shows, never what gets installed.

use serde::{Deserialize, Serialize};
use valhsync_core::{ChangeKind, ModChange};

use crate::error::Result;
use crate::paths::{AppPaths, read_json, write_json_atomic};

/// How many entries are kept. Enough to cover "what happened over the last few
/// weeks", far short of a file worth paging.
const KEEP: usize = 50;

/// One sync that changed something.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Public key of the server it came from, so entries can be filtered to
    /// the server being looked at.
    pub server_id: String,
    pub server_name: String,
    /// Pack applied, so the same pack is never recorded twice.
    pub pack_id: String,
    /// When it was applied here, RFC 3339.
    pub at: String,
    /// What the admin wrote about it, if anything.
    #[serde(default)]
    pub notes: Option<String>,
    pub changes: Vec<ModChange>,
}

impl Entry {
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        let count = |k: ChangeKind| self.changes.iter().filter(|c| c.kind == k).count();
        (
            count(ChangeKind::Added),
            count(ChangeKind::Updated),
            count(ChangeKind::Removed),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct News {
    /// Newest first.
    pub entries: Vec<Entry>,
}

impl News {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        Ok(read_json(&paths.news_file())?.unwrap_or_default())
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        write_json_atomic(&paths.news_file(), self)
    }

    /// Add an entry, unless there is nothing to say or it is already there.
    ///
    /// Returns whether anything was added. A repair, or a sync that only
    /// touched BepInEx itself, changes no mods and is not news; applying the
    /// same pack twice is not news either.
    pub fn record(&mut self, entry: Entry) -> bool {
        if entry.changes.is_empty() && entry.notes.is_none() {
            return false;
        }
        // The note counts as much as the pack. Notes are not part of a pack
        // id -- an admin who rewrites the message without touching a mod
        // publishes the same pack -- so deduplicating on the id alone threw
        // away exactly the entries that exist to be read. A player would
        // never see a word their admin wrote unless a mod moved with it.
        if self.entries.iter().any(|e| {
            e.server_id == entry.server_id && e.pack_id == entry.pack_id && e.notes == entry.notes
        }) {
            return false;
        }
        self.entries.insert(0, entry);
        self.entries.truncate(KEEP);
        true
    }

    /// What this server last said, if it has said anything.
    ///
    /// Used to notice a note that has changed since the player last saw one,
    /// which is the only way a message with no mods behind it ever reaches
    /// them.
    #[must_use]
    pub fn latest_notes<'a>(&'a self, server_id: &str) -> Option<&'a str> {
        self.entries
            .iter()
            .filter(|e| e.server_id == server_id)
            .find_map(|e| e.notes.as_deref())
    }

    /// Entries from one server, newest first.
    pub fn for_server<'a>(&'a self, server_id: &'a str) -> impl Iterator<Item = &'a Entry> {
        self.entries
            .iter()
            .filter(move |e| e.server_id == server_id)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn entry(pack: &str, changes: Vec<ModChange>) -> Entry {
        Entry {
            server_id: "server-one".into(),
            server_name: "Northwatch".into(),
            pack_id: pack.into(),
            at: "2026-09-12T04:00:00Z".into(),
            notes: None,
            changes,
        }
    }

    fn added(name: &str) -> ModChange {
        ModChange {
            name: name.into(),
            kind: ChangeKind::Added,
        }
    }

    #[test]
    fn a_rewritten_note_is_kept_even_when_the_pack_is_the_same() {
        // The admin fixed a typo, or added the warning they forgot. Nothing
        // about the mods changed, so the pack id did not either -- and that
        // used to mean nobody read the corrected note.
        let said = |text: &str| Entry {
            notes: Some(text.to_string()),
            ..entry("same-pack", Vec::new())
        };
        let mut news = News::default();
        assert!(news.record(said("empty your chests")));
        assert!(news.record(said("empty your chests AND your cart")));
        assert_eq!(news.for_server("server-one").count(), 2);
        assert_eq!(
            news.latest_notes("server-one"),
            Some("empty your chests AND your cart")
        );
    }

    #[test]
    fn the_same_note_on_the_same_pack_is_still_recorded_once() {
        let said = || Entry {
            notes: Some("a word".to_string()),
            ..entry("same-pack", Vec::new())
        };
        let mut news = News::default();
        assert!(news.record(said()));
        assert!(!news.record(said()));
        assert_eq!(news.for_server("server-one").count(), 1);
    }

    #[test]
    fn a_sync_that_changed_no_mod_is_not_news() {
        let mut news = News::default();
        assert!(!news.record(entry("p1", vec![])));
        assert!(news.entries.is_empty());
    }

    /// A note is worth keeping even when the mods are untouched: "the server
    /// moves to a new address on Friday" is exactly that.
    #[test]
    fn a_note_alone_is_worth_keeping() {
        let mut news = News::default();
        let mut e = entry("p1", vec![]);
        e.notes = Some("Server moves on Friday".into());
        assert!(news.record(e));
        assert_eq!(news.entries.len(), 1);
    }

    #[test]
    fn the_same_pack_is_recorded_once() {
        let mut news = News::default();
        assert!(news.record(entry("p1", vec![added("Seasonality")])));
        assert!(!news.record(entry("p1", vec![added("Seasonality")])));
        assert!(news.record(entry("p2", vec![added("PlantEverything")])));
        assert_eq!(news.entries.len(), 2);
    }

    #[test]
    fn the_newest_is_first_and_the_oldest_falls_off() {
        let mut news = News::default();
        for i in 0..KEEP + 10 {
            assert!(news.record(entry(&format!("p{i}"), vec![added("Mod")])));
        }
        assert_eq!(news.entries.len(), KEEP);
        assert_eq!(news.entries[0].pack_id, format!("p{}", KEEP + 9));
    }

    #[test]
    fn entries_are_kept_apart_by_server() {
        let mut news = News::default();
        news.record(entry("p1", vec![added("Mod")]));
        let mut other = entry("p1", vec![added("Mod")]);
        other.server_id = "server-two".into();
        // Same pack id, different server: both are that player's history.
        assert!(news.record(other));
        assert_eq!(news.for_server("server-one").count(), 1);
        assert_eq!(news.for_server("server-two").count(), 1);
    }

    #[test]
    fn counts_split_the_three_kinds() {
        let mut e = entry("p1", vec![added("A"), added("B")]);
        e.changes.push(ModChange {
            name: "C".into(),
            kind: ChangeKind::Removed,
        });
        assert_eq!(e.counts(), (2, 0, 1));
    }
}
