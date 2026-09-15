# Code signing policy

How ValhSync's Windows binaries are signed, who can cause a signature to
happen, and what the signature does and does not promise.

**Status, 15 September 2026.** ValhSync has applied to the
[SignPath Foundation](https://signpath.org/) for a certificate. Until that is
granted, **released binaries are unsigned** and Windows SmartScreen warns on
first run — the [player guide](player-guide.html) says so and explains how to
check a download by its SHA-256 instead. The 0.0.4 build was also put through VirusTotal
and no engine flagged it ([report](https://www.virustotal.com/gui/file/0673c62743e4c568595c6f3bd2bd6cc7bafc4dff31686790d5ef570294d0a8c5)), which is evidence rather than proof:
a scan says what seventy vendors thought on one day, a signature says who
built the file. This document describes the process
that is already wired into the release workflow and that takes effect the day
a certificate exists.

## What gets signed

Only the two Windows executables, `valhsync.exe` and `valhsync-server.exe`,
and only when they were built by the `Release` workflow from a `v*` tag in
[this repository](https://github.com/Ztnzvoid/ValhSync). Nothing is ever
signed from a developer's machine: there is no certificate to sign with there,
by design.

Linux archives are not Authenticode-signed — the format has no such thing.
Every release carries a `SHA256SUMS` file for both platforms.

## Roles

The project is maintained by one person, so the roles below are not a
separation of duties between people; they are a separation between what a
human does and what a machine may do on its own.

- **Author** — the maintainer, who writes the code and pushes a release tag.
- **Reviewer** — the maintainer, who reads the diff a tag contains before
  pushing it.
- **Approver** — the maintainer, who approves each signing request in
  SignPath. Signing is *not* automatic on a tag: the request waits for a human
  to look at which commit, which workflow run and which artifact it came from.

GitHub access and the SignPath account are protected by multi-factor
authentication.

If a second maintainer ever joins, Author and Approver will be split between
two people and this file will say so.

## How a signature is obtained

1. A tag `vX.Y.Z` is pushed. GitHub Actions builds the workspace with
   `--locked` from that exact commit.
2. The Windows job uploads the two unsigned executables as a workflow artifact
   and submits a signing request through
   [SignPath's GitHub action](https://github.com/SignPath/github-action-submit-signing-request).
3. SignPath verifies with GitHub, not with us, where that artifact came from:
   repository, commit, workflow file and run. A request that does not match
   the configured origin is refused before a human sees it.
4. The Approver approves it. The signed binaries come back to the same job,
   are packaged into the release archives, and the checksums are computed from
   the signed files.

The private key never leaves SignPath. Nobody on this project can extract it,
and no local build can produce a signed binary.

## What the signature means

That the file is the one this repository's release workflow built from the
tagged source, and that it has not been altered since. It is not a review of
the code, an endorsement by SignPath or by Microsoft, or a claim that the mods
a server publishes through ValhSync are safe — those come from a server admin,
and the [security model](https://github.com/Ztnzvoid/ValhSync/blob/main/SECURITY.md)
says plainly that trusting an admin is the decision a player makes.

## Privacy

ValhSync collects nothing and reports nothing to this project.

- The launcher talks to the servers the player added, and to nowhere else. It
  has no telemetry, no analytics, no crash reporting and no update ping to any
  service of ours; a newer launcher is offered by the player's own server.
- The publisher talks to the players who ask it for the pack. It contacts a
  Discord webhook only if the admin pasted one into the configuration, and
  never contacts anything else.
- Nothing is written outside the game folder on a player's machine, and
  outside its own configuration folder and the server's `BepInEx` tree on an
  admin's.

There is therefore no data collection to disclose or to switch off. If that
ever changes, it will be opt-in and this section will describe it before the
release that introduces it.

## Uninstalling

There is no installer. Delete `valhsync.exe` and the folder it sits in.

- To put a game folder back the way it was: *Go back to the previous version*
  in the launcher undoes the last sync from the backup taken before it, and
  *Play without mods* disables BepInEx without removing anything.
- To forget every server and setting: *Settings → Reset everything*. It
  touches neither the game nor the mods already installed.
- The launcher's own files live in `%APPDATA%\valhsync` (known servers and
  settings) and `%LOCALAPPDATA%\valhsync\backups` (what a rollback restores
  from); deleting those two folders removes every trace of it.

## Attribution

Free code signing provided by [SignPath.io](https://signpath.io/), certificate
by [SignPath Foundation](https://signpath.org/).
