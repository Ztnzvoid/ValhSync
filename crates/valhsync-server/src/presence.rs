//! Who is connected right now, read from the server's own log.
//!
//! Valheim puts a platform id on a line at exactly two moments, and they are
//! the two that matter:
//!
//! ```text
//! [Info   : Unity Log] 09/19/2026 00:26:50: Got connection SteamID 76561190000000001
//! [Info   : Unity Log] 09/19/2026 00:31:12: Closing socket 76561190000000001
//! ```
//!
//! The later of the two, for one id, is what that player is doing now.
//!
//! Nothing is inferred from anything else. `Got character ZDOID from Bjorn`
//! is a name with no id; `Got handshake from client` repeats the id of the
//! line above it; and the disconnect chatter around a closing socket
//! (`RPC_Disconnect`, `Got status changed msg ...ClosedByPeer`, `Socket
//! closed by peer Steamworks...`) names nobody at all. Guessing which
//! connection those belong to would put the wrong lamp next to the wrong
//! player, which is the one thing this is not allowed to do.
//!
//! Chat reaches the log too, so both markers are matched through
//! [`crate::names::split_prefix`]: a line only counts when the server said
//! it, with nothing but the log's own framing in front. A player typing
//! "Closing socket 76561190000000001" into the chat box changes nothing here.
//!
//! A Valheim server truncates its log when it starts, so what this reads can
//! only ever describe the session that is running: nobody is left "connected"
//! across a restart. An admin stopping the server is a separate matter -- the
//! window knows whether the game is up, and says nobody is connected when it
//! is not, rather than trusting a log that stops mid-session.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};

use crate::names::{next_line, split_prefix};

/// The server saying somebody arrived.
const CONNECTED: &str = "Got connection SteamID ";
/// The server saying a connection is over.
const CLOSED: &str = "Closing socket ";

/// How many ids are worth remembering. A long-lived server sees more people
/// than a window can show; the ones that matter are the ones on screen, and
/// the list is capped so a log nobody rotates cannot grow this without end.
const MAX_IDS: usize = 500;

/// What one id is doing, and when it last changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub online: bool,
    /// The log's own timestamp for that event, as the server wrote it.
    /// `None` when the line carried none, which happens on servers whose
    /// logs are written without one.
    pub at: Option<String>,
}

impl Presence {
    /// The time of day, for a window that has one line to say it in.
    ///
    /// The date is dropped and the seconds with it: an admin looking at a
    /// lamp wants "since 21:58", not a full stamp. Locale-proof by
    /// construction -- whichever way round the date is written, the time is
    /// the piece with colons in it.
    #[must_use]
    pub fn clock(&self) -> Option<String> {
        let stamp = self.at.as_ref()?;
        let mut parts = stamp.split_whitespace();
        let time = parts.by_ref().find(|p| p.contains(':'))?;
        let hhmm = match time.rsplit_once(':') {
            Some((head, _seconds)) if head.contains(':') => head,
            _ => time,
        };
        // A twelve-hour clock puts the half of the day after the time.
        match parts.next() {
            Some(half) if half.eq_ignore_ascii_case("AM") || half.eq_ignore_ascii_case("PM") => {
                Some(format!("{hhmm} {half}"))
            }
            _ => Some(hhmm.to_string()),
        }
    }
}

/// Read every connect and disconnect in `lines`, keeping the last of each.
#[must_use]
pub fn from_lines<'a>(lines: impl Iterator<Item = &'a str>) -> HashMap<String, Presence> {
    let mut seen: HashMap<String, Presence> = HashMap::new();
    for line in lines {
        let Some((id, presence)) = event(line) else {
            continue;
        };
        if seen.len() >= MAX_IDS && !seen.contains_key(id) {
            continue;
        }
        seen.insert(id.to_string(), presence);
    }
    seen
}

/// As [`from_lines`], over a log file, without reading it into memory.
pub fn from_file(path: &Path) -> Result<HashMap<String, Presence>> {
    let file = File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut seen: HashMap<String, Presence> = HashMap::new();
    while next_line(&mut reader, &mut buf)? {
        let Ok(line) = std::str::from_utf8(&buf) else {
            continue;
        };
        let Some((id, presence)) = event(line) else {
            continue;
        };
        if seen.len() >= MAX_IDS && !seen.contains_key(id) {
            continue;
        }
        seen.insert(id.to_string(), presence);
    }
    Ok(seen)
}

/// One line, if it is one of the two the server writes about a connection.
fn event(line: &str) -> Option<(&str, Presence)> {
    let line = line.trim_end();
    for (marker, online) in [(CONNECTED, true), (CLOSED, false)] {
        let Some((at, body)) = split_prefix(line, marker) else {
            continue;
        };
        let id = body.strip_prefix(marker)?.trim();
        if !is_steam_id(id) {
            return None;
        }
        return Some((
            id,
            Presence {
                online,
                at: at.map(str::to_string),
            },
        ));
    }
    None
}

/// The bare 17-digit id these two lines carry. Not [`crate::names`]'s
/// looser test: those lines come from the permission lists, where a platform
/// prefix is normal, and these two never have one.
fn is_steam_id(id: &str) -> bool {
    id.len() == 17 && id.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "76561190000000001";
    const TWO: &str = "76561190000000002";

    fn log() -> Vec<String> {
        vec![
            format!("[Info   : Unity Log] 09/18/2026 20:40:26: Got connection SteamID {ONE}"),
            format!("[Info   : Unity Log] 09/18/2026 20:40:26: Got handshake from client {ONE}"),
            "[Info   : Unity Log] 09/18/2026 20:40:26: Server: New peer connected,sending global keys".into(),
            "[Info   : Unity Log] 09/18/2026 20:40:48: Got character ZDOID from Bjorn : 1934795049:82".into(),
            format!("[Info   : Unity Log] 09/18/2026 21:25:28: Closing socket {ONE}"),
            format!("[Info   : Unity Log] 09/18/2026 21:58:51: Got connection SteamID {TWO}"),
        ]
    }

    #[test]
    fn the_last_word_on_an_id_is_the_one_that_counts() {
        let seen = from_lines(log().iter().map(String::as_str));
        assert_eq!(
            seen[ONE],
            Presence {
                online: false,
                at: Some("09/18/2026 21:25:28".into())
            }
        );
        assert!(seen[TWO].online);
        assert_eq!(seen[TWO].clock().unwrap(), "21:58");
        assert_eq!(seen.len(), 2, "nothing else in that log names an id");
    }

    #[test]
    fn a_player_cannot_disconnect_somebody_by_typing_it_in_chat() {
        let forged = format!(
            "[Info   : Unity Log] 09/18/2026 22:00:00: Got text msg from user: 1934795049 \
             text: Closing socket {ONE}"
        );
        let mut lines = log();
        lines.push(forged);
        let seen = from_lines(lines.iter().map(String::as_str));
        // The genuine close at 21:25 stands; the chat line added nothing.
        assert_eq!(seen[ONE].clock().unwrap(), "21:25");
        assert!(seen[TWO].online, "and nobody else was touched");
    }

    #[test]
    fn a_line_that_is_not_an_id_is_not_a_player() {
        let lines = [
            "[Info   : Unity Log] 09/18/2026 21:25:28: Socket closed by peer Steamworks.SteamNetConnectionStatusChangedCallback_t",
            "[Info   : Unity Log] 09/18/2026 21:25:28: Closing socket 12345",
            "[Info   : Unity Log] 09/18/2026 21:25:28: RPC_Disconnect",
        ];
        assert!(from_lines(lines.into_iter()).is_empty());
    }

    #[test]
    fn a_twelve_hour_clock_keeps_its_half_of_the_day() {
        let p = Presence {
            online: true,
            at: Some("09/18/2026 9:58:51 PM".into()),
        };
        assert_eq!(p.clock().unwrap(), "9:58 PM");
    }

    #[test]
    fn a_log_without_timestamps_still_says_who_is_there() {
        let line = format!("Got connection SteamID {ONE}");
        let seen = from_lines(std::iter::once(line.as_str()));
        assert!(seen[ONE].online);
        assert!(seen[ONE].clock().is_none());
    }
}
