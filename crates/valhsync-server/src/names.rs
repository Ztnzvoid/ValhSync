//! Names for the ids on the permission lists.
//!
//! [`crate::players`] works in Platform User IDs, because that is all Valheim
//! accepts in `adminlist.txt` and the other two. Seventeen digits are no way
//! to decide who to ban: one wrong digit and the admin has thrown out the
//! wrong person, and nothing on screen would have told them. The names are
//! there to be had -- the server writes them into its own log -- so this
//! module reads them back out.
//!
//! The one line in a Valheim server log that holds an id and a name together
//! is the player history, which the server prints once per world load, one
//! line per player the world has ever seen:
//!
//! ```text
//! [Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 0:  Bjorn (Steam_76561190000000001, BD56457E688A5A38)
//! ```
//!
//! Two things a caller has to know about that, because neither is obvious:
//!
//! * The name is the one the world recorded when that player last visited,
//!   not the one they are using now. Somebody who renames their character
//!   shows up under the old name until the server next restarts and reprints
//!   its history. It still identifies the person, which is the point.
//! * Everything else in the log that looks like it pairs the two holds only
//!   half. `Got character ZDOID from Ragnar : 287929059:4` is a name with no
//!   id; `PlayFab socket with remote ID playfab/... received local Platform
//!   ID Steam_...` is an id with no name; the mods' own
//!   `Adding peer (Steam_...) to validated list` likewise. Joining those
//!   across lines would mean guessing which connection a later line belongs
//!   to, and a guess that is wrong bans the wrong player. Nothing here
//!   guesses.
//!
//! Steam's own logs under `logs/` are no help at all: `connection_log.txt`
//! and its per-port sibling are the dedicated server's anonymous Steam login
//! (`[A:1:0:0]`), and never mention a player.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};

/// The sentence the server prints, which is what everything here is anchored
/// to.
const MARKER: &str = "Player history entry with index ";

/// Longest line held in memory while reading a file. Server logs carry mod
/// output, base64 lobby blobs and the occasional megabyte of stack trace; a
/// log being written to also ends in a line with no newline on it yet. Lines
/// past this are stepped over rather than buffered, so a damaged 100 MB log
/// cannot be read into memory by accident.
const MAX_LINE: usize = 64 * 1024;

/// Longest thing accepted as a name. Valheim caps character names far below
/// this; the limit is here so that a malformed line cannot put a paragraph
/// in the player list.
const MAX_NAME: usize = 64;

/// How many distinct players are worth remembering. A long-lived server's
/// history is dozens of people, not thousands, and the ceiling means a log
/// full of junk that happens to parse cannot grow the list without bound.
/// Ids already known keep being updated after the ceiling is reached.
const MAX_SEEN: usize = 10_000;

/// One player the server has seen, and the name it saw them under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// The Platform User ID, exactly as the log spells it, which is the
    /// spelling the permission lists want: `Steam_76561198000000000`.
    pub id: String,
    pub name: String,
    /// The log's own timestamp for the line, kept as text rather than a date.
    ///
    /// Valheim writes it with the server machine's locale, so `09/12/2026` is
    /// the twelfth of September on this server and the ninth of December on
    /// the next one, with nothing in the line to say which. Parsing it would
    /// mean showing an admin a confident wrong date; showing the log's own
    /// text cannot mislead.
    pub at: Option<String>,
}

/// Every player a log knows about, each under the last name it saw.
///
/// Order is first sighting first, so a list on screen does not reshuffle
/// itself when a player rejoins.
#[must_use]
pub fn seen_in<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<Seen> {
    let mut found = Found::default();
    for line in lines {
        found.read(line);
    }
    found.seen
}

/// The same, over a log file on disk.
///
/// Reads it a line at a time and keeps only the players: the file this is
/// pointed at is the live server log, which is tens of megabytes on a server
/// that has been up for a week.
pub fn seen_in_file(path: &Path) -> Result<Vec<Seen>> {
    let file = File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut found = Found::default();
    while next_line(&mut reader, &mut buf)
        .with_context(|| format!("cannot read {}", path.display()))?
    {
        // Lossy on purpose: a name written in some other encoding costs that
        // one player a few readable characters, where refusing the file
        // costs the admin every name in it.
        let line = String::from_utf8_lossy(&buf);
        found.read(line.trim_end_matches(['\n', '\r']));
    }
    Ok(found.seen)
}

/// The players gathered so far, and where each one sits in the list.
#[derive(Debug, Default)]
struct Found {
    seen: Vec<Seen>,
    at_index: HashMap<String, usize>,
}

impl Found {
    /// Take one line. A line that is not a history entry is not news.
    fn read(&mut self, line: &str) {
        let Some(entry) = history_entry(line) else {
            return;
        };
        if let Some(&i) = self.at_index.get(entry.id) {
            // The last word wins: a log holding two starts of the same server
            // holds the old name first.
            entry.name.clone_into(&mut self.seen[i].name);
            self.seen[i].at = entry.at.map(str::to_owned);
            return;
        }
        if self.seen.len() >= MAX_SEEN {
            return;
        }
        self.at_index.insert(entry.id.to_owned(), self.seen.len());
        self.seen.push(Seen {
            id: entry.id.to_owned(),
            name: entry.name.to_owned(),
            at: entry.at.map(str::to_owned),
        });
    }
}

/// One line's worth, still borrowed from it.
struct Entry<'a> {
    id: &'a str,
    name: &'a str,
    at: Option<&'a str>,
}

/// Pull a player out of one line, when it is a history entry and nothing else.
///
/// ```text
/// [Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 0:  Bjorn (Steam_76561190000000001, BD56457E688A5A38)
///                      ^^^^^^^^^^^^^^^^^^                                       ^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^
///                      at                                                       name   id
/// ```
///
/// The second id is the player's crossplay id at the time, which is not the
/// one the permission lists take and changes between sessions, so it is read
/// past rather than kept.
fn history_entry(line: &str) -> Option<Entry<'_>> {
    let (at, body) = split_prefix(line.trim_end())?;
    // The index itself says nothing, but insisting on it keeps the shape
    // honest: this is the server's line, not a sentence that contains it.
    let (index, rest) = body.strip_prefix(MARKER)?.split_once(':')?;
    if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // A line the server was still writing when we read it stops short of the
    // closing bracket, and is dropped here rather than half understood.
    let (name, ids) = rest.trim().strip_suffix(')')?.rsplit_once('(')?;
    let name = name.trim();
    let id = ids.split_once(',').map_or(ids, |(first, _)| first).trim();
    if !is_name(name) || !is_id(id) {
        return None;
    }
    Some(Entry { id, name, at })
}

/// Split a log line into its timestamp and what the server actually said,
/// and refuse anything where the two are separated by something else.
///
/// This is the whole defence against forgery. Chat reaches the log
/// (`Got text msg from user: ... `), so without it any player could type a
/// history entry of their own and put a name of their choosing beside
/// somebody else's id -- in front of the admin who is deciding who to ban.
/// A real entry has only the log's framing in front of it: BepInEx's tag if
/// this is a modded server, then Valheim's timestamp, which whatever the
/// machine's locale is made of digits and punctuation. Words in front of the
/// marker mean somebody is quoting it.
fn split_prefix(line: &str) -> Option<(Option<&str>, &str)> {
    let line = match line.strip_prefix('[') {
        Some(tagged) => tagged.split_once("] ")?.1,
        None => line,
    };
    let at = line.find(MARKER)?;
    let (prefix, body) = line.split_at(at);
    if prefix.is_empty() {
        return Some((None, body));
    }
    if !prefix.chars().all(is_stamp) {
        return None;
    }
    let stamp = prefix.trim_end().strip_suffix(':')?.trim();
    Some(((!stamp.is_empty()).then_some(stamp), body))
}

/// What a timestamp may be made of, in any locale the server might run in:
/// its numbers, the separators between them, and at most an `AM` or `PM`.
fn is_stamp(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, ' ' | '/' | '-' | '.' | ',' | ':' | 'A' | 'P' | 'M')
}

/// A Platform User ID as the permission lists spell it, or the bare 17-digit
/// Steam id older servers write. Anything else in that position means the
/// line was not what it looked like.
fn is_id(id: &str) -> bool {
    let bare_steam = id.len() == 17 && id.bytes().all(|b| b.is_ascii_digit());
    let platform = id.split_once('_').is_some_and(|(platform, rest)| {
        !platform.is_empty()
            && platform.chars().all(|c| c.is_ascii_alphanumeric())
            && !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    bare_steam || platform
}

/// A name has to be something an admin can read on one line next to an id.
fn is_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MAX_NAME && !name.chars().any(char::is_control)
}

/// Read the next line into `buf`, without its newline, keeping at most
/// [`MAX_LINE`] of it and stepping over the rest. False at the end of the
/// file.
///
/// [`BufRead::read_line`] would grow the buffer to whatever the file holds,
/// which on a log that a crash left without newlines is the whole file.
fn next_line<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>) -> std::io::Result<bool> {
    buf.clear();
    let mut anything = false;
    loop {
        let (used, ended) = {
            let chunk = reader.fill_buf()?;
            if chunk.is_empty() {
                // End of the file, possibly in the middle of a line the
                // server has not finished writing. What is in hand is what
                // there is.
                break;
            }
            anything = true;
            let (piece, ended) = match chunk.iter().position(|&b| b == b'\n') {
                Some(end) => (&chunk[..end], true),
                None => (chunk, false),
            };
            let room = MAX_LINE.saturating_sub(buf.len());
            buf.extend_from_slice(&piece[..piece.len().min(room)]);
            (piece.len() + usize::from(ended), ended)
        };
        reader.consume(used);
        if ended {
            break;
        }
    }
    Ok(anything)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines copied out of a real modded server's `BepInEx/LogOutput.log`,
    /// with the ids altered. Everything tested here is tested against these.
    const HISTORY: [&str; 3] = [
        "[Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 0:  Bjorn (Steam_76561190000000001, BD56457E688A5A38)",
        "[Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 1:  Sigrun (Steam_76561190000000002, 65C4B8613741BD8D)",
        "[Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 2:  Halvar (Steam_76561190000000003, 10E86ECBB07CA0F3)",
    ];

    #[test]
    fn the_player_history_gives_every_id_a_name() {
        let seen = seen_in(HISTORY.into_iter());
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].id, "Steam_76561190000000001");
        assert_eq!(seen[0].name, "Bjorn");
        assert_eq!(seen[0].at.as_deref(), Some("09/12/2026 05:16:23"));
        assert_eq!(seen[2].name, "Halvar");
    }

    #[test]
    fn the_last_name_wins_when_somebody_renames() {
        // A log that spans two starts of the server holds the history twice,
        // oldest first. The admin wants the name that player answers to now.
        let renamed = "[Info   : Unity Log] 09/12/2026 22:07:11: Player history entry with index 0:  Ragnar (Steam_76561190000000001, BD56457E688A5A38)";
        let seen = seen_in(HISTORY.into_iter().chain([renamed]));
        assert_eq!(seen.len(), 3, "the same player twice is still one player");
        assert_eq!(seen[0].name, "Ragnar");
        assert_eq!(seen[0].at.as_deref(), Some("09/12/2026 22:07:11"));
        assert_eq!(seen[1].name, "Sigrun", "and nobody else moved");
    }

    #[test]
    fn the_lines_that_hold_only_half_a_pairing_are_left_alone() {
        // Every one of these is from the same log, around a player joining.
        // They are the reason this module reads the history and nothing
        // else: each has a name or an id, never both, and pairing them
        // across lines would be a guess.
        let joining = [
            "[Info   : Unity Log] 09/12/2026 19:19:35: PlayFab listen socket child connected to remote player 898A84F1BE899959",
            "[Info   : Unity Log] 09/12/2026 19:19:35: PlayFab socket with remote ID playfab/898A84F1BE899959 received local Platform ID Steam_76561190000000001",
            "[Info   :AzuCraftyBoxes] Adding peer (Steam_76561190000000001) to validated list",
            "[Info   : Unity Log] 09/12/2026 19:19:35: Got handshake from client playfab/898A84F1BE899959",
            "[Info   : Unity Log] 09/12/2026 19:20:01: Got character ZDOID from Ragnar : 287929059:4",
            "[Info   : Unity Log] 09/12/2026 19:19:35: Player joined server \"Midgard\" that has join code 236486, now 1 player(s)",
        ];
        assert!(seen_in(joining.into_iter()).is_empty());
    }

    #[test]
    fn a_player_cannot_put_a_name_of_their_choosing_next_to_an_id() {
        // Chat is in the same log. If the marker alone were enough, anybody
        // on the server could label somebody else's id and watch the admin
        // ban them.
        let shout = "[Info   : Unity Log] 09/12/2026 05:16:22: Got text msg from user:                      Bjorn Player history entry with index 0:  Griefer (Steam_76561190000000002, BD56457E688A5A38)";
        assert!(seen_in([shout].into_iter()).is_empty());
    }

    #[test]
    fn a_line_the_server_is_still_writing_is_not_read_early() {
        // The live log always ends mid-line. Half an entry must produce
        // nothing rather than a player called "Bjorn (Steam_7656119000".
        let half = "[Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 0:  Bjorn (Steam_7656119000";
        assert!(seen_in([half].into_iter()).is_empty());
    }

    #[test]
    fn mod_output_that_reads_like_an_entry_needs_the_whole_shape() {
        let near_misses = [
            // No index: a sentence about the feature, not an entry.
            "[Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index :  Bjorn (Steam_76561190000000001, X)",
            // Not an id in the brackets.
            "[Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 0:  Bjorn (unknown platform, X)",
            // No name left once the brackets are taken off.
            "[Info   : Unity Log] 09/12/2026 05:16:23: Player history entry with index 0:  (Steam_76561190000000001, X)",
            "",
            "[Info   : Unity Log] 09/12/2026 05:16:23: Checking for any blocked players in the historical player list...",
        ];
        assert!(seen_in(near_misses.into_iter()).is_empty());
    }

    #[test]
    fn an_entry_is_read_on_a_server_that_logs_without_bepinex() {
        // Valheim writes the timestamp itself, so an unmodded server's log
        // is the same line without the tag in front of it.
        let plain = "09/12/2026 05:16:23: Player history entry with index 0:  Bjorn (Steam_76561190000000001, BD56457E688A5A38)";
        let seen = seen_in([plain].into_iter());
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].name, "Bjorn");
    }

    #[test]
    fn a_file_is_read_the_same_way_the_lines_are() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("LogOutput.log");
        let mut text = String::new();
        for line in HISTORY {
            text.push_str(line);
            text.push('\n');
            // BepInEx puts a blank line between entries.
            text.push('\n');
        }
        std::fs::write(&log, text.as_bytes()).unwrap();
        let seen = seen_in_file(&log).unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[1].name, "Sigrun");
    }

    #[test]
    fn a_missing_log_says_which_one() {
        let err = seen_in_file(Path::new("no/such/LogOutput.log"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("LogOutput.log"), "{err}");
    }

    #[test]
    fn one_ruined_line_does_not_cost_the_rest_of_the_log() {
        // A crashed server leaves a line with no end to it, and a mod can
        // write bytes that are not UTF-8. Either would lose every name after
        // it if the reader gave up on them.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("LogOutput.log");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(HISTORY[0].as_bytes());
        bytes.push(b'\n');
        bytes.extend(std::iter::repeat_n(b'x', MAX_LINE * 3));
        bytes.push(b'\n');
        bytes.extend_from_slice(&[0xFF, 0xFE, b'\n']);
        bytes.extend_from_slice(HISTORY[2].as_bytes());
        bytes.push(b'\n');
        std::fs::write(&log, &bytes).unwrap();

        let seen = seen_in_file(&log).unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].name, "Bjorn");
        assert_eq!(seen[1].name, "Halvar");
    }
}
