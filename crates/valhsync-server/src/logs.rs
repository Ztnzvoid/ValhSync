//! The game server's log, wherever this installation happens to put it, and
//! when the world was last written to disk.
//!
//! Nothing here is specific to one install: the sources are looked for in the
//! order that tells the admin the most, and the first one that exists wins.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read as _, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::detect::ServerArgs;

/// Lines kept in memory: enough to hold a whole start-up, few enough that the
/// window cannot grow without bound on a server that has run for days.
const KEEP: usize = 500;

/// How far back to start reading when a log is opened. A server log can reach
/// tens of megabytes; only the end of it is worth showing.
const FIRST_READ: u64 = 96 * 1024;

/// A single read never takes more than this, so a burst of output cannot
/// stall a frame.
const MAX_CHUNK: u64 = 512 * 1024;

/// Where a server writes its output, most informative first.
///
/// * `-logFile` if the start script asks for one, since that is the admin's
///   own choice and holds everything Unity prints;
/// * `BepInEx/LogOutput.log`, which holds the same Unity lines plus every
///   mod's, and exists on any modded server;
/// * `valheim_server_Data/output_log.txt`, the older Unity layout.
#[must_use]
pub fn find_sources(root: &Path, args: Option<&ServerArgs>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(rel) = args.and_then(|a| a.log_file.as_deref()) {
        push(resolve(root, rel));
    }
    push(root.join("BepInEx/LogOutput.log"));
    push(root.join("valheim_server_Data/output_log.txt"));
    out
}

/// The world's save file, so the window can say when it was last written.
///
/// `-savedir` if the script sets one, otherwise the place Valheim puts a
/// dedicated server's worlds on this platform.
#[must_use]
pub fn world_save(root: &Path, args: &ServerArgs) -> Option<PathBuf> {
    let world = args.world.as_deref()?.trim();
    if world.is_empty() {
        return None;
    }
    let base = match args.savedir.as_deref() {
        Some(dir) => resolve(root, dir),
        None => default_savedir()?,
    };
    let db = base.join("worlds_local").join(format!("{world}.db"));
    db.is_file().then_some(db)
}

/// When that file was last written.
#[must_use]
pub fn saved_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Valheim keeps a dedicated server's worlds under the Unity persistent data
/// path, next to the game's own saves.
fn default_savedir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let appdata = std::env::var_os("APPDATA")?;
        // The parent of `AppData/Roaming` is `AppData`; worlds live in
        // `LocalLow` beside it.
        Some(
            PathBuf::from(appdata)
                .parent()?
                .join("LocalLow/IronGate/Valheim"),
        )
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config/unity3d/IronGate/Valheim"))
    }
}

/// A path from a start script: absolute as written, otherwise relative to the
/// server folder, which is the script's working directory.
fn resolve(root: &Path, value: &str) -> PathBuf {
    let p = PathBuf::from(value.trim().trim_matches('"'));
    if p.is_absolute() { p } else { root.join(p) }
}

/// What the log says about the session that is running.
///
/// Both facts come from the line Valheim repeats while the server is up:
/// `Session "..." with join code 123456 and IP ...:2456 is active with 0
/// player(s)`. Nothing is inferred when the line has not appeared yet.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Session {
    /// The crossplay join code, which is how players reach a server that has
    /// no port open.
    pub join_code: Option<String>,
    pub players: Option<u32>,
}

/// Read the most recent session line. Lines are given newest last, as the
/// tail holds them.
pub fn read_session<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> Session {
    for line in lines.rev() {
        if !line.contains("is active with") {
            continue;
        }
        return Session {
            join_code: after(line, "join code ").map(str::to_owned),
            players: after(line, "is active with ").and_then(|v| v.parse().ok()),
        };
    }
    Session::default()
}

/// The word that follows `marker`, when the line holds one.
fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = line.split_once(marker)?.1;
    let word = rest.split_whitespace().next()?;
    (!word.is_empty()).then_some(word)
}

/// The end of a log file, re-read as it grows.
#[derive(Debug)]
pub struct Tail {
    path: PathBuf,
    /// How far into the file we have read.
    offset: u64,
    /// A last line with no newline yet, waiting for the rest of it.
    partial: String,
    lines: VecDeque<String>,
    /// Lines appended by the last `poll`, before the ring drops them again.
    /// A busy server writes hundreds of lines a minute, so anything that
    /// wants to notice a particular one has to see it as it arrives rather
    /// than go looking for it afterwards.
    fresh: Vec<String>,
    /// Set when the file cannot be read at all, to show instead of lines.
    error: Option<String>,
}

impl Tail {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            partial: String::new(),
            lines: VecDeque::new(),
            fresh: Vec::new(),
            error: None,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator {
        self.lines.iter().map(String::as_str)
    }

    /// What the last `poll` added, oldest first.
    #[must_use]
    pub fn fresh(&self) -> &[String] {
        &self.fresh
    }

    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Read whatever has been appended since last time. Returns true when
    /// something changed, so the view only scrolls when there is news.
    pub fn poll(&mut self) -> bool {
        self.fresh.clear();
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) => {
                let msg = format!("{}: {e}", self.path.display());
                let changed = self.error.as_deref() != Some(msg.as_str());
                self.error = Some(msg);
                return changed;
            }
        };
        self.error = None;
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return false;
        };

        // A restarted server truncates its log, and a rotated one replaces it
        // with a shorter file. Either way, read the new one from its start.
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        // First look at the file: skip to near its end rather than replay days.
        if self.offset == 0 && len > FIRST_READ {
            self.offset = len - FIRST_READ;
            self.partial.clear();
        }
        if len == self.offset {
            return false;
        }
        let behind = len - self.offset;
        let to_read = behind.min(MAX_CHUNK);
        if to_read < behind {
            // Too far behind to catch up in one frame: keep only the tail.
            self.offset = len - to_read;
            self.partial.clear();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }

        let mut reader = BufReader::new(file.take(to_read));
        let mut buf = Vec::new();
        let mut changed = false;
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    self.offset += n as u64;
                    let complete = buf.ends_with(b"\n");
                    let text = String::from_utf8_lossy(&buf);
                    self.partial.push_str(text.trim_end_matches(['\n', '\r']));
                    if complete {
                        let line = std::mem::take(&mut self.partial);
                        // BepInEx writes a blank line between entries; one is
                        // a separator, a screenful of them is not.
                        if !(line.is_empty() && self.lines.back().is_some_and(String::is_empty)) {
                            self.push(line);
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }

    fn push(&mut self, line: String) {
        if self.lines.len() == KEEP {
            self.lines.pop_front();
        }
        self.fresh.push(line.clone());
        self.lines.push_back(line);
    }
}

/// What the game server printed to its own console, out of one log line.
///
/// Valheim tags these itself. The dedicated server really does have a
/// console -- it says `type "help" - for commands` on start-up and answers
/// what is typed into it -- but the answer is written into the same log as
/// everything else, where a few lines of it sit among thousands of
/// `Destroying abandoned non persistent zdo`. Pulling them out by their tag
/// is what turns "somewhere in that file" into a console.
///
/// ```text
/// [Info   : Unity Log] 09/12/2026 05:16:22: Console: /die - Suicide
///                                                    ^^^^^^^^^^^^^
/// ```
#[must_use]
pub fn console_text(line: &str) -> Option<&str> {
    const TAG: &str = "Console: ";
    let at = line.find(TAG)?;
    // Anchored to the log's own framing, so a player shouting "Console: rm
    // -rf" in chat cannot write a line into the admin's console view. Every
    // real one is introduced by the timestamp's colon-space, or opens the
    // line on a server logging without timestamps.
    let before = &line[..at];
    if !(before.is_empty() || before.ends_with(": ")) {
        return None;
    }
    Some(line[at + TAG.len()..].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn append(path: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn console_lines_are_picked_out_of_the_noise() {
        let log = "[Info   : Unity Log] 09/12/2026 05:16:22: Console: /die - Suicide";
        assert_eq!(console_text(log), Some("/die - Suicide"));
        // A server logging without timestamps still gets one.
        assert_eq!(console_text("Console: hello"), Some("hello"));
        // Anything else in that file is not console output.
        assert_eq!(
            console_text("[Info   : Unity Log] Destroying abandoned zdo 1:2"),
            None
        );
    }

    #[test]
    fn a_player_cannot_write_into_the_console_view() {
        // Chat reaches the log. If the tag were enough on its own, anybody on
        // the server could put whatever they liked in front of the admin.
        let shout = "[Info   : Unity Log] 09/12/2026 05:16:22: Got text msg from user:                      Ztnzvoid Console: ban everyone";
        assert_eq!(console_text(shout), None);
    }

    #[test]
    fn fresh_holds_only_the_latest_arrivals() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("server.log");
        append(
            &log, "one
",
        );
        let mut tail = Tail::new(log.clone());
        assert!(tail.poll());
        assert_eq!(tail.fresh(), ["one"]);

        append(
            &log,
            "two
three
",
        );
        assert!(tail.poll());
        // Not "one" again: a console reply must be counted once, not on
        // every frame for as long as it stays in the ring.
        assert_eq!(tail.fresh(), ["two", "three"]);

        assert!(!tail.poll());
        assert!(tail.fresh().is_empty());
    }

    #[test]
    fn reads_only_what_was_appended() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("server.log");
        append(&log, "one\ntwo\n");

        let mut tail = Tail::new(log.clone());
        assert!(tail.poll());
        assert_eq!(tail.lines().collect::<Vec<_>>(), ["one", "two"]);

        assert!(!tail.poll(), "nothing was appended");
        append(&log, "three\n");
        assert!(tail.poll());
        assert_eq!(tail.lines().collect::<Vec<_>>(), ["one", "two", "three"]);
    }

    #[test]
    fn a_half_written_line_waits_for_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("server.log");
        append(&log, "start");
        let mut tail = Tail::new(log.clone());
        tail.poll();
        assert_eq!(tail.lines().len(), 0);
        append(&log, "ed\n");
        tail.poll();
        assert_eq!(tail.lines().collect::<Vec<_>>(), ["started"]);
    }

    #[test]
    fn a_restart_truncates_and_we_follow() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("server.log");
        append(&log, "old session\n");
        let mut tail = Tail::new(log.clone());
        tail.poll();
        std::fs::write(&log, "new\n").unwrap();
        tail.poll();
        assert_eq!(
            tail.lines().collect::<Vec<_>>(),
            ["old session", "new"],
            "old lines stay on screen, the new file is read from its start"
        );
    }

    #[test]
    fn a_missing_log_is_reported_once() {
        let mut tail = Tail::new(PathBuf::from("no/such/server.log"));
        assert!(tail.poll());
        assert!(tail.error().is_some());
        assert!(!tail.poll(), "the same error is not news");
    }

    #[test]
    fn the_session_line_gives_the_join_code_and_the_players() {
        let lines = [
            "[Info   : Unity Log] Game server connected",
            "[Info   : Unity Log] Session \"Midgard\" with join code 024150 and IP              203.0.113.10:2456 is active with 3 player(s)",
            "[Info   : Unity Log] World saved",
        ];
        let session = read_session(lines.into_iter());
        assert_eq!(session.join_code.as_deref(), Some("024150"));
        assert_eq!(session.players, Some(3));
    }

    #[test]
    fn a_log_without_a_session_line_says_nothing() {
        let lines = ["starting", "loading world"];
        assert_eq!(read_session(lines.into_iter()), Session::default());
    }

    #[test]
    fn only_logs_that_exist_are_offered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(find_sources(root, None).is_empty());

        std::fs::create_dir_all(root.join("BepInEx")).unwrap();
        std::fs::write(root.join("BepInEx/LogOutput.log"), b"x").unwrap();
        assert_eq!(
            find_sources(root, None),
            [root.join("BepInEx/LogOutput.log")]
        );

        std::fs::write(root.join("custom.log"), b"x").unwrap();
        let args = ServerArgs {
            log_file: Some("custom.log".into()),
            ..ServerArgs::default()
        };
        assert_eq!(
            find_sources(root, Some(&args)),
            [root.join("custom.log"), root.join("BepInEx/LogOutput.log")],
            "the admin's own choice comes first"
        );
    }
}
