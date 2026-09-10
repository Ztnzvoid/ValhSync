# Running valsync-server unattended

`valsync-server serve` is a foreground console program in v1. To keep it
running across reboots:

## Linux: systemd

See [valsync-server.service](valsync-server.service); the header explains the
setup. Logs go to the journal.

## Windows

Three options, from simplest to most robust.

### Task Scheduler (no extra software)

1. Task Scheduler > Create Task.
2. General: "Run whether user is logged on or not", "Run with highest
   privileges" unchecked (not needed).
3. Triggers: "At startup".
4. Actions: Start a program
   - Program: `C:\valsync\valsync-server.exe`
   - Arguments: `--config C:\valsync\valsync-server.toml serve`
   - Start in: `C:\valsync`
5. Settings: "If the task fails, restart every 1 minute".

Output is not visible; add `RUST_LOG=info` to the environment and redirect
with a small `.bat` if you want a log file:

```bat
@echo off
cd /d C:\valsync
valsync-server.exe --config valsync-server.toml serve >> valsync-server.log 2>&1
```

### NSSM (Non-Sucking Service Manager)

```
nssm install ValSync "C:\valsync\valsync-server.exe" "--config C:\valsync\valsync-server.toml serve"
nssm set ValSync AppDirectory C:\valsync
nssm set ValSync AppStdout C:\valsync\valsync-server.log
nssm set ValSync AppStderr C:\valsync\valsync-server.log
nssm set ValSync AppRotateFiles 1
nssm start ValSync
```

NSSM handles restarts and log rotation. It is the recommended option on
Windows until ValSync ships a native service.

### sc.exe

`sc.exe create` expects a program that speaks the Windows service protocol,
which `valsync-server` does not yet; use NSSM or the Task Scheduler instead.

## Firewall

Allow inbound TCP 2470 (or whatever `[server] bind` says) for
`valsync-server.exe`. Players outside the LAN also need the port forwarded on
the router, exactly like UDP 2456-2457 for the game.

## Dedicated user

On Linux the unit runs as `valsync`, a system user with no shell. On Windows,
run the task or service as a standard (non-administrator) account that can
read the dedicated server's folder and write to `C:\valsync`.
