//! Command-line interface. Same engine as the window; this is what tests and
//! admins use, and what players fall back to when something needs a look.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use valhsync_core::limits::human_bytes;
use valhsync_core::{Action, Invite};

use crate::backup::Backup;
use crate::engine::{self, Context, Event, Prepared, Progress};
use crate::error::SyncError;
use crate::servers::{JoinOutcome, ServerBook};
use crate::{game, invite_file, vanilla};

#[derive(Parser, Debug)]
#[command(
    name = "valhsync",
    version,
    about = "Syncs your Valheim mods with a server, then starts the game"
)]
pub struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Add a server from its invite code, a file containing one, or simply
    /// its address (`valheim.example.org` or `1.2.3.4:2470`).
    Join {
        code: String,
        /// Accept the fingerprint the server presents without asking.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Accept a new key for a server already known under this URL.
        #[arg(long)]
        replace_key: bool,
    },
    /// List known servers.
    Servers,
    /// Forget a server.
    Remove { server: String },
    /// Choose the server used when none is named.
    Default { server: String },
    /// Show what a sync would do, without changing anything.
    Status {
        server: Option<String>,
        /// Accept a manifest older than the last one applied (admin rolled back).
        #[arg(long)]
        allow_older: bool,
    },
    /// Download and apply the server's pack.
    Sync {
        server: Option<String>,
        /// Skip the confirmation asked on a first sync.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Accept a manifest older than the last one applied (admin rolled back).
        #[arg(long)]
        allow_older: bool,
    },
    /// Sync, then start Valheim through Steam and connect to the server.
    Play {
        server: Option<String>,
        #[arg(long, short = 'y')]
        yes: bool,
        /// Start the game without syncing first.
        #[arg(long)]
        no_sync: bool,
        /// Accept a manifest older than the last one applied (admin rolled back).
        #[arg(long)]
        allow_older: bool,
    },
    /// Undo the last sync exactly.
    Rollback {
        /// Only list available backups.
        #[arg(long)]
        list: bool,
    },
    /// Play without mods: disable or re-enable BepInEx.
    Vanilla { mode: VanillaMode },
    /// Show or set the Valheim folder.
    GameRoot {
        path: Option<PathBuf>,
        /// Go back to automatic detection.
        #[arg(long)]
        clear: bool,
    },
    /// Print what ValhSync knows about this machine.
    Doctor,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum VanillaMode {
    On,
    Off,
    Status,
}

/// Entry point used by `main`.
pub fn run() -> Result<()> {
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_target(false)
            .init();
    }
    let cli = Cli::parse();
    let mut ctx = Context::discover()?;

    match invite_file::import_if_present(&ctx.paths) {
        Ok(Some((server, _))) => println!(
            "Server \"{}\" imported from {}.",
            server.name,
            crate::INVITE_FILE_NAME
        ),
        Ok(None) => {}
        Err(e) => eprintln!("warning: {}: {e}", crate::INVITE_FILE_NAME),
    }

    match cli.cmd {
        Cmd::Join {
            code,
            yes,
            replace_key,
        } => cmd_join(&ctx, &code, yes, replace_key),
        Cmd::Servers => cmd_servers(&ctx),
        Cmd::Remove { server } => {
            let mut book = ServerBook::load(&ctx.paths)?;
            let removed = book.remove(&server)?;
            book.save(&ctx.paths)?;
            println!(
                "Removed \"{}\". Its files stay installed until the next sync with another server.",
                removed.name
            );
            Ok(())
        }
        Cmd::Default { server } => {
            let mut book = ServerBook::load(&ctx.paths)?;
            let id = book.resolve(Some(&server))?.id.clone();
            book.set_default(&id);
            book.save(&ctx.paths)?;
            println!("Default server set.");
            Ok(())
        }
        Cmd::Status {
            server,
            allow_older,
        } => {
            ctx.allow_older = allow_older;
            let prepared = prepare(&ctx, server.as_deref())?;
            print_plan(&prepared);
            Ok(())
        }
        Cmd::Sync {
            server,
            yes,
            allow_older,
        } => {
            ctx.allow_older = allow_older;
            let prepared = prepare(&ctx, server.as_deref())?;
            sync(&ctx, &prepared, yes)?;
            Ok(())
        }
        Cmd::Play {
            server,
            yes,
            no_sync,
            allow_older,
        } => {
            ctx.allow_older = allow_older;
            cmd_play(&ctx, server.as_deref(), yes, no_sync)
        }
        Cmd::Rollback { list } => cmd_rollback(&ctx, list),
        Cmd::Vanilla { mode } => cmd_vanilla(&ctx, mode),
        Cmd::GameRoot { path, clear } => cmd_game_root(ctx, path, clear),
        Cmd::Doctor => cmd_doctor(&ctx),
    }
}

fn cmd_join(ctx: &Context, code: &str, yes: bool, replace_key: bool) -> Result<()> {
    let text = if std::path::Path::new(code).is_file() {
        std::fs::read_to_string(code).with_context(|| format!("cannot read {code}"))?
    } else {
        code.to_string()
    };
    let invite = if text.contains(valhsync_core::invite::PREFIX) {
        Invite::parse(&text)?
    } else {
        // An address: the server tells us its key, and the player confirms
        // the fingerprint against what the admin announced.
        let found = engine::discover(ctx, text.trim())?;
        println!("Server \"{}\" at {}", found.invite.name, found.invite.url);
        println!("  {} files in the pack", found.files);
        println!("  Key fingerprint: {}", found.fingerprint);
        println!(
            "
That fingerprint is what proves the mods come from your admin."
        );
        println!("Check it against what they told you.");
        if !yes && !confirm("Add this server?")? {
            bail!("cancelled; nothing was added");
        }
        found.invite
    };

    let mut book = ServerBook::load(&ctx.paths)?;
    let outcome = book.join(&invite, replace_key)?;
    book.save(&ctx.paths)?;
    let server = book.resolve(Some(&invite.name))?;
    match outcome {
        JoinOutcome::Added => println!("Added \"{}\" ({}).", server.name, server.url),
        JoinOutcome::AlreadyKnown => println!("\"{}\" was already known.", server.name),
        JoinOutcome::Updated => println!("Updated \"{}\" ({}).", server.name, server.url),
        JoinOutcome::KeyReplaced => println!("Replaced the key of \"{}\".", server.name),
    }
    println!("Key fingerprint: {}", server.fingerprint());
    println!("Next: `valhsync status` to preview, `valhsync play` to sync and start the game.");
    Ok(())
}

fn cmd_servers(ctx: &Context) -> Result<()> {
    let book = ServerBook::load(&ctx.paths)?;
    if book.servers.is_empty() {
        println!("No server yet. Import an invite code with `valhsync join <code>`.");
        return Ok(());
    }
    let installed = ctx.installed()?;
    for s in &book.servers {
        let default = if book.default.as_deref() == Some(&s.id) {
            "*"
        } else {
            " "
        };
        let active = if installed.as_ref().is_some_and(|i| i.server_id == s.id) {
            "  [installed]"
        } else {
            ""
        };
        println!(
            "{default} {}  {}  key {}{active}",
            s.name,
            s.url,
            s.fingerprint()
        );
    }
    println!("\n* = default");
    Ok(())
}

fn prepare(ctx: &Context, selector: Option<&str>) -> Result<Prepared> {
    let book = ServerBook::load(&ctx.paths)?;
    let server = book.resolve(selector)?.clone();
    let mut console = ConsoleProgress::default();
    Ok(engine::prepare(ctx, &server, &mut console)?)
}

fn print_plan(prepared: &Prepared) {
    let plan = &prepared.plan;
    let c = plan.counts();
    println!(
        "Server \"{}\"  pack {}  game {}",
        prepared.manifest.server_name,
        short_id(&prepared.manifest.pack_id),
        prepared.install.root.display()
    );
    if plan.is_noop() {
        println!("Up to date ({} files).", c.keep + c.seed_kept);
        return;
    }
    let section = |title: &str, action: Action| {
        let items: Vec<_> = plan.with_action(action).collect();
        if items.is_empty() {
            return;
        }
        println!("{title} ({}):", items.len());
        for i in items {
            println!("    {}  {}", i.path, human_bytes(i.size));
        }
    };
    section("To install", Action::Add);
    section("To update", Action::Replace);
    section("To remove (no longer in the pack)", Action::Remove);
    section(
        "To quarantine (unknown to the server, may break the connection)",
        Action::Quarantine,
    );
    if c.seed_kept > 0 {
        println!("Kept as you set them ({} config files)", c.seed_kept);
    }
    println!("Download: {}", human_bytes(plan.download_bytes));
}

fn confirm(prompt: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let l = line.trim().to_lowercase();
    Ok(l == "y" || l == "yes" || l == "o" || l == "oui")
}

fn sync(ctx: &Context, prepared: &Prepared, yes: bool) -> Result<()> {
    print_plan(prepared);
    if prepared.is_up_to_date() {
        engine::apply(ctx, prepared, &mut engine::Silent)?;
        return Ok(());
    }
    if prepared.needs_confirmation && !yes && !confirm("Apply these changes?")? {
        bail!(SyncError::NeedsConfirmation);
    }
    let mut console = ConsoleProgress::default();
    let applied = engine::apply(ctx, prepared, &mut console)?;
    let c = applied.counts;
    println!(
        "Done: {} installed, {} updated, {} removed, {} quarantined ({} downloaded).",
        c.add,
        c.replace,
        c.remove,
        c.quarantine,
        human_bytes(applied.downloaded_bytes)
    );
    if let Some(q) = &applied.quarantine_dir {
        println!(
            "Quarantined files are in {} (nothing was deleted).",
            prepared.install.root.join(q).display()
        );
    }
    if let Some(stamp) = &applied.backup_stamp {
        println!("Backup {stamp} kept; `valhsync rollback` undoes this sync.");
    }
    if let Some(hint) = game::bepinex_hint(&prepared.install) {
        println!("\nNote for this platform:\n  {hint}");
    }
    Ok(())
}

fn cmd_play(ctx: &Context, selector: Option<&str>, yes: bool, no_sync: bool) -> Result<()> {
    let prepared = prepare(ctx, selector)?;
    if no_sync {
        println!("Skipping sync as asked.");
    } else {
        sync(ctx, &prepared, yes)?;
    }
    let method = game::launch(&prepared.install, &prepared.manifest.game_address)?;
    println!(
        "Starting Valheim via {} and connecting to {}...",
        match &method {
            game::LaunchMethod::SteamExe(p) => p.display().to_string(),
            game::LaunchMethod::SteamCommand(c) | game::LaunchMethod::SteamUrl(c) => c.clone(),
        },
        prepared.manifest.game_address
    );
    Ok(())
}

fn cmd_rollback(ctx: &Context, list: bool) -> Result<()> {
    if list {
        let backups = Backup::list(&ctx.paths)?;
        if backups.is_empty() {
            println!("No backup.");
        }
        for b in backups {
            let r = &b.record;
            let status = match (&r.completed_at, &r.restored_at) {
                (_, Some(_)) => "restored",
                (Some(_), None) => "ok",
                (None, None) => "interrupted",
            };
            println!(
                "{}  {}  {} change(s)  [{status}]",
                r.stamp,
                r.server_name,
                r.done.len()
            );
        }
        return Ok(());
    }
    let (stamp, report) = engine::rollback(ctx)?;
    println!(
        "Backup {stamp} restored: {} file(s) put back, {} removed, {} taken out of quarantine.",
        report.restored, report.deleted, report.unquarantined
    );
    println!("The next `valhsync sync` will re-apply the server's pack.");
    Ok(())
}

fn cmd_vanilla(ctx: &Context, mode: VanillaMode) -> Result<()> {
    let install = game::locate(&ctx.settings)?;
    let state = match mode {
        VanillaMode::Status => vanilla::state(&install.root),
        VanillaMode::On => {
            if game::is_running() {
                bail!(SyncError::GameRunning);
            }
            vanilla::set(&install.root, false)?
        }
        VanillaMode::Off => {
            if game::is_running() {
                bail!(SyncError::GameRunning);
            }
            vanilla::set(&install.root, true)?
        }
    };
    println!(
        "{}",
        match state {
            vanilla::ModsState::On => "Mods enabled (winhttp.dll in place).",
            vanilla::ModsState::Off =>
                "Mods disabled (winhttp.dll renamed to winhttp.dll.off). The next sync re-enables them.",
            vanilla::ModsState::NotInstalled => "BepInEx is not installed in this game folder.",
        }
    );
    Ok(())
}

fn cmd_game_root(mut ctx: Context, path: Option<PathBuf>, clear: bool) -> Result<()> {
    if clear {
        ctx.settings.game_root = None;
        ctx.settings.save(&ctx.paths)?;
        println!("Back to automatic detection.");
    } else if let Some(p) = path {
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        if !game::looks_like_valheim(&p) {
            bail!(SyncError::NotAGameFolder(p));
        }
        ctx.settings.game_root = Some(p.clone());
        ctx.settings.save(&ctx.paths)?;
        println!("Game folder set to {}.", p.display());
    }
    match game::locate(&ctx.settings) {
        Ok(install) => println!("Valheim: {} ({:?})", install.root.display(), install.flavor),
        Err(e) => println!("{e}"),
    }
    Ok(())
}

fn cmd_doctor(ctx: &Context) -> Result<()> {
    println!("valhsync {}", env!("CARGO_PKG_VERSION"));
    println!("Config dir:   {}", ctx.paths.config_dir.display());
    println!("Backups dir:  {}", ctx.paths.backups_dir.display());
    let roots = game::steam_roots();
    println!(
        "Steam:        {}",
        if roots.is_empty() {
            "not found".to_string()
        } else {
            roots[0].display().to_string()
        }
    );
    match game::locate(&ctx.settings) {
        Ok(install) => {
            println!(
                "Valheim:      {} ({:?})",
                install.root.display(),
                install.flavor
            );
            println!("Mods:         {:?}", vanilla::state(&install.root));
            let q = engine::quarantine_dir(&install.root);
            if q.is_dir() {
                println!("Quarantine:   {}", q.display());
            }
        }
        Err(e) => println!("Valheim:      {e}"),
    }
    println!(
        "Game running: {}",
        if game::is_running() { "yes" } else { "no" }
    );
    match ctx.installed()? {
        Some(s) => println!(
            "Installed:    {} files from \"{}\" (pack {}), applied {}",
            s.files.len(),
            s.server_name,
            short_id(&s.pack_id),
            s.applied_at
        ),
        None => println!("Installed:    nothing yet"),
    }
    let book = ServerBook::load(&ctx.paths)?;
    println!("Servers:      {}", book.servers.len());
    for s in &book.servers {
        println!("  - {}  {}  {}", s.name, s.url, s.fingerprint());
    }
    let backups = Backup::list(&ctx.paths)?;
    println!("Backups:      {}", backups.len());
    Ok(())
}

/// First characters of a pack id, never panicking on a short string.
fn short_id(id: &str) -> &str {
    id.get(..15).unwrap_or(id)
}

/// Prints progress on the console, one line per step and a live download bar.
#[derive(Debug, Default)]
struct ConsoleProgress {
    last_percent: Option<u64>,
}

impl Progress for ConsoleProgress {
    fn on(&mut self, event: Event<'_>) {
        match event {
            Event::Fetching { url } => println!("Contacting {url}..."),
            Event::Downloading {
                index,
                count,
                path,
                size,
            } => {
                self.last_percent = None;
                println!("[{index}/{count}] {path} ({})", human_bytes(size));
            }
            Event::Progress { done, total } => {
                if total == 0 {
                    return;
                }
                let percent = done * 100 / total;
                if self.last_percent != Some(percent) && percent % 10 == 0 {
                    self.last_percent = Some(percent);
                    println!(
                        "    {percent}%  {} / {}",
                        human_bytes(done),
                        human_bytes(total)
                    );
                }
            }
            Event::Applying { changes } => println!("Applying {changes} change(s)..."),
            Event::Planned(_) | Event::Done => {}
        }
    }
}
