# ValhSync

**The server is the source of truth for a Valheim mod pack.** The admin runs
`valhsync-server` next to the dedicated server; players run `valhsync`, press
**Play**, and their BepInEx folder is brought in line with the server's before
the game starts. No more "Incompatible version", no more zips of DLLs sent by
hand after every mod update.

> **Early version.** 0.0.3 is the third release there has ever been. One
> server has actually run it — Windows, a dedicated server beside it, a handful
> of players — and the whole chain works there. Linux, hosted providers, Proton
> and anything that is not that setup are tested but have not met a real server
> yet. Back up the server's `BepInEx` folder before pointing ValhSync at one
> that matters, and open an issue when something breaks: that is what this
> stage is for.

```
[Warning:AzuCraftyBoxes] Peer (Steam_7656119xxxxxxxxxx) never sent version
                         or couldn't due to previous disconnect, disconnecting
```

That log line is why this exists.

## How it works

```
 Server machine                                     Player's PC
 ┌──────────────────────────────┐                  ┌────────────────────────────────┐
 │ Valheim Dedicated Server     │                  │ valhsync (launcher)             │
 │  └ BepInEx/ (server mods)    │                  │  1. GET /manifest.json + .sig  │
 │                              │   HTTP over TCP  │  2. verify Ed25519 signature   │
 │ valhsync-server               │ ◀──────────────▶ │  3. compare with local hashes  │
 │  ├ scans pack + client extras│                  │  4. download /files/<blake3>   │
 │  ├ signs the manifest        │                  │  5. apply atomically, journaled│
 │  └ content-addressed store   │                  │  6. start Valheim via Steam    │
 └──────────────────────────────┘                  └────────────────────────────────┘
```

- **Signed manifest.** The server publishes a JSON manifest and a detached
  Ed25519 signature. The launcher pins the server's public key when the invite
  code is imported and verifies the signature *before* parsing anything.
- **Content-addressed files.** Every file is fetched by its BLAKE3 digest and
  verified after download. A URL can only ever name a hash, never a path.
- **Nothing is ever lost.** Files the launcher replaces or removes go to a
  backup; unknown DLLs found in the mod folders go to a quarantine folder
  inside the game directory. `valhsync rollback` undoes the last sync exactly.
- **Download everything first, then apply.** A network failure mid-sync leaves
  the installation untouched. A failure while applying replays the journal
  backwards on the spot.
- **Configs are seeded, not imposed.** `BepInEx/config/*` is installed when
  absent and then left alone, so keybinds survive syncs (the admin can enforce
  specific files).

The launcher writes only inside the game folder (BepInEx paths, `winhttp.dll`,
`doorstop_config.ini`), plus its own state under the user profile. It never
executes a downloaded file; it asks Steam to start Valheim.

Full documentation, in one page:
**[docs/index.html](docs/index.html)** — what it does, how it works, what is
guaranteed, what every dependency is licensed under, and what has actually
been run.

## Status

| Milestone | State |
|---|---|
| M0 workspace, CI (fmt, clippy `-D warnings`, tests on Windows + Linux, cargo-deny) | done |
| M1 `valhsync-core`: manifest, path validation, hashing, signatures, scan, plan | done, 37 unit tests incl. a malicious-path battery |
| M2 `valhsync-server`: config, keys, store, HTTP API, folder watcher | done |
| M3 launcher CLI: join, status, sync, play, rollback, vanilla, doctor | done, end-to-end test against a real server router |
| M4 Steam/Valheim discovery, process detection, launch | done; `+connect` on Valheim 1.0 **to verify on a real server** |
| M5 egui window, Valheim palette, FR/EN | done |
| M6 release workflow, docs | done; first tagged release pending |

Remaining limitation: server-side TLS is not built in; put the server behind a
reverse proxy if you want HTTPS. Integrity does not depend on it, the manifest
is signed.

Verified end to end on a real dedicated server (Valheim 1.0.7,
BepInExPack_Valheim 5.4.2350): the launcher synced a vanilla install, started
the game through `steam -applaunch 892970 +connect <host>:2456`, and the player
joined with the server's mods loaded (`Network version check, their:39,
mine:39`).

**Crossplay servers take the public address, never a local one.** A server
started with `-crossplay` relays everything through PlayFab: `+connect` with a
`192.168.x` address fails with `Timed out attempting to connect` even on the
same LAN, while the public address resolves the lobby and connects
(`Connecting to server with PlayFab-backend`). Iron Gate's own manual says it:
"it's not possible to connect using a local IP address or a loopback IP
address". `valhsync-server` warns when `game_address` is a local address.

Also verified: the pack drops `.doorstop_version`, `doorstop_config.ini`,
`winhttp.dll`, `doorstop_libs/`, `start_game_bepinex.sh`,
`start_server_bepinex.sh`, `changelog.txt` and `BepInEx/` at the root, no
`unstripped_corlib/`; the default include list covers what players need.
Scanning works while `valheim_server.exe` is running. The game logs its
version as `Valheim version: 1.0.7 (network version 39)` and the handshake as
`Network version check, their:39, mine:39`, which a later version can use to
warn about a game-version mismatch.

## Nothing to open, nothing to install

- **Players** run one executable, no installer, no administrator rights, and
  make outbound HTTP requests only. Their router and firewall are never
  touched; the game connects to the server exactly as it did before.
- **Admins** do not need an open port either. The recommended way to publish
  is `valhsync-server export`: it writes the pack as plain files that any web
  space serves (GitHub Pages, S3/R2, Cloudflare Pages, your host's FTP). The
  signature travels with the files, so the host is irrelevant to integrity.
  Running `valhsync-server serve` on your own machine (the game's port, in TCP) is the LAN and
  advanced option, not the default.

## Quick start: admin

Run `valhsync-server` with no arguments and you get a window: four tabs, and
everything below available from it.

- **Server** — start, restart and stop the dedicated server (stopping sends
  Ctrl+C to its console, so Valheim writes the world before it exits; the
  process is never killed), follow its log, read players online, join code and
  version. Starting also fills in your public address if what is in the field
  cannot work, and brings publishing online behind it. Publishing follows the
  game server from then on, however it was started.
- **Mods** — one row per mod in the server's BepInEx folder, each either *sent
  to players* or *server only*. Admin tools and DiscordConnector belong in the
  second; there is no reason to push them down everyone's connection. Drop a
  `.zip` from anywhere, a mod folder or a bare `.dll` on the window to install
  one; an update replaces the version that was there. Turn one off from its own
  row, or take it out — removing moves it to a folder ValhSync owns rather than
  deleting it, and the window says which.
- **World backups** — a copy of the world beside it, taken before a change.
  ValhSync protected every file it put on a player's machine and nothing on
  yours, which is the wrong way round: the world is the one thing that cannot
  be downloaded again.
- **Players** — admins, bans and the permitted list, with Iron Gate's own
  warning about that last one on the card. The same from the console's prompt:
  `ban <id>`, `unban`, `admin`, `permit`, `banned`. These write the three files
  Iron Gate documents, which is the only channel a dedicated server has from
  outside the game: it does not read its console, whatever its start-up banner
  says, and Valheim has no RCON. Kicking somebody connected right now, and
  saving on demand, are an admin pressing F5 in the game — typing those here
  says so rather than failing quietly.
- **Patch notes** — published signed with the pack. What was added, updated and
  removed writes itself; you add why it matters. Players read it behind a
  button and keep it afterwards, with a history. Optionally posted to a Discord
  webhook as well, once per pack that is genuinely new.
- **Settings** — where the dedicated server lives, the name and address players
  see, publishing (a static folder you upload, or the live server), and the
  invite code. Nothing to save: what is on screen is what the server publishes.

The command line does the same things and is what a service unit runs:

```bash
valhsync-server init --name "My server" --game-address valheim.example.org:2456 --public-url https://you.github.io/valheim-pack
```

`init` finds the dedicated server (Steam app 896660), writes a commented
`valhsync-server.toml`, generates the signing key and prints the **invite code**.
Then:

```bash
valhsync-server scan                    # review what would be published
valhsync-server export ./pack-site      # static files: manifest.json, manifest.sig, files/<hash>
```

Upload `pack-site/` to the web space `public_url` points at. `export --watch`
keeps the folder current whenever a mod changes, so pair it with whatever
already uploads for you (rclone, a git push, the Nextcloud client). Prefer a
live server on your machine? `valhsync-server serve` does the same over TCP
the game's own port in TCP, with automatic rebuilds. Valheim uses that port in
UDP only, so a router rule covering TCP+UDP — which is the usual shape — carries
both. ValhSync never asks for a port the game does not already use.

Hand players the invite code, or better, a zip of `valhsync.exe` plus a
`valhsync-invite.txt` containing the code: the launcher imports it on first
start.

Server-only mods (DiscordConnector...) go in `[pack] exclude`; client-only mods
(Unshamed, ConfigManager...) go in the `client-extras/` folder laid out like the
game root. **No local dedicated server** (G-Portal, Nitrado...)? Leave
`server_root` out and keep a copy of the pack in `client_extras`: the
publisher can run anywhere, only `game_address` has to point at the game host.

Setting up a server on Windows or Linux, start to finish:
**[docs/server-guide.html](docs/server-guide.html)**. Reference for every
option: [docs/admin-guide.md](docs/admin-guide.md). Running it unattended:
[docs/deploy](docs/deploy).

### Updating the launcher your players run

If `valhsync.exe` sits beside `valhsync-server.exe` — which it does when you
unpack a release archive — the publisher offers that build to launchers as a
signed document naming it by digest. A player on an older build sees it, with
your server's name and key fingerprint beside it, and one click replaces their
launcher and restarts it.

Read that trade before relying on it: it means the bytes in that one file reach
every player who accepts. The publisher signs them with the key they have
pinned; it cannot check where the file came from. It is spelled out in
[SECURITY.md](SECURITY.md). Players can always decline and fetch a release from
this repository instead.

## Quick start: player

Double-click `valhsync.exe`, paste the invite code (or have
`valhsync-invite.txt` next to the executable), press **PLAY**. The first time,
the launcher shows what it is about to install, replace or quarantine and asks
for confirmation. After that, PLAY syncs silently and starts the game.

From a terminal the same executable is a CLI:

```
valhsync join <code>        valhsync status         valhsync sync [--yes]
valhsync play               valhsync rollback       valhsync vanilla on|off
valhsync game-root <dir>    valhsync servers        valhsync doctor
```

Linux and Steam Deck: the sync works the same; BepInEx itself only loads if
Valheim's Steam launch options are set (`WINEDLLOVERRIDES="winhttp=n,b"
%command%` under Proton, `./start_game_bepinex.sh %command%` for the native
build). The launcher detects the case and tells you; it does not edit Steam's
settings. Consoles cannot load mods at all: a modded server excludes PS5 and
Switch 2 players.

More: [docs/player-guide.html](docs/player-guide.html).

## Security model, honestly

A BepInEx plugin is arbitrary .NET code running with the player's rights.
Joining a ValhSync server means trusting its admin, exactly like accepting their
zip of mods. ValhSync cannot protect against a malicious admin. It does protect
against everything else: tampering in transit (signature + digests), a swapped
server key (pinned key, explicit re-import required), path tricks in a manifest
(`../`, drive letters, UNC, reserved names, symlinks: the whole manifest is
rejected), oversized packs (size and count limits), and its own bugs (journaled
backups, rollback). The whole of it is in
[SECURITY.md](SECURITY.md).

## Languages

Both windows speak English, Français, Deutsch, Español, Italiano, Polski,
Português and Русский, chosen from a menu in the header and remembered.
English is the default and the fallback for anything else; the language the
system asks for is offered first.

Every string in both windows goes through one table per window, so a missing
translation is a build error rather than a sentence in the wrong language:
[`crates/valhsync/src/gui/i18n.rs`](crates/valhsync/src/gui/i18n.rs) and
[`crates/valhsync-server/src/gui/i18n.rs`](crates/valhsync-server/src/gui/i18n.rs).

Translations other than English and French were written to be idiomatic rather
than literal and have not been reviewed by native speakers — corrections are
very welcome, and each is one row. The places most likely to read oddly are
the strings that follow a numeral, where Polish and Russian inflect and the
table currently carries one form.

The documentation is English only. The software is not.

## What has been tested

Narrower than the code's reach, and worth saying so.

| | |
|---|---|
| **Windows, x86_64** | Verified end to end: a PC running the Valheim dedicated server with the publisher beside it, and several players who synced from it on their own machines and joined. |
| **Linux, x86_64** | Builds, and the whole suite passes in CI on every commit — including end-to-end tests that stand a real publisher on a socket and drive the launcher against it. No window has ever been opened on Linux, and no game started. |
| Proton, Steam Deck, macOS | Untested. |
| A dedicated server in Docker | Untested, and partly out of reach by design: the publisher finds the game server among processes, so it cannot see one in another container. Publishing from a mounted volume should work; starting, stopping and the console will not. |
| Hosted servers | The layout has a test; no real provider has been on the other end. |

If you try one of these, an issue saying what happened is worth more than any
amount of reasoning from here.

## Building

Rust 1.88+ (`rust-toolchain.toml` pins 1.95 for development).

```bash
cargo build --release          # target/release/valhsync(.exe), valhsync-server(.exe)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Linux needs the usual winit/egui build packages (`libxkbcommon-dev
libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
libgl1-mesa-dev`).

Workspace layout: `crates/valhsync-core` (pure logic, no network, where the
tests live), `crates/valhsync-server`, `crates/valhsync` (CLI + window in one
executable). Dependencies are permissively licensed; `cargo deny` enforces the allow-list
in `deny.toml` — MIT, Apache-2.0, BSD, ISC, Unicode and MPL-2.0, the last of
which covers one file-scoped-copyleft crate reached through `directories`. No
GPL, AGPL or unlicensed code.

### Release archives

```powershell
pwsh scripts/package.ps1        # dist/*.zip and dist/*.zip.sha256
```

Two archives, because two different people receive them:

| Archive | For | Holds |
| --- | --- | --- |
| `valhsync-<version>-<target>` | the admin | both programs, both guides, the threat model |
| `valhsync-launcher-<version>-<target>` | players | the launcher, the player guide, the licences |

An admin forwards the second one and nothing else. Both are built from a
scratch folder holding only what the script copies in, so no configuration, no
signing key and no server address ever travels with them — and the build
remaps its source paths, so the binaries do not carry the name of whoever
built them. Tagging `v*` makes CI produce the same four files, plus the Linux
tarballs and a combined `SHA256SUMS`.

## Redistributing mods

Serving DLLs from your own server is common between friends, but some mod
authors forbid redistribution. Check the licenses of what you publish.

## License

MIT OR Apache-2.0, at your option. Written from scratch; not a fork of any
existing mod manager.

The windows embed the **Cinzel** typeface by Natanael Gama, under the SIL Open
Font License 1.1. Its licence travels with the source in
`crates/valhsync-ui/assets/OFL-Cinzel.txt` and with every release package.
The body text is **Source Serif 4** by Frank Grießhammer, under the same
licence (`crates/valhsync-ui/assets/OFL-SourceSerif.txt`).
