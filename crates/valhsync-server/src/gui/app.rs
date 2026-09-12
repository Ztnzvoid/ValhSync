//! The admin window: detect, configure, publish, invite.
//!
//! Text is inline in both languages rather than in a key table: this window
//! has few strings and keeping the French and the English side by side makes
//! it obvious when one drifts.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align, Color32, Layout, RichText};
use valhsync_core::limits::human_bytes;
use valhsync_core::manifest::MAX_NOTES;
use valhsync_ui::frame as chrome;
use valhsync_ui::theme as th;
use valhsync_ui::widgets as w;

use super::worker::{self, Msg, Reporter};
use crate::config::{self, Config};
use crate::{detect, gameserver, logs, wizard};

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

/// Which worker a message came from. They report through the same type but
/// have different lifetimes: a one-shot job ends, the live server runs on,
/// and the address check repeats on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chan {
    Job,
    Serve,
    Ip,
}

/// The two halves of the window: what the server is doing, and how it is set
/// up. Everything that changes minute to minute is on the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Mods,
    Status,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Fr,
    En,
}

impl Lang {
    /// English by default; the header switches language in one click.
    fn detect() -> Self {
        Self::En
    }

    /// What the configuration says, or the system's answer when it says
    /// nothing.
    fn from_code(code: Option<&str>) -> Self {
        match code {
            Some(c) if c.eq_ignore_ascii_case("fr") => Self::Fr,
            Some(c) if c.eq_ignore_ascii_case("en") => Self::En,
            _ => Self::detect(),
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Fr => Self::En,
            Self::En => Self::Fr,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Fr => "FR",
            Self::En => "EN",
        }
    }

    /// Pick the French or the English wording.
    fn t<'a>(self, fr: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Fr => fr,
            Self::En => en,
        }
    }
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
    command: String,
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
            lang: Lang::from_code(cfg.language.as_deref()),
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
            command: String::new(),
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
            let msg = app
                .lang
                .t(
                    "Aucune configuration trouvée : ValhSync a rempli ce qu'il a pu détecter. Vérifiez, puis Enregistrer.",
                    "No configuration found: ValhSync filled in what it could detect. Check it, then Save.",
                )
                .to_string();
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

    fn t(&self, fr: &'static str, en: &'static str) -> &'static str {
        self.lang.t(fr, en)
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
            self.session = logs::read_session(tail.lines());
            self.game_version = valhsync_core::gamelog::read_version(tail.lines());
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
                let msg = self
                    .t(
                        "Le serveur local s'est arrêté.",
                        "The live server has stopped.",
                    )
                    .to_string();
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
                    self.t("Pack construit :", "Pack built:"),
                    self.t("fichiers", "files"),
                    human_bytes(bytes)
                );
                self.notify(msg, th::MOSS);
            }
            Msg::Exported { dir, files, copied } => {
                let msg = format!(
                    "{} {files} {} → {} ({copied} {})",
                    self.t("Export :", "Export:"),
                    self.t("fichiers", "files"),
                    dir.display(),
                    self.t("copiés", "copied")
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
                    let msg = format!(
                        "{} {ip}",
                        self.t("Adresse publique détectée :", "Public address detected:")
                    );
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
                let msg = format!("{} {detail}", self.t("Lien joignable :", "Link reachable:"));
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
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                th::backdrop(ui.ctx(), ui.painter(), ui.max_rect().expand(18.0));
                let (status, mods, settings) = (
                    self.t("État du serveur", "Server"),
                    self.t("Mods", "Mods"),
                    self.t("Paramètres", "Settings"),
                );
                w::tabs(
                    ui,
                    &mut self.tab,
                    &[
                        (Tab::Status, status),
                        (Tab::Mods, mods),
                        (Tab::Settings, settings),
                    ],
                );
                ui.add_space(12.0);
                // Nothing is published from a configuration that was never
                // written: the publisher reads it from disk. Saving is
                // automatic, so reaching here means something is stopping it,
                // and that is worth more than a line at the bottom.
                if self.never_saved && self.save_error.is_some() {
                    w::notice(
                        ui,
                        th::GOLD,
                        self.t(
                            "La configuration n'a jamais pu être écrite, donc rien n'est publié et vos joueurs ne trouveront pas le serveur. La raison est en bas de la fenêtre ; elle s'enregistrera seule une fois corrigée.",
                            "This configuration has never been written, so nothing is published and your players will not find the server. The reason is at the bottom of the window; it saves itself once that is fixed.",
                        ),
                    );
                    ui.add_space(12.0);
                }
                if let Some(path) = self.new_key_at.clone() {
                    let text = format!(
                        "{}
{path}",
                        self.t(
                            "Nouvelle clé de signature générée. C'est l'identité de ce serveur : sauvegardez ce dossier. Si elle change, tous vos joueurs sont refusés et doivent réimporter un code d'invitation.",
                            "A new signing key was generated. It is this server's identity: back this folder up. If it changes, every player is refused and has to import a fresh invite code.",
                        )
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
                        }
                        Tab::Mods => self.card_mods(ui),
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
                    w::header(ui, "V A L H S Y N C   ·   S E R V E U R");
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        chrome::window_controls(ui);
                        ui.add_space(8.0);
                        if ui.button(self.lang.other().code()).clicked() {
                            self.lang = self.lang.other();
                        }
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
                self.console_line(ui);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    // No Save button. What an admin changes here is what the
                    // server publishes, and asking them to confirm it twice
                    // only produced servers running on a configuration that
                    // was on screen but never on disk.
                    if let Some(why) = self.save_error.clone() {
                        w::dot(ui, th::BLOOD_LIT);
                        ui.label(
                            RichText::new(format!(
                                "{} {why}",
                                self.t("Non enregistré :", "Not saved:")
                            ))
                            .small()
                            .color(th::BLOOD_LIT),
                        );
                    } else if self.dirty() {
                        w::dot(ui, th::GOLD);
                        ui.label(
                            RichText::new(self.t("Enregistrement…", "Saving…"))
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
            w::section(ui, self.t("I · Serveur de jeu", "I · Game server"));

            if self.server_outdated {
                w::notice(
                    ui,
                    th::BLOOD_LIT,
                    self.t(
                        "Steam a une mise à jour en attente pour le serveur dédié. Tant qu'elle n'est pas faite, les joueurs dont Valheim est à jour seront refusés : « Version incompatible ». Steam → Bibliothèque → Outils → Valheim Dedicated Server.",
                        "Steam has an update waiting for the dedicated server. Until it is applied, players whose Valheim is current will be refused with \"Version incompatible\". Steam → Library → Tools → Valheim Dedicated Server.",
                    ),
                );
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
                        self.t(
                            "Arrêt en cours, sauvegarde du monde",
                            "Stopping, saving the world",
                        )
                    } else if self.game_running {
                        self.t("En ligne", "Online")
                    } else {
                        self.t("Hors ligne", "Offline")
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.server_buttons(ui, stopping);
                });
            });

            ui.add_space(6.0);
            self.publishing_line(ui);
            ui.add_space(6.0);
            if self.game_running {
                self.session_facts(ui);
            } else if let Some(ip) = self.public_ip.clone() {
                ui.label(
                    RichText::new(format!("{} {ip}", self.t("IP publique", "public IP")))
                        .text_style(th::label_style())
                        .color(th::RUNE),
                );
            }
            ui.add_space(8.0);
            if self.scripts.is_empty() {
                w::notice(
                    ui,
                    th::GOLD,
                    self.t(
                        "Aucun script de démarrage. Créez-en un dans Paramètres.",
                        "No start script yet. Write one from the Settings tab.",
                    ),
                );
            } else if let Some(s) = self.scripts.get(self.script_index) {
                w::hint(
                    ui,
                    &format!(
                        "{} {}",
                        self.t("Lancé par", "Started by"),
                        s.path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                );
            }
            ui.add_space(8.0);
            w::hint(
                ui,
                self.t(
                    "Démarrer lance le serveur de jeu, renseigne l'adresse publique si besoin, et met la publication en ligne derrière.",
                    "Start brings up the game server, fills in the public address if it needs filling, and puts publishing online behind it.",
                ),
            );
            w::hint(
                ui,
                self.t(
                    "Arrêter, c'est envoyer Ctrl+C à sa fenêtre : Valheim écrit le monde sur le disque avant de quitter. ValhSync ne tue jamais le processus.",
                    "Stopping sends Ctrl+C to its window: Valheim writes the world to disk before it quits. ValhSync never kills the process.",
                ),
            );
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
        let label = self.t("Publication", "Publishing");
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
                        self.t("démarrage…", "starting…").to_string()
                    } else {
                        self.t("suit le serveur de jeu", "follows the game server")
                            .to_string()
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    w::dot(ui, th::GOLD);
                    ui.label(
                        RichText::new(format!(
                            "{label} · {} — {why}",
                            self.t("hors ligne", "offline")
                        ))
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

    /// Type a command into the dedicated server's console from here.
    ///
    /// One way only: Valheim answers in its own window and in the log, which
    /// is on the next card down. The field says so rather than leaving an
    /// admin waiting for a reply that is never coming back here.
    fn console_line(&mut self, ui: &mut egui::Ui) {
        let hint = self.t(
            "Commande serveur, par ex. « help »",
            "Server command, e.g. \"help\"",
        );
        let send_label = self.t("Envoyer", "Send");
        let tip = self.t(
            "Tapée dans la console du serveur. La réponse arrive dans la console ci-dessus, pas ici.",
            "Typed into the server's console. Its answer lands in the console above, not here.",
        );
        // Shown whether the server is up or not. A prompt that disappears
        // when there is nothing to talk to cannot be found again, and leaves
        // an admin wondering whether the window has one at all.
        let running = self.game_running;
        let why = self.t(
            "Le serveur de jeu est arrêté : il n'y a pas de console où taper.",
            "The game server is stopped: there is no console to type into.",
        );
        ui.horizontal(|ui| {
            let send = ui
                .add_enabled(running, egui::Button::new(send_label))
                .on_hover_text(tip)
                .on_disabled_hover_text(why)
                .clicked();
            let typed = ui
                .add_enabled(
                    running,
                    egui::TextEdit::singleline(&mut self.command)
                        .desired_width(ui.available_width())
                        .font(egui::TextStyle::Monospace)
                        .hint_text(if running { hint } else { why }),
                )
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if send || typed {
                let line = std::mem::take(&mut self.command);
                match gameserver::send_command(&line) {
                    Ok(()) => {
                        let msg = format!("{} {line}", self.t("Envoyé :", "Sent:"));
                        self.notify(msg, th::MOSS);
                    }
                    Err(e) => {
                        self.command = line;
                        self.notify(format!("{e:#}"), th::BLOOD_LIT);
                    }
                }
            }
        });
    }

    /// What the log says about the session: players, join code, which
    /// Valheim, and when the world was last written.
    fn session_facts(&mut self, ui: &mut egui::Ui) {
        let mut facts = Vec::new();
        if let Some(n) = self.session.players {
            facts.push(format!(
                "{n} {}",
                if n == 1 {
                    self.t("joueur connecté", "player online")
                } else {
                    self.t("joueurs connectés", "players online")
                }
            ));
        }
        if let Some(code) = &self.session.join_code {
            facts.push(format!("{} {code}", self.t("code crossplay", "join code")));
        }
        if let Some(version) = &self.game_version {
            facts.push(format!("Valheim {version}"));
        }
        if let Some(ip) = &self.public_ip {
            facts.push(format!("{} {ip}", self.t("IP publique", "public IP")));
        }
        if let Some(pid) = self.game_pid {
            facts.push(format!("{} {pid}", self.t("processus", "process")));
        }
        if let Some(saved) = self.world_saved {
            facts.push(format!(
                "{} {}",
                self.t("sauvegardé il y a", "saved"),
                Self::ago(saved)
            ));
        }
        if facts.is_empty() {
            w::hint(
                ui,
                self.t(
                    "En attente de la première ligne de session dans la console.",
                    "Waiting for the first session line in the console.",
                ),
            );
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
                self.t("Redémarrage…", "Restarting…")
            } else {
                self.t("Arrêt en cours…", "Stopping…")
            };
            ui.add_enabled(
                false,
                egui::Button::new(RichText::new(label).color(th::BONE_DIM)),
            );
            return;
        }
        if self.game_running {
            // Quieter than Stop: same Ctrl+C, same saved world, and the
            // window brings it back up once the process has actually gone.
            // Laid out right to left, so it sits left of Stop.
            if ui
                .small_button(self.t("Redémarrer", "Restart"))
                .on_hover_text(self.t(
                    "Arrête proprement, attend que le monde soit écrit, puis relance.",
                    "Stops cleanly, waits for the world to be written, then starts it again.",
                ))
                .clicked()
            {
                self.request_stop(true);
            }
            if ui
                .add(egui::Button::new(
                    RichText::new(self.t("Arrêter et sauvegarder", "Stop and save"))
                        .color(th::BONE),
                ))
                .clicked()
            {
                self.request_stop(false);
            }
            return;
        }
        let can_start = !self.scripts.is_empty();
        if ui
            .add_enabled(
                can_start,
                egui::Button::new(
                    RichText::new(self.t("Démarrer", "Start"))
                        .strong()
                        .color(if can_start { th::NIGHT } else { th::BONE_DIM }),
                )
                .fill(if can_start { th::GOLD } else { th::LEATHER }),
            )
            .on_disabled_hover_text(self.t(
                "Aucun script de démarrage : onglet Paramètres, carte « Dossier du serveur ».",
                "No start script: Settings tab, \"Server folder\" card.",
            ))
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
                    self.t(
                        "Ctrl+C envoyé. Le monde est sauvegardé, puis le serveur repart.",
                        "Ctrl+C sent. The world is saved, then the server comes back.",
                    )
                } else {
                    self.t(
                        "Ctrl+C envoyé. Valheim sauvegarde le monde puis quitte.",
                        "Ctrl+C sent. Valheim saves the world, then quits.",
                    )
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
        let msg = self
            .t(
                "Serveur de jeu lancé dans sa propre fenêtre.",
                "Game server started in its own window.",
            )
            .to_string();
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
        }
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
                    RichText::new(self.t("Arrêter", "Stop")).color(th::BONE),
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
                egui::Button::new(
                    RichText::new(self.t("Démarrer", "Start"))
                        .strong()
                        .color(if can { th::NIGHT } else { th::BONE_DIM }),
                )
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
            w::section(ui, self.t("I · Serveur dédié", "I · Dedicated server"));

            let hint_text = self.t("Dossier du serveur dédié", "Dedicated server folder");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.server_root)
                        .desired_width(ui.available_width() - 120.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(hint_text),
                );
                if ui.button(self.t("Détecter", "Detect")).clicked() {
                    if let Some(p) = detect::detect_server_root() {
                        self.server_root = p.display().to_string();
                        self.pull_fields();
                        self.refresh_detection();
                        let msg = format!(
                            "{} {}",
                            self.t("Serveur dédié trouvé :", "Dedicated server found:"),
                            p.display()
                        );
                        self.notify(msg, th::MOSS);
                    } else {
                        let msg = self.t(
                            "Aucun serveur dédié trouvé. Indiquez le dossier contenant valheim_server.exe.",
                            "No dedicated server found. Point to the folder containing valheim_server.exe.",
                        )
                        .to_string();
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
                            self.t(
                                "Serveur dédié avec BepInEx",
                                "Dedicated server with BepInEx",
                            )
                        } else {
                            self.t(
                                "Serveur dédié trouvé, mais sans BepInEx : installez BepInExPack_Valheim d'abord",
                                "Dedicated server found, but no BepInEx: install BepInExPack_Valheim first",
                            )
                        },
                    );
                } else {
                    w::notice(
                        ui,
                        th::BLOOD_LIT,
                        self.t(
                            "Ce dossier ne contient pas valheim_server.exe.",
                            "This folder has no valheim_server.exe in it.",
                        ),
                    );
                }
            }

            if !self.scripts.is_empty() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.t("Script de démarrage", "Start script"))
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
                        Some(true) => bits.push(self.t("public", "public").into()),
                        Some(false) => bits.push(self.t("privé", "private").into()),
                        None => {}
                    }
                    if a.has_password {
                        bits.push(self.t("mot de passe défini", "password set").to_string());
                    }
                    ui.label(
                        RichText::new(bits.join(" · "))
                            .text_style(th::label_style())
                            .color(th::RUNE),
                    );
                }
            }
        });
    }

    fn card_identity(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(
                ui,
                self.t("II · Identité et adresse", "II · Identity and address"),
            );

            let width = (ui.available_width() - 24.0).max(200.0);
            w::field(
                ui,
                self.t("Nom affiché aux joueurs", "Name shown to players"),
                &mut self.name,
                width,
            );
            ui.add_space(6.0);

            ui.label(
                RichText::new(self.t(
                    "Adresse du serveur de jeu (host:port)",
                    "Game server address (host:port)",
                ))
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
                            .button(self.t("Utiliser l'IP détectée", "Use detected IP"))
                            .on_hover_text(format!("{addr}  ·  {}", worker::IP_ECHO_SERVICE))
                            .clicked()
                        {
                            self.game_address = addr;
                        }
                    }
                    Some(_) => {
                        ui.label(
                            RichText::new(
                                self.t("Correspond à votre IP publique", "Matches your public IP"),
                            )
                            .small()
                            .color(th::MOSS),
                        )
                        .on_hover_text(worker::IP_ECHO_SERVICE);
                    }
                    None => {
                        ui.label(
                            RichText::new(self.t("Détection de l'IP…", "Detecting your IP…"))
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
                        self.t(
                            "Adresse locale, et votre serveur tourne en crossplay : personne ne pourra se connecter, pas même sur votre réseau. Iron Gate : « it's not possible to connect using a local IP address ». Utilisez votre IP publique.",
                            "Local address, and your server runs with crossplay: nobody will connect, not even on your own network. Iron Gate: \"it's not possible to connect using a local IP address\". Use your public IP.",
                        )
                    } else {
                        self.t(
                            "Adresse locale : seuls les joueurs de votre réseau pourront se connecter.",
                            "Local address: only players on your own network will connect.",
                        )
                    },
                );
            } else if !addr.is_empty() && !valhsync_core::manifest::is_valid_game_address(addr) {
                ui.add_space(4.0);
                w::notice(
                    ui,
                    th::BLOOD_LIT,
                    self.t(
                        "Adresse invalide : uniquement lettres, chiffres, . - _ : [ ]",
                        "Invalid address: only letters, digits, . - _ : [ ]",
                    ),
                );
            }
        });
    }

    fn card_mods(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            self.mod_list(ui);
            ui.add_space(10.0);
            th::hairline(ui);
            ui.add_space(8.0);
            self.pack_notes(ui);
        });
    }

    /// The mods found on the server, and which side each one runs on.
    #[allow(clippy::too_many_lines)] // one list, read top to bottom
    fn mod_list(&mut self, ui: &mut egui::Ui) {
        w::section(ui, self.t("Mods du pack", "Pack mods"));
        if self.mods.is_empty() {
            w::hint(
                ui,
                self.t(
                    "Aucun mod trouvé dans BepInEx/plugins.",
                    "No mod found in BepInEx/plugins.",
                ),
            );
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
                w::hint(ui, self.t("Lus dans", "Read from"));
                ui.label(
                    RichText::new(plugins.display().to_string())
                        .monospace()
                        .small()
                        .color(th::RUNE),
                );
                if ui.small_button(self.t("Ouvrir", "Open")).clicked() {
                    open_path(&plugins);
                }
            });
        }
        w::hint(
                ui,
                self.t(
                    "Décochez « envoyé aux joueurs » pour un mod qui ne doit tourner que sur le serveur (DiscordConnector, outils d'admin).",
                    "Untick \"sent to players\" for a mod that must run on the server only (DiscordConnector, admin tools).",
                ),
            );
        ui.add_space(8.0);

        let server_only = self.mods.iter().filter(|m| m.server_only).count();
        let sent = self.mods.len() - server_only;
        let mut changed: Vec<(String, bool, bool)> = Vec::new();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{sent} {}  ·  {server_only} {}",
                    self.t("envoyés", "sent"),
                    self.t("serveur seul", "server only")
                ))
                .text_style(th::label_style())
                .color(th::BONE_DIM),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        server_only > 0,
                        egui::Button::new(self.t("Tout envoyer", "Send all")),
                    )
                    .clicked()
                {
                    for m in self.mods.iter().filter(|m| m.server_only) {
                        changed.push((m.folder.clone(), m.loose, false));
                    }
                }
                if ui
                    .add_enabled(sent > 0, egui::Button::new(self.t("Aucun", "None")))
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
                        RichText::new(self.t("client seul", "client only"))
                            .small()
                            .color(th::RUNE),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .selectable_label(m.server_only, self.t("Serveur seul", "Server only"))
                        .clicked()
                        && !m.server_only
                    {
                        changed.push((m.folder.clone(), m.loose, true));
                    }
                    if ui
                        .selectable_label(
                            !m.server_only,
                            self.t("Envoyé aux joueurs", "Sent to players"),
                        )
                        .clicked()
                        && m.server_only
                    {
                        changed.push((m.folder.clone(), m.loose, false));
                    }
                });
            });
        }
        for (folder, loose, server_only) in changed {
            self.set_server_only(&folder, loose, server_only);
            self.mods = self.collect_mods();
        }
        if let Some(extras) = self.cfg.pack.client_extras.clone() {
            ui.add_space(10.0);
            th::hairline(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new(self.t(
                    "Optionnel · mods qui ne tournent QUE chez les joueurs",
                    "Optional · mods that run ONLY on players",
                ))
                .font(th::display_font(13.0))
                .color(th::GOLD_LIT),
            );
            w::hint(
                    ui,
                    self.t(
                        "Unshamed, ConfigurationManager, EquipmentAndQuickSlots… Ils ne sont pas installés sur le serveur, donc ValhSync ne peut pas les y trouver : déposez-les ici, à la même arborescence que le jeu (BepInEx/plugins/...). Si vous n'en avez aucun, ignorez ce dossier.",
                        "Unshamed, ConfigurationManager, EquipmentAndQuickSlots… They are not installed on the server, so ValhSync cannot find them there: drop them here, laid out like the game (BepInEx/plugins/...). If you have none, ignore this folder.",
                    ),
                );
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(extras.display().to_string())
                        .monospace()
                        .small()
                        .color(th::RUNE),
                );
                if ui.small_button(self.t("Ouvrir", "Open")).clicked() {
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
            RichText::new(self.t(
                "Optionnel · mot aux joueurs",
                "Optional · a word to players",
            ))
            .font(th::display_font(13.0))
            .color(th::GOLD_LIT),
        );
        let hint = self.t(
            "Affiché dans le launcher avant que le joueur accepte la synchronisation. La liste des mods qui changent est calculée toute seule : écrivez ici ce qu'elle ne peut pas dire (« ce mod remet sa config à zéro », « videz vos coffres avant »). Rien à dire ? Laissez vide.",
            "Shown in the launcher before a player accepts the sync. The list of mods that change is worked out on its own: write here what it cannot say (\"this mod resets its own config\", \"empty your chests first\"). Nothing to say? Leave it empty.",
        );
        w::hint(ui, hint);
        let placeholder = self.t("Rien de particulier.", "Nothing in particular.");
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
                format!("{} {}", used - MAX_NOTES, self.t("de trop", "too many")),
                th::BLOOD_LIT,
            )
        } else {
            (
                format!("{} {}", MAX_NOTES - used, self.t("restants", "left")),
                th::BONE_DIM,
            )
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
    }

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_publish(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("III · Publication", "III · Publishing"));
            let (label_export, label_live) = (
                self.t("Fichiers statiques", "Static files"),
                self.t("Serveur local", "Live server"),
            );
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, PublishMode::Export, label_export);
                ui.selectable_value(&mut self.mode, PublishMode::Live, label_live);
            });
            ui.add_space(6.0);

            match self.mode {
                PublishMode::Export => {
                    w::hint(
                        ui,
                        self.t(
                            "Recommandé : aucun port à ouvrir. ValhSync écrit un dossier à déposer sur n'importe quel espace web (GitHub Pages, S3, votre hébergeur).",
                            "Recommended: no port to open. ValhSync writes a folder you upload to any web space (GitHub Pages, S3, your host).",
                        ),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.export_dir)
                                .desired_width(ui.available_width() - 210.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(self.t("Exporter", "Export")),
                            )
                            .clicked()
                        {
                            self.pull_fields();
                            let cfg = self.cfg.clone();
                            let data = self.data_dir.clone();
                            let dir = PathBuf::from(self.export_dir.trim());
                            self.start_job(move |rep| {
                                rep.run(|| worker::export(&cfg, &data, dir));
                            });
                        }
                        if ui.button(self.t("Ouvrir", "Open")).clicked() {
                            let dir = PathBuf::from(self.export_dir.trim());
                            let _ = std::fs::create_dir_all(&dir);
                            open_path(&dir);
                        }
                    });
                    w::hint(
                        ui,
                        self.t(
                            "Ensuite, mettez l'URL de ce dossier dans « URL publique » ci-dessous.",
                            "Then put that folder's URL in \"Public URL\" below.",
                        ),
                    );
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
                                self.t("En ligne", "Online")
                            } else {
                                self.t("Hors ligne", "Offline")
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
                                    RichText::new(self.t("Port", "Port"))
                                        .small()
                                        .color(th::BONE_DIM),
                                );
                                let mut port = self.bind_port();
                                if ui
                                    .add(egui::DragValue::new(&mut port).range(1024..=65_533))
                                    .on_hover_text(self.t(
                                        "Celui du jeu par défaut. Le changer voudrait dire ouvrir un port de plus.",
                                        "The game's, by default. Changing it would mean opening one more port.",
                                    ))
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
                            &format!(
                                "{} {why}",
                                self.t(
                                    "La publication ne peut pas démarrer :",
                                    "Publishing cannot start:"
                                )
                            ),
                        );
                        ui.add_space(6.0);
                    }
                    w::hint(
                        ui,
                        self.t(
                            "La publication suit le serveur de jeu : elle se met en ligne toute seule dès qu'il tourne, et le bouton Démarrer du panneau I la monte avec lui. L'arrêter ici la laisse arrêtée jusqu'au prochain démarrage du serveur.",
                            "Publishing follows the game server: it goes online by itself as soon as the game is up, and Start on panel I brings both. Stopping it here keeps it stopped until the game server is next started.",
                        ),
                    );
                    w::hint(
                        ui,
                        self.t(
                            "ValhSync sert le pack depuis cette machine, sur le port du jeu mais en TCP : Valheim ne l'utilise qu'en UDP, donc aucun nouveau port à ouvrir. Vérifiez seulement que votre règle de routeur couvre TCP et UDP.",
                            "ValhSync serves the pack from this machine, on the game's port but in TCP: Valheim only uses it in UDP, so there is no new port to open. Just check that your router rule covers TCP as well as UDP.",
                        ),
                    );
                }
            }

            ui.add_space(8.0);
            w::field(
                ui,
                self.t(
                    "URL publique (celle que les joueurs contacteront)",
                    "Public URL (what players will contact)",
                ),
                &mut self.public_url,
                (ui.available_width() - 24.0).max(200.0),
            );

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Button::new(self.t("Analyser le pack", "Scan the pack")),
                    )
                    .clicked()
                {
                    self.pull_fields();
                    let cfg = self.cfg.clone();
                    let data = self.data_dir.clone();
                    self.start_job(move |rep| rep.run(|| worker::scan(&cfg, &data)));
                }
                if let Some(s) = &self.summary {
                    ui.label(
                        RichText::new(format!(
                            "{} {} · {}",
                            s.files,
                            self.t("fichiers", "files"),
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
                    RichText::new(format!(
                        "{} {}",
                        s.skipped.len(),
                        self.t("fichier(s) ignoré(s)", "file(s) skipped")
                    ))
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
            w::section(ui, self.t("IV · Code d'invitation", "IV · Invite code"));
            w::hint(
                ui,
                self.t(
                    "Envoyez-le à vos joueurs. Ils peuvent aussi ajouter le serveur par son adresse seule : dans ce cas, donnez-leur l'empreinte de clé ci-dessous pour qu'ils la vérifient.",
                    "Send it to your players. They can also add the server by its address alone: give them the key fingerprint below so they can check it.",
                ),
            );
            ui.add_space(4.0);
            if self.invite.is_empty() {
                // A code is missing for exactly one of two reasons, and the
                // admin can act on either -- as long as the window says which.
                if self.edited().public_url().is_none() {
                    w::notice(
                        ui,
                        th::GOLD,
                        self.t(
                            "Pas encore d'adresse à mettre dans le code. Renseignez ci-dessus l'adresse de votre serveur de jeu — celle que vos joueurs utilisent déjà dans Valheim — ou, si vous hébergez l'export, son URL publique.",
                            "No address to put in the code yet. Fill in your game server's address above — the one your players already use in Valheim — or, if you host the export, its public URL.",
                        ),
                    );
                } else {
                    w::hint(
                        ui,
                        self.t(
                            "Le code apparaîtra après le premier Enregistrer : c'est à ce moment que votre clé de signature est créée.",
                            "The code appears after the first Save: that is when your signing key is created.",
                        ),
                    );
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
                            self.t("Empreinte de la clé :", "Key fingerprint:"),
                            key.public().fingerprint()
                        ))
                        .text_style(th::label_style())
                        .color(th::RUNE),
                    );
                }
            });
            ui.add_space(8.0);
            self.key_takeover(ui);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(self.t("Copier", "Copy")).clicked() {
                    ui.ctx().copy_text(self.invite.clone());
                    let msg = self.t("Code copié.", "Code copied.").to_string();
                    self.notify(msg, th::MOSS);
                }
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Button::new(self.t("Tester le lien", "Test the link")),
                    )
                    .on_hover_text(self.t(
                        "Récupère le pack comme le ferait un joueur, et vérifie la signature.",
                        "Fetches the pack the way a player would, and checks the signature.",
                    ))
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
                    .button(self.t(
                        "Préparer le dossier à envoyer aux joueurs",
                        "Prepare the folder to send to players",
                    ))
                    .on_hover_text(self.t(
                        "Crée un dossier avec valhsync.exe et le code : les joueurs n'ont rien à coller.",
                        "Creates a folder holding valhsync.exe and the code: players paste nothing.",
                    ))
                    .clicked()
                {
                    match self.prepare_player_folder() {
                        Ok(dir) => {
                            let msg = format!(
                                "{} {}",
                                self.t("Dossier joueur prêt :", "Player folder ready:"),
                                dir.display()
                            );
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
        let label = self.t(
            "Reprendre la clé d'une autre installation",
            "Take over another install's key",
        );
        let hint = self.t("Chemin d'un server.key", "Path to a server.key");
        let button = self.t("Reprendre", "Take over");
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
                            self.t("Clé reprise, empreinte :", "Key taken over, fingerprint:"),
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
            w::section(ui, self.t("II · Console", "II · Console"));

            if self.log_sources.is_empty() {
                w::hint(
                    ui,
                    self.t(
                        "Aucun journal trouvé. BepInEx en écrit un ; sinon, l'assistant de script peut en ajouter un dans Paramètres.",
                        "No log found. BepInEx writes one; failing that, the script wizard can add one from the Settings tab.",
                    ),
                );
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
                    if ui
                        .button(self.t("Ouvrir le fichier", "Open the file"))
                        .clicked()
                        && let Some(path) = self.log_sources.get(self.log_index)
                    {
                        open_path(path);
                    }
                    // The lines on screen, not the whole file: what an admin
                    // pastes into a forum is what they were just reading.
                    let copied = ui
                        .button(self.t("Copier", "Copy"))
                        .on_hover_text(self.t(
                            "Copie les lignes affichées dans le presse-papiers.",
                            "Copies the lines shown to the clipboard.",
                        ))
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
                            self.t("Rien à copier.", "Nothing to copy.")
                        } else {
                            self.t("Console copiée.", "Console copied.")
                        }
                        .to_string();
                        self.notify(msg, if empty { th::GOLD } else { th::MOSS });
                    }
                    let follow = self.t("Suivre", "Follow");
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
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .auto_shrink([false, false])
                        .stick_to_bottom(self.log_follow)
                        .show(ui, |ui| {
                            let Some(tail) = &self.log else { return };
                            if tail.lines().len() == 0 {
                                w::hint(ui, self.t("(vide)", "(empty)"));
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
            w::section(ui, self.t("V · Script de démarrage", "V · Start script"));
            w::hint(
                ui,
                self.t(
                    "Steam remplace start_headless_server.bat à chaque mise à jour. ValhSync écrit une copie à vous, qu'il ne touchera jamais.",
                    "Steam replaces start_headless_server.bat on every update. ValhSync writes a copy of your own, which it will never touch.",
                ),
            );
            ui.add_space(8.0);

            let full = ui.available_width();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    w::field(
                        ui,
                        self.t("Nom du serveur", "Server name"),
                        &mut self.recipe.name,
                        full * 0.45,
                    );
                });
                ui.vertical(|ui| {
                    w::field(
                        ui,
                        self.t("Monde", "World"),
                        &mut self.recipe.world,
                        full * 0.45,
                    );
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(self.t("Mot de passe", "Password"))
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
                    ui.label(
                        RichText::new(self.t("Port", "Port"))
                            .small()
                            .color(th::BONE_DIM),
                    );
                    ui.add(egui::DragValue::new(&mut self.recipe.port).range(1024..=65_533));
                });
            });
            ui.add_space(8.0);
            let (crossplay, public, journal) = (
                self.t("Crossplay", "Crossplay"),
                self.t("Listé publiquement", "Listed publicly"),
                self.t("Écrire un journal", "Write a log file"),
            );
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.recipe.crossplay, crossplay);
                ui.checkbox(&mut self.recipe.public, public);
                ui.checkbox(&mut self.recipe.log_file, journal)
                    .on_hover_text(self.t(
                        "Pour un serveur sans BepInEx. La sortie part alors dans le fichier au lieu de la console.",
                        "For a server with no BepInEx. Its output then goes to the file instead of the console.",
                    ));
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(self.t("Sauvegarde auto toutes les", "Autosave every"))
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
                    RichText::new(self.t("Sauvegardes conservées", "Backups kept"))
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
                    self.t("Remplacer ?", "Replace?")
                } else {
                    self.t("Écrire le script", "Write the script")
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
                w::hint(
                    ui,
                    self.t(
                        "Ce fichier existe déjà. Cliquez une seconde fois pour l'écraser.",
                        "That file already exists. Click once more to overwrite it.",
                    ),
                );
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
                self.t("Pack jamais construit", "Pack never built"),
                self.t(
                    "Enregistrez, puis publiez : sans manifeste, le code ne mène à rien.",
                    "Save, then publish: with no manifest the code leads nowhere.",
                ),
            )
        } else if self.mode == PublishMode::Live && self.serving_at.is_none() {
            (
                th::GOLD,
                self.t("Publication arrêtée", "Publishing stopped"),
                self.t(
                    "Démarrez le serveur local dans Publication ci-dessus, sinon personne ne peut télécharger le pack.",
                    "Start the live server under Publishing above, or nobody can download the pack.",
                ),
            )
        } else if self.mode == PublishMode::Export && self.public_url.trim().is_empty() {
            (
                th::GOLD,
                self.t("URL publique manquante", "Public URL missing"),
                self.t(
                    "Le code pointe sur cette machine. Exportez le dossier, déposez-le sur votre espace web, puis mettez son URL dans « URL publique ».",
                    "The code points at this machine. Export the folder, upload it to your web space, then put its URL in \"Public URL\".",
                ),
            )
        } else {
            match self.link_ok {
                Some(true) => (
                    th::MOSS,
                    self.t("Lien vérifié", "Link verified"),
                    self.t(
                        "Un joueur a récupéré ce pack depuis cette adresse, signature comprise.",
                        "The pack was fetched from this address, signature and all.",
                    ),
                ),
                Some(false) => (
                    th::BLOOD_LIT,
                    self.t("Lien injoignable", "Link unreachable"),
                    self.t(
                        "L'adresse publiée n'a pas répondu. Voyez le message ci-dessous.",
                        "The published address did not answer. See the message below.",
                    ),
                ),
                None => (
                    th::GOLD.gamma_multiply(0.55),
                    self.t("Non testé", "Not tested"),
                    self.t(
                        "« Tester le lien » récupère le pack comme le ferait un joueur.",
                        "\"Test the link\" fetches the pack the way a player would.",
                    ),
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
                let msg = format!("{} {}", self.t("Script écrit :", "Script written:"), name);
                self.notify(msg, th::MOSS);
            }
            Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
        }
    }

    /// What a wizard complaint means, in the window's two languages.
    fn wizard_issue(&self, issue: wizard::Issue) -> &'static str {
        use wizard::{Field, Issue};
        match issue {
            Issue::NameEmpty => self.t("Donnez un nom au serveur.", "Give the server a name."),
            Issue::WorldEmpty => self.t("Donnez un nom au monde.", "Give the world a name."),
            Issue::PasswordTooShort => self.t(
                "Valheim exige un mot de passe d'au moins 5 caractères.",
                "Valheim requires a password of at least 5 characters.",
            ),
            Issue::PasswordInName => self.t(
                "Valheim refuse de démarrer si le nom du serveur contient le mot de passe.",
                "Valheim refuses to start when the server name contains the password.",
            ),
            Issue::BadCharacters(Field::Name) => self.t(
                "Le nom contient un guillemet ou une apostrophe : le script ne les supporte pas.",
                "The name holds a quote, which the script cannot carry.",
            ),
            Issue::BadCharacters(Field::World) => self.t(
                "Le nom du monde contient un guillemet ou une apostrophe.",
                "The world name holds a quote.",
            ),
            Issue::BadCharacters(Field::Password) => self.t(
                "Le mot de passe contient un guillemet ou une apostrophe.",
                "The password holds a quote.",
            ),
            Issue::PortOutOfRange => self.t(
                "Choisissez un port entre 1024 et 65533 : le serveur utilise aussi le suivant.",
                "Pick a port between 1024 and 65533: the server also uses the next one.",
            ),
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
