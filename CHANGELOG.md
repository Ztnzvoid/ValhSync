# Changelog

Notable changes, newest first. Dates are ISO 8601.

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
  code and version, types commands into its console, chooses which mods reach
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
