//! Reading Valheim's own version out of a BepInEx log.
//!
//! Both sides need it: the publisher, to say which Valheim a pack is meant
//! for, and the launcher, to say so before a player meets Jötunn's wall.
//! Valheim prints the line on every start-up, and the *network* version is
//! what decides whether a client may connect -- matching mods do not help when
//! it differs.

use std::path::Path;

/// Which Valheim a log belongs to, e.g. `1.0.7 (network version 39)`.
/// Lines are given oldest first; the most recent start-up wins.
#[must_use]
pub fn read_version<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> Option<String> {
    for line in lines.rev() {
        if let Some((_, rest)) = line.split_once("Valheim version: ") {
            let text = rest.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// The network version out of such a string, which is what has to match.
#[must_use]
pub fn network_version(version: &str) -> Option<u32> {
    version
        .split_once("network version ")?
        .1
        .trim_end_matches(')')
        .trim()
        .parse()
        .ok()
}

/// The network version a Valheim install last ran as, from the BepInEx log in
/// its own folder. `None` when it has never run with BepInEx.
#[must_use]
pub fn network_version_of(game_root: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(game_root.join("BepInEx/LogOutput.log")).ok()?;
    network_version(&read_version(text.lines())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_says_which_valheim_this_is() {
        let lines = [
            "[Info   : Unity Log] 09/11/2026 15:18:48: Valheim version: 1.0.7 (network version 39)",
            "[Info   : Unity Log] 09/11/2026 15:18:48: Worldgenerator version setup:2",
        ];
        let version = read_version(lines.into_iter()).unwrap();
        assert_eq!(version, "1.0.7 (network version 39)");
        assert_eq!(network_version(&version), Some(39));
    }

    #[test]
    fn a_restarted_game_reports_its_newest_line() {
        let lines = [
            "Valheim version: 1.0.7 (network version 39)",
            "Valheim version: 1.0.12 (network version 40)",
        ];
        assert_eq!(
            network_version(&read_version(lines.into_iter()).unwrap()),
            Some(40)
        );
    }

    #[test]
    fn nothing_is_invented() {
        assert_eq!(read_version(["nothing here"].into_iter()), None);
        assert_eq!(network_version("1.0.7"), None);
        assert_eq!(network_version_of(Path::new("no/such/game")), None);
    }
}
