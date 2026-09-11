//! The admin window: detect, configure, publish, invite.
//!
//! Text is inline in both languages rather than in a key table: this window
//! has few strings and keeping the French and the English side by side makes
//! it obvious when one drifts.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText};
use valsync_core::limits::human_bytes;
use valsync_ui::frame as chrome;
use valsync_ui::theme as th;
use valsync_ui::widgets as w;

use super::worker::{self, Msg, Reporter};
use crate::config::{self, Config};
use crate::{detect, gameserver};

const POLL_GAME_SERVER: Duration = Duration::from_secs(2);
const NOTICE_TTL: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Fr,
    En,
}

impl Lang {
    fn detect() -> Self {
        let code = sys_locale::get_locale().unwrap_or_default().to_lowercase();
        if code.starts_with("fr") {
            Self::Fr
        } else {
            Self::En
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
    /// Excluded from the pack: it runs on the server only.
    server_only: bool,
    /// Comes from the client-extras folder: it runs on players only.
    client_only: bool,
}

#[derive(Debug, Clone)]
struct PackSummary {
    files: usize,
    bytes: u64,
    pack_id: String,
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
    mode: PublishMode,

    summary: Option<PackSummary>,
    invite: String,
    serving_at: Option<String>,
    stop_serving: Option<tokio::sync::oneshot::Sender<()>>,

    game_running: bool,
    game_checked: Instant,

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
                    "Aucune configuration trouvée : ValSync a rempli ce qu'il a pu détecter. Vérifiez, puis Enregistrer.",
                    "No configuration found: ValSync filled in what it could detect. Check it, then Save.",
                )
                .to_string();
            app.notify(msg, th::GOLD_LIT);
        }
        app.refresh_invite();
        app
    }

    /// A best-effort configuration for a machine that has never run ValSync.
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

    fn t(&self, fr: &'static str, en: &'static str) -> &'static str {
        self.lang.t(fr, en)
    }

    fn notify(&mut self, message: String, color: Color32) {
        self.notice = Some((message, color, Instant::now()));
    }

    fn dirty(&self) -> bool {
        self.never_saved || self.cfg != self.saved
    }

    /// Copy the text fields back into the configuration.
    fn pull_fields(&mut self) {
        self.cfg.server.name.clone_from(&self.name);
        self.cfg.server.game_address = self.game_address.trim().to_string();
        self.cfg.server.bind.clone_from(&self.bind);
        let url = self.public_url.trim();
        self.cfg.server.public_url = (!url.is_empty()).then(|| url.to_string());
        let root = self.server_root.trim();
        self.cfg.pack.server_root = (!root.is_empty()).then(|| PathBuf::from(root));
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
                if !e.path().is_dir() {
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
                    folder,
                });
            }
        };
        add(self.cfg.pack.server_root.as_ref(), false, &mut out);
        add(self.cfg.pack.client_extras.as_ref(), true, &mut out);
        for m in &mut out {
            m.server_only = self
                .cfg
                .pack
                .exclude
                .iter()
                .any(|p| p == &exclude_pattern(&m.folder));
        }
        out.sort_by_key(|m| m.folder.to_lowercase());
        out
    }

    fn set_server_only(&mut self, folder: &str, server_only: bool) {
        let pattern = exclude_pattern(folder);
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
                    pack_id,
                    invite,
                    skipped,
                } => {
                    self.invite = invite;
                    let short = pack_id.get(..15).unwrap_or(&pack_id).to_string();
                    self.summary = Some(PackSummary {
                        files,
                        bytes,
                        pack_id,
                        skipped,
                    });
                    let msg = format!(
                        "{} {files} {} · {} · {short}",
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

fn exclude_pattern(folder: &str) -> String {
    format!("BepInEx/plugins/{folder}/**")
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        if self.game_checked.elapsed() >= POLL_GAME_SERVER {
            self.game_running = gameserver::is_running();
            self.game_checked = Instant::now();
        }
        if self.busy || self.serving_at.is_some() {
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
        chrome::paint_window(ctx);
        self.top_bar(ctx);
        self.bottom_bar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.card_game_server(ui);
                    ui.add_space(12.0);
                    self.card_identity(ui);
                    ui.add_space(12.0);
                    self.card_mods(ui);
                    ui.add_space(12.0);
                    self.card_publish(ui);
                    ui.add_space(12.0);
                    self.card_invite(ui);
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
                    w::header(ui, "V A L S Y N C   ·   S E R V E U R");
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
                        ui.label(
                            RichText::new(self.config_path.display().to_string())
                                .small()
                                .color(th::BONE_DIM),
                        );
                    });
                });
            });
    }

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn card_game_server(&mut self, ui: &mut egui::Ui) {
        th::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("1 · Serveur de jeu", "1 · Game server"));

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
                            self.t("Serveur dédié avec BepInEx", "Dedicated server with BepInEx")
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
                                ui.selectable_value(&mut self.script_index, i, label);
                            }
                        });
                });
                if let Some(s) = self.scripts.get(self.script_index) {
                    self.cfg.game_server.start_script = Some(s.path.clone());
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
                        bits.push(
                            self.t("mot de passe défini", "password set")
                                .to_string(),
                        );
                    }
                    ui.label(RichText::new(bits.join(" · ")).small().color(th::RUNE));
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                w::status_dot(
                    ui,
                    if self.game_running { th::MOSS } else { th::BONE_DIM },
                    if self.game_running {
                        self.t("Serveur de jeu en ligne", "Game server online")
                    } else {
                        self.t("Serveur de jeu arrêté", "Game server stopped")
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let can_start = !self.game_running && !self.scripts.is_empty();
                    if ui
                        .add_enabled(
                            can_start,
                            egui::Button::new(self.t("Démarrer le serveur", "Start the server")),
                        )
                        .clicked()
                        && let Some(s) = self.scripts.get(self.script_index)
                    {
                        let launch = gameserver::Launch::Script(s.path.clone());
                        match gameserver::start(&launch) {
                            Ok(()) => {
                                self.game_running = true;
                                self.game_checked = Instant::now();
                                let msg = self.t(
                                    "Serveur de jeu lancé dans sa propre fenêtre.",
                                    "Game server started in its own window.",
                                ).to_string();
                                self.notify(msg, th::MOSS);
                            }
                            Err(e) => self.notify(format!("{e:#}"), th::BLOOD_LIT),
                        }
                    }
                });
            });
            w::hint(
                ui,
                self.t(
                    "ValSync ne l'arrête jamais : Ctrl+C dans sa fenêtre, c'est ce qui sauvegarde le monde.",
                    "ValSync never stops it: Ctrl+C in its window is what saves the world.",
                ),
            );
        });
    }

    fn card_identity(&mut self, ui: &mut egui::Ui) {
        th::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("2 · Identité et adresse", "2 · Identity and address"));

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
                        egui::Button::new(self.t("Détecter mon IP publique", "Detect my public IP")),
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
            if !addr.is_empty() && valsync_core::manifest::is_private_host(addr) {
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
            } else if !addr.is_empty() && !valsync_core::manifest::is_valid_game_address(addr) {
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
        th::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("3 · Mods", "3 · Mods"));
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
            let mut changed: Vec<(String, bool)> = Vec::new();
            egui::ScrollArea::vertical()
                .max_height(190.0)
                .id_salt("mods")
                .show(ui, |ui| {
                    for m in &self.mods {
                        ui.horizontal(|ui| {
                            let mut sent = !m.server_only;
                            if ui.checkbox(&mut sent, "").changed() {
                                changed.push((m.folder.clone(), !sent));
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
            for (folder, server_only) in changed {
                self.set_server_only(&folder, server_only);
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
                        "Unshamed, ConfigurationManager, EquipmentAndQuickSlots… Ils ne sont pas installés sur le serveur, donc ValSync ne peut pas les y trouver : déposez-les ici, à la même arborescence que le jeu (BepInEx/plugins/...). Si vous n'en avez aucun, ignorez ce dossier.",
                        "Unshamed, ConfigurationManager, EquipmentAndQuickSlots… They are not installed on the server, so ValSync cannot find them there: drop them here, laid out like the game (BepInEx/plugins/...). If you have none, ignore this folder.",
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
        th::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("4 · Publication", "4 · Publishing"));
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
                            "Recommandé : aucun port à ouvrir. ValSync écrit un dossier à déposer sur n'importe quel espace web (GitHub Pages, S3, votre hébergeur).",
                            "Recommended: no port to open. ValSync writes a folder you upload to any web space (GitHub Pages, S3, your host).",
                        ),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.export_dir)
                                .desired_width(ui.available_width() - 210.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        if ui
                            .add_enabled(!self.busy, egui::Button::new(self.t("Exporter", "Export")))
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
                            "ValSync sert le pack depuis cette machine. Il faut ouvrir ce port TCP dans le pare-feu et le routeur pour les joueurs hors de votre réseau.",
                            "ValSync serves the pack from this machine. This TCP port must be open in the firewall and router for players outside your network.",
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
                    .add_enabled(!self.busy, egui::Button::new(self.t("Analyser le pack", "Scan the pack")))
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
                            "{} {} · {} · {}",
                            s.files,
                            self.t("fichiers", "files"),
                            human_bytes(s.bytes),
                            s.pack_id.get(..15).unwrap_or(&s.pack_id)
                        ))
                        .small()
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
        th::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section(ui, self.t("5 · Code d'invitation", "5 · Invite code"));
            w::hint(
                ui,
                self.t(
                    "À donner aux joueurs, ou à placer dans un fichier valsync-invite.txt à côté de valsync.exe : le launcher l'importe tout seul.",
                    "Hand it to players, or drop it in a valsync-invite.txt next to valsync.exe: the launcher imports it by itself.",
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
            w::code_block(ui, &self.invite);
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
                        "Crée un dossier avec valsync.exe et le code : les joueurs n'ont rien à coller.",
                        "Creates a folder holding valsync.exe and the code: players paste nothing.",
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

        std::fs::write(dir.join("valsync-invite.txt"), format!("{}\n", self.invite))
            .with_context(|| format!("cannot write the invite file in {}", dir.display()))?;

        // The launcher sits next to this executable in a release package.
        let launcher = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .map(|d| {
                d.join(if cfg!(windows) {
                    "valsync.exe"
                } else {
                    "valsync"
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
                    "Ajoutez valsync.exe dans ce dossier, a cote de valsync-invite.txt,\n                     puis envoyez le dossier (ou son zip) a vos joueurs.\n\n                     Add valsync.exe to this folder, next to valsync-invite.txt,\n                     then send the folder (or a zip of it) to your players.\n",
                )
                .ok();
            }
        }
        Ok(dir)
    }
}

/// Show a file or folder in the system file manager. Best effort.
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
