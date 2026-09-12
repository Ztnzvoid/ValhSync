//! The admin window: detect, configure, publish, invite.
//!
//! Every string it says lives in [`super::i18n`], one key per sentence: the
//! pairs that used to sit inline could hold two languages and no more.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align, Color32, Layout, RichText};
use valhsync_core::limits::human_bytes;
use valhsync_core::manifest::MAX_NOTES;
use valhsync_ui::frame as chrome;
use valhsync_ui::theme as th;
use valhsync_ui::widgets as w;

use super::i18n::{Key, Lang, text};
use super::worker::{self, Msg, Reporter};
use crate::config::{self, Config};
use crate::{detect, gameserver, install, logs, names, players, wizard, worldbackup};

const POLL_GAME_SERVER: Duration = Duration::from_secs(2);
/// How long the configuration has to stop changing before it is written.
///
/// Long enough that typing a note is one write rather than one per keystroke
/// -- each write wakes the publisher's watcher and rebuilds the pack -- and
/// short enough that nobody wonders whether it took.
const AUTOSAVE_SETTLE: Duration = Duration::from_millis(1200);
/// How long to wait before trying to publish again after a failed attempt.
/// Long enough that a misconfiguration does not retry in a loop, short enough
/// that fixing it takes effect without touching the button.
const PUBLISH_RETRY: Duration = Duration::from_secs(20);
/// How often the machine's public address is re-checked. A home connection
/// changes it on a reboot or a lease renewal, not minute to minute.
const POLL_PUBLIC_IP: Duration = Duration::from_secs(900);
const NOTICE_TTL: Duration = Duration::from_secs(12);
/// How often the open log is re-read. Fast enough to watch a start-up, slow
/// enough that the file is touched once a second and no more.
const POLL_LOG: Duration = Duration::from_millis(900);

/// How often the permission lists are re-read while their tab is open. They
/// change when somebody edits a text file, which is not something that needs
/// noticing in the same second.
const POLL_LISTS: Duration = Duration::from_secs(2);

/// Which worker a message came from. They report through the same type but
/// have different lifetimes: a one-shot job ends, the live server runs on,
/// and the address check repeats on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chan {
    Job,
    Serve,
    Ip,
}

/// What a row's buttons do to a mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    /// Out of BepInEx, kept on the server.
    Disable,
    /// Back into BepInEx.
    Enable,
    /// Out of the server entirely -- into a folder, not into nothing.
    Remove,
}

/// Who said a line in the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Echo {
    /// Typed at the prompt.
    Sent,
    /// ValhSync answering it.
    Answer,
    /// Printed by the game server's own console.
    Server,
}

/// The two halves of the window: what the server is doing, and how it is set
/// up. Everything that changes minute to minute is on the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Mods,
    Notes,
    Players,
    Status,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishMode {
    /// Static files to upload anywhere. No port.
    Export,
    /// Live HTTP server on this machine.
    Live,
}

#[derive(Debug, Clone)]
struct ModEntry {
    folder: String,
    /// A plugin sitting directly in `plugins`, not in a folder of its own.
    loose: bool,
    /// Excluded from the pack: it runs on the server only.
    server_only: bool,
    /// Comes from the client-extras folder: it runs on players only.
    client_only: bool,
}

#[derive(Debug, Clone)]
struct PackSummary {
    files: usize,
    bytes: u64,
    skipped: Vec<(String, String)>,
}

pub(super) struct App {
    lang: Lang,
    config_path: PathBuf,
    data_dir: PathBuf,
    cfg: Config,
    /// Config as last written to disk, to know whether anything changed.
    saved: Config,
    /// No configuration file exists yet: everything is worth saving.
    never_saved: bool,

    // editable mirrors of the config, so typing never fights validation
    name: String,
    game_address: String,
    server_root: String,
    export_dir: String,
    bind: String,
    public_url: String,
    /// The admin's word to players, as typed. Empty means there is none.
    notes: String,
    /// The Discord webhook, as typed. A credential: it is kept out of every
    /// log and error by `config::Secret`, and the field is masked here so it
    /// does not travel in a screenshot either.
    discord_webhook: String,

    mods: Vec<ModEntry>,
    scripts: Vec<detect::StartScript>,
    script_index: usize,
    /// True once the admin picked a start script themselves. Until then the
    /// detected one is only a suggestion, and must not mark the file dirty.
    script_chosen: bool,
    mode: PublishMode,

    summary: Option<PackSummary>,
    invite: String,
    serving_at: Option<String>,
    stop_serving: Option<tokio::sync::oneshot::Sender<()>>,

    game_running: bool,
    /// The process the poll matched. Shown on the panel: "cannot start, one is
    /// already running" is a dead end unless it says which one.
    game_pid: Option<u32>,
    /// The shell ValhSync started the script with, kept so the console does
    /// not sit on "Terminate batch job (Y/N)?" after a stop.
    game_shell: Option<gameserver::Shell>,
    game_checked: Instant,

    tab: Tab,
    /// Every log this installation writes, and which one is open.
    log_sources: Vec<PathBuf>,
    log_index: usize,
    log: Option<logs::Tail>,
    log_checked: Instant,
    /// What the game server's console has printed. Its own list rather than a
    /// filter over the log: these lines are a handful among thousands, and on
    /// a busy server they are pushed out of the log ring within seconds.
    console: VecDeque<(Echo, String)>,
    /// The prompt's line, and the folder its commands write to.
    command: String,
    lists_dir: Option<PathBuf>,
    /// Admins, banned, permitted, in `players::Roll::ALL` order. Re-read
    /// rather than remembered across a write: the files are Valheim's, and an
    /// admin may well have a text editor open on one.
    lists: [Vec<String>; 3],
    lists_error: Option<String>,
    player_id: String,
    lists_checked: Instant,
    /// When the game server has gone down on its own recently. Bounded
    /// restarts need a memory, and this is it.
    crashes: Vec<Instant>,
    /// Every player this server has ever logged, id to name. Read from the
    /// log beside the lists, so a row can say who an id belongs to.
    known_names: std::collections::HashMap<String, String>,
    /// The mod whose Remove has been clicked once. A second click on the same
    /// row carries it out; a click anywhere else forgets it.
    remove_armed: Option<String>,
    /// Follow the end of the file, until the admin scrolls up to read.
    log_follow: bool,
    session: logs::Session,
    /// The Valheim the server runs, read from its log.
    game_version: Option<String>,
    /// Steam has an update waiting for the dedicated server: the game and the
    /// server are separate Steam apps, and updating one leaves the other on an
    /// older network version, which refuses every connection.
    server_outdated: bool,
    /// When the world file was last written, so the panel can say how long ago.
    world_saved: Option<SystemTime>,
    /// When the configuration first differed from what is on disk, so it can
    /// be written once the admin stops typing rather than on every keystroke.
    dirty_since: Option<Instant>,
    /// Why the configuration could not be written. Standing, not a notice:
    /// nothing will be saved until it is fixed.
    save_error: Option<String>,
    /// Set when a stop was asked for, cleared when the process is gone.
    stop_requested: Option<Instant>,
    /// The stop under way is half of a restart: start it again once the world
    /// has been written and the process has actually gone.
    restart_after_stop: bool,
    /// The world file, when the start script says enough to find it.
    world_file: Option<PathBuf>,

    /// The start-script wizard, and the name it would write to.
    recipe: wizard::Recipe,
    recipe_file: String,
    /// Second click confirms replacing a script that already exists.
    recipe_replace: bool,

    /// Result of the last "test the link", which is the only honest answer to
    /// "will my players be able to fetch this?".
    link_ok: Option<bool>,
    checking_link: bool,

    /// Window title as last set, so it is only pushed when it changes.
    title: String,
    busy: bool,
    rx: Option<Receiver<Msg>>,
    /// The live server owns its own channel. It outlives the one-shot jobs,
    /// so sharing `rx` with them let a scan or an export drop the receiver
    /// the publisher was still reporting through: it then stopped in silence
    /// and the card kept offering to stop something already gone.
    serve_rx: Option<Receiver<Msg>>,
    /// What the internet sees this machine as, checked on its own so the
    /// admin never has to ask, and never has to wait for a button either.
    public_ip: Option<String>,
    ip_checked: Option<Instant>,
    ip_rx: Option<Receiver<Msg>>,
    /// Start was pressed before the address was known. Publishing waits for
    /// it rather than going online announcing a name that resolves to
    /// nothing, and starts by itself the moment the answer arrives.
    publish_when_addressed: bool,
    /// The admin stopped publishing by hand while the game server stayed up.
    /// Their choice stands until they start it again, or until the game
    /// server is restarted -- a fresh start is a fresh intent.
    publish_paused: bool,
    /// When publishing was last attempted on its own, so a configuration it
    /// refuses is not retried every couple of seconds.
    publish_tried: Option<Instant>,
    /// Set when this run had to mint a signing key. The key is the server's
    /// identity and every player pins it, so a new one is never a detail:
    /// it used to be a line in a log nobody reads, and an admin whose players
    /// were suddenly all refused had nothing to go on.
    new_key_at: Option<String>,
    /// What to type into the dedicated server's console.
    /// Path of a signing key to take over from another install.
    key_import: String,
    /// Why publishing is not up, when it tried and could not. Kept on the
    /// card rather than flashed as a notice: it is a standing condition, and
    /// a message that expires leaves the admin with a dead server and no
    /// reason for it.
    publish_error: Option<String>,
    notice: Option<(String, Color32, Instant)>,
    egui_ctx: egui::Context,
}

impl App {
    #[allow(clippy::too_many_lines)] // one field per line of state, read top to bottom
    pub(super) fn new(ctx: &egui::Context, config_path: PathBuf, data_dir: PathBuf) -> Self {
        let new_key_at = match crate::keys::load_or_adopt(&data_dir) {
            Ok((_, crate::keys::Origin::Created)) => {
                Some(crate::keys::key_path(&data_dir).display().to_string())
            }
            _ => None,
        };
        let (cfg, loaded) = match Config::load(&config_path) {
            Ok(cfg) => (cfg, true),
            Err(_) => (Self::fresh_config(&config_path), false),
        };
        let cfg_publishes_live = cfg.publishes_live();
        let export_dir = cfg.pack.export_dir.clone().unwrap_or_else(|| {
            config_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
                .join("pack-site")
        });

        let mut app = Self {
            lang: Lang::detect(cfg.language.as_deref()),
            name: cfg.server.name.clone(),
            game_address: cfg.server.game_address.clone(),
            server_root: cfg
                .pack
                .server_root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            bind: cfg.server.bind.clone(),
            public_url: cfg.server.public_url.clone().unwrap_or_default(),
            export_dir: export_dir.display().to_string(),
            notes: cfg.pack.notes.clone().unwrap_or_default(),
            discord_webhook: cfg
                .server
                .discord_webhook
                .as_ref()
                .map(|s| s.as_str().to_string())
                .unwrap_or_default(),
            saved: cfg.clone(),
            never_saved: !loaded,
            cfg,
            config_path,
            data_dir,
            mods: Vec::new(),
            scripts: Vec::new(),
            script_index: 0,
            script_chosen: false,
            mode: if cfg_publishes_live {
                PublishMode::Live
            } else {
                PublishMode::Export
            },
            summary: None,
            invite: String::new(),
            serving_at: None,
            stop_serving: None,
            serve_rx: None,
            public_ip: None,
            ip_checked: None,
            ip_rx: None,
            publish_when_addressed: false,
            publish_paused: false,
            publish_tried: None,
            new_key_at,
            key_import: String::new(),
            publish_error: None,
            game_running: false,
            game_pid: None,
            game_shell: None,
            // Force a process check on the very first frame.
            game_checked: Instant::now()
                .checked_sub(POLL_GAME_SERVER)
                .unwrap_or_else(Instant::now),
            tab: Tab::Status,
            log_sources: Vec::new(),
            log_index: 0,
            log: None,
            console: VecDeque::new(),
            command: String::new(),
            lists_dir: None,
            lists: [Vec::new(), Vec::new(), Vec::new()],
            lists_error: None,
            player_id: String::new(),
            remove_armed: None,
            known_names: std::collections::HashMap::new(),
            crashes: Vec::new(),
            lists_checked: Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
            log_checked: Instant::now(),
            log_follow: true,
            session: logs::Session::default(),
            game_version: None,
            server_outdated: false,
            world_saved: None,
            dirty_since: None,
            save_error: None,
            stop_requested: None,
            restart_after_stop: false,
            world_file: None,
            recipe: wizard::Recipe::default(),
            recipe_file: String::from("start_valheim_server.bat"),
            recipe_replace: false,
            link_ok: None,
            checking_link: false,
            title: String::new(),
            busy: false,
            rx: None,
            notice: None,
            egui_ctx: ctx.clone(),
        };
        app.refresh_detection();
        if !loaded {
            let msg = app.t(Key::NoConfigFound).to_string();
            app.notify(msg, th::GOLD_LIT);
        }
        app.refresh_invite();
        app
    }

    /// A best-effort configuration for a machine that has never run ValhSync.
    fn fresh_config(config_path: &Path) -> Config {
        let mut cfg = Config::default();
        cfg.pack.server_root = detect::detect_server_root();
        if let Some(root) = &cfg.pack.server_root
            && let Some(script) = detect::find_start_scripts(root).into_iter().next()
        {
            if let Some(name) = &script.args.name {
                cfg.server.name.clone_from(name);
            }
            if let Some(port) = script.args.port {
                cfg.server.game_address = format!("your.public.address:{port}");
            }
            cfg.game_server.start_script = Some(script.path);
        }
        cfg.pack.client_extras = Some(
            config_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
                .join("client-extras"),
        );
        cfg
    }

    /// The taskbar should say which server this window configures.
    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.name.trim();
        let wanted = if name.is_empty() {
            "ValhSync · Serveur".to_string()
        } else {
            format!("ValhSync · {name}")
        };
        if wanted != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(wanted.clone()));
            self.title = wanted;
        }
    }

    /// Short on purpose: it is written a hundred and seventy-five times.
    fn t(&self, key: Key) -> &'static str {
        text(self.lang, key)
    }

    fn notify(&mut self, message: String, color: Color32) {
        self.notice = Some((message, color, Instant::now()));
    }

    fn dirty(&self) -> bool {
        self.never_saved || self.edited() != self.saved
    }

    /// The configuration as the fields currently read it. Drawing the window
    /// never writes to the stored configuration: everything goes through here,
    /// so the window cannot report changes it made to itself.
    fn edited(&self) -> Config {
        let mut cfg = self.cfg.clone();
        cfg.server.name.clone_from(&self.name);
        cfg.server.game_address = self.game_address.trim().to_string();
        cfg.server.bind.clone_from(&self.bind);
        let url = self.public_url.trim();
        cfg.server.public_url = (!url.is_empty()).then(|| url.to_string());
        let root = self.server_root.trim();
        cfg.pack.server_root = (!root.is_empty()).then(|| PathBuf::from(root));
        if self.script_chosen || cfg.game_server.start_script.is_some() {
            cfg.game_server.start_script =
                self.scripts.get(self.script_index).map(|s| s.path.clone());
        }
        cfg.server.publish_live = Some(self.mode == PublishMode::Live);
        cfg.language = Some(self.lang.code().to_string());
        let dir = self.export_dir.trim();
        cfg.pack.export_dir = (!dir.is_empty()).then(|| PathBuf::from(dir));
        let notes = self.notes.trim();
        cfg.pack.notes = (!notes.is_empty()).then(|| notes.to_string());
        let hook = self.discord_webhook.trim();
        cfg.server.discord_webhook = (!hook.is_empty()).then(|| hook.into());
        cfg
    }

    /// Take what the fields say into the configuration.
    fn pull_fields(&mut self) {
        self.cfg = self.edited();
    }

    /// Re-read what is on disk: mods, start scripts.
    fn refresh_detection(&mut self) {
        self.scripts = self
            .cfg
            .pack
            .server_root
            .as_deref()
            .map(detect::find_start_scripts)
            .unwrap_or_default();
        self.script_index = self
            .cfg
            .game_server
            .start_script
            .as_ref()
            .and_then(|p| self.scripts.iter().position(|s| &s.path == p))
            .unwrap_or(0);
        self.mods = self.collect_mods();
        self.refresh_log_sources();
        self.fill_recipe_from_script();
        self.server_outdated =
            valhsync_core::steam::app_state(valhsync_core::steam::VALHEIM_SERVER_APP_ID, &[])
                .is_some_and(|s| s.update_pending);
    }

    /// Which logs this installation writes, and where its world file is.
    /// Both come from the server folder and the start script, never from a
    /// setting the admin has to fill in.
    fn refresh_log_sources(&mut self) {
        let root = self.cfg.pack.server_root.clone();
        let args = self.scripts.get(self.script_index).map(|s| s.args.clone());
        let Some(root) = root else {
            self.log_sources.clear();
            self.log = None;
            self.world_file = None;
            return;
        };
        self.world_file = args.as_ref().and_then(|a| logs::world_save(&root, a));

        let sources = logs::find_sources(&root, args.as_ref());
        if sources == self.log_sources {
            return;
        }
        // Stay on the same file across a refresh when it is still there.
        let open = self.log.as_ref().map(|t| t.path().to_path_buf());
        self.log_sources = sources;
        self.log_index = open
            .and_then(|p| self.log_sources.iter().position(|s| *s == p))
            .unwrap_or(0);
        self.open_log();
    }

    fn open_log(&mut self) {
        // Seed the console from the whole file before following the tail.
        // Otherwise a window opened on a server that has been up for hours
        // starts with an empty console and stays that way until the server
        // happens to say something.
        if let Some(path) = self.log_sources.get(self.log_index) {
            self.console.clear();
            for line in logs::console_history(path, 200) {
                self.record_console(Echo::Server, line);
            }
        }
        self.log = self
            .log_sources
            .get(self.log_index)
            .cloned()
            .map(logs::Tail::new);
        self.log_follow = true;
        self.log_checked = Instant::now()
            .checked_sub(POLL_LOG)
            .unwrap_or_else(Instant::now);
    }

    /// Re-read the open log, and with it what the session line says.
    fn poll_log(&mut self) {
        if self.log_checked.elapsed() < POLL_LOG {
            return;
        }
        self.log_checked = Instant::now();
        if let Some(tail) = &mut self.log
            && tail.poll()
        {
            let spoken: Vec<String> = tail
                .fresh()
                .iter()
                .filter_map(|line| logs::console_text(line))
                .map(str::to_owned)
                .collect();
            self.session = logs::read_session(tail.lines());
            self.game_version = valhsync_core::gamelog::read_version(tail.lines());
            for line in spoken {
                self.record_console(Echo::Server, line);
            }
        }
        self.world_saved = self.world_file.as_deref().and_then(logs::saved_at);
    }

    /// A duration in the plainest words: "3 min", "2 h 10". Only ever used
    /// for something that happened, so it is always in the past.
    fn ago(when: SystemTime) -> String {
        let secs = when.elapsed().map(|d| d.as_secs()).unwrap_or_default();
        match secs {
            0..=59 => format!("{secs} s"),
            60..=3599 => format!("{} min", secs / 60),
            _ => format!("{} h {:02}", secs / 3600, (secs % 3600) / 60),
        }
    }

    /// Start the wizard from whatever the selected script already says, so
    /// editing an existing server is a matter of changing one field.
    fn fill_recipe_from_script(&mut self) {
        let Some(script) = self.scripts.get(self.script_index) else {
            return;
        };
        let a = &script.args;
        let default = wizard::Recipe::default();
        self.recipe = wizard::Recipe {
            name: a.name.clone().unwrap_or_default(),
            world: a.world.clone().unwrap_or_default(),
            // The password is never read out of a script, so it is always
            // typed again here.
            password: std::mem::take(&mut self.recipe.password),
            port: a.port.unwrap_or(default.port),
            public: a.public.unwrap_or(default.public),
            crossplay: a.crossplay,
            save_interval: a.save_interval.unwrap_or(default.save_interval),
            backups: a.backups.unwrap_or(default.backups),
            log_file: a.log_file.is_some() || default.log_file,
        };
        // Never offer to overwrite the file Steam owns.
        if !script.is_stock
            && let Some(name) = script.path.file_name().and_then(|n| n.to_str())
        {
            self.recipe_file = name.to_string();
        }
    }

    fn collect_mods(&self) -> Vec<ModEntry> {
        let mut out: Vec<ModEntry> = Vec::new();
        let add = |dir: Option<&PathBuf>, client_only: bool, out: &mut Vec<ModEntry>| {
            let Some(plugins) = dir.map(|d| d.join("BepInEx").join("plugins")) else {
                return;
            };
            let Ok(entries) = std::fs::read_dir(&plugins) else {
                return;
            };
            for e in entries.flatten() {
                let path = e.path();
                // A mod is a folder, or a plugin dropped loose in `plugins`.
                let is_loose_plugin = path.is_file()
                    && path
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("dll"));
                if !path.is_dir() && !is_loose_plugin {
                    continue;
                }
                let Some(folder) = e.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                if out.iter().any(|m| m.folder == folder) {
                    continue;
                }
                out.push(ModEntry {
                    server_only: false,
                    client_only,
                    loose: is_loose_plugin,
                    folder,
                });
            }
        };
        add(self.cfg.pack.server_root.as_ref(), false, &mut out);
        add(self.cfg.pack.client_extras.as_ref(), true, &mut out);
        for m in &mut out {
            let pattern = exclude_pattern(&m.folder, m.loose);
            m.server_only = self.cfg.pack.exclude.iter().any(|p| p == &pattern);
        }
        out.sort_by_key(|m| m.folder.to_lowercase());
        out
    }

    fn set_server_only(&mut self, folder: &str, loose: bool, server_only: bool) {
        let pattern = exclude_pattern(folder, loose);
        if server_only {
            if !self.cfg.pack.exclude.contains(&pattern) {
                self.cfg.pack.exclude.push(pattern);
            }
        } else {
            self.cfg.pack.exclude.retain(|p| p != &pattern);
        }
    }

    /// The invite code, if a signing key already exists. Opening the window
    /// must not create one: a release package would then ship with a key in
    /// it. The key is generated on the first Save, Scan or Export.
    /// The port out of `bind`, falling back to the game's.
    fn bind_port(&self) -> u16 {
        self.bind
            .trim()
            .rsplit_once(':')
            .and_then(|(_, p)| p.trim().parse().ok())
            .unwrap_or(config::DEFAULT_PORT)
    }

    /// Can players actually reach what the address field says? Empty, a LAN
    /// address, or one of the template's examples all mean no.
    fn address_is_usable(&self) -> bool {
        let addr = self.game_address.trim();
        !addr.is_empty()
            && !valhsync_core::manifest::is_private_host(addr)
            && !config::is_placeholder_address(addr)
    }

    /// Put the detected address in the field when what is there cannot work.
    /// Returns true when the field ends up usable.
    fn adopt_detected_ip(&mut self) -> bool {
        if self.address_is_usable() {
            return true;
        }
        let Some(ip) = self.public_ip.clone() else {
            return false;
        };
        self.game_address = format!("{ip}:{}", self.game_port());
        true
    }

    /// The port the start script says the game listens on.
    fn game_port(&self) -> u16 {
        self.scripts
            .get(self.script_index)
            .and_then(|s| s.args.port)
            .unwrap_or(2456)
    }

    /// Ask what the internet sees us as. Runs on its own channel: it must not
    /// hold `busy`, and it must not take the slot a real job needs.
    fn detect_public_ip(&mut self) {
        if self.ip_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let rep = Reporter {
            tx,
            ctx: self.egui_ctx.clone(),
        };
        self.ip_checked = Some(Instant::now());
        self.ip_rx = Some(rx);
        std::thread::spawn(move || rep.run(worker::public_ip));
    }

    fn refresh_invite(&mut self) {
        self.invite = crate::keys::load(&self.data_dir)
            .ok()
            .and_then(|kp| worker::invite_code(&self.cfg, &kp).ok())
            .unwrap_or_default();
    }

    fn start_job<F>(&mut self, job: F)
    where
        F: FnOnce(Reporter) + Send + 'static,
    {
        if self.busy {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let rep = Reporter {
            tx,
            ctx: self.egui_ctx.clone(),
        };
        self.busy = true;
        self.rx = Some(rx);
        std::thread::spawn(move || job(rep));
    }

    /// Start the live server on its own channel. It is not a one-shot job:
    /// it reports for as long as it serves, so it must not take the slot the
    /// short jobs reuse, and it must not hold `busy` while it runs.
    fn start_serving(&mut self) {
        if self.serve_rx.is_some() || self.serving_at.is_some() {
            return;
        }
        self.pull_fields();
        if let Err(e) = self.cfg.validate() {
            self.publish_error = Some(format!("{e:#}"));
            return;
        }
        self.publish_error = None;
        let (tx, rx) = mpsc::channel();
        let rep = Reporter {
            tx,
            ctx: self.egui_ctx.clone(),
        };
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        self.stop_serving = Some(stop_tx);
        self.serve_rx = Some(rx);
        let cfg = self.cfg.clone();
        let data = self.data_dir.clone();
        let path = self.config_path.clone();
        std::thread::spawn(move || {
            worker::serve_blocking(cfg, data, path, stop_rx, &rep);
        });
    }

    /// Ask the live server to stop, and say so if it is already gone.
    fn stop_serving(&mut self) {
        if let Some(stop) = self.stop_serving.take() {
            let _ = stop.send(());
        } else {
            // Nothing left to signal: the worker is gone and only the label
            // survived it. Clear it here rather than wait for a report that
            // will never come.
            self.serving_at = None;
            self.serve_rx = None;
        }
    }

    /// Write the configuration once it has stopped changing.
    ///
    /// On a timer rather than on a button: what an admin edits here is what
    /// the server publishes, and asking them to confirm it a second time only
    /// produced servers running on a configuration that was on screen and
    /// never on disk -- with the launcher's "cannot reach the server" as the
    /// first anyone heard of it.
    fn autosave(&mut self) {
        if self.busy || !self.dirty() {
            self.dirty_since = None;
            self.save_error = None;
            return;
        }
        let since = *self.dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() < AUTOSAVE_SETTLE {
            return;
        }
        // Re-armed either way: a configuration that does not validate is
        // checked again after the next pause, so the reason on the bar
        // follows what is being typed and clears itself when it is fixed.
        self.dirty_since = Some(Instant::now());
        self.save();
    }

    fn save(&mut self) -> bool {
        self.pull_fields();
        if let Err(e) = self.cfg.validate() {
            self.save_error = Some(format!("{e:#}"));
            return false;
        }
        match config::save(&self.cfg, &self.config_path) {
            Ok(()) => {
                self.saved = self.cfg.clone();
                self.never_saved = false;
                self.save_error = None;
                self.dirty_since = None;
                self.refresh_detection();
                if let Ok(kp) = worker::load_key(&self.data_dir) {
                    self.invite = worker::invite_code(&self.cfg, &kp).unwrap_or_default();
                }
                true
            }
            Err(e) => {
                self.save_error = Some(format!("{e:#}"));
                false
            }
        }
    }

    /// Read both channels: the one-shot jobs, and the live server. A worker
    /// that dies without reporting closes its channel, and that is a report
    /// too -- otherwise the window waits for a message nobody will send and
    /// the only way out is to restart it.
    fn drain(&mut self) {
        let mut msgs: Vec<(Msg, Chan)> = Vec::new();
        let job_gone = Self::collect(self.rx.as_ref(), &mut msgs, Chan::Job);
        let serve_gone = Self::collect(self.serve_rx.as_ref(), &mut msgs, Chan::Serve);
        let ip_gone = Self::collect(self.ip_rx.as_ref(), &mut msgs, Chan::Ip);
        for (msg, chan) in msgs {
            self.apply(msg, chan);
        }
        if job_gone {
            self.busy = false;
            self.checking_link = false;
            self.rx = None;
        }
        if ip_gone {
            self.ip_rx = None;
        }
        if serve_gone {
            self.serve_rx = None;
            if self.serving_at.take().is_some() {
                self.stop_serving = None;
                let msg = self.t(Key::LiveServerStopped).to_string();
                self.notify(msg, th::GOLD);
            }
        }
    }

    /// Act on one message from `chan`.
    #[allow(clippy::too_many_lines)] // one arm per message, read top to bottom
    fn apply(&mut self, msg: Msg, chan: Chan) {
        match msg {
            Msg::Scanned {
                files,
                bytes,
                invite,
                skipped,
            } => {
                self.invite = invite;
                self.summary = Some(PackSummary {
                    files,
                    bytes,
                    skipped,
                });
                let msg = format!(
                    "{} {files} {} · {}",
                    self.t(Key::PackBuilt),
                    self.t(Key::Files),
                    human_bytes(bytes)
                );
                self.notify(msg, th::MOSS);
            }
            Msg::Exported { dir, files, copied } => {
                let msg = format!(
                    "{} {files} {} → {} ({copied} {})",
                    self.t(Key::ExportPrefix),
                    self.t(Key::Files),
                    dir.display(),
                    self.t(Key::Copied)
                );
                self.notify(msg, th::MOSS);
            }
            Msg::Serving(url) => self.serving_at = Some(url),
            Msg::ServeStopped(err) => {
                self.serving_at = None;
                self.stop_serving = None;
                if let Some(e) = err {
                    self.notify(e, th::BLOOD_LIT);
                }
            }
            Msg::PublicIp(ip) => {
                self.public_ip = Some(ip.clone());
                // Only write it into the address when what is there
                // cannot work anyway: an empty field, a LAN address no
                // outside player can reach, or the example the template
                // writes. A deliberate hostname is the admin's, and gets
                // left alone.
                if !self.address_is_usable() {
                    self.game_address = format!("{ip}:{}", self.game_port());
                    let msg = format!("{} {ip}", self.t(Key::PublicAddressDetected));
                    self.notify(msg, th::MOSS);
                }
                // Start was pressed before this answer arrived.
                if self.publish_when_addressed && self.address_is_usable() {
                    self.publish_when_addressed = false;
                    self.start_serving();
                }
            }
            Msg::LinkChecked(detail) => {
                self.link_ok = Some(true);
                let msg = format!("{} {detail}", self.t(Key::LinkReachable));
                self.notify(msg, th::MOSS);
            }
            Msg::Error(e) => {
                // The address check runs by itself every so often. A
                // provider hiccup there is not news the admin asked for.
                if chan == Chan::Ip {
                    return;
                }
                if self.checking_link {
                    self.link_ok = Some(false);
                }
                self.notify(e, th::BLOOD_LIT);
            }
            Msg::Idle => {
                if chan == Chan::Ip {
                    self.ip_rx = None;
                } else if chan == Chan::Serve {
                    self.serving_at = None;
                    self.stop_serving = None;
                    self.serve_rx = None;
                } else {
                    // A one-shot job is over. Whether the live server is
                    // up has nothing to do with it: tying the two left
                    // every button disabled for the rest of the session.
                    self.busy = false;
                    self.checking_link = false;
                    self.rx = None;
                }
            }
        }
    }

    /// Drain one channel into `out`. Returns true when the sender is gone.
    fn collect(rx: Option<&Receiver<Msg>>, out: &mut Vec<(Msg, Chan)>, chan: Chan) -> bool {
        let Some(rx) = rx else { return false };
        loop {
            match rx.try_recv() {
                Ok(m) => out.push((m, chan)),
                Err(mpsc::TryRecvError::Empty) => return false,
                Err(mpsc::TryRecvError::Disconnected) => return true,
            }
        }
    }
}

/// The prompt verb a button stands for, so a click and a typed line take
/// exactly the same path.
fn verb_add(roll: players::Roll) -> &'static str {
    match roll {
        players::Roll::Admin => "admin",
        players::Roll::Banned => "ban",
        players::Roll::Permitted => "permit",
    }
}

fn verb_remove(roll: players::Roll) -> &'static str {
    match roll {
        players::Roll::Admin => "unadmin",
        players::Roll::Banned => "unban",
        players::Roll::Permitted => "unpermit",
    }
}

/// How a mod is named in `exclude`: a folder and everything under it, or the
/// single file of a loose plugin.
fn exclude_pattern(name: &str, loose: bool) -> String {
    if loose {
        format!("BepInEx/plugins/{name}")
    } else {
        format!("BepInEx/plugins/{name}/**")
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.take_dropped_files(ctx);
        if self
            .ip_checked
            .is_none_or(|at| at.elapsed() >= POLL_PUBLIC_IP)
        {
            self.detect_public_ip();
        }
        self.autosave();
        self.poll_game_server();
        self.follow_game_with_publishing();
        if self.tab == Tab::Status {
            self.poll_log();
        }
        // The files are Valheim's, and an admin may well have a text editor
        // open on one. Read them when the tab is looked at rather than
        // trusting a copy taken at start-up.
        if self.tab == Tab::Players && self.lists_checked.elapsed() >= POLL_LISTS {
            self.lists_checked = Instant::now();
            self.refresh_lists();
        }
        if self.busy || self.serving_at.is_some() || self.tab == Tab::Status {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else {
            ctx.request_repaint_after(POLL_GAME_SERVER);
        }
        if let Some((_, _, since)) = &self.notice
            && since.elapsed() > NOTICE_TTL
        {
            self.notice = None;
        }

        chrome::clamp_to_display(ctx);
        chrome::handle_edge_resize(ctx);
        self.update_title(ctx);
        self.top_bar(ctx);
        self.bottom_bar(ctx);
        self.console_panel(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                th::backdrop(ui.ctx(), ui.painter(), ui.max_rect().expand(18.0));
                let (status, mods, players, notes, settings) = (
                    self.t(Key::TabStatus),
                    self.t(Key::TabMods),
                    self.t(Key::TabPlayers),
                    self.t(Key::TabNotes),
                    self.t(Key::TabSettings),
                );
                w::tabs(
                    ui,
                    &mut self.tab,
                    &[
                        (Tab::Status, status),
                        (Tab::Mods, mods),
                        (Tab::Players, players),
                        (Tab::Notes, notes),
                        (Tab::Settings, settings),
                    ],
                );
                ui.add_space(12.0);
                // Nothing is published from a configuration that was never
                // written: the publisher reads it from disk. Saving is
                // automatic, so reaching here means something is stopping it,
                // and that is worth more than a line at the bottom.
                if self.never_saved && self.save_error.is_some() {
                    w::notice(ui, th::GOLD, self.t(Key::NeverSavedWarning));
                    ui.add_space(12.0);
                }
                if let Some(path) = self.new_key_at.clone() {
                    let text = format!(
                        "{}
{path}",
                        self.t(Key::NewKeyGenerated)
                    );
                    w::notice(ui, th::GOLD, &text);
                    ui.add_space(12.0);
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        Tab::Status => {
                            self.card_status(ui);
                            ui.add_space(12.0);
                            self.card_logs(ui);
                            ui.add_space(12.0);
                            // Operating the server, not configuring it: a
                            // world is backed up before a change, which is a
                            // thing done on the tab where changes are made.
                            self.card_world(ui);
                        }
                        Tab::Mods => self.card_mods(ui),
                        Tab::Players => self.card_players(ui),
                        Tab::Notes => self.card_notes(ui),
                        Tab::Settings => {
                            self.card_server_folder(ui);
                            ui.add_space(12.0);
                            self.card_identity(ui);
                            ui.add_space(12.0);
                            self.card_publish(ui);
                            ui.add_space(12.0);
                            self.card_invite(ui);
                            ui.add_space(12.0);
                            self.card_wizard(ui);
                        }
                    });
            });
        chrome::draw_border(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(stop) = self.stop_serving.take() {
            let _ = stop.send(());
        }
    }
}

// ---- cards ---------------------------------------------------------------

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .frame(
                egui::Frame::new()
                    .fill(th::NIGHT)
                    .inner_margin(egui::Margin {
                        left: 18,
                        right: 0,
                        top: 10,
                        bottom: 12,
                    })
                    .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT)),
            )
            .show(ctx, |ui| {
                chrome::draggable(ui, ui.max_rect());
                ui.horizontal(|ui| {
                    w::header(
                        ui,
                        "V A L H S Y N C   ·   S E R V E U R",
                        Some(concat!("v", env!("CARGO_PKG_VERSION"), " · alpha")),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        chrome::window_controls(ui);
                        ui.add_space(8.0);
                        // Each language named in itself: "Deutsch", not
                        // "German". Somebody who has landed in a window they
                        // cannot read needs to recognise their own word for
                        // their own language, not ours for it. The choice
                        // goes into the configuration through `edited()`,
                        // like every other field, and autosave writes it.
                        egui::ComboBox::from_id_salt("language")
                            .selected_text(self.lang.name())
                            .width(124.0)
                            .show_ui(ui, |ui| {
                                for lang in Lang::ALL {
                                    ui.selectable_value(&mut self.lang, lang, lang.name());
                                }
                            });
                    });
                });
            });
    }

    fn bottom_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom")
            .frame(
                egui::Frame::new()
                    .fill(th::PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT)),
            )
            .show(ctx, |ui| {
                if let Some((message, color, _)) = self.notice.clone() {
                    ui.horizontal_wrapped(|ui| {
                        w::dot(ui, color);
                        ui.label(RichText::new(message).color(th::BONE));
                    });
                    ui.add_space(6.0);
                }
                // The console line lives down here, where a prompt belongs:
                // always in reach whichever tab is open, and out of the card
                // that describes the server rather than drives it.
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    // No Save button. What an admin changes here is what the
                    // server publishes, and asking them to confirm it twice
                    // only produced servers running on a configuration that
                    // was on screen but never on disk.
                    if let Some(why) = self.save_error.clone() {
                        w::dot(ui, th::BLOOD_LIT);
                        ui.label(
                            RichText::new(format!("{} {why}", self.t(Key::NotSaved)))
                                .small()
                                .color(th::BLOOD_LIT),
                        );
                    } else if self.dirty() {
                        w::dot(ui, th::GOLD);
                        ui.label(
                            RichText::new(self.t(Key::Saving))
                                .small()
                                .color(th::BONE_DIM),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.busy {
                            ui.add(egui::Spinner::new().color(th::GOLD));
                        }
                        // Only the file name: the full path is long enough to
                        // push everything else off the bar.
                        let name = self
                            .config_path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(name)
                                .text_style(th::label_style())
                                .color(th::BONE_DIM),
                        )
                        .on_hover_text(self.config_path.display().to_string());
                    });
                });
            });
    }

    /// What the server is doing right now. Everything here changes on its
    /// own; nothing here is a setting.
    fn card_status(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionGameServer));

            if self.server_outdated {
                w::notice(ui, th::BLOOD_LIT, self.t(Key::ServerUpdatePending));
                ui.add_space(8.0);
            }

            let stopping = self.stop_requested.is_some() && self.game_running;
            ui.horizontal(|ui| {
                w::status_dot_lit(
                    ui,
                    if self.game_running {
                        th::MOSS
                    } else {
                        th::GOLD.gamma_multiply(0.25)
                    },
                    if stopping {
                        self.t(Key::StoppingSavingWorld)
                    } else if self.game_running {
                        self.t(Key::Online)
                    } else {
                        self.t(Key::Offline)
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    // Stacked, not side by side: Restart beside Stop reads as
                    // the pair of equals it is not. One is what an admin
                    // reaches for; the other is underneath it.
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 0.0),
                        Layout::top_down(Align::Max),
                        |ui| self.server_buttons(ui, stopping),
                    );
                });
            });

            ui.add_space(6.0);
            self.publishing_line(ui);
            ui.add_space(6.0);
            if self.game_running {
                self.session_facts(ui);
            } else if let Some(ip) = self.public_ip.clone() {
                ui.label(
                    RichText::new(format!("{} {ip}", self.t(Key::PublicIp)))
                        .text_style(th::label_style())
                        .color(th::RUNE),
                );
            }
            ui.add_space(8.0);
            if self.scripts.is_empty() {
                w::notice(ui, th::GOLD, self.t(Key::NoStartScript));
            } else if let Some(s) = self.scripts.get(self.script_index) {
                w::hint(
                    ui,
                    &format!(
                        "{} {}",
                        self.t(Key::StartedBy),
                        s.path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                );
            }
            ui.add_space(8.0);
            w::hint(ui, self.t(Key::StartHint));
            w::hint(ui, self.t(Key::StopHint));
        });
    }

    /// Whether players can actually fetch the pack, on the tab that opens.
    ///
    /// It used to be visible only on the publishing card, two tabs away: an
    /// admin watching a healthy green "Online" had no way to see that nothing
    /// was being served, and the launcher's "cannot reach the server" was the
    /// first they heard of it.
    fn publishing_line(&mut self, ui: &mut egui::Ui) {
        if self.mode != PublishMode::Live {
            return;
        }
        let label = self.t(Key::Publishing);
        if let Some(url) = self.serving_at.clone() {
            ui.horizontal(|ui| {
                w::dot(ui, th::MOSS);
                ui.label(
                    RichText::new(format!("{label} · {url}"))
                        .text_style(th::label_style())
                        .color(th::RUNE),
                );
            });
        } else {
            {
                let why = self.publish_error.clone().unwrap_or_else(|| {
                    if self.game_running {
                        self.t(Key::PublishStarting).to_string()
                    } else {
                        self.t(Key::PublishFollowsGame).to_string()
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    w::dot(ui, th::GOLD);
                    ui.label(
                        RichText::new(format!("{label} · {} — {why}", self.t(Key::OfflineLower)))
                            .text_style(th::label_style())
                            .color(if self.publish_error.is_some() {
                                th::BLOOD_LIT
                            } else {
                                th::BONE_DIM
                            }),
                    );
                });
            }
        }
    }

    /// The console: what the server says, what the admin types, and what
    /// ValhSync answers.
    ///
    /// Its own panel rather than a strip in the bottom bar, and resizable,
    /// because the useful thing to do with a console is read back through it.
    /// Drag its top edge.
    ///
    /// The server half was always there and never visible: Valheim tags what
    /// it prints to its console in the log, perhaps fifteen lines among the
    /// tens of thousands a session writes, and often outside the window the
    /// log view reads at all.
    fn console_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("console")
            .resizable(true)
            .default_height(164.0)
            .min_height(96.0)
            .max_height(ctx.screen_rect().height() * 0.7)
            .frame(
                egui::Frame::new()
                    .fill(th::PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT)),
            )
            .show(ctx, |ui| {
                w::section(ui, self.t(Key::SectionServerConsole));
                // The prompt first, then the transcript filling what is left,
                // so dragging the panel taller gives the extra height to the
                // thing worth reading.
                self.console_line(ui);
                ui.add_space(6.0);
                self.console_transcript(ui);
            });
    }

    fn console_transcript(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(th::NIGHT)
            .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT))
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.console.is_empty() {
                            w::hint(ui, self.t(Key::ConsoleTip));
                            return;
                        }
                        for (echo, line) in &self.console {
                            let (text, colour) = match echo {
                                // The admin's own words, marked the way a
                                // prompt marks them, so the panel reads as a
                                // conversation and not as more log.
                                Echo::Sent => (format!("> {line}"), th::GOLD),
                                Echo::Answer => (line.clone(), th::BONE),
                                Echo::Server => (line.clone(), th::RUNE),
                            };
                            ui.label(RichText::new(text).monospace().small().color(colour));
                        }
                    });
            });
    }

    /// One line, carried out against the files Valheim reads.
    fn console_line(&mut self, ui: &mut egui::Ui) {
        let hint = self.t(Key::ConsolePrompt);
        let send_label = self.t(Key::Send);
        let tip = self.t(Key::ConsoleTip);
        ui.horizontal(|ui| {
            let send = ui
                .add(egui::Button::new(send_label))
                .on_hover_text(tip)
                .clicked();
            let typed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.command)
                        .desired_width(ui.available_width())
                        .font(egui::TextStyle::Monospace)
                        .hint_text(hint),
                )
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if send || typed {
                let line = std::mem::take(&mut self.command);
                if !line.trim().is_empty() {
                    self.run_command(&line);
                }
            }
        });
    }

    /// Parse it, do it, and put every word of it in the transcript.
    ///
    /// Errors are answers too: an admin who mistypes an id wants to see what
    /// they typed and what was wrong with it, in the place they typed it.
    fn run_command(&mut self, line: &str) {
        self.record_console(Echo::Sent, line.trim().to_string());
        let Some(dir) = self.lists_dir.clone() else {
            let why = self.t(Key::NoListsDir).to_string();
            self.record_console(Echo::Answer, why);
            return;
        };
        let said = players::parse(line).and_then(|cmd| players::run(&dir, cmd));
        match said {
            Ok(lines) => {
                for line in lines {
                    self.record_console(Echo::Answer, line);
                }
                self.refresh_lists();
            }
            Err(e) => self.record_console(Echo::Answer, format!("{e:#}")),
        }
    }

    /// The three files Valheim reads, one card each.
    ///
    /// A list rather than a prompt, because a prompt makes somebody guess a
    /// syntax and a list shows them the state. The prompt is still there, on
    /// the console, for anybody who would rather type.
    fn card_players(&mut self, ui: &mut egui::Ui) {
        if self.lists_dir.is_none() {
            th::card(ui, |ui| {
                ui.set_width(ui.available_width());
                w::notice(ui, th::GOLD, self.t(Key::NoListsDir));
            });
            return;
        }
        if let Some(error) = self.lists_error.clone() {
            th::card(ui, |ui| {
                ui.set_width(ui.available_width());
                w::notice(ui, th::BLOOD_LIT, &error);
            });
            ui.add_space(12.0);
        }
        for (slot, roll) in players::Roll::ALL.into_iter().enumerate() {
            self.card_one_list(ui, slot, roll);
            ui.add_space(12.0);
        }
        self.card_seen(ui);
        ui.add_space(12.0);
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::hint(ui, self.t(Key::PlayersIntro));
        });
    }

    /// Everyone the server has ever logged, so an id can be picked instead of
    /// typed.
    ///
    /// Typing seventeen digits by hand is how the wrong person gets banned.
    fn card_seen(&mut self, ui: &mut egui::Ui) {
        if self.known_names.is_empty() {
            return;
        }
        let mut pick = None;
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SeenPlayers));
            w::hint(ui, self.t(Key::NameFromLog));
            ui.add_space(6.0);
            let mut rows: Vec<(&String, &String)> = self.known_names.iter().collect();
            rows.sort_by_key(|(_, name)| name.to_lowercase());
            for (id, name) in rows {
                ui.horizontal(|ui| {
                    w::dot(ui, th::RUNE);
                    ui.label(RichText::new(name).color(th::BONE));
                    ui.label(RichText::new(id).monospace().small().color(th::BONE_DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(self.t(Key::Copy)).clicked() {
                            pick = Some(id.clone());
                        }
                    });
                });
            }
        });
        if let Some(id) = pick {
            // Into the field the buttons above read from, rather than to the
            // clipboard: the next thing the admin does is press Add.
            self.player_id = id;
        }
    }

    fn card_one_list(&mut self, ui: &mut egui::Ui, slot: usize, roll: players::Roll) {
        let section = match roll {
            players::Roll::Admin => Key::SectionAdmins,
            players::Roll::Banned => Key::SectionBanned,
            players::Roll::Permitted => Key::SectionPermitted,
        };
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(section));
            // Iron Gate's warning, on the card it belongs to. It is the one
            // that empties a server when nobody reads it.
            if roll == players::Roll::Permitted {
                w::notice(ui, th::GOLD, self.t(Key::PermittedWarning));
                ui.add_space(6.0);
            }
            w::hint(ui, roll.file_name());
            ui.add_space(6.0);

            let mut drop = None;
            if self.lists[slot].is_empty() {
                w::hint(ui, self.t(Key::ListEmpty));
            } else {
                for id in &self.lists[slot] {
                    ui.horizontal(|ui| {
                        // The name first when there is one: an admin decides
                        // about a person, and seventeen digits are not one.
                        if let Some(name) = self.known_names.get(id) {
                            ui.label(RichText::new(name).color(th::BONE));
                            ui.label(RichText::new(id).monospace().small().color(th::BONE_DIM));
                        } else {
                            ui.label(RichText::new(id).monospace().color(th::BONE));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button(self.t(Key::RemoveFromList)).clicked() {
                                drop = Some(id.clone());
                            }
                        });
                    });
                }
            }
            ui.add_space(6.0);
            let mut add = None;
            ui.horizontal(|ui| {
                if ui.button(self.t(Key::AddToList)).clicked() {
                    add = Some(self.player_id.clone());
                }
                let id_hint = self.t(Key::PlayerIdHint);
                let entered = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.player_id)
                            .desired_width(ui.available_width())
                            .font(egui::TextStyle::Monospace)
                            .hint_text(id_hint),
                    )
                    .lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if entered {
                    add = Some(self.player_id.clone());
                }
            });

            // Both go through the console, so every change an admin makes has
            // one place it is written down -- including the note about
            // whether a running server picks it up.
            if let Some(id) = drop {
                self.run_command(&format!("{} {id}", verb_remove(roll)));
            }
            if let Some(id) = add
                && !id.trim().is_empty()
            {
                self.player_id.clear();
                self.run_command(&format!("{} {id}", verb_add(roll)));
            }
        });
    }

    /// Add one line to the console, oldest dropped first.
    ///
    /// Deep enough to hold what `help` prints and a long list after it, which
    /// is what an admin scrolls back through.
    fn record_console(&mut self, echo: Echo, line: String) {
        const KEEP: usize = 400;
        if self.console.len() == KEEP {
            self.console.pop_front();
        }
        self.console.push_back((echo, line));
    }

    /// Re-read the three files from disk.
    fn refresh_lists(&mut self) {
        let root = self.cfg.pack.server_root.clone();
        let args = self.scripts.get(self.script_index).map(|s| s.args.clone());
        self.lists_dir = root.and_then(|root| players::lists_dir(&root, args.as_ref()));
        let Some(dir) = self.lists_dir.clone() else {
            self.lists_error = None;
            return;
        };
        self.lists_error = None;
        // The same log the console follows. Cheap enough to re-read on the
        // lists' own two-second tick, and it means a player who joined a
        // minute ago has a name here.
        if let Some(path) = self.log_sources.get(self.log_index) {
            self.known_names = names::seen_in_file(path)
                .unwrap_or_default()
                .into_iter()
                .map(|seen| (seen.id, seen.name))
                .collect();
        }
        for (slot, roll) in players::Roll::ALL.iter().enumerate() {
            match players::read(&dir, *roll) {
                Ok(ids) => self.lists[slot] = ids,
                Err(e) => self.lists_error = Some(format!("{e:#}")),
            }
        }
    }

    /// What the log says about the session: players, join code, which
    /// Valheim, and when the world was last written.
    fn session_facts(&mut self, ui: &mut egui::Ui) {
        let mut facts = Vec::new();
        if let Some(n) = self.session.players {
            facts.push(format!(
                "{n} {}",
                if n == 1 {
                    self.t(Key::PlayerOnline)
                } else {
                    self.t(Key::PlayersOnline)
                }
            ));
        }
        if let Some(code) = &self.session.join_code {
            facts.push(format!("{} {code}", self.t(Key::JoinCode)));
        }
        if let Some(version) = &self.game_version {
            facts.push(format!("Valheim {version}"));
        }
        if let Some(ip) = &self.public_ip {
            facts.push(format!("{} {ip}", self.t(Key::PublicIp)));
        }
        if let Some(pid) = self.game_pid {
            facts.push(format!("{} {pid}", self.t(Key::Process)));
        }
        if let Some(saved) = self.world_saved {
            facts.push(format!("{} {}", self.t(Key::SavedAgo), Self::ago(saved)));
        }
        if facts.is_empty() {
            w::hint(ui, self.t(Key::WaitingSessionLine));
        } else {
            ui.label(
                RichText::new(facts.join("   ·   "))
                    .text_style(th::label_style())
                    .color(th::RUNE),
            );
        }
    }

    /// One button. Which one it is, the lamp beside it has already said.
    fn server_buttons(&mut self, ui: &mut egui::Ui, stopping: bool) {
        if stopping {
            let label = if self.restart_after_stop {
                self.t(Key::Restarting)
            } else {
                self.t(Key::Stopping)
            };
            ui.add_enabled(
                false,
                egui::Button::new(RichText::new(label).color(th::BONE_DIM)),
            );
            return;
        }
        if self.game_running {
            if ui
                .add(egui::Button::new(
                    RichText::new(self.t(Key::StopAndSave)).color(th::BONE),
                ))
                .clicked()
            {
                self.request_stop(false);
            }
            ui.add_space(4.0);
            // Quieter than Stop, and under it: same Ctrl+C, same saved world,
            // and the window brings the server back up once the process has
            // actually gone.
            if ui
                .small_button(self.t(Key::Restart))
                .on_hover_text(self.t(Key::RestartHint))
                .clicked()
            {
                self.request_stop(true);
            }
            return;
        }
        let can_start = !self.scripts.is_empty();
        if ui
            .add_enabled(
                can_start,
                egui::Button::new(
                    RichText::new(self.t(Key::Start))
                        .strong()
                        .color(if can_start { th::NIGHT } else { th::BONE_DIM }),
                )
                .fill(if can_start { th::GOLD } else { th::LEATHER }),
            )
            .on_disabled_hover_text(self.t(Key::NoStartScriptHint))
            .clicked()
        {
            self.start_everything();
        }
    }

    /// Ask the dedicated server to stop, optionally to start it again after.
    ///
    /// Both buttons go through here because both are the same Ctrl+C: the
    /// difference is only whether the window brings it back once the process
    /// has gone, which it cannot know until it has.
    fn request_stop(&mut self, then_restart: bool) {
        match gameserver::stop() {
            Ok(()) => {
                self.stop_requested = Some(Instant::now());
                self.restart_after_stop = then_restart;
                let msg = if then_restart {
                    self.t(Key::StopThenRestartSent)
                } else {
                    self.t(Key::StopSent)
                }
                .to_string();
                self.notify(msg, th::MOSS);
            }
            Err(e) => {
                self.restart_after_stop = false;
                self.notify(format!("{e:#}"), th::BLOOD_LIT);
            }
        }
    }

    /// One press brings the whole thing up: the game server in its window,
    /// the public address filled in when what is there cannot work, and
    /// publishing online behind it. An admin who starts a server means to be
    /// joinable, and being joinable takes all three -- a server running
    /// beside a pack nobody can fetch is the state this tool exists to avoid.
    fn start_everything(&mut self) {
        let Some(path) = self.scripts.get(self.script_index).map(|s| s.path.clone()) else {
            return;
        };
        match gameserver::start(&gameserver::Launch(path)) {
            Ok(shell) => self.game_shell = Some(shell),
            Err(e) => {
                self.notify(format!("{e:#}"), th::BLOOD_LIT);
                return;
            }
        }
        self.game_running = true;
        self.game_checked = Instant::now();
        self.game_pid = None;
        self.stop_requested = None;
        let msg = self.t(Key::GameServerStarted).to_string();
        self.notify(msg, th::MOSS);

        // Pressing Start is a fresh intent: it undoes an earlier stop.
        self.publish_paused = false;
        self.publish_tried = None;
        self.follow_game_with_publishing();
    }

    /// Is the dedicated server up? Everything the window says about a
    /// session hangs off the answer.
    fn poll_game_server(&mut self) {
        if self.game_checked.elapsed() < POLL_GAME_SERVER {
            return;
        }
        let was = self.game_running;
        self.game_pid = gameserver::pid();
        self.game_running = self.game_pid.is_some();
        self.game_checked = Instant::now();
        if was == self.game_running {
            return;
        }
        // A server that has just started writes a log that did not exist.
        self.refresh_log_sources();
        if self.game_running {
            // A restart is a fresh intent, like pressing Start.
            self.publish_paused = false;
            self.publish_tried = None;
            return;
        }
        self.session = logs::Session::default();
        // Whether this exit was one we asked for. Read before it is cleared,
        // because it is the whole difference between a crash and a Stop.
        let was_asked_for = self.stop_requested.is_some() || self.restart_after_stop;
        self.stop_requested = None;
        // The game is gone; the shell that ran the script is only a question
        // waiting for an answer.
        if self
            .game_shell
            .as_mut()
            .is_some_and(gameserver::Shell::close_if_game_gone)
        {
            self.game_shell = None;
        }
        // The world is written and the process has gone: the other half of a
        // restart. Waiting for that rather than sleeping is the point --
        // Valheim takes as long as its world takes.
        if std::mem::take(&mut self.restart_after_stop) {
            self.start_everything();
            return;
        }
        if !was_asked_for {
            self.restart_after_crash();
        }
    }

    /// Bring a server back that nobody asked to stop.
    ///
    /// Only when the admin has turned it on, and only for an exit this window
    /// did not ask for -- a Stop or a Restart from here is a deliberate act,
    /// and a server that comes back from one of those is fighting its admin.
    /// A Ctrl+C typed in the server's own console is indistinguishable from a
    /// crash out here; it restarts, and the setting's own description says so.
    ///
    /// Bounded, because the failure this exists for is also the failure that
    /// loops. A server that dies three times inside the window below is not
    /// crashing, it is broken -- a mod that will not load, a port already
    /// taken, a world it cannot open -- and restarting it a fourth time only
    /// buries the reason further up the log.
    fn restart_after_crash(&mut self) {
        const GIVE_UP_AFTER: usize = 3;
        const FORGET_AFTER: Duration = Duration::from_secs(20 * 60);

        if !self.cfg.game_server.restart_on_crash || self.scripts.is_empty() {
            return;
        }
        let now = Instant::now();
        self.crashes
            .retain(|at| now.duration_since(*at) < FORGET_AFTER);
        if self.crashes.len() >= GIVE_UP_AFTER {
            let msg = self.t(Key::CrashLoop).to_string();
            self.notify(msg, th::BLOOD_LIT);
            return;
        }
        self.crashes.push(now);
        let msg = self.t(Key::RestartingAfterCrash).to_string();
        self.notify(msg, th::GOLD);
        self.start_everything();
    }

    /// Publishing follows the game server.
    ///
    /// Not a chain hung off the button: the server is just as often already
    /// running when the window opens, or started from its own shortcut, and
    /// in both of those there is no click to hang anything off. A running
    /// game server beside a pack nobody can fetch is the exact state this
    /// tool exists to prevent, so the window keeps reconciling the two
    /// instead of waiting to be asked.
    fn follow_game_with_publishing(&mut self) {
        // Static publishing is a folder the admin uploads; there is nothing
        // to bring online for it.
        if self.mode != PublishMode::Live || !self.game_running {
            return;
        }
        if self.publish_paused || self.serving_at.is_some() || self.serve_rx.is_some() {
            return;
        }
        // A configuration the publisher refuses must not be retried, and
        // complained about, every couple of seconds.
        if self
            .publish_tried
            .is_some_and(|at| at.elapsed() < PUBLISH_RETRY)
        {
            return;
        }
        self.publish_tried = Some(Instant::now());
        if self.adopt_detected_ip() {
            self.start_serving();
        } else {
            self.publish_when_addressed = true;
            self.detect_public_ip();
        }
    }

    /// The live server's one button, the twin of the game server's.
    fn publish_button(&mut self, ui: &mut egui::Ui) {
        if self.serving_at.is_some() {
            if ui
                .add(egui::Button::new(
                    RichText::new(self.t(Key::Stop)).color(th::BONE),
                ))
                .clicked()
            {
                // Stopping by hand outranks following the game server, or
                // the window would put it straight back.
                self.publish_paused = true;
                self.stop_serving();
            }
            return;
        }
        let can = !self.busy && self.serve_rx.is_none();
        if ui
            .add_enabled(
                can,
                egui::Button::new(RichText::new(self.t(Key::Start)).strong().color(if can {
                    th::NIGHT
                } else {
                    th::BONE_DIM
                }))
                .fill(if can { th::GOLD } else { th::LEATHER }),
            )
            .clicked()
        {
            self.publish_paused = false;
            self.publish_tried = None;
            self.start_serving();
        }
    }

    /// Where the dedicated server lives and which script starts it.
    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_server_folder(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionDedicatedServer));

            let hint_text = self.t(Key::DedicatedServerFolder);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.server_root)
                        .desired_width(ui.available_width() - 120.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(hint_text),
                );
                if ui.button(self.t(Key::Detect)).clicked() {
                    if let Some(p) = detect::detect_server_root() {
                        self.server_root = p.display().to_string();
                        self.pull_fields();
                        self.refresh_detection();
                        let msg = format!("{} {}", self.t(Key::DedicatedServerFound), p.display());
                        self.notify(msg, th::MOSS);
                    } else {
                        let msg = self.t(Key::DedicatedServerNotFound).to_string();
                        self.notify(msg, th::BLOOD_LIT);
                    }
                }
            });

            let root = PathBuf::from(self.server_root.trim());
            let valid = detect::looks_like_server_root(&root);
            if !self.server_root.trim().is_empty() {
                if valid {
                    let bep = detect::has_bepinex(&root);
                    w::status_dot(
                        ui,
                        if bep { th::MOSS } else { th::GOLD },
                        if bep {
                            self.t(Key::ServerWithBepInEx)
                        } else {
                            self.t(Key::ServerWithoutBepInEx)
                        },
                    );
                } else {
                    w::notice(ui, th::BLOOD_LIT, self.t(Key::NotAServerFolder));
                }
            }

            if !self.scripts.is_empty() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.t(Key::StartScript))
                            .small()
                            .color(th::BONE_DIM),
                    );
                    let current = self.scripts[self.script_index.min(self.scripts.len() - 1)]
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let mut picked = self.script_index;
                    egui::ComboBox::from_id_salt("script")
                        .selected_text(current)
                        .width(280.0)
                        .show_ui(ui, |ui| {
                            for (i, s) in self.scripts.iter().enumerate() {
                                let label = format!(
                                    "{}{}",
                                    s.path.file_name().unwrap_or_default().to_string_lossy(),
                                    if s.is_stock { "  (Steam)" } else { "" }
                                );
                                ui.selectable_value(&mut picked, i, label);
                            }
                        });
                    if picked != self.script_index {
                        self.script_index = picked;
                        self.script_chosen = true;
                        // Another script can mean another log and another world.
                        self.refresh_log_sources();
                        self.fill_recipe_from_script();
                    }
                });
                if let Some(s) = self.scripts.get(self.script_index) {
                    let a = &s.args;
                    let mut bits = Vec::new();
                    if let Some(n) = &a.name {
                        bits.push(n.clone());
                    }
                    if let Some(p) = a.port {
                        bits.push(format!("port {p}"));
                    }
                    if a.crossplay {
                        bits.push("crossplay".into());
                    }
                    match a.public {
                        Some(true) => bits.push(self.t(Key::Public).into()),
                        Some(false) => bits.push(self.t(Key::Private).into()),
                        None => {}
                    }
                    if a.has_password {
                        bits.push(self.t(Key::PasswordSet).to_string());
                    }
                    ui.label(
                        RichText::new(bits.join(" · "))
                            .text_style(th::label_style())
                            .color(th::RUNE),
                    );
                }
            }

            ui.add_space(10.0);
            th::hairline(ui);
            ui.add_space(8.0);
            let mut back = self.cfg.game_server.restart_on_crash;
            if ui
                .checkbox(&mut back, self.t(Key::RestartOnCrash))
                .changed()
            {
                // It lives in `cfg` rather than in a widget field, and
                // `edited()` clones `cfg`, so autosave sees this on its own.
                self.cfg.game_server.restart_on_crash = back;
            }
            w::hint(ui, self.t(Key::RestartOnCrashHint));
        });
    }

    /// The world, and copies of it.
    ///
    /// ValhSync backs up every file it puts on a player's machine, journals
    /// the change and can roll it back exactly. It has never done anything for
    /// the one file on the admin's machine that cannot be downloaded again.
    /// Adding or updating a mod is precisely when a world gets corrupted.
    fn card_world(&mut self, ui: &mut egui::Ui) {
        let Some(root) = self.cfg.pack.server_root.clone() else {
            return;
        };
        let args = self.scripts.get(self.script_index).map(|s| s.args.clone());
        let world = args.as_ref().and_then(|a| logs::world_save(&root, a));
        let into = worldbackup::dir_for(&root, args.as_ref());

        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionWorld));
            w::hint(ui, self.t(Key::BackupHint));
            ui.add_space(6.0);

            let Some(world) = world else {
                w::notice(ui, th::GOLD, self.t(Key::NoWorldFound));
                return;
            };
            ui.label(
                RichText::new(world.display().to_string())
                    .monospace()
                    .small()
                    .color(th::RUNE),
            );
            // Said before the click, not after: whether the copy can be
            // trusted is the one thing worth knowing beforehand.
            if let Some(warning) = worldbackup::trust_now().warning() {
                ui.add_space(4.0);
                w::notice(ui, th::GOLD, warning);
            }
            ui.add_space(6.0);

            let Some(into) = into else {
                return;
            };
            let mut take = false;
            ui.horizontal(|ui| {
                take = ui.button(self.t(Key::BackupWorld)).clicked();
                if let Ok(kept) = worldbackup::list(&into) {
                    ui.label(
                        RichText::new(format!("{} {}", kept.len(), self.t(Key::BackupsKept)))
                            .text_style(th::label_style())
                            .color(th::BONE_DIM),
                    );
                    if !kept.is_empty() && ui.small_button(self.t(Key::Open)).clicked() {
                        open_path(&into);
                    }
                }
            });
            if take {
                match worldbackup::take(&world, &into, "by hand") {
                    Ok(done) => {
                        // Ten is a lot of worlds and not much disk, and the
                        // oldest is the least likely to be wanted.
                        let _ = worldbackup::prune(&into, 10);
                        let msg = self.t(Key::BackupTaken).replacen(
                            "{}",
                            &done.path.display().to_string(),
                            1,
                        );
                        self.notify(msg, th::MOSS);
                    }
                    Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
                }
            }
        });
    }

    fn card_identity(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionIdentity));

            let width = (ui.available_width() - 24.0).max(200.0);
            w::field(ui, self.t(Key::NameShownToPlayers), &mut self.name, width);
            ui.add_space(6.0);

            ui.label(
                RichText::new(self.t(Key::GameAddressLabel))
                    .small()
                    .color(th::BONE_DIM),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.game_address)
                        .desired_width(width - 200.0)
                        .font(egui::TextStyle::Monospace),
                );
                let detected = self
                    .public_ip
                    .clone()
                    .map(|ip| format!("{ip}:{}", self.game_port()));
                match detected {
                    Some(addr) if addr != self.game_address.trim() => {
                        if ui
                            .button(self.t(Key::UseDetectedIp))
                            .on_hover_text(format!("{addr}  ·  {}", worker::IP_ECHO_SERVICE))
                            .clicked()
                        {
                            self.game_address = addr;
                        }
                    }
                    Some(_) => {
                        ui.label(
                            RichText::new(self.t(Key::MatchesPublicIp))
                                .small()
                                .color(th::MOSS),
                        )
                        .on_hover_text(worker::IP_ECHO_SERVICE);
                    }
                    None => {
                        ui.label(
                            RichText::new(self.t(Key::DetectingIp))
                                .small()
                                .color(th::BONE_DIM),
                        )
                        .on_hover_text(worker::IP_ECHO_SERVICE);
                    }
                }
            });

            let addr = self.game_address.trim();
            let crossplay = self
                .scripts
                .get(self.script_index)
                .is_some_and(|s| s.args.crossplay);
            if !addr.is_empty() && valhsync_core::manifest::is_private_host(addr) {
                ui.add_space(4.0);
                w::notice(
                    ui,
                    th::BLOOD_LIT,
                    if crossplay {
                        self.t(Key::LocalAddressCrossplay)
                    } else {
                        self.t(Key::LocalAddress)
                    },
                );
            } else if !addr.is_empty() && !valhsync_core::manifest::is_valid_game_address(addr) {
                ui.add_space(4.0);
                w::notice(ui, th::BLOOD_LIT, self.t(Key::InvalidAddress));
            }
        });
    }

    fn card_mods(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            // First, not last. Adding a mod is what an admin opens this tab
            // to do; the list below is what they check afterwards.
            self.drop_zone(ui);
            ui.add_space(10.0);
            th::hairline(ui);
            ui.add_space(8.0);
            self.mod_list(ui);
        });
    }

    /// The mods that are installed but turned off.
    ///
    /// Shown rather than hidden: a mod that has vanished from the list and
    /// left no trace is a mod an admin reinstalls a week later, wondering why
    /// it was not there.
    fn disabled_mods(&mut self, ui: &mut egui::Ui, act: &mut Option<(String, bool, Act)>) {
        let Some(root) = self.cfg.pack.server_root.clone() else {
            return;
        };
        let off = install::disabled(&root);
        if off.is_empty() {
            return;
        }
        ui.add_space(10.0);
        th::hairline(ui);
        ui.add_space(8.0);
        ui.label(
            RichText::new(self.t(Key::DisabledMods))
                .font(th::display_font(13.0))
                .color(th::GOLD_LIT),
        );
        w::hint(ui, self.t(Key::DisabledHint));
        ui.add_space(4.0);
        for name in off {
            ui.horizontal(|ui| {
                w::dot(ui, th::EDGE);
                ui.label(RichText::new(&name).color(th::BONE_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button(self.t(Key::Enable)).clicked() {
                        *act = Some((name.clone(), false, Act::Enable));
                    }
                });
            });
        }
    }

    /// Carry out what a row's button asked for, and say where things went.
    fn do_mod_action(&mut self, folder: &str, loose: bool, what: Act) {
        let Some(root) = self.cfg.pack.server_root.clone() else {
            return;
        };
        let done = match what {
            Act::Disable => install::disable(&root, folder, loose).map(|_| None),
            Act::Enable => install::enable(&root, folder).map(|_| None),
            Act::Remove => install::remove(&root, folder, loose).map(Some),
        };
        match done {
            Ok(gone) => {
                // Where it went, not just that it went. "Removed" on its own
                // reads as "deleted", and nothing here deletes anything.
                let msg = gone.map_or_else(
                    || format!("{folder} \u{b7} {}", self.t(Key::DisabledHint)),
                    |path| {
                        self.t(Key::RemovedTo)
                            .replacen("{}", &path.display().to_string(), 1)
                    },
                );
                self.notify(msg, th::MOSS);
                self.mods = self.collect_mods();
            }
            Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
        }
    }

    /// The note to players, on its own. It is written at a different moment
    /// from the one where mods are chosen -- after, when there is something
    /// to say about them -- and it wants the room to say it.
    fn card_notes(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            self.pack_notes(ui);
        });
    }

    /// Where a mod is dropped to be installed.
    ///
    /// The manual version is a download, a guess at whether the files go at
    /// the root or under `plugins/`, a folder named by hand, and remembering
    /// to delete the old version first. Each of those is a way to end up with
    /// two versions of one mod loaded, which BepInEx settles by refusing both.
    fn drop_zone(&mut self, ui: &mut egui::Ui) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let (title, hint) = (self.t(Key::DropTitle), self.t(Key::DropHint));
        let label = if hovering {
            self.t(Key::DropHover)
        } else {
            title
        };

        let colour = if hovering { th::GOLD_LIT } else { th::GOLD };
        th::callout(ui, colour, |ui| {
            ui.label(RichText::new(label).strong().color(colour));
            w::hint(ui, hint);
        });
    }

    /// Install whatever was dropped on the window, wherever it was dropped.
    ///
    /// On the window rather than on the rectangle: somebody dragging a file
    /// aims at the words, not at a hit box, and a drop that lands two pixels
    /// outside and silently does nothing is the worst version of this.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let Some(root) = self.cfg.pack.server_root.clone() else {
            let msg = self.t(Key::DropNeedsRoot).to_string();
            self.notify(msg, th::BLOOD_LIT);
            return;
        };
        let mut installed = 0;
        for path in dropped {
            match crate::install::install(&root, &path) {
                Ok(done) => {
                    installed += 1;
                    let what = if done.replaced {
                        self.t(Key::DropReplaced)
                    } else {
                        self.t(Key::DropInstalled)
                    };
                    let msg = format!(
                        "{what} {} ({} {})",
                        done.name,
                        done.files,
                        self.t(Key::Files)
                    );
                    self.notify(msg, th::MOSS);
                }
                Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
            }
        }
        if installed > 0 {
            // The pack is rebuilt by the folder watcher on its own; what the
            // window has to refresh is its own idea of what is in there.
            self.mods = self.collect_mods();
            self.tab = Tab::Mods;
            let msg = self.t(Key::DropRestart).to_string();
            self.notify(msg, th::GOLD);
        }
    }

    /// The mods found on the server, and which side each one runs on.
    #[allow(clippy::too_many_lines)] // one list, read top to bottom
    fn mod_list(&mut self, ui: &mut egui::Ui) {
        w::section(ui, self.t(Key::PackMods));
        if self.mods.is_empty() {
            w::hint(ui, self.t(Key::NoModsFound));
            return;
        }
        // Say where the list comes from: it is read from the server's own
        // BepInEx folder, which is not obvious from a list of names.
        if let Some(plugins) = self
            .cfg
            .pack
            .server_root
            .as_ref()
            .map(|r| r.join("BepInEx").join("plugins"))
        {
            ui.horizontal(|ui| {
                w::hint(ui, self.t(Key::ReadFrom));
                ui.label(
                    RichText::new(plugins.display().to_string())
                        .monospace()
                        .small()
                        .color(th::RUNE),
                );
                if ui.small_button(self.t(Key::Open)).clicked() {
                    open_path(&plugins);
                }
            });
        }
        w::hint(ui, self.t(Key::ServerOnlyHint));
        ui.add_space(8.0);

        let server_only = self.mods.iter().filter(|m| m.server_only).count();
        let sent = self.mods.len() - server_only;
        let mut changed: Vec<(String, bool, bool)> = Vec::new();
        let armed = self.remove_armed.clone();
        let mut arm: Option<String> = None;
        let mut act: Option<(String, bool, Act)> = None;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{sent} {}  ·  {server_only} {}",
                    self.t(Key::SentCount),
                    self.t(Key::ServerOnlyCount)
                ))
                .text_style(th::label_style())
                .color(th::BONE_DIM),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(server_only > 0, egui::Button::new(self.t(Key::SendAll)))
                    .clicked()
                {
                    for m in self.mods.iter().filter(|m| m.server_only) {
                        changed.push((m.folder.clone(), m.loose, false));
                    }
                }
                if ui
                    .add_enabled(sent > 0, egui::Button::new(self.t(Key::NoneOfThem)))
                    .clicked()
                {
                    for m in self.mods.iter().filter(|m| !m.server_only) {
                        changed.push((m.folder.clone(), m.loose, true));
                    }
                }
            });
        });
        ui.add_space(6.0);
        th::hairline(ui);
        ui.add_space(6.0);

        // One row per mod, full width, with the choice spelled out on the
        // right rather than left to a bare tick: "sent" and "server only"
        // are the two things an admin is deciding between, and a checkbox
        // says neither of them.
        for m in &self.mods {
            ui.horizontal(|ui| {
                w::dot(ui, if m.server_only { th::GOLD } else { th::MOSS });
                ui.label(RichText::new(&m.folder).color(if m.server_only {
                    th::BONE_DIM
                } else {
                    th::BONE
                }));
                if m.client_only {
                    ui.label(
                        RichText::new(self.t(Key::ClientOnly))
                            .small()
                            .color(th::RUNE),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Removing is two clicks, and the second one says what it
                    // is: this is the only control on the tab that takes a
                    // mod away, and a stray click on a row is cheap to make.
                    let arming = armed.as_deref() == Some(m.folder.as_str());
                    let label = if arming {
                        self.t(Key::ConfirmRemove)
                    } else {
                        self.t(Key::RemoveMod)
                    };
                    let button = egui::Button::new(RichText::new(label).color(if arming {
                        th::BLOOD_LIT
                    } else {
                        th::BONE_DIM
                    }));
                    if ui.add(button).clicked() {
                        if arming {
                            act = Some((m.folder.clone(), m.loose, Act::Remove));
                        } else {
                            arm = Some(m.folder.clone());
                        }
                    }
                    if ui.small_button(self.t(Key::Disable)).clicked() {
                        act = Some((m.folder.clone(), m.loose, Act::Disable));
                    }
                    ui.add_space(8.0);
                    if ui
                        .selectable_label(m.server_only, self.t(Key::ServerOnly))
                        .clicked()
                        && !m.server_only
                    {
                        changed.push((m.folder.clone(), m.loose, true));
                    }
                    if ui
                        .selectable_label(!m.server_only, self.t(Key::SentToPlayers))
                        .clicked()
                        && m.server_only
                    {
                        changed.push((m.folder.clone(), m.loose, false));
                    }
                });
            });
        }
        self.disabled_mods(ui, &mut act);
        for (folder, loose, server_only) in changed {
            self.set_server_only(&folder, loose, server_only);
            self.mods = self.collect_mods();
        }
        if let Some(folder) = arm {
            self.remove_armed = Some(folder);
        }
        if let Some((folder, loose, what)) = act {
            self.remove_armed = None;
            self.do_mod_action(&folder, loose, what);
        }
        if let Some(extras) = self.cfg.pack.client_extras.clone() {
            ui.add_space(10.0);
            th::hairline(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new(self.t(Key::ClientExtrasTitle))
                    .font(th::display_font(13.0))
                    .color(th::GOLD_LIT),
            );
            w::hint(ui, self.t(Key::ClientExtrasHint));
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(extras.display().to_string())
                        .monospace()
                        .small()
                        .color(th::RUNE),
                );
                if ui.small_button(self.t(Key::Open)).clicked() {
                    let _ = std::fs::create_dir_all(&extras);
                    open_path(&extras);
                }
            });
        }
    }

    /// The admin's word to players, published inside the signed manifest.
    ///
    /// It sits under the mod list because this is the tab where the pack is
    /// decided: whatever players need to hear is almost always about what was
    /// just changed here, and the launcher's own diff can name the mods that
    /// moved but not what that will do to a save.
    fn pack_notes(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(self.t(Key::NotesTitle))
                .font(th::display_font(13.0))
                .color(th::GOLD_LIT),
        );
        let hint = self.t(Key::NotesHint);
        w::hint(ui, hint);
        let placeholder = self.t(Key::NotesPlaceholder);
        ui.add(
            egui::TextEdit::multiline(&mut self.notes)
                .desired_width(ui.available_width())
                .desired_rows(4)
                .hint_text(placeholder),
        );
        // Counted in bytes, as the manifest counts them: a box saying 300 left
        // while saving was refused would be worse than one whose count falls
        // two at a time on an accent.
        let used = self.notes.trim().len();
        let (text, color) = if used > MAX_NOTES {
            (
                format!("{} {}", used - MAX_NOTES, self.t(Key::TooMany)),
                th::BLOOD_LIT,
            )
        } else {
            (
                format!("{} {}", MAX_NOTES - used, self.t(Key::Left)),
                th::BONE_DIM,
            )
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(text).small().color(color));
        });

        // Under the note, because it is what carries the note out: the
        // announcement is this text plus the mods that moved.
        ui.add_space(12.0);
        th::hairline(ui);
        ui.add_space(8.0);
        ui.label(
            RichText::new(self.t(Key::DiscordTitle))
                .font(th::display_font(13.0))
                .color(th::GOLD_LIT),
        );
        w::hint(ui, self.t(Key::DiscordHint));
        ui.add_space(4.0);
        let placeholder = self.t(Key::DiscordPlaceholder);
        ui.add(
            egui::TextEdit::singleline(&mut self.discord_webhook)
                .desired_width(ui.available_width())
                .font(egui::TextStyle::Monospace)
                // Masked: an admin showing this window to somebody, or
                // screen-sharing while they set the server up, would
                // otherwise be handing out posting rights to their Discord.
                .password(true)
                .hint_text(placeholder),
        );
    }

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_publish(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionPublishing));
            let (label_export, label_live) = (self.t(Key::StaticFiles), self.t(Key::LiveServer));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, PublishMode::Export, label_export);
                ui.selectable_value(&mut self.mode, PublishMode::Live, label_live);
            });
            ui.add_space(6.0);

            match self.mode {
                PublishMode::Export => {
                    w::hint(ui, self.t(Key::ExportHint));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.export_dir)
                                .desired_width(ui.available_width() - 210.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        if ui
                            .add_enabled(!self.busy, egui::Button::new(self.t(Key::Export)))
                            .clicked()
                        {
                            self.pull_fields();
                            let cfg = self.cfg.clone();
                            let data = self.data_dir.clone();
                            let dir = PathBuf::from(self.export_dir.trim());
                            self.start_job(move |rep| {
                                rep.run_then(
                                    || worker::export(&cfg, &data, dir),
                                    |rep| worker::announce(&cfg, &data, rep),
                                );
                            });
                        }
                        if ui.button(self.t(Key::Open)).clicked() {
                            let dir = PathBuf::from(self.export_dir.trim());
                            let _ = std::fs::create_dir_all(&dir);
                            open_path(&dir);
                        }
                    });
                    w::hint(ui, self.t(Key::ExportThenUrl));
                }
                PublishMode::Live => {
                    let serving = self.serving_at.is_some();
                    ui.horizontal(|ui| {
                        w::status_dot_lit(
                            ui,
                            if serving {
                                th::MOSS
                            } else {
                                th::GOLD.gamma_multiply(0.25)
                            },
                            if serving {
                                self.t(Key::Online)
                            } else {
                                self.t(Key::Offline)
                            },
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            self.publish_button(ui);
                        });
                    });
                    ui.add_space(6.0);
                    match self.serving_at.clone() {
                        Some(url) => {
                            ui.label(RichText::new(url).monospace().small().color(th::MOSS));
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(self.t(Key::Port)).small().color(th::BONE_DIM),
                                );
                                let mut port = self.bind_port();
                                if ui
                                    .add(egui::DragValue::new(&mut port).range(1024..=65_533))
                                    .on_hover_text(self.t(Key::PortHint))
                                    .changed()
                                {
                                    self.bind = format!("0.0.0.0:{port}");
                                }
                            });
                        }
                    }
                    ui.add_space(6.0);
                    if let Some(why) = self.publish_error.clone() {
                        w::notice(
                            ui,
                            th::BLOOD_LIT,
                            &format!("{} {why}", self.t(Key::PublishCannotStart)),
                        );
                        ui.add_space(6.0);
                    }
                    w::hint(ui, self.t(Key::PublishFollowsHint));
                    w::hint(ui, self.t(Key::PublishTcpHint));
                }
            }

            ui.add_space(8.0);
            w::field(
                ui,
                self.t(Key::PublicUrlLabel),
                &mut self.public_url,
                (ui.available_width() - 24.0).max(200.0),
            );

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new(self.t(Key::ScanThePack)))
                    .clicked()
                {
                    self.pull_fields();
                    let cfg = self.cfg.clone();
                    let data = self.data_dir.clone();
                    self.start_job(move |rep| {
                        rep.run_then(
                            || worker::scan(&cfg, &data),
                            |rep| worker::announce(&cfg, &data, rep),
                        );
                    });
                }
                if let Some(s) = &self.summary {
                    ui.label(
                        RichText::new(format!(
                            "{} {} · {}",
                            s.files,
                            self.t(Key::Files),
                            human_bytes(s.bytes)
                        ))
                        .text_style(th::label_style())
                        .color(th::BONE_DIM),
                    );
                }
            });
            if let Some(s) = &self.summary
                && !s.skipped.is_empty()
            {
                ui.label(
                    RichText::new(format!("{} {}", s.skipped.len(), self.t(Key::FilesSkipped)))
                        .small()
                        .color(th::GOLD),
                );
            }
        });
    }

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_invite(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionInvite));
            w::hint(ui, self.t(Key::InviteHint));
            ui.add_space(4.0);
            if self.invite.is_empty() {
                // A code is missing for exactly one of two reasons, and the
                // admin can act on either -- as long as the window says which.
                if self.edited().public_url().is_none() {
                    w::notice(ui, th::GOLD, self.t(Key::InviteNoAddress));
                } else {
                    w::hint(ui, self.t(Key::InviteAfterSave));
                }
                return;
            }
            self.reachability(ui);
            ui.add_space(6.0);
            th::callout(ui, th::GOLD, |ui| {
                w::code_block(ui, &self.invite);
                if let Ok(key) = crate::keys::load(&self.data_dir) {
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            self.t(Key::KeyFingerprint),
                            key.public().fingerprint()
                        ))
                        .text_style(th::label_style())
                        .color(th::RUNE),
                    );
                }
                ui.add_space(6.0);
                // Against the code itself. It used to sit under the key field
                // below, which put two unrelated things between an admin and
                // the one button they came to this card for.
                let code = self.invite.clone();
                if ui
                    .add_enabled(
                        !code.is_empty(),
                        egui::Button::new(RichText::new(self.t(Key::CopyTheCode)).strong().color(
                            if code.is_empty() {
                                th::BONE_DIM
                            } else {
                                th::NIGHT
                            },
                        ))
                        .fill(if code.is_empty() {
                            th::LEATHER
                        } else {
                            th::GOLD
                        }),
                    )
                    .on_hover_text(self.t(Key::CopyTheCodeHint))
                    .clicked()
                {
                    ui.ctx().copy_text(code);
                    let msg = self.t(Key::CodeCopied).to_string();
                    self.notify(msg, th::MOSS);
                }
            });
            ui.add_space(8.0);
            self.key_takeover(ui);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new(self.t(Key::TestTheLink)))
                    .on_hover_text(self.t(Key::TestTheLinkHint))
                    .clicked()
                {
                    self.pull_fields();
                    self.link_ok = None;
                    self.checking_link = true;
                    let cfg = self.cfg.clone();
                    let data = self.data_dir.clone();
                    self.start_job(move |rep| {
                        rep.run(|| worker::check_link(&cfg, &data));
                    });
                }
                if ui
                    .button(self.t(Key::PreparePlayerFolder))
                    .on_hover_text(self.t(Key::PreparePlayerFolderHint))
                    .clicked()
                {
                    match self.prepare_player_folder() {
                        Ok(dir) => {
                            let msg =
                                format!("{} {}", self.t(Key::PlayerFolderReady), dir.display());
                            self.notify(msg, th::MOSS);
                            open_path(&dir);
                        }
                        Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
                    }
                }
            });
        });
    }
}

impl App {
    /// Build the folder an admin zips and sends: the launcher plus the invite
    /// code, so the player only has to double-click.
    /// The server's own log, followed as it is written.
    /// Take over the signing key of another install.
    ///
    /// The key is the server's identity, and an admin who moved machines, or
    /// unpacked a build somewhere new before this was sorted out, has a
    /// perfectly good one sitting in a folder. Without this the only way back
    /// was to make every player import a fresh code.
    fn key_takeover(&mut self, ui: &mut egui::Ui) {
        let label = self.t(Key::KeyTakeover);
        let hint = self.t(Key::KeyPathHint);
        let button = self.t(Key::TakeOver);
        w::hint(ui, label);
        ui.horizontal(|ui| {
            let go = ui.button(button).clicked();
            ui.add(
                egui::TextEdit::singleline(&mut self.key_import)
                    .desired_width(ui.available_width())
                    .font(egui::TextStyle::Monospace)
                    .hint_text(hint),
            );
            if go {
                let from = PathBuf::from(self.key_import.trim());
                match crate::keys::import(&self.data_dir, &from) {
                    Ok(kp) => {
                        self.key_import.clear();
                        self.new_key_at = None;
                        self.refresh_invite();
                        let msg = format!(
                            "{} {}",
                            self.t(Key::KeyTakenOver),
                            kp.public().fingerprint()
                        );
                        self.notify(msg, th::MOSS);
                    }
                    Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
                }
            }
        });
    }

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_logs(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionConsole));

            if self.log_sources.is_empty() {
                w::hint(ui, self.t(Key::NoLogFound));
                return;
            }

            ui.horizontal(|ui| {
                if self.log_sources.len() > 1 {
                    let current = self.log_sources[self.log_index]
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let mut picked = self.log_index;
                    egui::ComboBox::from_id_salt("log-source")
                        .selected_text(current)
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            for (i, path) in self.log_sources.iter().enumerate() {
                                let label = path.file_name().unwrap_or_default().to_string_lossy();
                                ui.selectable_value(&mut picked, i, label);
                            }
                        });
                    if picked != self.log_index {
                        self.log_index = picked;
                        self.open_log();
                    }
                } else if let Some(path) = self.log_sources.first() {
                    // Monospace, not the carved label face: that one is cut
                    // for headings, and a file name set in it reads as a
                    // proclamation rather than a path.
                    ui.label(
                        RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                            .monospace()
                            .small()
                            .color(th::BONE_DIM),
                    )
                    .on_hover_text(path.display().to_string());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(self.t(Key::OpenTheFile)).clicked()
                        && let Some(path) = self.log_sources.get(self.log_index)
                    {
                        open_path(path);
                    }
                    // The lines on screen, not the whole file: what an admin
                    // pastes into a forum is what they were just reading.
                    let copied = ui
                        .button(self.t(Key::Copy))
                        .on_hover_text(self.t(Key::CopyLinesHint))
                        .clicked();
                    if copied {
                        let text = self
                            .log
                            .as_ref()
                            .map(|t| t.lines().collect::<Vec<_>>().join("\n"))
                            .unwrap_or_default();
                        let empty = text.is_empty();
                        ui.ctx().copy_text(text);
                        let msg = if empty {
                            self.t(Key::NothingToCopy)
                        } else {
                            self.t(Key::ConsoleCopied)
                        }
                        .to_string();
                        self.notify(msg, if empty { th::GOLD } else { th::MOSS });
                    }
                    let follow = self.t(Key::Follow);
                    ui.checkbox(&mut self.log_follow, follow);
                });
            });
            ui.add_space(6.0);

            let error = self.log.as_ref().and_then(|t| t.error().map(str::to_owned));
            if let Some(error) = error {
                w::notice(ui, th::BLOOD_LIT, &error);
                return;
            }
            egui::Frame::new()
                .fill(th::NIGHT)
                .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT))
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    // Whatever is left of the tab, not a fixed box with dead
                    // space under it. Floored so it stays usable on a short
                    // window, and capped so an enormous one does not put the
                    // last line a screen away from the first.
                    let height = (ui.available_height() - 16.0).clamp(180.0, 1200.0);
                    egui::ScrollArea::vertical()
                        .max_height(height)
                        .auto_shrink([false, false])
                        .stick_to_bottom(self.log_follow)
                        .show(ui, |ui| {
                            let Some(tail) = &self.log else { return };
                            if tail.lines().len() == 0 {
                                w::hint(ui, self.t(Key::EmptyLog));
                                return;
                            }
                            for line in tail.lines() {
                                ui.label(
                                    RichText::new(line)
                                        .monospace()
                                        .size(11.0)
                                        .color(log_colour(line)),
                                );
                            }
                        });
                });
        });
    }

    /// Writes the start script Steam will not overwrite, with the two rules
    /// Valheim enforces checked before anything is written.
    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_wizard(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t(Key::SectionStartScript));
            w::hint(ui, self.t(Key::WizardHint));
            ui.add_space(8.0);

            let full = ui.available_width();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    w::field(
                        ui,
                        self.t(Key::ServerName),
                        &mut self.recipe.name,
                        full * 0.45,
                    );
                });
                ui.vertical(|ui| {
                    w::field(ui, self.t(Key::World), &mut self.recipe.world, full * 0.45);
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(self.t(Key::Password))
                            .small()
                            .color(th::BONE_DIM),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.recipe.password)
                            .desired_width(full * 0.45)
                            .password(true)
                            .font(egui::TextStyle::Monospace),
                    );
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new(self.t(Key::Port)).small().color(th::BONE_DIM));
                    ui.add(egui::DragValue::new(&mut self.recipe.port).range(1024..=65_533));
                });
            });
            ui.add_space(8.0);
            let (crossplay, public, journal) = (
                self.t(Key::Crossplay),
                self.t(Key::ListedPublicly),
                self.t(Key::WriteLogFile),
            );
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.recipe.crossplay, crossplay);
                ui.checkbox(&mut self.recipe.public, public);
                ui.checkbox(&mut self.recipe.log_file, journal)
                    .on_hover_text(self.t(Key::LogFileHint));
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(self.t(Key::AutosaveEvery))
                        .small()
                        .color(th::BONE_DIM),
                );
                ui.add(
                    egui::DragValue::new(&mut self.recipe.save_interval)
                        .range(60..=7200)
                        .speed(30)
                        .suffix(" s"),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(self.t(Key::BackupsKept))
                        .small()
                        .color(th::BONE_DIM),
                );
                ui.add(egui::DragValue::new(&mut self.recipe.backups).range(0..=32));
            });

            let issues = self.recipe.issues();
            if !issues.is_empty() {
                ui.add_space(8.0);
                for issue in &issues {
                    w::notice(ui, th::GOLD, self.wizard_issue(*issue));
                }
            }

            ui.add_space(10.0);
            th::hairline(ui);
            ui.add_space(10.0);
            let root = PathBuf::from(self.server_root.trim());
            let can_write = issues.is_empty() && detect::looks_like_server_root(&root);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.recipe_file)
                        .desired_width(260.0)
                        .font(egui::TextStyle::Monospace),
                );
                let exists = root.join(self.recipe_file.trim()).exists();
                let label = if exists && !self.recipe_replace {
                    self.t(Key::ReplaceQuestion)
                } else {
                    self.t(Key::WriteTheScript)
                };
                if ui
                    .add_enabled(
                        can_write,
                        egui::Button::new(RichText::new(label).strong().color(if can_write {
                            th::NIGHT
                        } else {
                            th::BONE_DIM
                        }))
                        .fill(if can_write {
                            th::GOLD
                        } else {
                            th::LEATHER
                        }),
                    )
                    .clicked()
                {
                    if exists && !self.recipe_replace {
                        self.recipe_replace = true;
                    } else {
                        self.write_script(&root);
                    }
                }
            });
            if self.recipe_replace {
                w::hint(ui, self.t(Key::FileExistsHint));
            }
        });
    }

    /// Whether a player could fetch this pack right now, and what to do when
    /// they could not. Nothing here is guessed at: it reads the publishing
    /// mode, whether the live server is up, and the last real test.
    fn reachability(&mut self, ui: &mut egui::Ui) {
        let published = self.summary.is_some()
            || self
                .data_dir
                .join("published")
                .join("manifest.json")
                .is_file();
        let (colour, state, what_to_do) = if !published {
            (
                th::GOLD,
                self.t(Key::PackNeverBuilt),
                self.t(Key::PackNeverBuiltHint),
            )
        } else if self.mode == PublishMode::Live && self.serving_at.is_none() {
            (
                th::GOLD,
                self.t(Key::PublishingStopped),
                self.t(Key::PublishingStoppedHint),
            )
        } else if self.mode == PublishMode::Export && self.public_url.trim().is_empty() {
            (
                th::GOLD,
                self.t(Key::PublicUrlMissing),
                self.t(Key::PublicUrlMissingHint),
            )
        } else {
            match self.link_ok {
                Some(true) => (
                    th::MOSS,
                    self.t(Key::LinkVerified),
                    self.t(Key::LinkVerifiedHint),
                ),
                Some(false) => (
                    th::BLOOD_LIT,
                    self.t(Key::LinkUnreachable),
                    self.t(Key::LinkUnreachableHint),
                ),
                None => (
                    th::GOLD.gamma_multiply(0.55),
                    self.t(Key::NotTested),
                    self.t(Key::NotTestedHint),
                ),
            }
        };
        ui.horizontal(|ui| {
            w::lamp(ui, colour, 20.0);
            ui.label(
                RichText::new(state)
                    .text_style(th::label_style())
                    .color(th::BONE),
            );
        });
        w::hint(ui, what_to_do);
    }

    /// Write it, then re-detect so the new script is the one the panel uses.
    fn write_script(&mut self, root: &Path) {
        let name = self.recipe_file.trim().to_string();
        match wizard::write(root, &name, &self.recipe, self.recipe_replace) {
            Ok(path) => {
                self.recipe_replace = false;
                self.refresh_detection();
                if let Some(i) = self.scripts.iter().position(|s| s.path == path) {
                    self.script_index = i;
                    self.script_chosen = true;
                }
                let msg = format!("{} {}", self.t(Key::ScriptWritten), name);
                self.notify(msg, th::MOSS);
            }
            Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
        }
    }

    /// What a wizard complaint means, in the window's language.
    fn wizard_issue(&self, issue: wizard::Issue) -> &'static str {
        use wizard::{Field, Issue};
        match issue {
            Issue::NameEmpty => self.t(Key::IssueNameEmpty),
            Issue::WorldEmpty => self.t(Key::IssueWorldEmpty),
            Issue::PasswordTooShort => self.t(Key::IssuePasswordTooShort),
            Issue::PasswordInName => self.t(Key::IssuePasswordInName),
            Issue::BadCharacters(Field::Name) => self.t(Key::IssueQuoteInName),
            Issue::BadCharacters(Field::World) => self.t(Key::IssueQuoteInWorld),
            Issue::BadCharacters(Field::Password) => self.t(Key::IssueQuoteInPassword),
            Issue::PortOutOfRange => self.t(Key::IssuePortOutOfRange),
        }
    }

    fn prepare_player_folder(&self) -> anyhow::Result<PathBuf> {
        use anyhow::Context as _;

        let base = self
            .config_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let dir = base.join("pour-les-joueurs");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;

        std::fs::write(
            dir.join("valhsync-invite.txt"),
            format!("{}\n", self.invite),
        )
        .with_context(|| format!("cannot write the invite file in {}", dir.display()))?;

        // The launcher sits next to this executable in a release package.
        let launcher = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .map(|d| {
                d.join(if cfg!(windows) {
                    "valhsync.exe"
                } else {
                    "valhsync"
                })
            })
            .filter(|p| p.is_file());
        match launcher {
            Some(src) => {
                let dst = dir.join(src.file_name().unwrap_or_default());
                std::fs::copy(&src, &dst)
                    .with_context(|| format!("cannot copy {}", src.display()))?;
            }
            None => {
                std::fs::write(
                    dir.join("A-LIRE.txt"),
                    "Ajoutez valhsync.exe dans ce dossier, a cote de valhsync-invite.txt,\n                     puis envoyez le dossier (ou son zip) a vos joueurs.\n\n                     Add valhsync.exe to this folder, next to valhsync-invite.txt,\n                     then send the folder (or a zip of it) to your players.\n",
                )
                .ok();
            }
        }
        Ok(dir)
    }
}

/// Show a file or folder in the system file manager. Best effort.
/// Errors in blood, warnings in brass, the rest in bone. The words come from
/// BepInEx's own level tags and from Unity's, which both spell them out.
fn log_colour(line: &str) -> Color32 {
    let head: String = line.chars().take(40).collect::<String>().to_lowercase();
    if head.contains("error") || head.contains("fatal") || head.contains("exception") {
        th::BLOOD_LIT
    } else if head.contains("warning") {
        th::GOLD
    } else {
        th::BONE_DIM
    }
}

fn open_path(path: &Path) {
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}
