# Security

## Reporting

Open a private security advisory on the GitHub repository, or write to the
maintainer listed in `Cargo.toml`. Please do not file public issues for
vulnerabilities before a fix is available.

## Threat model

ValSync moves executable code (BepInEx plugins, .NET DLLs) from a server
admin's machine onto players' machines and runs nothing itself. The trust
boundary is explicit:

**Trusted:** the server admin whose invite code a player imported. A plugin is
arbitrary code with the player's rights; importing an invite means trusting
that admin, exactly like accepting a zip of mods from them. ValSync cannot and
does not try to protect against a malicious admin.

**Not trusted:** everyone else. The network between player and server, anyone
who can reach the server's port, anyone who can hand a player a link or a
file, and ValSync's own bugs.

## What is guaranteed against untrusted parties

| Threat | Defense | Where |
|---|---|---|
| Manifest forged or altered in transit | Ed25519 detached signature over the exact bytes, verified **before** parsing, with the key pinned at invite import | `valsync-core::manifest::parse_verified` |
| File altered in transit or on the server | BLAKE3 digest of every file checked after download; nothing is applied until every file verified | `valsync::http::Client::download`, `engine::apply` |
| Server key silently swapped | A signature failure on a known URL is reported as a key mismatch; a new key is only accepted through an explicit `join --replace-key` | `servers::ServerBook::join`, `engine::prepare` |
| Replay of an old, validly signed manifest | The launcher remembers `generated_at` of the last applied manifest and refuses an older one unless `--allow-older` | `engine::prepare` |
| Path traversal, absolute paths, drive letters, UNC, reserved device names, trailing dots/spaces, control characters | Strict syntax check plus an allow-list of roots (`winhttp.dll`, `doorstop_config.ini`, `BepInEx/`, `unstripped_corlib/`, `doorstop_libs/`, the doorstop `.sh` launchers); one bad entry rejects the whole manifest | `valsync-core::path` (test battery) |
| Symbolic links or junctions inside the game folder | Never followed; every component is checked at plan time and again at apply time | `path::ensure_within_root` |
| Case-collision on case-insensitive filesystems | Duplicate paths differing only by case are rejected | `manifest::validate` |
| Oversized manifest or files, unbounded downloads | Manifest capped at 8 MiB; per-file, per-pack and file-count limits; downloads stop as soon as the announced size is exceeded; no automatic decompression | `Limits`, `http.rs` |
| Command injection through the game address | `game_address` restricted to `[A-Za-z0-9.-_:[]]` on both sides; Steam is started with an argv, never through a shell | `manifest::is_valid_game_address`, `game::launch` |
| Terminal escape sequences in names | Server and invite names refuse control characters | `manifest::is_clean_text` |
| Server-side path access through URLs | Files are served from a content-addressed store by 64-hex digest only; the digest must be named by the live manifest; no listing, no write endpoints | `valsync-server::serve::file`, `store::Store` |
| Half-applied sync after a crash or network loss | Everything downloaded and verified first; every change journaled with the previous bytes stashed; automatic rollback on failure; `valsync rollback` on demand | `engine::apply`, `backup::Backup` |
| Loss of the player's own data | ValSync never deletes a file it did not install; unknown files are moved to a quarantine folder, replaced files to a backup | `plan.rs`, `backup.rs` |
| Panics on hostile input | `valsync-core` denies `unwrap`/`expect`/`panic`; `unsafe` is denied workspace-wide except one documented `AttachConsole` call | `Cargo.toml` lints |
| Supply chain | `Cargo.lock` committed, `cargo deny` (advisories, licenses, sources) in CI, rustls with bundled roots, no default features on `reqwest`/`axum` | `deny.toml`, CI |

## Known limitations

- **The admin is trusted.** A malicious admin can ship a malicious DLL, or a
  `doorstop_config.ini` that points at any assembly. This is inherent to
  distributing mods and is stated in the player documentation.
- **No transport encryption by default.** The server speaks plain HTTP.
  Confidentiality of the pack is not a goal (mod files are public). Integrity
  does not depend on the transport. Admins who want HTTPS can front the server
  with a reverse proxy; the launcher accepts `https://` URLs and verifies
  certificates with bundled roots.
- **Availability.** `valsync-server` has no rate limiting or authentication.
  Anyone who can reach the port can download the pack. Run it on a dedicated
  low-privilege user and, if exposure worries you, restrict the port to known
  players at the firewall.
- **Local attackers** with write access to the player's game folder or profile
  can alter state files or race the launcher. ValSync does not defend against
  a compromised local account.
- **DLL search order on Windows.** Like any Windows program, `valsync.exe`
  should not be run from a folder where untrusted parties can drop DLLs.
- **`valsync-invite.txt` auto-import** adds a server without a prompt. It can
  never replace an already pinned key, and the first sync with a new server
  always shows the plan and asks for confirmation.
- **The server's signing key** is a plain file (`valsync-server-data/keys/server.key`,
  mode 0600 on Unix). Back it up; treat it like a password.

## Hardening checklist for admins

1. Run `valsync-server` as a dedicated non-administrator user (systemd unit
   provided in `docs/deploy/`).
2. Give it read-only access to the game server's folder.
3. Back up `server.key` offline; rotate it (`rotate-key --yes`) if it leaks.
4. Set `public_url` explicitly; do not rely on LAN detection for internet
   players.
5. Review `valsync-server scan` output before the first `serve`, and again
   after adding a mod.
6. Distribute invite codes and the launcher through a channel your players
   already trust.
