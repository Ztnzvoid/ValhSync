# ValhSync

**One button. Your mods match the server's, then Valheim starts.**

Paste the address of the server you already play on — the same `ip:port` you
type into Valheim — press **PLAY**, and the launcher brings your `BepInEx`
folder in line with that server's before the game opens. Only what changed is
downloaded. Nothing you put in that folder yourself is ever deleted.

## Read this first — this is a very early version

**0.0.3 is the third release and the first one published anywhere.** One
server has actually run it: a Windows machine, a dedicated server beside it, a
handful of players. Everything else — Linux, hosted providers, Proton, any
setup that is not that one — is tested but has never met a real server.

**Expect bugs.** You are among the first people to run this. Back up your
`BepInEx` folder before pointing it at anything you care about, and please
report what breaks: finding things is what this stage is for.

> **Everything happens on the GitHub repository, not here:**
> **https://github.com/Ztnzvoid/ValhSync**
>
> Issues, releases, the full documentation, and the source. Thunderstore is a
> mirror for convenience; the repository is the project.

ValhSync was built with the help of an AI assistant (Claude), directed and
reviewed by a human. Said plainly, so you know what you are running.

## This is a program, not a plugin

ValhSync is a standalone `.exe`. It is **not** a BepInEx plugin and it does not
load into the game — it is the thing that installs the plugins for you.

If you installed this through a mod manager, it did not "enable" anything:
find **`valhsync.exe`** in the profile folder the manager just wrote to, and
run it. Copying it somewhere you can reach — your desktop — works just as
well, and is what most people should do.

You still need BepInEx in your Valheim install; the server's pack normally
carries it.

## What it does

- **Adds a server from its address.** The launcher fetches that server's
  signing key and shows you its fingerprint. Compare it with the one your
  admin announced, accept once, and from then on nothing unsigned by that key
  is ever installed. An invite code (`valhsync1:…`) does the same job.
- **Installs only what changed**, verifies every file against a signature and
  a hash before writing a byte, and applies nothing half-way: a crash
  mid-sync leaves a working game folder.
- **Never deletes your things.** Replaced files go to a backup. Mods you
  installed yourself that are not in the server's pack are set aside inside
  the game folder, not removed. The last sync can be rolled back exactly.
- **Shows what the admin wrote** behind a *What's new* button — the mods
  added, updated and removed by name, plus whatever they wanted to say about
  the release.
- **Play without mods** disables BepInEx by renaming one file, and the next
  sync puts it back.
- Speaks English, French, German, Spanish, Italian, Polish, Portuguese and
  Russian.

## Windows will warn you

ValhSync is not signed with a code-signing certificate yet, so Windows shows
*"Windows protected your PC"* on first run: **More info → Run anyway**. A
certificate has been applied for. Until it exists, the real check is the
SHA-256 published with every release on GitHub — compare it with
`Get-FileHash valhsync.exe -Algorithm SHA256`.

## Running a server?

The admin's half — starting and stopping the dedicated server, installing mods
by dropping them on a window, admin and ban lists, patch notes, world backups,
publishing the pack signed — is in the full download on GitHub. Same page:
**https://github.com/Ztnzvoid/ValhSync**

## Licence

MIT OR Apache-2.0. Mods themselves belong to their own authors: some forbid
redistribution, so check before publishing a pack from your server.
