//! `valsync-server`: scans a Valheim mod pack, publishes a signed manifest and
//! serves the files to ValSync launchers.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use valsync_core::{Invite, Keypair};
use valsync_server::config::{Config, TemplateOptions};
use valsync_server::{config, detect, keys, net, pack, serve};

#[derive(Parser, Debug)]
#[command(
    name = "valsync-server",
    version,
    about = "Publishes a signed Valheim mod pack for ValSync launchers"
)]
struct Cli {
    /// Configuration file.
    #[arg(long, global = true, default_value = "valsync-server.toml")]
    config: PathBuf,

    /// Where keys, the content store and the published manifest live.
    /// Defaults to `valsync-server-data` next to the configuration file.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create the configuration, generate the signing key, print the invite code.
    Init {
        /// Game root of the dedicated server (auto-detected when omitted).
        #[arg(long)]
        server_root: Option<PathBuf>,
        /// Name shown to players.
        #[arg(long)]
        name: Option<String>,
        /// host:port of the game server handed to Valheim.
        #[arg(long)]
        game_address: Option<String>,
        /// URL players reach this server at (goes into the invite code).
        #[arg(long)]
        public_url: Option<String>,
        /// Overwrite an existing configuration (the key is kept).
        #[arg(long)]
        force: bool,
    },
    /// Build the manifest once and print what changed.
    Scan,
    /// Build the manifest, serve it, and rebuild whenever the pack changes.
    Serve,
    /// Write the pack as static files (manifest.json, manifest.sig, files/<hash>)
    /// to upload on any web space. No port to open.
    Export {
        /// Output folder, e.g. a GitHub Pages checkout or a folder your
        /// uploader (rclone, git, FTP sync, Nextcloud client) picks up.
        dir: PathBuf,
        /// Keep running and re-export whenever the pack changes.
        #[arg(long)]
        watch: bool,
    },
    /// Print the invite code again.
    Invite,
    /// Generate a new signing key. Every player must import the new invite code.
    RotateKey {
        /// Confirm; without it the command only explains the consequences.
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let data_dir = cli.data_dir.clone().unwrap_or_else(|| {
        cli.config
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            .join("valsync-server-data")
    });

    match cli.cmd {
        Cmd::Init {
            server_root,
            name,
            game_address,
            public_url,
            force,
        } => cmd_init(
            &cli.config,
            &data_dir,
            InitArgs {
                server_root,
                name,
                game_address,
                public_url,
                force,
            },
        ),
        Cmd::Scan => {
            let cfg = Config::load(&cli.config)?;
            print_warnings(&cfg);
            let kp = keys::load(&data_dir)?;
            let outcome = pack::build(&cfg, &kp, &data_dir)?;
            pack::print_summary(&outcome);
            Ok(())
        }
        Cmd::Serve => {
            let cfg = Config::load(&cli.config)?;
            print_warnings(&cfg);
            let kp = keys::load(&data_dir)?;
            tokio::runtime::Runtime::new()?.block_on(serve::run(cfg, kp, data_dir))
        }
        Cmd::Export { dir, watch } => {
            let cfg = Config::load(&cli.config)?;
            print_warnings(&cfg);
            let kp = keys::load(&data_dir)?;
            export_once(&cfg, &kp, &data_dir, &dir)?;
            println!(
                "Upload that folder as-is. Set [server] public_url to the URL where \
                 manifest.json ends up (without the file name), then `valsync-server invite`."
            );
            if watch {
                watch_and_export(&cfg, &kp, &data_dir, &dir)?;
            }
            Ok(())
        }
        Cmd::Invite => {
            let cfg = Config::load(&cli.config)?;
            let kp = keys::load(&data_dir)?;
            print_invite(&cfg, &kp)
        }
        Cmd::RotateKey { yes } => {
            let cfg = Config::load(&cli.config)?;
            if !yes {
                bail!(
                    "rotating the key invalidates every player's pinned key: they will all \
                     have to import the new invite code. Run again with --yes to proceed."
                );
            }
            let kp = keys::rotate(&data_dir)?;
            println!(
                "New key generated. Old key kept in {}.",
                data_dir.join("keys").display()
            );
            println!(
                "Restart `valsync-server serve`, then send the new invite code to every player:\n"
            );
            print_invite(&cfg, &kp)
        }
    }
}

struct InitArgs {
    server_root: Option<PathBuf>,
    name: Option<String>,
    game_address: Option<String>,
    public_url: Option<String>,
    force: bool,
}

fn cmd_init(config_path: &Path, data_dir: &Path, args: InitArgs) -> Result<()> {
    if config_path.exists() && !args.force {
        bail!(
            "{} already exists; edit it, or run `init --force` to regenerate it (the key is kept)",
            config_path.display()
        );
    }

    let server_root = match args.server_root {
        Some(p) => {
            if !p.is_dir() {
                bail!("{} is not a directory", p.display());
            }
            Some(p)
        }
        None => detect::detect_server_root(),
    };
    match &server_root {
        Some(p) => {
            println!("Dedicated server found: {}", p.display());
            if !detect::has_bepinex(p) {
                println!("  (no BepInEx folder there yet; install BepInExPack_Valheim first)");
            }
        }
        None => println!(
            "No dedicated server found. Set [pack] server_root in the config, or use only \
             client_extras for a hosted server."
        ),
    }

    let lan = net::lan_ip().map_or_else(|| "your.public.address".to_string(), |ip| ip.to_string());
    let opts = TemplateOptions {
        name: args.name.unwrap_or_else(|| "My Valheim server".into()),
        game_address: args.game_address.unwrap_or_else(|| format!("{lan}:2456")),
        public_url: args.public_url,
        server_root,
        client_extras: data_dir
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            .join("client-extras"),
    };
    std::fs::create_dir_all(&opts.client_extras)
        .with_context(|| format!("cannot create {}", opts.client_extras.display()))?;
    if let Some(parent) = config_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(config_path, config::template(&opts))
        .with_context(|| format!("cannot write {}", config_path.display()))?;
    println!("Configuration written: {}", config_path.display());

    let (kp, created) = keys::load_or_create(data_dir)?;
    println!(
        "{} {}",
        if created {
            "Signing key generated:"
        } else {
            "Existing signing key kept:"
        },
        keys::key_path(data_dir).display()
    );
    println!("  Back it up. Losing it means sending every player a new invite code.\n");

    let cfg = Config::load(config_path)?;
    println!("Next steps:");
    println!(
        "  1. Check [pack] and [server] in {}",
        config_path.display()
    );
    println!("  2. `valsync-server scan` to review the pack");
    println!("  3. Publish it, one of:");
    println!(
        "     - `valsync-server export <folder>` and upload the folder to any web space \
         (GitHub Pages, S3, your host): nothing to open on the router"
    );
    println!(
        "     - `valsync-server serve` on this machine: needs TCP {} open in the firewall \
         and router\n",
        cfg.bind_addr()?.port()
    );
    print_invite(&cfg, &kp)
}

/// Print non-fatal configuration warnings, once, before publishing.
fn print_warnings(cfg: &Config) {
    for w in cfg.warnings() {
        eprintln!("warning: {w}");
    }
}

fn export_once(cfg: &Config, kp: &Keypair, data_dir: &Path, dir: &Path) -> Result<()> {
    let outcome = pack::build(cfg, kp, data_dir)?;
    let report = pack::export_static(&outcome.published, data_dir, dir)?;
    println!(
        "Exported {} files to {} ({} copied, {} stale removed).",
        report.files,
        dir.display(),
        report.copied,
        report.removed
    );
    Ok(())
}

/// Re-export after every change to the pack folders, with the same settle
/// delay as `serve`. Blocks until Ctrl+C.
fn watch_and_export(cfg: &Config, kp: &Keypair, data_dir: &Path, dir: &Path) -> Result<()> {
    use notify::Watcher;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    })
    .context("cannot create the folder watcher")?;
    for path in cfg.watch_paths() {
        watcher
            .watch(&path, notify::RecursiveMode::Recursive)
            .with_context(|| format!("cannot watch {}", path.display()))?;
        println!("Watching {}", path.display());
    }
    println!("Re-exporting on change. Press Ctrl+C to stop.");

    let settle = Duration::from_secs(2);
    loop {
        // Block for the first event, then absorb the burst that follows it.
        if rx.recv().is_err() {
            return Ok(());
        }
        let mut last = Instant::now();
        while last.elapsed() < settle {
            if rx
                .recv_timeout(settle.saturating_sub(last.elapsed()))
                .is_ok()
            {
                last = Instant::now();
            }
        }
        match export_once(cfg, kp, data_dir, dir) {
            Ok(()) => {}
            Err(e) => eprintln!("export failed, previous export kept: {e:#}"),
        }
    }
}

fn print_invite(cfg: &Config, kp: &Keypair) -> Result<()> {
    let url = cfg.public_url();
    let invite = Invite::new(&url, &kp.public(), cfg.server.name.trim()).encode()?;
    println!("Invite code for \"{}\" ({url}):\n", cfg.server.name.trim());
    println!("{invite}\n");
    if cfg.server.public_url.is_none() {
        println!(
            "Note: no [server] public_url set, so this code uses the LAN address. Players \
             outside your network need public_url set to your public IP or DNS name."
        );
    }
    println!(
        "Send it to players, or drop it in a file named valsync-invite.txt next to \
         valsync.exe: the launcher imports it on first start."
    );
    Ok(())
}
