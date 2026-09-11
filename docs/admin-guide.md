# ValSync admin guide

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
`.doorstop_version`, `BepInEx/{core,plugins,patchers,config}/**` and
`unstripped_corlib/**`. Logs and caches are excluded. Nothing outside
`BepInEx/` and the doorstop files can ever be published: the launcher refuses
any other path, whatever the manifest says.

## 2. Install

Download the release for the server's OS and put `valsync-server(.exe)`
somewhere convenient, for example next to the dedicated server's `.bat`. Then:

```bash
valsync-server init --name "My server" \
                    --game-address valheim.example.org:2456 \
                    --public-url http://valheim.example.org:2470
```

`init`:

1. looks for the dedicated server (Steam app 896660, then common folders);
2. writes `valsync-server.toml` next to itself, fully commented;
3. creates `client-extras/`;
4. generates the Ed25519 signing key under `valsync-server-data/keys/`;
5. prints the invite code.

**Back up `valsync-server-data/keys/server.key`.** Losing it means every
player has to import a new invite code. Leaking it means anyone can publish a
pack in your server's name.

`--public-url` is what players' launchers connect to. Without it, `init` uses
the machine's LAN address, which only works for players on your network.

## 3. Configure

Open `valsync-server.toml`. The sections that matter:

```toml
[server]
name = "My server"                       # shown in the launcher
bind = "0.0.0.0:2470"                    # open this TCP port
public_url = "http://valheim.example.org:2470"
game_address = "valheim.example.org:2456" # handed to Valheim (+connect)

[pack]
server_root = 'C:\Program Files (x86)\Steam\steamapps\common\Valheim dedicated server'
exclude = [
  "BepInEx/LogOutput.log", "BepInEx/cache/**",
  "BepInEx/plugins/DiscordConnector/**",  # server only
]
client_extras = 'C:\valsync\client-extras'
managed_roots = ["BepInEx/plugins", "BepInEx/patchers"]

[policy]
default = "enforce"                      # always match the server
seed = ["BepInEx/config/**"]             # installed once, then the player's
enforce = ["BepInEx/config/BepInEx.cfg"] # except these
```

- `managed_roots` are the folders ValSync owns on the player's side. A DLL
  found there that the manifest does not know (a leftover of another mod
  pack) is moved to `BepInEx/_valsync_quarantine/<date>/` before the game
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
valsync-server scan     # builds the manifest, prints what changed, publishes nothing new to players yet
valsync-server serve    # serves it; Ctrl+C to stop
```

`serve` listens on `bind`, logs each rebuild, and keeps the previous
generation's files available for a launcher that fetched the old manifest a
moment ago. Visit `http://<host>:2470/` in a browser: it shows the invite code
and the pack summary. `/health` returns JSON for monitoring.

Run it as a service: see [deploy/](deploy/).

## 5. Distribute the launcher

Two options, the second is friendlier:

1. Send players the invite code; they paste it into "Add a server".
2. Make a zip containing `valsync.exe` and a text file named
   `valsync-invite.txt` whose content is the invite code. On first start the
   launcher imports it: players see your server immediately.

The invite code contains the URL, the server name and the **public key**. It
is not secret, but it is the trust anchor: players should get it from you, not
from a random link.

## 6. Day to day

- **Updating a mod**: replace the files in the server's `BepInEx/plugins` (or in
  `client-extras/`). `serve` republishes within seconds. Players get the change
  at their next PLAY, downloading only what changed.
- **Removing a mod**: delete its folder. Players' launchers remove the files
  they had installed (they go to the player's backup, not the trash).
- **Checking who is in sync**: not in v1. The server log shows requests; the
  launcher shows the pack id on the player's side.
- **Rotating the key** (`valsync-server rotate-key --yes`): only if the key
  leaked. Every player must import the new invite code.

## 7. Hosted servers (G-Portal, Nitrado, ...)

You cannot run `valsync-server` on the game host. You do not need to: run it on
your own PC or a VPS with a local copy of the pack in `client_extras`, leave
`server_root` out, and set `game_address` to the hosted server. Keep that copy
in sync with what you upload to the host.

## 7b. No port to open: static hosting or a tunnel

The launcher only ever downloads files. Two ways to publish without touching
the router:

**Static hosting.** `valsync-server export <folder>` writes the pack as plain
files in the exact layout the launcher expects (`manifest.json`,
`manifest.sig`, `files/<blake3>`). Upload that folder anywhere that serves
files over HTTP(S): GitHub Pages, S3/R2, the web space of your ISP, a
Nextcloud public folder. Set `public_url` to the URL of that folder and hand
out the invite. Integrity does not depend on the host: the manifest is signed
and every file is verified by digest, so even a compromised web space cannot
make players install something you did not sign. Re-run `export` (and upload)
after each mod change; `serve` is not needed at all in this mode. Mind mod
licenses before hosting DLLs on a public site.

**Outbound tunnel.** Keep `serve` running locally and expose it through
Cloudflare Tunnel, ngrok or similar: the tunnel opens an outbound connection,
so no inbound port is needed. Set `public_url` to the tunnel's `https://` URL.

**Reuse the game's rule.** If your router forwards 2456 as "TCP+UDP" (many do
by default), bind ValSync on TCP 2456: `bind = "0.0.0.0:2456"`. Valheim only
uses UDP on that port, so the two do not collide. Check the rule before
relying on it.

## 8. Network and reverse proxies

`valsync-server` speaks plain HTTP. Integrity does not depend on the transport
(the manifest is signed, every file is verified by digest), so HTTP is fine.
If you want HTTPS anyway, put it behind Caddy, nginx or a Cloudflare tunnel
and set `public_url` to the `https://` address. The launcher accepts both.

Ports: TCP 2470 for ValSync (configurable), UDP 2456-2457 for the game itself,
as before.

## 9. Things to say to your players

- The launcher needs Valheim to be closed while it syncs.
- Console players (PS5, Switch 2) cannot use mods; a modded server excludes
  them.
- On Linux and Steam Deck, BepInEx only loads with the right Steam launch
  options; the launcher prints them.
- Some mod authors forbid redistribution of their files. Check before you
  publish a mod from your server.
