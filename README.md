# ValhSync

**The server is the source of truth for a Valheim mod pack.** The admin runs
`valhsync-server` next to the dedicated server; players run `valhsync`, press
**Play**, and their BepInEx folder is brought in line with the server's before
the game starts. No more "Incompatible version", no more zips of DLLs sent by
hand after every mod update.

> Version française plus bas : [En français](#en-français).

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

Full details: [docs/admin-guide.md](docs/admin-guide.md). Running it as a
service: [docs/deploy](docs/deploy).

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

More: [docs/player-guide.md](docs/player-guide.md).

## Security model, honestly

A BepInEx plugin is arbitrary .NET code running with the player's rights.
Joining a ValhSync server means trusting its admin, exactly like accepting their
zip of mods. ValhSync cannot protect against a malicious admin. It does protect
against everything else: tampering in transit (signature + digests), a swapped
server key (pinned key, explicit re-import required), path tricks in a manifest
(`../`, drive letters, UNC, reserved names, symlinks: the whole manifest is
rejected), oversized packs (size and count limits), and its own bugs (journaled
backups, rollback). See [§8 of the spec](docs/cahier-des-charges.html).

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
executable). Dependencies are permissive-licensed only; `cargo deny` refuses
copyleft, which also enforces the "no code from GPL tools" rule.

## Redistributing mods

Serving DLLs from your own server is common between friends, but some mod
authors forbid redistribution. Check the licenses of what you publish.

## License

MIT OR Apache-2.0, at your option. Written from scratch; not a fork of any
existing mod manager.

The windows embed the **Cinzel** typeface by Natanael Gama, under the SIL Open
Font License 1.1. Its licence travels with the source in
`crates/valhsync-ui/assets/OFL-Cinzel.txt` and with every release package.

---

## En français

ValhSync synchronise les mods d'un serveur Valheim avec ceux des joueurs. Le
serveur publie un manifeste signé de son pack BepInEx ; le launcher du joueur
compare, télécharge ce qui manque, met en quarantaine ce qui n'a rien à faire
là, puis lance le jeu via Steam. Plus de « Incompatible version », plus de zip
de DLL à renvoyer à chaque mise à jour.

**Rien à ouvrir, rien à installer.** Le joueur lance un exécutable, sans
installation ni droits admin, et ne fait que des connexions sortantes : sa box
n'est jamais touchée. L'admin non plus n'a pas de port à ouvrir : la voie
recommandée est `valhsync-server export`, qui produit des fichiers statiques à
déposer sur n'importe quel espace web (GitHub Pages, S3, l'hébergement de ton
FAI) ; la signature voyage avec les fichiers, l'hébergeur n'a aucune prise sur
l'intégrité. `serve` sur ta machine reste l'option LAN/avancée : il écoute sur
le port du jeu, en TCP, donc aucun nouveau port à ouvrir.

**Côté admin** : `valhsync-server init` détecte le serveur dédié installé par
Steam, écrit une configuration commentée, génère la clé de signature et affiche
le code d'invitation. `valhsync-server export <dossier>` (ou `export --watch`
pour le tenir à jour tout seul) produit le pack à uploader ; `serve` le publie
en direct et le reconstruit quand un mod change. Les mods serveur seuls vont dans `exclude`, les
mods client seuls dans le dossier `client-extras/`. Un serveur hébergé en ligne
(G-Portal, Nitrado…) fonctionne aussi : le publieur tourne où tu veux avec une
copie du pack, seul `game_address` pointe vers l'hébergeur. Ouvre le port TCP
le port du jeu. Guide complet : [docs/admin-guide.md](docs/admin-guide.md).

**Côté joueur** : double-clic sur `valhsync.exe`, coller le code (ou avoir le
fichier `valhsync-invite.txt` à côté de l'exe), bouton **JOUER**. La première
fois, le launcher montre ce qu'il va faire et demande confirmation. Ensuite,
JOUER synchronise et lance. « Revenir à la version précédente » annule
exactement la dernière synchro ; « Jouer sans mods » désactive BepInEx sans
rien supprimer. Linux et Steam Deck : la synchro est identique, mais BepInEx ne
se charge que si les options de lancement Steam sont réglées ; le launcher
l'indique, il ne modifie pas Steam. Les consoles ne chargent aucun mod.

**Sécurité** : rejoindre un serveur, c'est faire confiance à son admin, comme
en acceptant son zip. Contre tout le reste (altération en transit, clé
changée, chemins piégés, packs démesurés, bugs du launcher), ValhSync se
protège : signature Ed25519, hachage BLAKE3 de chaque fichier, clé épinglée,
rejet du manifeste entier au moindre chemin douteux, limites de taille,
sauvegardes journalisées et retour arrière.

Reste à vérifier sur le vrai serveur : les arguments exacts de lancement
acceptés par Valheim 1.0 via Steam, et le contenu exact du pack BepInEx
5.4.2350 à la racine du jeu. Le TLS côté serveur n'est pas intégré (reverse
proxy si besoin ; l'intégrité ne dépend pas du transport).
