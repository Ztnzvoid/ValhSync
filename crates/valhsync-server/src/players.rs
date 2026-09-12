//! Who may play, who may not, and who runs the place.
//!
//! Iron Gate's own manual is the whole specification here:
//!
//! > You can edit three separate text-files to set 1) who has admin
//! > privileges, 2) who is banned from your server, and 3) who is permitted on
//! > your server. These three files, located in the default save path, are
//! > called `adminlist.txt`, `bannedlist.txt`, and `permittedlist.txt`. Add
//! > one Platform User ID per line to set desired roles.
//!
//! That is the only channel a dedicated server offers from outside the game:
//! it does not read its console (see [`crate::gameserver`]), and Valheim has
//! no RCON. Everything else -- kicking somebody who is connected right now,
//! saving on demand -- is an admin pressing F5 in the game.
//!
//! Each file's first lines are comments Valheim ships, or that an admin has
//! written for themselves. They are kept exactly as found: a tool that eats
//! the note somebody left for their future self is a tool they stop using.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::detect::ServerArgs;

/// The three files, and nothing else in that folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Roll {
    Admin,
    Banned,
    Permitted,
}

impl Roll {
    pub const ALL: [Roll; 3] = [Roll::Admin, Roll::Banned, Roll::Permitted];

    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            Roll::Admin => "adminlist.txt",
            Roll::Banned => "bannedlist.txt",
            Roll::Permitted => "permittedlist.txt",
        }
    }
}

/// Where Valheim keeps the worlds, and with them these three files.
///
/// `-savedir` when the start script sets one, because then nothing is in the
/// default place; otherwise the platform's own path.
#[must_use]
pub fn lists_dir(server_root: &Path, args: Option<&ServerArgs>) -> Option<PathBuf> {
    if let Some(dir) = args.and_then(|a| a.savedir.as_deref()) {
        let dir = dir.trim();
        if !dir.is_empty() {
            let path = Path::new(dir);
            return Some(if path.is_absolute() {
                path.to_path_buf()
            } else {
                server_root.join(path)
            });
        }
    }
    crate::logs::default_savedir()
}

/// One file, read.
///
/// A missing file is an empty list, not an error: Valheim creates them on
/// first run, and an admin who has never banned anybody may not have one.
pub fn read(dir: &Path, roll: Roll) -> Result<Vec<String>> {
    let path = dir.join(roll.file_name());
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    Ok(entries(&text))
}

/// The ids in a list file: everything that is not a comment or blank.
fn entries(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(str::to_owned)
        .collect()
}

/// The comment header, kept verbatim.
fn header(text: &str) -> Vec<String> {
    text.lines()
        .take_while(|l| l.trim().is_empty() || l.trim_start().starts_with("//"))
        .map(str::to_owned)
        .collect()
}

/// Add one id.
///
/// Says so rather than silently doing nothing when it is already there: an
/// admin banning somebody for the second time wants to know which of the two
/// spellings they used the first time.
pub fn add(dir: &Path, roll: Roll, id: &str) -> Result<()> {
    let id = checked_id(id)?;
    let mut ids = read(dir, roll)?;
    if ids.iter().any(|existing| existing == &id) {
        bail!("{id} is already on {}", roll.file_name());
    }
    ids.push(id);
    write(dir, roll, &ids)
}

/// Take one id off a list.
pub fn remove(dir: &Path, roll: Roll, id: &str) -> Result<()> {
    let id = checked_id(id)?;
    let mut ids = read(dir, roll)?;
    let before = ids.len();
    ids.retain(|existing| existing != &id);
    if ids.len() == before {
        bail!("{id} is not on {}", roll.file_name());
    }
    write(dir, roll, &ids)
}

/// A Platform User ID, as the manual spells it, or a bare SteamID64.
///
/// The manual asks for `[Platform]_[UserID]`, case sensitive, which is what
/// the F2 panel shows in game. Plenty of servers -- including the one this
/// was written against -- hold bare 17-digit Steam ids, which Valheim has
/// always accepted, so both are taken. What is refused is anything that would
/// put a line in the file Valheim then silently ignores: a profile URL, a
/// nickname, a comment marker, or a second line smuggled in on the end.
fn checked_id(id: &str) -> Result<String> {
    let id = id.trim();
    if id.is_empty() {
        bail!("no id given");
    }
    if id.len() > 64 {
        bail!("that is too long to be a player id");
    }
    if id.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("a player id is one word with no spaces in it");
    }
    if id.contains('/') {
        bail!("that looks like a link. Valheim wants the id itself, not the page it is on");
    }
    let bare_steam = id.len() == 17 && id.chars().all(|c| c.is_ascii_digit());
    let platform = id.split_once('_').is_some_and(|(platform, rest)| {
        !platform.is_empty()
            && platform.chars().all(|c| c.is_ascii_alphanumeric())
            && !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    if !(bare_steam || platform) {
        bail!(
            "{id} is not a player id. Valheim wants a Platform User ID such as \
             Steam_76561198000000000 -- the F2 panel in game shows it -- or a bare \
             17-digit SteamID64"
        );
    }
    Ok(id.to_string())
}

/// Rewrite one file: its own comments, then the ids.
///
/// Written beside and renamed over, so a server reading the file never finds
/// it half-written.
fn write(dir: &Path, roll: Roll, ids: &[String]) -> Result<()> {
    let path = dir.join(roll.file_name());
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::new();
    for line in header(&existing) {
        out.push_str(&line);
        out.push('\n');
    }
    if out.trim().is_empty() {
        out.clear();
        out.push_str("// ");
        out.push_str(default_note(roll));
        out.push('\n');
    }
    // A set, and one an admin reads with their eyes: sorted, and each id once.
    for id in ids.iter().collect::<BTreeSet<_>>() {
        out.push_str(id);
        out.push('\n');
    }
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let tmp = path.with_extension("txt.valhsync-tmp");
    fs::write(&tmp, out.as_bytes()).with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

fn default_note(roll: Roll) -> &'static str {
    match roll {
        Roll::Admin => "List admin players ID  ONE per line",
        Roll::Banned => "List banned players ID  ONE per line",
        Roll::Permitted => "List permitted players ID ONE per line",
    }
}

/// One line typed at the prompt.
///
/// The verbs are Valheim's own, from the admin console the manual documents:
/// `ban`, `unban`, `banned`. Somebody who already administers a server types
/// what they already know, and the two that cannot work from outside the game
/// -- `kick` and `save` -- are recognised so they can be answered properly
/// rather than falling through to "unknown command".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Add(Roll, String),
    Remove(Roll, String),
    Show(Roll),
    OnlyInGame(&'static str),
    Help,
}

/// What the prompt understands, in the order it is worth reading.
pub const HELP: &[&str] = &[
    "admin <id>      give somebody admin, and unadmin <id> to take it back",
    "ban <id>        ban somebody, and unban <id>",
    "permit <id>     add to the permitted list -- read the warning first",
    "unpermit <id>",
    "admins | banned | permitted     show a list",
    "",
    "An id is a Platform User ID such as Steam_76561198000000000, which the",
    "F2 panel shows in game, or a bare 17-digit SteamID64.",
];

/// Read one line. The error is what the admin is told, so it says what to do.
pub fn parse(line: &str) -> Result<Command> {
    let line = line.trim();
    let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let needs_id = |cmd: fn(String) -> Command| -> Result<Command> {
        if rest.is_empty() {
            bail!("{verb} needs a player id after it. Type help for the list");
        }
        Ok(cmd(rest.to_string()))
    };
    match verb.to_ascii_lowercase().as_str() {
        "" | "help" | "?" => Ok(Command::Help),
        "admin" => needs_id(|id| Command::Add(Roll::Admin, id)),
        "unadmin" => needs_id(|id| Command::Remove(Roll::Admin, id)),
        "ban" => needs_id(|id| Command::Add(Roll::Banned, id)),
        "unban" => needs_id(|id| Command::Remove(Roll::Banned, id)),
        "permit" => needs_id(|id| Command::Add(Roll::Permitted, id)),
        "unpermit" => needs_id(|id| Command::Remove(Roll::Permitted, id)),
        "admins" | "adminlist" => Ok(Command::Show(Roll::Admin)),
        "banned" | "bannedlist" => Ok(Command::Show(Roll::Banned)),
        "permitted" | "permittedlist" => Ok(Command::Show(Roll::Permitted)),
        // The two an admin will reach for first, and the two that have no
        // channel from out here at all.
        "kick" => Ok(Command::OnlyInGame("kick")),
        "save" => Ok(Command::OnlyInGame("save")),
        other => bail!("{other} is not something ValhSync can do. Type help"),
    }
}

/// Carry one out, and say what happened.
pub fn run(dir: &Path, command: Command) -> Result<Vec<String>> {
    match command {
        Command::Help => Ok(HELP.iter().map(|l| (*l).to_string()).collect()),
        Command::Add(roll, id) => {
            add(dir, roll, &id)?;
            let mut said = vec![format!("{id} added to {}", roll.file_name())];
            if roll == Roll::Permitted {
                // Iron Gate's own warning, in bold in the manual, and the one
                // that empties a server when nobody reads it.
                said.push(
                    "Careful: with anybody on the permitted list, everybody else is refused."
                        .to_string(),
                );
            }
            said.push(RELOAD_NOTE.to_string());
            Ok(said)
        }
        Command::Remove(roll, id) => Ok(vec![
            format!("{id} removed from {}", roll.file_name()),
            RELOAD_NOTE.to_string(),
        ]),
        Command::Show(roll) => {
            let ids = read(dir, roll)?;
            if ids.is_empty() {
                return Ok(vec![format!("{} is empty", roll.file_name())]);
            }
            let mut said = vec![format!("{} ({}):", roll.file_name(), ids.len())];
            said.extend(ids);
            Ok(said)
        }
        Command::OnlyInGame(verb) => Ok(vec![format!(
            "{verb} only works from inside the game: an admin presses F5 and types it there. A dedicated server offers no channel for it from out here."
        )]),
    }
}

/// Said after every change, because it is the one thing ValhSync cannot
/// promise. Iron Gate's manual documents the files and says nothing about
/// when a running server re-reads them.
const RELOAD_NOTE: &str = concat!(
    "Written. Whether the running server picks it up without a restart is ",
    "not documented -- restart it if you need to be sure.",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verbs_are_the_ones_an_admin_already_knows() {
        assert_eq!(
            parse("ban 76561190000000001").unwrap(),
            Command::Add(Roll::Banned, "76561190000000001".into())
        );
        assert_eq!(
            parse("UNBAN 76561190000000001").unwrap(),
            Command::Remove(Roll::Banned, "76561190000000001".into())
        );
        assert_eq!(parse("banned").unwrap(), Command::Show(Roll::Banned));
        assert_eq!(parse("").unwrap(), Command::Help);
    }

    #[test]
    fn a_verb_with_no_id_says_what_is_missing() {
        let err = parse("ban").unwrap_err().to_string();
        assert!(err.contains("needs a player id"), "{err}");
    }

    #[test]
    fn kick_and_save_are_answered_rather_than_rejected() {
        // The first two things somebody types. "unknown command" would be
        // true and useless; they exist, just not from out here.
        for verb in ["kick", "save"] {
            let dir = tempfile::tempdir().unwrap();
            let said = run(dir.path(), parse(verb).unwrap()).unwrap();
            assert!(said[0].contains("F5"), "{said:?}");
        }
    }

    #[test]
    fn permitting_somebody_repeats_iron_gates_warning() {
        let dir = tempfile::tempdir().unwrap();
        let said = run(dir.path(), parse("permit 76561190000000001").unwrap()).unwrap();
        assert!(
            said.iter().any(|l| l.contains("everybody else is refused")),
            "{said:?}"
        );
    }

    #[test]
    fn showing_a_list_reads_the_file() {
        let dir = tempfile::tempdir().unwrap();
        add(dir.path(), Roll::Admin, "76561190000000001").unwrap();
        let said = run(dir.path(), parse("admins").unwrap()).unwrap();
        assert!(said[0].contains("adminlist.txt (1)"), "{said:?}");
        assert_eq!(said[1], "76561190000000001");
    }

    #[test]
    fn a_missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path(), Roll::Banned).unwrap().is_empty());
    }

    #[test]
    fn the_admins_own_comments_survive_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adminlist.txt");
        // The header Valheim ships, plus a note this admin wrote themselves.
        fs::write(
            &path,
            "// List admin players ID  ONE per line\n\
             // SteamID64: 17 digits, from the F2 panel\n\
             76561190000000001\n",
        )
        .unwrap();

        add(dir.path(), Roll::Admin, "76561190000000002").unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("from the F2 panel"), "{after}");
        assert_eq!(
            read(dir.path(), Roll::Admin).unwrap(),
            ["76561190000000001", "76561190000000002"]
        );
    }

    #[test]
    fn both_spellings_of_an_id_are_taken() {
        assert!(checked_id("76561190000000001").is_ok());
        assert!(checked_id("Steam_76561190000000001").is_ok());
        assert!(checked_id("XBox_2535400000000000").is_ok());
    }

    #[test]
    fn what_valheim_would_ignore_is_refused_here_instead() {
        for bad in [
            "",
            "https://steamcommunity.com/id/somebody",
            "// not an id",
            "Bjorn the Red",
            "76561190000000001\n76561198032401516",
            "nobody",
        ] {
            let err = checked_id(bad).unwrap_err().to_string();
            assert!(!err.is_empty(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn adding_twice_says_so_rather_than_doing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        add(dir.path(), Roll::Banned, "76561190000000001").unwrap();
        let err = add(dir.path(), Roll::Banned, "76561190000000001")
            .unwrap_err()
            .to_string();
        assert!(err.contains("already on bannedlist.txt"), "{err}");
    }

    #[test]
    fn removing_somebody_who_was_never_there_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let err = remove(dir.path(), Roll::Admin, "76561190000000001")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not on adminlist.txt"), "{err}");
    }

    #[test]
    fn a_new_file_gets_the_header_valheim_ships() {
        let dir = tempfile::tempdir().unwrap();
        add(dir.path(), Roll::Permitted, "76561190000000001").unwrap();
        let text = fs::read_to_string(dir.path().join("permittedlist.txt")).unwrap();
        assert!(text.starts_with("// List permitted players ID"), "{text}");
    }

    #[test]
    fn nothing_is_left_beside_the_file_it_replaced() {
        let dir = tempfile::tempdir().unwrap();
        add(dir.path(), Roll::Admin, "76561190000000001").unwrap();
        let left: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, ["adminlist.txt"]);
    }
}
