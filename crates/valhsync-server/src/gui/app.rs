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
use valhsync_ui::frame as chrome;
use valhsync_ui::theme as th;
use valhsync_ui::widgets as w;

use super::worker::{self, Msg, Reporter};
use crate::config::{self, Config};
use crate::{detect, gameserver, logs, wizard};

const POLL_GAME_SERVER: Duration = Duration::from_secs(2);
const NOTICE_TTL: Duration = Duration::from_secs(12);
/// How often the open log is re-read. Fast enough to watch a start-up, slow
/// enough that the file is touched once a second and no more.
const POLL_LOG: Duration = Duration::from_millis(900);

/// The two halves of the window: what the server is doing, and how it is set
/// up. Everything that changes minute to minute is on the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
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
    /// When the world file was last written, so the panel can say how long ago.
    world_saved: Option<SystemTime>,
    /// Set when a stop was asked for, cleared when the process is gone.
    stop_requested: Option<Instant>,
    /// The world file, when the start script says enough to find it.
    world_file: Option<PathBuf>,

    /// The start-script wizard, and the name it would write to.
    recipe: wizard::Recipe,
    recipe_file: String,
    /// Second click confirms replacing a script that already exists.
    recipe_replace: bool,

    /// Window title as last set, so it is only pushed when it changes.
    title: String,
    busy: bool,
    rx: Option<Receiver<Msg>>,
    notice: Option<(String, Color32, Instant)>,
    egui_ctx: egui::Context,
}

impl App {
    pub(super) fn new(ctx: &egui::Context, config_path: PathBuf, data_dir: PathBuf) -> Self {
        let (cfg, loaded) = match Config::load(&config_path) {
            Ok(cfg) => (cfg, true),
            Err(_) => (Self::fresh_config(&config_path), false),
        };
        let export_dir = config_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            .join("pack-site");

        let mut app = Self {
            lang: Lang::detect(),
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
            saved: cfg.clone(),
            never_saved: !loaded,
            cfg,
            config_path,
            data_dir,
            mods: Vec::new(),
            scripts: Vec::new(),
            script_index: 0,
            script_chosen: false,
            mode: PublishMode::Export,
            summary: None,
            invite: String::new(),
            serving_at: None,
            stop_serving: None,
            game_running: false,
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
            world_saved: None,
            stop_requested: None,
            world_file: None,
            recipe: wizard::Recipe::default(),
            recipe_file: String::from("start_valheim_server.bat"),
            recipe_replace: false,
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

    fn save(&mut self) -> bool {
        self.pull_fields();
        if let Err(e) = self.cfg.validate() {
            self.notify(format!("{e:#}"), th::BLOOD_LIT);
            return false;
        }
        match config::save(&self.cfg, &self.config_path) {
            Ok(()) => {
                self.saved = self.cfg.clone();
                self.never_saved = false;
                self.refresh_detection();
                if let Ok(kp) = worker::load_key(&self.data_dir) {
                    self.invite = worker::invite_code(&self.cfg, &kp).unwrap_or_default();
                }
                let msg = format!(
                    "{} {}",
                    self.t("Configuration enregistrée :", "Configuration saved:"),
                    self.config_path.display()
                );
                self.notify(msg, th::MOSS);
                true
            }
            Err(e) => {
                self.notify(format!("{e:#}"), th::BLOOD_LIT);
                false
            }
        }
    }

    fn drain(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut msgs = Vec::new();
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        for msg in msgs {
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
                Msg::Serving(url) => {
                    self.serving_at = Some(url);
                    self.busy = false;
                }
                Msg::ServeStopped(err) => {
                    self.serving_at = None;
                    self.stop_serving = None;
                    if let Some(e) = err {
                        self.notify(e, th::BLOOD_LIT);
                    }
                }
                Msg::PublicIp(ip) => {
                    let port = self
                        .scripts
                        .get(self.script_index)
                        .and_then(|s| s.args.port)
                        .unwrap_or(2456);
                    self.game_address = format!("{ip}:{port}");
                    let msg = format!(
                        "{} {ip}",
                        self.t("Adresse publique détectée :", "Public address detected:")
                    );
                    self.notify(msg, th::MOSS);
                }
                Msg::Error(e) => self.notify(e, th::BLOOD_LIT),
                Msg::Idle => {
                    // A live server reports Idle only once it has stopped.
                    if self.serving_at.is_none() {
                        self.busy = false;
                    }
                    self.rx = None;
                }
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
        if self.game_checked.elapsed() >= POLL_GAME_SERVER {
            let was = self.game_running;
            self.game_running = gameserver::is_running();
            self.game_checked = Instant::now();
            // A server that has just started writes a log that did not exist.
            if was != self.game_running {
                self.refresh_log_sources();
                if !self.game_running {
                    self.session = logs::Session::default();
                    self.stop_requested = None;
                }
            }
        }
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

        chrome::handle_edge_resize(ctx);
        self.update_title(ctx);
        self.top_bar(ctx);
        self.bottom_bar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                th::backdrop(ui.ctx(), ui.painter(), ui.max_rect().expand(18.0));
                let (status, settings) = (
                    self.t("État du serveur", "Server"),
                    self.t("Paramètres", "Settings"),
                );
                w::tabs(
                    ui,
                    &mut self.tab,
                    &[(Tab::Status, status), (Tab::Settings, settings)],
                );
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        Tab::Status => {
                            self.card_status(ui);
                            ui.add_space(12.0);
                            self.card_logs(ui);
                        }
                        Tab::Settings => {
                            self.card_server_folder(ui);
                            ui.add_space(12.0);
                            self.card_identity(ui);
                            ui.add_space(12.0);
                            self.card_mods(ui);
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
                ui.horizontal(|ui| {
                    let dirty = self.dirty();
                    if ui
                        .add_enabled(
                            dirty,
                            egui::Button::new(
                                RichText::new(self.t("Enregistrer", "Save"))
                                    .strong()
                                    .color(if dirty { th::NIGHT } else { th::BONE_DIM }),
                            )
                            .fill(if dirty {
                                th::GOLD
                            } else {
                                th::LEATHER
                            }),
                        )
                        .clicked()
                    {
                        self.save();
                    }
                    if dirty {
                        ui.label(
                            RichText::new(
                                self.t("Modifications non enregistrées", "Unsaved changes"),
                            )
                            .small()
                            .color(th::GOLD_LIT),
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

            if self.game_running {
                ui.add_space(6.0);
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
                            "En attente de la première ligne de session dans le journal.",
                            "Waiting for the first session line in the log.",
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
            w::hint(
                ui,
                self.t(
                    "Arrêter, c'est envoyer Ctrl+C à sa fenêtre : Valheim écrit le monde sur le disque avant de quitter. ValhSync ne tue jamais le processus.",
                    "Stopping sends Ctrl+C to its window: Valheim writes the world to disk before it quits. ValhSync never kills the process.",
                ),
            );
        });
    }

    /// Start and stop side by side, so the pair reads as one control.
    fn server_buttons(&mut self, ui: &mut egui::Ui, stopping: bool) {
        let can_stop = self.game_running && !stopping;
        if ui
            .add_enabled(
                can_stop,
                egui::Button::new(
                    RichText::new(self.t("Arrêter et sauvegarder", "Stop and save"))
                        .color(if can_stop { th::BONE } else { th::BONE_DIM }),
                ),
            )
            .clicked()
        {
            match gameserver::stop() {
                Ok(()) => {
                    self.stop_requested = Some(Instant::now());
                    let msg = self
                        .t(
                            "Ctrl+C envoyé. Valheim sauvegarde le monde puis quitte.",
                            "Ctrl+C sent. Valheim saves the world, then quits.",
                        )
                        .to_string();
                    self.notify(msg, th::MOSS);
                }
                Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
            }
        }
        let can_start = !self.game_running && !self.scripts.is_empty();
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
            .clicked()
            && let Some(s) = self.scripts.get(self.script_index)
        {
            let launch = gameserver::Launch(s.path.clone());
            match gameserver::start(&launch) {
                Ok(()) => {
                    self.game_running = true;
                    self.game_checked = Instant::now();
                    self.stop_requested = None;
                    let msg = self
                        .t(
                            "Serveur de jeu lancé dans sa propre fenêtre.",
                            "Game server started in its own window.",
                        )
                        .to_string();
                    self.notify(msg, th::MOSS);
                }
                Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
            }
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
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Button::new(
                            self.t("Détecter mon IP publique", "Detect my public IP"),
                        ),
                    )
                    .on_hover_text(worker::IP_ECHO_SERVICE)
                    .clicked()
                {
                    self.start_job(|rep| rep.run(worker::public_ip));
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

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_mods(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("III · Mods", "III · Mods"));
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
            ui.add_space(6.0);
            let mut changed: Vec<(String, bool, bool)> = Vec::new();
            egui::ScrollArea::vertical()
                .max_height(190.0)
                .id_salt("mods")
                .show(ui, |ui| {
                    for m in &self.mods {
                        ui.horizontal(|ui| {
                            let mut sent = !m.server_only;
                            if ui.checkbox(&mut sent, "").changed() {
                                changed.push((m.folder.clone(), m.loose, !sent));
                            }
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
                            if m.server_only {
                                ui.label(
                                    RichText::new(self.t("serveur seul", "server only"))
                                        .small()
                                        .color(th::GOLD),
                                );
                            }
                        });
                    }
                });
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
        });
    }

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_publish(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("IV · Publication", "IV · Publishing"));
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
                    w::hint(
                        ui,
                        self.t(
                            "ValhSync sert le pack depuis cette machine. Il faut ouvrir ce port TCP dans le pare-feu et le routeur pour les joueurs hors de votre réseau.",
                            "ValhSync serves the pack from this machine. This TCP port must be open in the firewall and router for players outside your network.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.bind)
                                .desired_width(180.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        match self.serving_at.clone() {
                            None => {
                                if ui
                                    .add_enabled(
                                        !self.busy,
                                        egui::Button::new(self.t("Démarrer", "Start")),
                                    )
                                    .clicked()
                                {
                                    self.pull_fields();
                                    if self.cfg.validate().is_err() {
                                        let e = self.cfg.validate().unwrap_err();
                                        self.notify(format!("{e:#}"), th::BLOOD_LIT);
                                    } else {
                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                        self.stop_serving = Some(tx);
                                        let cfg = self.cfg.clone();
                                        let data = self.data_dir.clone();
                                        self.start_job(move |rep| {
                                            worker::serve_blocking(cfg, data, rx, &rep);
                                        });
                                    }
                                }
                            }
                            Some(url) => {
                                if ui.button(self.t("Arrêter", "Stop")).clicked()
                                    && let Some(stop) = self.stop_serving.take()
                                {
                                    let _ = stop.send(());
                                }
                                ui.label(RichText::new(url).monospace().small().color(th::MOSS));
                            }
                        }
                    });
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

    fn card_invite(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("V · Code d'invitation", "V · Invite code"));
            w::hint(
                ui,
                self.t(
                    "Envoyez-le à vos joueurs. Ils peuvent aussi ajouter le serveur par son adresse seule : dans ce cas, donnez-leur l'empreinte de clé ci-dessous pour qu'ils la vérifient.",
                    "Send it to your players. They can also add the server by its address alone: give them the key fingerprint below so they can check it.",
                ),
            );
            ui.add_space(4.0);
            if self.invite.is_empty() {
                w::hint(
                    ui,
                    self.t(
                        "Le code apparaîtra après le premier Enregistrer : c'est à ce moment que votre clé de signature est créée.",
                        "The code appears after the first Save: that is when your signing key is created.",
                    ),
                );
                return;
            }
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
            ui.horizontal(|ui| {
                if ui.button(self.t("Copier", "Copy")).clicked() {
                    ui.ctx().copy_text(self.invite.clone());
                    let msg = self.t("Code copié.", "Code copied.").to_string();
                    self.notify(msg, th::MOSS);
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
    fn card_logs(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("II · Journal", "II · Log"));

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
                    ui.label(
                        RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                            .text_style(th::label_style())
                            .color(th::BONE_DIM),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(self.t("Ouvrir le fichier", "Open the file"))
                        .clicked()
                        && let Some(path) = self.log_sources.get(self.log_index)
                    {
                        open_path(path);
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
            w::section(ui, self.t("VI · Script de démarrage", "VI · Start script"));
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
