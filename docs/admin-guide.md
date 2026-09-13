# ValhSync admin guide

> Setting a server up for the first time, on Windows or Linux, is quicker from
> the [server guide](server-guide.html). This page is the reference: every
> option, every field, and the cases the guide does not stop for.

> **Early version.** One server has actually run ValhSync: Windows, a dedicated
> server beside it, a handful of players. Linux, hosted providers and anything
> else here is tested but has not met a real server yet. Back up the server's
> `BepInEx` folder before pointing ValhSync at one that matters.
>
> **Built with AI assistance.** All of ValhSync — the two programs, the tests,
> this documentation — was written with an AI assistant (Claude), directed and
> reviewed by a human.

You run a modded Valheim server and want every player to have exactly the
right mods, in the right versions, without sending zips around. This is the
whole procedure, from an installed dedicated server to a launcher your players
can double-click.

## 1. What gets published

The pack sent to players is:

**the server's BepInEx folder, minus exclusions, plus a client-extras folder.**

- Most mods run on both sides (anything based on ServerSync or Jötunn). They
  are picked up from the server's `BepInEx/plugins`.
- Some mods run **only on the server** (DiscordConnector, admin tools). List
  them in `[pack] exclude` so players never receive them.
- Some mods run **only on clients** (Unshamed, ConfigManager, EquipmentAndQuickSlots,
  BetterArchery...). Put them in the `client-extras/` folder, laid out exactly
  like the game root:

  ```
  client-extras/
  └── BepInEx/
      └── plugins/
          └── Unshamed/
              └── Unshamed.dll
  ```

  Files here override files of the same path coming from the server.

The default include list covers `winhttp.dll`, `doorstop_config.ini`,
`.doorstop_version`, `doorstop_libs/**`, `unstripped_corlib/**` and the whole
of `BepInEx/**` — not a list of its subfolders, because which ones exist
depends on the BepInEx version (`monomod`, `unity-libs`, `interop`) and mods
drop files wherever they like inside it. A mod that ships YAML tables, texture
packs or season files under `BepInEx/config/` is published with them. Only
logs, caches and `.bak`/`.old`/`.tmp` files are excluded.

Nothing outside `BepInEx/` and the doorstop files can ever be published: the
launcher refuses any other path, whatever the manifest says.

Player preferences are not overwritten. `BepInEx/config/*.cfg` is *seeded* —
sent once, then left alone — because those are the files BepInEx generates for
keybinds and UI. Everything else under `config/` is data the admin curates and
is kept in step with the server.

## 2. Install

Download the release for the server's OS and put `valhsync-server(.exe)`
somewhere convenient, for example next to the dedicated server's `.bat`. Then:

```bash
valhsync-server init --name "My server" \
                    --game-address valheim.example.org:2456 \
                    --public-url https://you.github.io/valheim-pack
```

`init`:

1. looks for the dedicated server (Steam app 896660, then common folders);
2. writes `valhsync-server.toml`, fully commented — in your configuration
   directory (`%APPDATA%\valhsync\` on Windows, `~/.config/valhsync/`
   elsewhere), or beside the executable if a configuration is already there;
3. creates `client-extras/`;
4. generates the Ed25519 signing key under `valhsync-server-data/keys/`,
   beside that configuration file;
5. prints the invite code.

**Back up `valhsync-server-data/keys/server.key`.** Losing it means every
player has to import a new invite code. Leaking it means anyone can publish a
pack in your server's name — and, since the update channel, offer them a
replacement launcher.

It lives beside your configuration, not beside the executable, so unpacking a
new build no longer mints a new identity. If you have a key from an older
install, Settings → Invite code → **Take over another install's key** adopts
it; the one it replaces is kept, renamed, beside it.

`--public-url` is what players' launchers connect to. Without it, `init` uses
the machine's LAN address, which only works for players on your network.

## 3. Configure

Open `valhsync-server.toml`. The sections that matter:

```toml
[server]
name = "My server"                       # shown in the launcher
bind = "0.0.0.0:2456"                    # the game's port, in TCP
public_url = "http://valheim.example.org:2456"
game_address = "valheim.example.org:2456" # handed to Valheim (+connect)

[pack]
server_root = 'C:\Program Files (x86)\Steam\steamapps\common\Valheim dedicated server'
exclude = [
  "BepInEx/LogOutput.log", "BepInEx/cache/**",
  "BepInEx/plugins/DiscordConnector/**",  # server only
]
client_extras = 'C:\valhsync\client-extras'
managed_roots = ["BepInEx/plugins", "BepInEx/patchers"]

[policy]
default = "enforce"                      # always match the server
seed = ["BepInEx/config/**"]             # installed once, then the player's
enforce = ["BepInEx/config/BepInEx.cfg"] # except these

[game_server]
start_script = 'C:\valheim\start-server.bat'  # your copy, not Steam's
stop_with_game = true                    # publishing follows the game server
restart_on_crash = false                 # bring it back if it exits by itself

[limits]
max_file_mb = 512                        # refuse a single file larger than this
max_pack_mb = 4096                       # ... or a whole pack larger than this
max_files = 10000
```

Three more keys sit outside those sections:

- `language` at the top of the file — `en`, `fr`, `de`, `es`, `it`, `pl`, `pt`,
  `ru`. Unset follows the system. The window's menu writes it here, so the
  choice survives a restart.
- `[server] discord_webhook` — a Discord webhook URL. Absent by default, and
  absent means the feature is off: nothing is ever sent anywhere unless you
  paste one in. When it is set, each published pack is announced in that
  channel with what changed and whatever you wrote in the patch note. The URL
  carries a token, so it is never printed back, not into the log and not into
  an error message.
- `[pack] export_dir` — where `export` writes the folder to upload. Unset means
  `pack-site` beside the configuration.

- `managed_roots` are the folders ValhSync owns on the player's side. A DLL
  found there that the manifest does not know (a leftover of another mod
  pack) is moved to `BepInEx/_valhsync_quarantine/<date>/` before the game
  starts. Files that mods generate next to their own DLL (translations,
  caches) are left alone. `BepInEx/config` is deliberately not a managed
  root: mods create their config files at runtime.
- `seed` vs `enforce`: a seeded file is installed only when absent. Use it for
  configs that hold keybinds and UI preferences. Enforce the ones that must
  match the server (anything with gameplay values that the server checks).

Every change to the config needs a restart of `serve`. Changes to the mod
folders do not: `serve` watches them and republishes after two quiet seconds.

## 4. Publish

```bash
valhsync-server scan     # builds the manifest, prints what changed, publishes nothing to players yet
```

Then pick one of the two publishing modes.

### 4a. Static files (recommended: no port to open)

```bash
valhsync-server export ./pack-site           # once
valhsync-server export ./pack-site --watch   # keeps it current while it runs
```

`pack-site/` contains `manifest.json`, `manifest.sig` and `files/<blake3>`,
the exact layout the launcher fetches. Upload it to any place that serves files
over HTTP(S) and set `public_url` to that folder's URL (the one where
`manifest.json` ends up). Examples:

- **GitHub Pages**: a repository with the folder at its root, Pages enabled;
  `public_url = "https://you.github.io/valheim-pack"`. `export --watch` into
  the checkout plus a `git push` after changes.
- **Cloudflare Pages / Netlify**: drag-and-drop upload of the folder, free,
  HTTPS.
- **S3 / R2 / Backblaze**: `rclone sync pack-site remote:bucket`.
- **Your ISP's web space**: any FTP/SFTP sync tool.

Integrity does not depend on the host: even a compromised web space cannot
make players install something you did not sign. What it can do is serve an
old copy; the launcher refuses a manifest older than the one it last applied.
Mind mod licenses before hosting DLLs on a public site.

### 4b. Live server on your machine (no new port)

```bash
valhsync-server serve    # Ctrl+C to stop
```

`serve` listens on `bind`, rebuilds automatically when a mod changes, and
keeps the previous generation's files available for a launcher that fetched
the old manifest a moment ago. Visit `http://<host>:2456/` in a browser: it
shows the invite code and the pack summary. `/health` returns JSON for
monitoring.

ValhSync listens on **the game's own port, in TCP**. Valheim uses that port in
UDP only, so the two never collide, and the router rule you already wrote for
the game carries ValhSync too — as long as that rule says **TCP and UDP**, which
is what most router pages write by default. Check yours before relying on it;
if it is UDP-only, widening it is one checkbox, not a new port.

ValhSync will never ask you to open a port the game does not already use.

Older setups on TCP 2470 keep working: a launcher that fails on the address it
was given retries the other port once. Internet players need that rule, or an
outbound tunnel (Cloudflare Tunnel, ngrok) in front of it.

Run it as a service: see [deploy/](deploy/).

## 5. The window

Double-clicked, `valhsync-server` opens a window with five tabs. Everything
below is also a command; the window is not a second way of configuring the
same thing, it is the same configuration file.

**Server** is what is happening now: online or offline, how many players, the
crossplay join code, how long ago the world was written to disk, the public
address, and the server's log as it is written. The log is found on its own —
the `-logFile` the start script asks for, else `BepInEx/LogOutput.log`, else
Unity's `output_log.txt`. Below it sits a console that carries the lines the
server prints to its own, which are otherwise a handful among tens of
thousands, and a prompt — see **Players** below for what the prompt takes.

*Start* runs the start script in its own console window. *Stop and save* sends
that console a Ctrl+C, which is exactly what an admin types into it: Valheim
writes the world to disk and then exits. ValhSync never terminates the
process, because a killed server loses everything since the last autosave.
*Restart* waits for the old process to actually be gone before starting the
new one.

There is no way to force a save without stopping, and ValhSync does not pretend
otherwise. The dedicated server does not read its own console, whatever its
start-up banner says, and Valheim has no RCON: saving on demand and kicking
somebody who is connected right now are an admin pressing F5 in the game. What
you can set is how often it saves itself — `-saveinterval`, 1800 seconds by
default — which the start-script wizard puts on the form.

**Mods** is one row per mod found in the server's `BepInEx/plugins`. Each row
says whether that mod is *sent to players* or *server only* — admin tools and
Discord bridges have no reason to travel — and carries two more buttons:

- *Disable* moves the mod out of `BepInEx` entirely. Renaming it in place
  would not do: BepInEx loads every `.dll` under `plugins` recursively, and
  anything left inside travels to players in the pack.
- *Remove* does not delete. The mod goes to a folder ValhSync owns, and the
  window says which, so a mistake at eleven at night can be walked back.

A mod is installed by dropping it on the window — see §7.

**Players** holds the admin list, the ban list and the permitted list, written
to the three files Iron Gate documents (`adminlist.txt`, `bannedlist.txt`,
`permittedlist.txt`), which is the only channel a dedicated server has from
outside the game. Each id is shown beside the name the server's log recorded
for it, because seventeen digits are no way to decide who to ban. The same
thing is on the console prompt: `admin`, `ban`, `permit` and their `un-`
forms, plus `admins`, `banned`, `permitted` to see a list. Valheim rereads
those files by itself; a ban takes effect on the banned player's next attempt.

**Patch notes** is the note players read. Most of it writes itself: the mods
added, updated and removed are computed from the pack and named for you, so
you never retype a version number. What you add is the half a diff cannot
produce — that a mod resets its own config, that a chest mod wants an empty
base first, that this update only matters to the people who were crashing on
the boat. Saving publishes it; players see it behind *What's new* at their
next sync, and it stays readable afterwards. Nothing waits on them agreeing
to it.

**Settings** holds everything else that is written to the file: the server
folder, the identity, which mods are server-only, how the pack is published,
the address and key fingerprint to hand out, the invite code, and the wizard
below. There is no Save button — it writes itself — and the language menu is
at the top right of the window.

**Backing up the world** is on the Server tab, next to the world's own line.
Both halves of a world travel together (`.db` and `.fwl`), the copy lands
beside `worlds_local` and never inside it — Valheim offers everything in there
as a joinable world — and a copy taken while the server is up may be torn, so
the window says so before the click rather than after.

### The start-script wizard

Steam overwrites `start_headless_server.bat` on every update, so the script
you actually run has to be a copy. The wizard writes that copy, and checks the
two rules Valheim enforces before it does: a password of at least five
characters, and a server name that does not contain the password. Getting
either wrong makes the server start and quit again with little explanation.

It also sets `-saveinterval` and `-backups`, which the file Iron Gate ships
leaves out. The password is written in plain text, as it is in that file;
ValhSync never reads it back out of the script, stores it, or publishes it.

## 6. Distribute the launcher

**The usual way: send the address.** `203.0.113.10:2456` — the same `ip:port`
players already paste into Valheim — goes into "Add a server". The launcher
fetches the key and shows its fingerprint; send yours in the same message so
they can compare before accepting.

Two other routes, when you want them to type nothing:

1. Send the invite code; they paste it into "Add a server". It carries the key,
   so there is no fingerprint to compare.
2. Make a zip containing `valhsync.exe` and a text file named
   `valhsync-invite.txt` whose content is the invite code. On first start the
   launcher imports it: players see your server immediately.

The invite code contains the URL, the server name and the **public key**. It
is not secret, but it is the trust anchor: players should get it from you, not
from a random link.

## 7. Day to day

- **Adding a mod, on Windows**: drop it on the Mods tab. A `.zip` from
  Thunderstore, Nexus, a GitHub release page or anywhere else; a mod folder; or
  a bare `.dll`. ValhSync works out whether the files sit at the archive's root,
  under `plugins/` or under a whole `BepInEx/` tree, names the folder from the
  manifest when the archive carries one, and clears the previous version out
  first so BepInEx is never asked to load two. Nothing is written outside
  `BepInEx/plugins`, whatever paths the archive claims.

  Stored, deflate and deflate64 are read — which is every archive anyone has
  ever published a mod in. An archive packed with bzip2, LZMA, zstandard or xz
  is refused by the name of its compression rather than half-read: extract it
  yourself and drop the folder. (bzip2 would mean shipping a C library, and the
  pure-Rust LZMA decoder available does not read the ZIP variant of LZMA
  correctly, which is worse than not offering it.)
- **Updating a mod**: replace the files in the server's `BepInEx/plugins` (or in
  `client-extras/`). `serve` republishes within seconds. Players get the change
  at their next PLAY, downloading only what changed.
- **Turning a mod off, or taking it out**: the two buttons on its row in the
  Mods tab. Disabling moves it out of `BepInEx` entirely, because BepInEx loads
  every `.dll` under `plugins` recursively and anything left inside would
  travel to players anyway. Removing moves it to a folder ValhSync owns rather
  than deleting it. Either way the pack is rebuilt: players' launchers take the
  files they had installed out of the game, into their own backup and not the
  trash.
- **Removing a mod by hand**: delete its folder in `BepInEx/plugins`. Same
  result, no undo.
- **Writing the patch note**: the Patch notes tab. The mod-by-mod list writes
  itself; you add the part a diff cannot produce. Saving publishes it.
- **Backing up the world**: the Server tab, beside the world's own line. Both
  halves together, beside `worlds_local` and never inside it. Do it before a
  change, which is exactly when a world gets corrupted.
- **Announcing a release**: set `[server] discord_webhook` and each published
  pack is posted to that channel with what changed and what you wrote. Nothing
  is sent anywhere while that key is absent.
- **Checking who is in sync**: not in v1. The server log shows requests; the
  launcher shows the pack id on the player's side.
- **Rotating the key** (`valhsync-server rotate-key --yes`): only if the key
  leaked. Every player must import the new invite code.

## 8. Hosted servers (G-Portal, Nitrado, ...)

You cannot run `valhsync-server` on the game host. You do not need to: run it on
your own PC or a VPS with a local copy of the pack in `client_extras`, leave
`server_root` out, and set `game_address` to the hosted server. Keep that copy
in sync with what you upload to the host.

## 9. What players type

One address: the game server's. It is the one they already have, it is the one
ValhSync listens on, and the launcher takes it as it is. The invite code carries
that same address, so the two routes agree.

Valheim's six-digit **join code is not an address**. It reaches the game through
PlayFab's relay, which carries no file transfer — no launcher can fetch a pack
through it. The launcher recognises one and says so rather than trying to
resolve it as a host.

## 10. Crossplay servers and `game_address`

If your server runs with `-crossplay` (the PlayFab backend), players never
reach it by local IP. Iron Gate's manual is explicit: *"You can connect to a
Crossplay server using the public IP address and port number, a join code or
via the server list, however it's not possible to connect using a local IP
address or a loopback IP address."*

So `game_address` must be your **public** IP or a DNS name, even for players
sitting on the same LAN as the server. A `192.168.x` address produces
`Timed out attempting to connect` in the client log, with nothing at all in the
server log. `valhsync-server` prints a warning when it sees a local address
there.

Without `-crossplay` (the Steam backend), the opposite is true for LAN play: a
local address works, and internet players need UDP 2456-2457 forwarded.

## 11. Network and reverse proxies

`valhsync-server` speaks plain HTTP. Integrity does not depend on the transport
(the manifest is signed, every file is verified by digest), so HTTP is fine.
If you want HTTPS anyway, put it behind Caddy, nginx or a Cloudflare tunnel
and set `public_url` to the `https://` address. The launcher accepts both.

Ports: the game's own port in TCP for ValhSync (`bind`, configurable), UDP 2456-2457 for the game itself,
as before.

## 12. Things to say to your players

- The launcher needs Valheim to be closed while it syncs.
- Console players (PS5, Switch 2) cannot use mods; a modded server excludes
  them.
- On Linux and Steam Deck, BepInEx only loads with the right Steam launch
  options; the launcher prints them.
- Some mod authors forbid redistribution of their files. Check before you
  publish a mod from your server.
- They join by pasting your server's address — the same `ip:port` they already
  type into Valheim — and comparing the key fingerprint you announce. An invite
  code does the same job without the comparison, if you prefer to send one.
- A newer launcher is offered through the same signed channel when
  `valhsync.exe` sits beside the publisher, so they only install by hand once.
