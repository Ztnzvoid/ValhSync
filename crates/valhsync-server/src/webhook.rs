//! Telling a Discord channel that a new pack went out.
//!
//! Publishing already produces the patch note: [`valhsync_core::changes`]
//! turns the plan into "these mods arrived, these moved, these went", and the
//! admin adds the part no diff can produce -- that a mod resets its own
//! config, that the update is only for the people who crashed on the boat.
//! The launcher shows that to a player who opens it. A player who has not
//! opened it hears nothing, which is how a server ends up with half its people
//! on last week's pack.
//!
//! So the same note is posted to a webhook the admin configured once. Two
//! things about that deserve saying out loud:
//!
//! * A webhook URL is a credential. Anybody holding it can post into that
//!   channel as the server, so it is never written to a log, an error or a
//!   panic message, and [`parse`] refuses to hold anything but a Discord
//!   address -- an admin who pastes the wrong line must not have their pack
//!   contents delivered to a stranger's host.
//! * Posting is an announcement, not a step of publishing. Every function here
//!   returns a [`Result`] the caller is expected to surface and carry on from:
//!   a webhook that was deleted, rate limited or simply unreachable must never
//!   be the reason a pack the admin already built fails to go out.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use valhsync_core::{ChangeKind, ModChange};

/// Hosts a webhook may live on.
///
/// Discord serves its API from `discord.com`, kept `discordapp.com` working
/// for the years of links that predate the rename, and runs the same API on
/// `ptb.` and `canary.` for its test builds -- webhook URLs copied out of
/// those clients carry those hosts. Nothing else is accepted: the point of the
/// list is that a mistyped or pasted-over URL fails when it is typed, in front
/// of the admin, rather than quietly shipping the pack's mod list to whatever
/// host happened to be on the clipboard.
const HOSTS: [&str; 6] = [
    "discord.com",
    "ptb.discord.com",
    "canary.discord.com",
    "discordapp.com",
    "ptb.discordapp.com",
    "canary.discordapp.com",
];

/// The most a Discord message may hold; it refuses a longer one outright.
const MAX_MESSAGE: usize = 2000;

/// How much of the admin's note is carried.
///
/// The note is the part of the message worth keeping -- the mod list can be
/// read in the launcher, the note exists nowhere else -- so it is only ever
/// cut to guarantee that some of the mod list still fits. Without a cap here a
/// note near [`valhsync_core::manifest::MAX_NOTES`] would fill the message on
/// its own and Discord would reject the whole thing.
const MAX_NOTE: usize = 400;

/// Long enough for a server name and a pack id, short enough that neither can
/// crowd out the message. Both are the admin's own values, so this only ever
/// fires on something odd.
const MAX_NAME: usize = 100;

/// How long a post may take before publishing gives up on it.
///
/// Short on purpose. The admin is watching a pack finish; a webhook pointing
/// at a host that no longer answers must cost them a few seconds, not a
/// connection timeout's worth of a frozen window.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A webhook address the admin configured, checked when it was typed.
///
/// Parsing is the whole point of the type: once a `Hook` exists, the scheme
/// and the host have already been agreed, so [`post`] cannot be talked into
/// sending a pack's contents somewhere else by a value that was never looked
/// at.
pub struct Hook {
    /// The full URL, including the token. Private, and it stays that way.
    url: reqwest::Url,
}

/// Written by hand rather than derived, because the derived one would print
/// the token. A `Hook` in a `{:?}` -- in a log line, in a panic message, in
/// some future struct that derives `Debug` and happens to hold one -- must not
/// leak posting rights to that channel.
impl std::fmt::Debug for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hook({} <redacted>)", self.host())
    }
}

impl Hook {
    /// The host, which is the only part of the URL that is safe to show.
    ///
    /// Useful for telling the admin where a post went without putting the
    /// token on screen or in a log file.
    #[must_use]
    pub fn host(&self) -> &str {
        self.url.host_str().unwrap_or("discord")
    }
}

/// Read what the admin typed, and refuse anything that is not a Discord
/// webhook.
///
/// The error says which rule was broken and never repeats the URL back: an
/// admin pasting into a config file wants to know that the host was wrong, and
/// a token that reaches a log or a screenshot has to be regenerated in
/// Discord.
pub fn parse(url: &str) -> Result<Hook> {
    let raw = url.trim();
    if raw.is_empty() {
        bail!("no webhook URL given");
    }
    let Ok(url) = reqwest::Url::parse(raw) else {
        bail!(
            "that is not a URL. Discord writes the webhook address out for you under \
             Server Settings -> Integrations -> Webhooks -> Copy Webhook URL"
        );
    };
    if url.scheme() != "https" {
        bail!(
            "a webhook URL has to be https. The URL carries a token that lets anybody \
             post in that channel, and plain http would put it on the wire for anyone \
             on the way to read"
        );
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("a Discord webhook URL carries no username or password in it");
    }
    let Some(host) = url.host_str() else {
        bail!("that URL has no host in it");
    };
    // `Url` has already lowercased the host of an https URL, so this compares
    // what an admin typed in any case against the list.
    if !HOSTS.contains(&host) {
        bail!(
            "{host} is not Discord. ValhSync only posts to {}, because the URL is a \
             credential and the message holds the pack's mod list -- sending either to \
             the wrong host cannot be taken back",
            HOSTS.join(", ")
        );
    }
    if !is_webhook_path(url.path()) {
        bail!(
            "that is a Discord URL but not a webhook one. A webhook address looks like \
             https://discord.com/api/webhooks/<id>/<token> and Discord copies it to the \
             clipboard for you"
        );
    }
    Ok(Hook { url })
}

/// Whether the path is `/api/webhooks/<id>/<token>`, with or without the `v10`
/// style API version Discord sometimes includes.
///
/// Checked so that a channel link or an invite -- both of which are Discord
/// URLs an admin might have in their clipboard -- is caught here rather than
/// at publish time as a bewildering 404.
fn is_webhook_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let Some(at) = parts.iter().position(|p| *p == "webhooks") else {
        return false;
    };
    // An id and a token have to follow it, both non-empty.
    parts.len() >= at + 3
}

/// Post one patch note, and say whether it arrived.
///
/// `Ok(())` means Discord answered 2xx -- it takes the message and returns 204
/// with no body. Every other outcome is an error the caller is expected to
/// show and then ignore: this is an announcement about a pack that has already
/// been published, so a dead webhook is worth a line in the window and nothing
/// more. Nothing in an error returned from here contains the URL.
pub fn post(
    hook: &Hook,
    server_name: &str,
    pack_id: &str,
    changes: &[ModChange],
    note: &str,
) -> Result<()> {
    let content = compose(server_name, pack_id, changes, note);
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("valhsync-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("cannot build the HTTP client")?;
    // `allowed_mentions` empty is what stops a mod named `@everyone` -- or an
    // admin's note mentioning a role -- from pinging the whole server. Markdown
    // is defused in `compose`; mentions cannot be, so they are refused by the
    // API instead.
    let payload = serde_json::to_vec(&serde_json::json!({
        "content": content,
        "allowed_mentions": { "parse": [] },
    }))
    .context("cannot build the webhook message")?;
    let response = client
        .post(hook.url.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload)
        .send()
        // `reqwest` puts the URL it was given into its own error text, which
        // here is the token. `without_url` is what takes it back out again.
        .map_err(reqwest::Error::without_url)
        .context("cannot reach the Discord webhook")?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    bail!("{}", refusal(status.as_u16()));
}

/// What a non-2xx answer means, in words an admin can act on.
///
/// Deliberately takes a bare number rather than the response, so it can be
/// read and tested without a network, and so there is no way for the URL to
/// reach the message.
fn refusal(status: u16) -> String {
    let said = match status {
        401 | 403 | 404 => {
            "Discord does not know that webhook any more. It was probably deleted, or its \
             token was reset -- make a new one in Discord and paste it in again"
        }
        429 => {
            "Discord is rate limiting this webhook. The pack went out, the announcement did \
             not -- try again in a minute"
        }
        400 => "Discord refused the message as malformed. The pack itself is published",
        other => return format!("Discord answered {other}; the pack itself is published"),
    };
    said.to_string()
}

/// The message that goes in the channel.
///
/// Kept apart from [`post`] so the wording and the length limit can be tested
/// without talking to anybody. The result always fits in a Discord message.
#[must_use]
pub fn compose(server_name: &str, pack_id: &str, changes: &[ModChange], note: &str) -> String {
    let head = head_line(server_name, pack_id, changes);
    let quoted = note_block(note);
    let listed = ordered(changes);
    // Widest list first, narrowing until the message fits. The mod list is the
    // part worth cutting: a player can read it in the launcher, whereas the
    // heading says what happened and the note exists nowhere else.
    for keep in (0..=listed.len()).rev() {
        let text = joined(&head, &quoted, &body_block(&listed, keep));
        if length(&text) <= MAX_MESSAGE {
            return text;
        }
    }
    // Only reachable if the heading and the note alone overflow, which the
    // caps above are there to prevent. Cutting is still better than handing
    // Discord something it will reject whole.
    clipped(&joined(&head, &quoted, ""))
}

/// The line that says what happened, with the totals so they are right even
/// when the list below is cut.
fn head_line(server_name: &str, pack_id: &str, changes: &[ModChange]) -> String {
    let name = escaped(&shortened(server_name.trim(), MAX_NAME));
    let name = if name.is_empty() {
        "This server".to_string()
    } else {
        format!("**{name}**")
    };
    let pack = escaped(&shortened(pack_id.trim(), MAX_NAME));
    let counts = counted(changes);
    if pack.is_empty() {
        format!("{name} -- new mod pack ({counts})")
    } else {
        format!("{name} -- new mod pack {pack} ({counts})")
    }
}

/// "2 added, 1 updated", leaving out the kinds that did not happen.
fn counted(changes: &[ModChange]) -> String {
    let mut said = Vec::new();
    for kind in KINDS {
        let n = changes.iter().filter(|c| c.kind == kind).count();
        if n > 0 {
            said.push(format!("{n} {}", word(kind)));
        }
    }
    if said.is_empty() {
        // A pack can be republished with only a note, or with changes outside
        // BepInEx/plugins, which `mods_touched` deliberately says nothing about.
        return "no mod changes".to_string();
    }
    said.join(", ")
}

/// The admin's note, as a quote so it reads as somebody talking rather than as
/// more of the machine's output.
fn note_block(note: &str) -> String {
    let note = note.trim();
    if note.is_empty() {
        return String::new();
    }
    let note = shortened(note, MAX_NOTE);
    note.lines()
        .map(|line| format!("> {}", escaped_line(line.trim_end())))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The three kinds, in the order they are worth reading: what arrived, what
/// moved, what went. Same order [`valhsync_core::changes::mods_touched`] uses.
const KINDS: [ChangeKind; 3] = [ChangeKind::Added, ChangeKind::Updated, ChangeKind::Removed];

fn word(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Updated => "updated",
        ChangeKind::Removed => "removed",
    }
}

fn heading(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "Added",
        ChangeKind::Updated => "Updated",
        ChangeKind::Removed => "Removed",
    }
}

/// Every change, grouped by kind, in reading order. Done here rather than
/// trusting the caller's order so that the lines are grouped whatever gets
/// handed in.
fn ordered(changes: &[ModChange]) -> Vec<(ChangeKind, &str)> {
    let mut out = Vec::with_capacity(changes.len());
    for kind in KINDS {
        for change in changes.iter().filter(|c| c.kind == kind) {
            out.push((kind, change.name.as_str()));
        }
    }
    out
}

/// The first `keep` mods as one line per kind, plus a line saying how many
/// were left off when they do not all fit.
fn body_block(listed: &[(ChangeKind, &str)], keep: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for kind in KINDS {
        let names: Vec<String> = listed
            .iter()
            .take(keep)
            .filter(|(k, _)| *k == kind)
            .map(|(_, name)| escaped(&shortened(name, MAX_NAME)))
            .collect();
        if !names.is_empty() {
            lines.push(format!("{}: {}", heading(kind), names.join(", ")));
        }
    }
    let left_out = listed.len().saturating_sub(keep);
    if left_out > 0 {
        // Said rather than silently dropped: an admin reading the channel has
        // to be able to tell a short list from a cut one.
        lines.push(format!(
            "...and {left_out} more not listed -- Discord caps a message at {MAX_MESSAGE} \
             characters. The launcher shows all of them."
        ));
    }
    lines.join("\n")
}

fn joined(head: &str, quoted: &str, body: &str) -> String {
    [head, quoted, body]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Discord's markdown, defused.
///
/// A mod actually published under the name `**boom**` -- or an admin's note
/// with an odd number of asterisks in it -- would otherwise turn the rest of
/// the message bold, or hide it behind a spoiler, or swallow it into a code
/// block. Discord honours a backslash before each of these, so every one gets
/// one and the name reads as it was written.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '*' | '_' | '~' | '`' | '|' => {
                out.push('\\');
                out.push(c);
            }
            // Newlines and the rest would break a line into two, and a control
            // character has no business in a mod name or a heading.
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The same, for something that starts a line.
///
/// `#`, `-`, `+` and `>` only mean anything at the start of a line, where they
/// make a heading, a bullet or a quote. Inside the admin's note they are far
/// more likely to be prose than intent, so they are shown as typed.
fn escaped_line(line: &str) -> String {
    let escaped = escaped(line);
    if escaped.starts_with(['#', '-', '+', '>']) {
        format!("\\{escaped}")
    } else {
        escaped
    }
}

/// How long Discord thinks a string is.
///
/// It counts UTF-16 code units, not characters, so an emoji outside the basic
/// plane costs two. Counting the same way is what stops a note full of them
/// from being refused by a limit we thought we were under.
fn length(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s` cut to `max` units, on a character boundary, with an ellipsis when
/// anything was lost.
fn shortened(s: &str, max: usize) -> String {
    if length(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if length(&out) + c.len_utf16() > max.saturating_sub(1) {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// The last resort: hand back something Discord will accept even if the pieces
/// above somehow did not fit.
fn clipped(s: &str) -> String {
    shortened(s, MAX_MESSAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(name: &str, kind: ChangeKind) -> ModChange {
        ModChange {
            name: name.to_string(),
            kind,
        }
    }

    const GOOD: &str = "https://discord.com/api/webhooks/123456789/abcdefTOKENghijkl";

    #[test]
    fn the_addresses_discord_hands_out_are_accepted() {
        for url in [
            GOOD,
            "https://discordapp.com/api/webhooks/1/t",
            "https://ptb.discord.com/api/webhooks/1/t",
            "https://canary.discord.com/api/webhooks/1/t",
            // Discord includes an API version in some of its own copy buttons.
            "https://discord.com/api/v10/webhooks/1/t",
            // Whatever surrounds a paste out of a chat window.
            "  https://discord.com/api/webhooks/1/t  ",
        ] {
            assert!(parse(url).is_ok(), "{url} was refused");
        }
    }

    #[test]
    fn anything_that_is_not_a_discord_webhook_is_refused() {
        for url in [
            "",
            "not a url",
            // The token would go over the wire in the clear.
            "http://discord.com/api/webhooks/1/t",
            // The whole reason for the allowlist: a pack's mod list must not
            // be posted to somebody else's host.
            "https://example.com/api/webhooks/1/t",
            "https://discord.com.evil.test/api/webhooks/1/t",
            "https://evil.test/discord.com/api/webhooks/1/t",
            // Discord URLs, but not webhook ones.
            "https://discord.com/channels/1/2",
            "https://discord.gg/abcdef",
            "https://discord.com/api/webhooks/1",
            // Credentials smuggled into the authority.
            "https://user:pw@discord.com/api/webhooks/1/t",
        ] {
            assert!(parse(url).is_err(), "{url} was accepted");
        }
    }

    #[test]
    fn a_refused_host_is_told_what_is_allowed() {
        let err = parse("https://example.com/api/webhooks/1/t")
            .unwrap_err()
            .to_string();
        assert!(err.contains("example.com is not Discord"), "{err}");
        assert!(err.contains("discord.com"), "{err}");
    }

    /// The URL grants posting rights on that channel to whoever holds it, so
    /// it must not reach an error, a log line or a `{:?}`. Everything that
    /// could carry it is checked here in one place.
    #[test]
    fn a_webhook_url_never_reaches_an_error_message() {
        let secrets = ["abcdefTOKENghijkl", "123456789"];
        // Every way parsing can fail, with a real-looking token in each.
        for url in [
            "http://discord.com/api/webhooks/123456789/abcdefTOKENghijkl",
            "https://example.com/api/webhooks/123456789/abcdefTOKENghijkl",
            "https://discord.com/channels/123456789/abcdefTOKENghijkl",
            "ftp://discord.com/api/webhooks/123456789/abcdefTOKENghijkl",
            "https://u:123456789@discord.com/api/webhooks/1/abcdefTOKENghijkl",
            ":://123456789/abcdefTOKENghijkl",
        ] {
            let err = format!("{:#}", parse(url).unwrap_err());
            for secret in secrets {
                assert!(!err.contains(secret), "{err} leaks {secret}");
            }
        }
        // And the type itself, which is what a stray `{:?}` would print.
        let hook = parse(GOOD).unwrap();
        let shown = format!("{hook:?}");
        for secret in secrets {
            assert!(!shown.contains(secret), "{shown} leaks {secret}");
        }
        assert!(shown.contains("discord.com"), "{shown}");
        // Nothing an HTTP answer can say carries it either.
        for status in [400, 401, 403, 404, 429, 500] {
            let said = refusal(status);
            for secret in secrets {
                assert!(!said.contains(secret), "{said} leaks {secret}");
            }
        }
    }

    #[test]
    fn the_message_says_the_server_the_pack_and_what_changed() {
        let changes = vec![
            change("Seasonality", ChangeKind::Added),
            change("EpicLoot", ChangeKind::Updated),
            change("OldChests", ChangeKind::Removed),
            change("PlantEverything", ChangeKind::Added),
        ];
        let text = compose("Frostveil", "a1b2c3", &changes, "Empty your chests first.");
        assert!(text.contains("**Frostveil**"), "{text}");
        assert!(text.contains("a1b2c3"), "{text}");
        assert!(text.contains("2 added, 1 updated, 1 removed"), "{text}");
        assert!(
            text.contains("Added: Seasonality, PlantEverything"),
            "{text}"
        );
        assert!(text.contains("Updated: EpicLoot"), "{text}");
        assert!(text.contains("Removed: OldChests"), "{text}");
        assert!(text.contains("> Empty your chests first."), "{text}");
    }

    /// The three kinds come out grouped whatever order they arrive in, so the
    /// message does not depend on how the caller built its list.
    #[test]
    fn the_kinds_are_grouped_in_reading_order() {
        let changes = vec![
            change("Gone", ChangeKind::Removed),
            change("New", ChangeKind::Added),
            change("Moved", ChangeKind::Updated),
        ];
        let text = compose("S", "p", &changes, "");
        let added = text.find("Added:").unwrap();
        let updated = text.find("Updated:").unwrap();
        let removed = text.find("Removed:").unwrap();
        assert!(added < updated && updated < removed, "{text}");
    }

    #[test]
    fn a_pack_with_nothing_to_list_still_says_something() {
        let text = compose("Frostveil", "a1b2c3", &[], "Config only, no new mods.");
        assert!(text.contains("no mod changes"), "{text}");
        assert!(text.contains("> Config only, no new mods."), "{text}");
    }

    /// A mod really can be published under a name full of asterisks, and one
    /// unbalanced marker turns the rest of the message bold, or hides it in a
    /// spoiler.
    #[test]
    fn a_mod_name_cannot_reach_into_discords_markdown() {
        let changes = vec![
            change("**boom**", ChangeKind::Added),
            change("a|b", ChangeKind::Added),
            change("back`tick", ChangeKind::Added),
            change("under_score", ChangeKind::Added),
        ];
        let text = compose("**server**", "``id``", &changes, "");
        // Our own emphasis around the name is the only unescaped `**` there is.
        assert!(text.contains("\\*\\*boom\\*\\*"), "{text}");
        assert!(text.contains("a\\|b"), "{text}");
        assert!(text.contains("back\\`tick"), "{text}");
        assert!(text.contains("under\\_score"), "{text}");
        assert!(text.contains("**\\*\\*server\\*\\***"), "{text}");
        assert!(!text.contains("``id``"), "{text}");
    }

    /// A note is prose. A line beginning with `-` is a dash, not a bullet, and
    /// a newline in it must not be able to close the quote.
    #[test]
    fn a_note_is_shown_as_it_was_typed() {
        let text = compose("S", "p", &[], "# not a heading\n- not a bullet\n*emph*");
        assert!(text.contains("> \\# not a heading"), "{text}");
        assert!(text.contains("> \\- not a bullet"), "{text}");
        assert!(text.contains("> \\*emph\\*"), "{text}");
    }

    #[test]
    fn a_long_mod_list_is_cut_at_the_tail_and_says_how_many_went() {
        let changes: Vec<ModChange> = (0..400)
            .map(|i| change(&format!("SomeFairlyLongModName{i}"), ChangeKind::Added))
            .collect();
        let text = compose("Frostveil", "a1b2c3", &changes, "Read the pins.");
        assert!(length(&text) <= MAX_MESSAGE, "{}", length(&text));
        // The heading, the counts and the note survive; the list is what gives.
        assert!(text.contains("**Frostveil**"), "{text}");
        assert!(text.contains("400 added"), "{text}");
        assert!(text.contains("> Read the pins."), "{text}");
        assert!(text.contains("more not listed"), "{text}");
        // The ones it did list are the first of them, not a random slice.
        assert!(text.contains("SomeFairlyLongModName0"), "{text}");
        assert!(!text.contains("SomeFairlyLongModName399"), "{text}");
    }

    /// Discord refuses a message over the limit whole, so nothing anybody can
    /// type may push it over -- not a note at the manifest's maximum, not a
    /// name that is one long mod, not emoji, which Discord counts double.
    #[test]
    fn nothing_composes_a_message_discord_would_refuse() {
        let cases: Vec<(String, String, Vec<ModChange>, String)> = vec![
            ("S".into(), "p".into(), Vec::new(), "x".repeat(2000)),
            (
                "N".repeat(500),
                "P".repeat(500),
                vec![change(&"M".repeat(5000), ChangeKind::Added)],
                "n".repeat(2000),
            ),
            (
                "🧙‍♂️".repeat(100),
                "🪓".repeat(100),
                (0..200)
                    .map(|i| change(&format!("🌲{i}"), ChangeKind::Removed))
                    .collect(),
                "🔥".repeat(1000),
            ),
            (
                "S".into(),
                "p".into(),
                vec![change(&"*".repeat(3000), ChangeKind::Updated)],
                "*".repeat(2000),
            ),
        ];
        for (name, pack, changes, note) in cases {
            let text = compose(&name, &pack, &changes, &note);
            assert!(
                length(&text) <= MAX_MESSAGE,
                "{} units for {name:.20}",
                length(&text)
            );
        }
    }

    #[test]
    fn cutting_a_string_keeps_it_readable_and_short() {
        assert_eq!(shortened("short", 10), "short");
        assert_eq!(shortened("abcdefghij", 5), "abcd…");
        // Cut on a character boundary, never through one.
        let emoji = shortened(&"🔥".repeat(10), 5);
        assert!(length(&emoji) <= 5, "{emoji}");
        assert!(emoji.ends_with('…'), "{emoji}");
    }

    #[test]
    fn the_host_is_the_only_part_of_a_hook_that_can_be_shown() {
        let hook = parse("https://canary.discord.com/api/webhooks/1/secrettoken").unwrap();
        assert_eq!(hook.host(), "canary.discord.com");
        assert!(!hook.host().contains("secrettoken"));
    }
}
