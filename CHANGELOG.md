# Changelog

Notable changes, newest first. Dates are ISO 8601.

ValhSync is early: 0.2.0 is the second release, and one server has run it in
anger. Nothing here is settled yet, including the shape of the configuration
file and the wire format.

## 0.2.0 — 2026-09-12

- **What changed, before agreeing to it.** A sync that touches mods now says
  which ones, by name: added, updated, removed. Computed from the plan, so
  nobody has to write it.
- **A word from the admin.** An optional note travels signed in the manifest,
  written from the Mods tab. For what a diff cannot say: that a mod resets its
  own config, that a chest mod wants an empty base first.
- **Kept afterwards.** Fifty entries of history per player, one per sync that
  changed something, behind a "What's new" button.
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

## 0.1.0 — 2026-09-12

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
