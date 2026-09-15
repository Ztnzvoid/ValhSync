# Changelog

Notable changes, newest first. Dates are ISO 8601.

ValhSync is early: 0.0.5 is the fifth release, and one server has run it in
anger. Nothing here is settled yet, including the shape of the configuration
file and the wire format.

The numbering was reset before publication, because 0.2 claimed more than the
project had earned. Binaries handed out before that carried 0.1.0 and 0.2.0;
they are the 0.0.1 and 0.0.2 below. Anybody holding one of those installs
0.0.3 by hand once -- a launcher will not offer itself as an update to a
version that sorts above it -- and every release after this one arrives on
its own.

## 0.0.5 — 2026-09-15

The manifest gains a field, so both halves matter: a launcher older than this
does not know the permission exists and keeps setting client-side mods aside,
whatever the server says. Players update through the same signed channel as
always, and then it takes effect.

- **A player's own mods are their own, unless the admin says otherwise.** A
  client-side mod -- a map overlay, an interface tweak, anything that never
  talks to the server -- used to be moved aside at the next sync, because the
  launcher quarantined everything the pack did not contain. There is now one
  switch on the Mods tab, on by default: *players may keep mods of their own*.
  Turned off, the old behaviour is back for admins who want every client
  identical, and the launcher now says why before it moves anything.

  It is a permission, not an inventory. The answer travels inside the signed
  manifest and the launcher acts on it alone; nothing about what a player has
  installed is ever sent to the server, and there is no route for it to be.

  Existing configurations gain the permissive default when they are next
  written, so an admin who wants the strict behaviour ticks the box off. A
  manifest from a server too old to have the field reads as strict, which is
  what those servers already did.

## 0.0.4 — 2026-09-14

Documentation, packaging and the groundwork for signed binaries. No change to
the wire format, so a 0.0.3 launcher and a 0.0.4 publisher still understand
each other.

- **The address is the way in.** Joining by the server's own `ip:port` -- the
  same one Valheim takes -- has always worked, but every page and the
  add-server dialog led with the invite code, so the simple path read as the
  fallback. Reversed everywhere, in all eight languages and in the CLI's help.
  The invite code keeps the job it is actually best at: a zip that arrives with
  the code beside the executable and nothing to type.
- **Built with AI assistance, said in the window.** The README, the site, both
  guides and each program's about line now say that ValhSync was written with
  an AI assistant, directed and reviewed by a person.
- **A code signing policy, and a release that can use one.** The release
  workflow submits the Windows binaries to SignPath before packaging them and
  builds the archives from what comes back signed; every step is gated on a
  repository variable, so a fork still produces a release. `docs/code-signing-
  policy.md` says what may be signed, who approves it, what data is collected
  (none) and how to uninstall.
- **Pictures where they belong.** Both guides carry screenshots, and the
  archives carry the pictures those pages point at -- an offline guide was
  three broken images.
- **A Thunderstore package**, built by a script that refuses a manifest whose
  version disagrees with the workspace, a description over the limit, or an
  icon that is not 256x256.
- Documentation caught up with the software in four places: players do not
  agree to a sync any more, the window has five tabs rather than two, the
  console prompt runs ValhSync's own player commands because Valheim does not
  read its console, and the launcher's update band is the theme's blue. The
  Discord announcement, restarting a crashed server, world backups, disabling
  or removing a mod from its row, and the `[game_server]` and `[limits]`
  configuration sections were undocumented until now.

## 0.0.3 — 2026-09-13

- **What changed, and no agreeing to it.** A sync that touches mods says which
  ones, by name: added, updated, removed, computed from the plan so nobody has
  to write it. The player is not asked to accept it -- adding a server is where
  they said who they trust, and asking again at the first sync was a second
  signature on the same line.
- **A word from the admin.** An optional note travels signed in the manifest,
  written from the Mods tab. For what a diff cannot say: that a mod resets its
  own config, that a chest mod wants an empty base first.
- **Kept afterwards.** The note stays readable after the sync, behind a
  "What's new" button, instead of vanishing the moment the files land.
- The publisher no longer skips a rebuild when only the note changed, and a
  running one now notices its configuration changing at all: it watches the
  file and re-reads it, instead of publishing the manifest it built at startup.
- **The configuration saves itself.** No Save button: what is on screen is what
  the server publishes, and asking for a second confirmation produced servers
  running on a configuration that was never written. The bar says what is
  happening, and why, when something stops it.
- **Restart**, beside Stop: the same Ctrl+C, and the server comes back once the
  world has actually been written.
- Publishing follows the game server however it was started, not only when it
  was started from the window; the publish mode, the window language and the
  export folder are remembered instead of resetting on every launch.
- Both executables carry Windows version metadata. Not a substitute for code
  signing, which is what actually settles an antivirus, but the part that costs
  nothing.
- The launcher says "server offline" rather than quoting a transport error at
  someone who cannot act on it.
- **Players**, on their own tab and at the console's prompt: admins, bans and
  the permitted list, written to the three files Iron Gate's manual documents.
  That is the only channel a dedicated server has from outside the game.
  `kick` and `save` are recognised and answered — they exist, but only from an
  admin pressing F5 in the game — rather than falling through to "unknown
  command". Iron Gate's warning about the permitted list, the one that empties
  a server when nobody reads it, is on the card.
- **A console that is a console**: its own resizable panel, and what the game
  server prints to its own console instead of a command prompt that never
  worked. Valheim's dedicated server prints `type "help" - for commands`
  on start-up and then never reads its console: keystrokes written into its
  input buffer are accepted by Windows and ignored by the server. Checked
  against a server up for sixteen hours — every call reported success and the
  log did not grow by a byte. The prompt is gone; the lines the server does
  print to its console are pulled out of the log, where fifteen of them sit
  among tens of thousands, and shown where they can be read.
- **Players**, on their own tab and at the console's prompt: admins, bans and
  the permitted list, written to the three files Iron Gate documents, with the
  names the server's own log recorded beside each id. `kick` and `save` are
  recognised and answered rather than failing quietly -- they exist, but only
  from an admin pressing F5 in the game.
- **World backups**, taken from the tab where mods are changed. Both halves of
  a world travel together, beside `worlds_local` and never inside it, and a
  copy taken while the server is running says so rather than pretending.
- **Bring the server back when it goes down on its own.** Off by default, and
  bounded at three restarts in twenty minutes: the failure this exists for is
  also the failure that loops.
- **Turn a mod off, or take it out**, from its row. Disabling moves it out of
  BepInEx entirely; removing moves it to a folder ValhSync owns rather than
  deleting it.
- **Announce a new pack in a Discord**, optionally. The address is treated as
  the credential it is: masked in the window, never in a log or an error.
- One press is one action. PLAY and UPDATE were disabled while a check ran --
  which is the second somebody who has just come back to the window reaches for
  them -- so the press went nowhere. It is held and carried out instead.
- The window obeys the person dragging it. The code handing the size over
  existed and never ran: it wrote "manual" into memory and overwrote it on the
  way out of the same frame.
- Content sits in a centred column, folding sections take the room the window
  has, and the progress bar has its own strip under the button that starts it,
  drawn in the theme rather than in egui's grey.
- The background mark is gone; the left edge burns instead, and the embers come
  off it.
- **Drop a mod on the window.** The Mods tab takes a `.zip` from Thunderstore,
  Nexus, a release page or anywhere else, a mod folder, or a bare `.dll`. It
  works out whether the files sit at the archive's root, under `plugins/` or
  under a whole `BepInEx/` tree, names the folder from the manifest when there
  is one, and clears the previous version out first so BepInEx is never asked
  to load two. Nothing is written outside `BepInEx/plugins`, whatever paths the
  archive claims. Stored, deflate and deflate64 are read; an archive packed
  with bzip2, LZMA, zstandard or xz is refused by the name of its compression
  rather than half-read, so the answer is "extract it and drop the folder"
  rather than a decoder error nobody can act on.
- **Eight languages**, chosen from a menu on either window and remembered:
  English, French, German, Spanish, Italian, Polish, Portuguese, Russian.
  English is the default, and the language the system asks for is offered
  first. The server window was half-translated by hand; every string in both
  windows now goes through one table, so a missing translation is a build
  error rather than a sentence in the wrong language.
- **The invite code has a button** that copies it, instead of asking an admin
  to select a long line of base64 out of a panel by hand.
- The documentation is three pages under `docs/`: what ValhSync is, a server
  guide for Windows and Linux, and a player guide. Each is one self-contained
  file, so it can be opened from a folder or sent to somebody without breaking.
  They travel inside the release archives too.
- Scrollbars, and the mark behind the windows, belong to the theme.

## 0.0.1 — 2026-09-12

First public release.

### The tool

- **Signed mod packs.** The publisher scans the dedicated server's BepInEx
  folder, builds a content-addressed pack and signs a manifest with an Ed25519
  key. The launcher pins that key from an invite code and verifies the
  signature before parsing anything.
- **Nothing is lost.** Files the launcher replaces go to a backup, unknown DLLs
  are set aside inside the game folder rather than deleted, and the last sync
  can be rolled back exactly.
- **One port.** The live publisher serves on the game's own port in TCP, which
  Valheim uses in UDP only: an admin never has to open a port the game does not
  already use. A static export to any web space is the alternative.
- **An admin window.** Starts and stops the dedicated server (Ctrl+C, so the
  world is written before it exits), follows its log, reports players, join
  code and version, chooses which mods reach
  players, and publishes.
- **A player launcher.** Several servers side by side, what will be installed
  or updated before it happens, one button to sync and play, a repair pass, and
  a switch to play without mods that renames `winhttp.dll` rather than deleting
  anything.
- **Launcher updates over the same channel.** A server can offer a newer
  ValhSync, signed with the key players already pinned and named by digest. The
  prompt says which server it comes from; see `SECURITY.md` for what that
  means, and the player guide for how to decline it.

### Known limitations

Stated in full in `SECURITY.md`. The short version: the admin is trusted, the
transport is plain HTTP by default (integrity does not depend on it), and the
update channel signs whatever bytes are in the launcher beside the publisher
without vouching for where they came from.
