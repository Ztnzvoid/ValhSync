//! Window state and layout. One screen: pick a server, read its status, press
//! PLAY. Everything slow runs on a worker thread and reports back through a
//! channel; the UI thread never blocks.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText};
use valhsync_core::limits::human_bytes;
use valhsync_core::{Action, Invite};

use super::i18n::{Key, Lang, text};
use crate::engine::{self, Applied, Context, Event, Prepared, Progress};
use crate::paths::AppPaths;
use crate::servers::{KnownServer, ServerBook};
use crate::settings::Settings;
use crate::{game, invite_file};
use valhsync_ui::frame as chrome;
use valhsync_ui::theme as th;

const NOTICE_TTL: Duration = Duration::from_secs(7);
/// How often the launcher asks the server again, on its own.
const AUTO_REFRESH: Duration = Duration::from_secs(25);
/// Rows of the mod list shown before "show all" takes over.
const MODS_SHOWN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Contacting,
    Downloading { index: usize, count: usize },
    Applying,
}

#[derive(Debug)]
enum Msg {
    Progress {
        done: u64,
        total: u64,
        phase: Phase,
        detail: String,
    },
    Prepared(Result<Box<Prepared>, Failure>),
    Discovered(Result<Box<engine::Discovered>, String>),
    Applied(Result<(Applied, Option<String>), String>),
    /// What the server offers as a newer launcher, if anything. Checked on
    /// the same trip as the manifest: it is the same key and the same server.
    Offered(Option<Box<valhsync_core::UpdateOffer>>),
    /// The new launcher is in place; the path is what to start.
    Updated(Result<std::path::PathBuf, String>),
    /// A job failed in a way it had no plan for. The window says so rather
    /// than closing, which is what the process did before.
    Crashed(String),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Check,
    Discover,
    Sync,
    SelfUpdate,
}

/// Worker-side handle: sends messages and wakes the UI.
#[derive(Clone)]
struct Reporter {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Reporter {
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

impl Progress for Reporter {
    #[allow(clippy::cast_precision_loss)] // a progress fraction
    fn on(&mut self, event: Event<'_>) {
        let msg = match event {
            Event::Fetching { .. } => Msg::Progress {
                done: 0,
                total: 0,
                phase: Phase::Contacting,
                detail: String::new(),
            },
            Event::Downloading {
                index, count, path, ..
            } => Msg::Progress {
                done: u64::MAX, // "no byte count in this event"
                total: 0,
                phase: Phase::Downloading { index, count },
                detail: path.to_string(),
            },
            Event::Progress { done, total } => Msg::Progress {
                done,
                total,
                phase: Phase::Downloading { index: 0, count: 0 },
                detail: String::new(),
            },
            Event::Applying { .. } => Msg::Progress {
                done: u64::MAX,
                total: 0,
                phase: Phase::Applying,
                detail: String::new(),
            },
            Event::Planned(_) | Event::Done => return,
        };
        self.send(msg);
    }
}

#[derive(Debug, Default)]
struct AddDialog {
    input: String,
    error: Option<String>,
    /// Set once an address answered: the player confirms the fingerprint
    /// before the key is pinned.
    pending: Option<engine::Discovered>,
    asking: bool,
}

#[derive(Debug)]
struct ProgressView {
    done: u64,
    total: u64,
    phase: Phase,
    detail: String,
    /// Smoothed download rate in bytes per second.
    speed: f64,
    sample: (Instant, u64),
}

impl ProgressView {
    fn new(phase: Phase) -> Self {
        Self {
            done: 0,
            total: 0,
            phase,
            detail: String::new(),
            speed: 0.0,
            sample: (Instant::now(), 0),
        }
    }

    #[allow(clippy::cast_precision_loss)] // a bar between 0 and 1
    fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }

    /// Fold a new byte count into a smoothed rate. Sampling over at least a
    /// quarter of a second keeps the figure readable instead of flickering.
    fn observe(&mut self, done: u64) {
        self.done = done;
        let elapsed = self.sample.0.elapsed().as_secs_f64();
        if elapsed < 0.25 {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let instant = done.saturating_sub(self.sample.1) as f64 / elapsed;
        self.speed = if self.speed == 0.0 {
            instant
        } else {
            self.speed.mul_add(0.7, instant * 0.3)
        };
        self.sample = (Instant::now(), done);
    }

    fn seconds_left(&self) -> Option<u64> {
        if self.speed < 1.0 || self.total <= self.done {
            return None;
        }
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        Some(((self.total - self.done) as f64 / self.speed) as u64)
    }
}

/// What the server's pack means for one mod on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModStatus {
    Installed,
    ToInstall,
    ToUpdate,
}

#[derive(Debug, Clone)]
struct ModRow {
    name: String,
    status: ModStatus,
}

/// Read the mod list out of a plan: one row per folder under
/// `BepInEx/plugins`, plus any loose plugin sitting directly in it.
fn mod_rows(prepared: &Prepared) -> Vec<ModRow> {
    use std::collections::BTreeMap;
    const PLUGINS: &str = "BepInEx/plugins/";

    let mut rows: BTreeMap<String, ModStatus> = BTreeMap::new();
    for item in &prepared.plan.items {
        let Some(rest) = item.path.strip_prefix(PLUGINS) else {
            continue;
        };
        let name = rest.split_once('/').map_or(rest, |(folder, _)| folder);
        let status = match item.action {
            Action::Add => ModStatus::ToInstall,
            Action::Replace => ModStatus::ToUpdate,
            _ => ModStatus::Installed,
        };
        rows.entry(name.to_string())
            .and_modify(|current| {
                // One file to install makes the whole mod "to install".
                if status != ModStatus::Installed && *current == ModStatus::Installed {
                    *current = status;
                }
            })
            .or_insert(status);
    }
    rows.into_iter()
        .map(|(name, status)| ModRow { name, status })
        .collect()
}

/// Where the game is, as far as the launcher can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GameState {
    Idle,
    /// Steam was asked to start the game; we are waiting for the process.
    Launching(Instant),
    Running,
}

/// One line of the mod list: a dot, the name, and what will happen to it.
fn mod_row(ui: &mut egui::Ui, row: &ModRow, lang: Lang) {
    let (colour, label) = match row.status {
        ModStatus::Installed => (th::MOSS, text(lang, Key::ModInstalled)),
        ModStatus::ToInstall => (th::GOLD, text(lang, Key::ModToInstall)),
        ModStatus::ToUpdate => (th::GOLD_LIT, text(lang, Key::ModToUpdate)),
    };
    ui.horizontal(|ui| {
        valhsync_ui::widgets::dot(ui, colour);
        ui.label(RichText::new(&row.name).color(th::BONE));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(label)
                    .text_style(th::label_style())
                    .color(colour),
            );
        });
    });
}

/// Why a check failed, and whether the server answered at all.
#[derive(Debug, Clone)]
struct Failure {
    message: String,
    offline: bool,
}

impl From<crate::SyncError> for Failure {
    fn from(error: crate::SyncError) -> Self {
        Self {
            offline: engine::is_unreachable(&error),
            message: error.to_string(),
        }
    }
}

#[derive(Debug)]
enum Status {
    NoServer,
    Checking,
    Ready,
    Error(Failure),
}

pub(super) struct App {
    lang: Lang,
    paths: AppPaths,
    settings: Settings,
    book: ServerBook,
    selected: Option<String>,
    status: Status,
    prepared: Option<Prepared>,
    mods: Vec<ModRow>,
    job: Option<(Job, Receiver<Msg>)>,
    progress: Option<ProgressView>,
    notice: Option<(String, Color32, Instant)>,
    /// A build this server offers that is worth taking.
    update: Option<valhsync_core::UpdateOffer>,
    /// The new launcher is running; this one has nothing left to do.
    quit_after_update: bool,
    add_dialog: Option<AddDialog>,
    confirm_open: bool,
    settings_open: bool,
    mods_open: bool,
    /// What each sync changed, kept so a player can read it afterwards.
    news: crate::news::News,
    news_open: bool,
    /// Height the window should have for what it currently shows.
    wanted_height: f32,
    /// Height the notice bar took last frame, zero when it is hidden.
    notice_height: f32,
    /// Window title as last set, so it is only pushed when it changes.
    title: String,
    game_state: GameState,
    game_root_input: String,
    last_check: Instant,
    was_focused: bool,
    fatal: Option<String>,
    egui_ctx: egui::Context,
}

impl App {
    pub(super) fn new(egui_ctx: &egui::Context) -> Self {
        // The launcher this one replaced is still on disk: Windows only lets
        // go of it once the process that was running it has exited, which by
        // now it has.
        crate::selfupdate::clean_stale();
        let (paths, settings, book, fatal) = match AppPaths::discover() {
            Ok(paths) => {
                let settings = Settings::load(&paths).unwrap_or_default();
                let book = ServerBook::load(&paths).unwrap_or_default();
                (paths, settings, book, None)
            }
            Err(e) => (
                AppPaths {
                    config_dir: PathBuf::new(),
                    backups_dir: PathBuf::new(),
                },
                Settings::default(),
                ServerBook::default(),
                Some(e.to_string()),
            ),
        };
        let lang = Lang::detect(settings.language.as_deref());
        let news = crate::news::News::load(&paths).unwrap_or_default();
        let mut app = Self {
            lang,
            game_root_input: settings
                .game_root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            paths,
            settings,
            book,
            selected: None,
            status: Status::NoServer,
            prepared: None,
            mods: Vec::new(),
            job: None,
            progress: None,
            notice: None,
            update: None,
            quit_after_update: false,
            add_dialog: None,
            confirm_open: false,
            settings_open: false,
            mods_open: false,
            news,
            news_open: false,
            wanted_height: 0.0,
            notice_height: 0.0,
            title: String::new(),
            game_state: GameState::Idle,
            last_check: Instant::now(),
            was_focused: true,
            fatal,
            egui_ctx: egui_ctx.clone(),
        };
        if app.fatal.is_none() {
            match invite_file::import_if_present(&app.paths) {
                Ok(Some((server, _))) => {
                    app.book = ServerBook::load(&app.paths).unwrap_or_default();
                    app.notify(format!("{}: {}", app.t(Key::Added), server.name), th::MOSS);
                }
                Ok(None) => {}
                Err(e) => app.notify(e.to_string(), th::BLOOD_LIT),
            }
            app.selected = app.book.resolve(None).ok().map(|s| s.id.clone());
            app.check();
        }
        app
    }

    fn t(&self, key: Key) -> &'static str {
        text(self.lang, key)
    }

    fn notify(&mut self, message: String, color: Color32) {
        self.notice = Some((message, color, Instant::now()));
    }

    fn selected_server(&self) -> Option<KnownServer> {
        let id = self.selected.as_ref()?;
        self.book.servers.iter().find(|s| &s.id == id).cloned()
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn start_job<F>(&mut self, job: Job, f: F)
    where
        F: FnOnce(Reporter) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let reporter = Reporter {
            tx,
            ctx: self.egui_ctx.clone(),
        };
        self.job = Some((job, rx));
        self.progress = None;
        std::thread::spawn(move || {
            // Whatever happens in here, the window keeps running and hears
            // about it. A worker taking the process down with it leaves the
            // person with a window that vanished and nothing to report.
            let work = reporter.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(work)));
            if let Err(payload) = outcome {
                reporter.send(Msg::Crashed(valhsync_core::crash::describe(&*payload)));
            }
            reporter.send(Msg::Done);
        });
    }

    /// Ask the server what would change. Runs on the worker.
    fn check(&mut self) {
        let Some(server) = self.selected_server() else {
            self.status = Status::NoServer;
            self.prepared = None;
            return;
        };
        if self.busy() {
            return;
        }
        self.status = Status::Checking;
        self.last_check = Instant::now();
        self.start_job(Job::Check, move |mut rep| {
            let result = Context::discover()
                .and_then(|ctx| engine::prepare(&ctx, &server, &mut rep))
                .map(Box::new)
                .map_err(Failure::from);
            rep.send(Msg::Prepared(result));
            // Same server, same pinned key, same trip. A server that offers
            // nothing is the normal case and says nothing about the sync.
            let offered = server.public_key().ok().and_then(|key| {
                crate::http::Client::new()
                    .ok()?
                    .fetch_update_offer(&server.url, &key)
                    .ok()
                    .flatten()
            });
            rep.send(Msg::Offered(offered.map(Box::new)));
        });
    }

    /// Take the build on offer: download it beside the running launcher,
    /// verify it, put it in place. Nothing is started here -- the window
    /// decides when to hand over.
    fn start_self_update(&mut self) {
        let (Some(server), Some(offer)) = (self.selected_server(), self.update.clone()) else {
            return;
        };
        if self.busy() {
            return;
        }
        self.start_job(Job::SelfUpdate, move |rep| {
            let result = (|| {
                let staged = crate::selfupdate::staging_path()?;
                let client = crate::http::Client::new()?;
                let mut seen = 0u64;
                client.download_update(&server.url, &offer, &staged, &mut |done| {
                    if done != seen {
                        seen = done;
                        rep.send(Msg::Progress {
                            done,
                            total: offer.size,
                            phase: Phase::Downloading { index: 1, count: 1 },
                            detail: offer.exe.clone(),
                        });
                    }
                })?;
                crate::selfupdate::install(&staged)
            })();
            rep.send(Msg::Updated(result.map_err(|e| e.to_string())));
        });
    }

    /// Is this offer worth putting in front of the player? `wanted` answers
    /// for the build; this also refuses one already installed, which is what
    /// a server whose document overstates its file would otherwise loop on.
    fn worth_offering(&self, offer: &valhsync_core::UpdateOffer) -> bool {
        let newest_seen = self.selected_server().and_then(|s| s.last_offer_at.clone());
        crate::selfupdate::wanted(offer)
            && self.settings.installed_build.as_deref() != Some(offer.blake3.as_str())
            && !offer.is_stale_against(newest_seen.as_deref())
    }

    fn start_sync(&mut self, launch: bool) {
        let Some(prepared) = self.prepared.clone() else {
            return;
        };
        if self.busy() {
            return;
        }
        self.confirm_open = false;
        self.start_job(Job::Sync, move |mut rep| {
            let result = Context::discover()
                .and_then(|ctx| engine::apply(&ctx, &prepared, &mut rep))
                .map_err(|e| e.to_string())
                .map(|applied| {
                    let launch_error = launch
                        .then(|| {
                            game::launch(&prepared.install, &prepared.manifest.game_address)
                                .err()
                                .map(|e| e.to_string())
                        })
                        .flatten();
                    (applied, launch_error)
                });
            rep.send(Msg::Applied(result));
        });
    }

    /// One button, three jobs: bring the installation up to date, then start
    /// the game, and say which it is doing.
    fn on_action(&mut self) {
        let Some(p) = &self.prepared else {
            return;
        };
        if p.is_up_to_date() {
            self.launch_game();
        } else if p.needs_confirmation {
            self.confirm_open = true;
        } else {
            self.start_sync(false);
        }
    }

    fn launch_game(&mut self) {
        let Some(p) = &self.prepared else {
            return;
        };
        match game::launch(&p.install, &p.manifest.game_address) {
            Ok(_) => self.game_state = GameState::Launching(Instant::now()),
            Err(e) => self.notify(e.to_string(), th::BLOOD_LIT),
        }
    }

    /// Follow the game after Steam was asked to start it, so the button can
    /// say what is actually happening.
    fn track_game(&mut self) {
        match self.game_state {
            GameState::Idle => {}
            GameState::Launching(since) => {
                if game::is_running() {
                    self.game_state = GameState::Running;
                } else if since.elapsed() > Duration::from_secs(90) {
                    // Steam never brought it up; stop claiming otherwise.
                    self.game_state = GameState::Idle;
                }
            }
            GameState::Running => {
                if !game::is_running() {
                    self.game_state = GameState::Idle;
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)] // one arm per message, read top to bottom
    fn drain_messages(&mut self) {
        let Some((job, rx)) = &self.job else {
            return;
        };
        let job = *job;
        let mut finished = false;
        let mut incoming = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            incoming.push(msg);
        }
        for msg in incoming {
            match msg {
                Msg::Progress {
                    done,
                    total,
                    phase,
                    detail,
                } => {
                    let view = self
                        .progress
                        .get_or_insert_with(|| ProgressView::new(phase));
                    if done != u64::MAX {
                        view.total = total;
                        view.observe(done);
                    }
                    if !matches!(phase, Phase::Downloading { index: 0, .. }) {
                        view.phase = phase;
                    }
                    if !detail.is_empty() {
                        view.detail = detail;
                    }
                }
                Msg::Prepared(Ok(prepared)) => {
                    self.mods = mod_rows(&prepared);
                    self.prepared = Some(*prepared);
                    self.status = Status::Ready;
                }
                Msg::Discovered(Ok(found)) => {
                    if let Some(dialog) = &mut self.add_dialog {
                        dialog.asking = false;
                        dialog.error = None;
                        dialog.pending = Some(*found);
                    }
                }
                Msg::Discovered(Err(e)) => {
                    if let Some(dialog) = &mut self.add_dialog {
                        dialog.asking = false;
                        dialog.error = Some(e);
                    }
                }
                Msg::Prepared(Err(failure)) => {
                    self.prepared = None;
                    self.mods.clear();
                    self.status = Status::Error(failure);
                }
                Msg::Applied(Ok((applied, launch_error))) => {
                    self.record_news();
                    let c = applied.counts;
                    let summary = format!(
                        "{}: {} {}, {} {}, {} {}",
                        self.t(Key::SyncDone),
                        c.add,
                        self.t(Key::PlanInstall),
                        c.replace,
                        self.t(Key::PlanUpdate),
                        c.quarantine,
                        self.t(Key::PlanQuarantine)
                    );
                    match launch_error {
                        Some(e) => self.notify(e, th::BLOOD_LIT),
                        None => self.notify(summary, th::MOSS),
                    }
                }
                Msg::Applied(Err(e)) => self.notify(e, th::BLOOD_LIT),
                Msg::Offered(offer) => {
                    self.update = offer.map(|o| *o).filter(|o| self.worth_offering(o));
                    // Remember the newest offer this server has made, so a
                    // genuine older one replayed at us later is recognised.
                    if let Some(offer) = &self.update {
                        let at = offer.generated_at.clone();
                        if let Some(id) = self.selected.clone()
                            && let Some(s) = self.book.servers.iter_mut().find(|s| s.id == id)
                        {
                            s.last_offer_at = Some(at);
                            if let Err(e) = self.book.save(&self.paths) {
                                self.notify(e.to_string(), th::BLOOD_LIT);
                            }
                        }
                    }
                }
                Msg::Updated(Ok(exe)) => {
                    // Remember what went in, so an offer that does not
                    // actually make the launcher newer is not taken twice.
                    if let Some(offer) = &self.update {
                        self.settings.installed_build = Some(offer.blake3.clone());
                        self.save_settings();
                    }
                    self.update = None;
                    match crate::selfupdate::relaunch(&exe) {
                        Ok(()) => self.quit_after_update = true,
                        Err(e) => self.notify(e.to_string(), th::BLOOD_LIT),
                    }
                }
                Msg::Updated(Err(e)) => {
                    let what = self.t(Key::UpdateFailed).to_string();
                    self.notify(format!("{what}: {e}"), th::BLOOD_LIT);
                }
                Msg::Crashed(what) => {
                    let where_ = valhsync_core::crash::log_path()
                        .map(|p| format!("\n{}", p.display()))
                        .unwrap_or_default();
                    let message = format!("{}: {what}{where_}", self.t(Key::Crashed));
                    self.status = Status::Error(Failure {
                        message,
                        offline: false,
                    });
                }
                Msg::Done => finished = true,
            }
        }
        if finished {
            self.job = None;
            self.progress = None;
            if job != Job::Check {
                self.check();
            }
        }
    }

    /// The taskbar should say which server is selected, and nothing more.
    fn update_title(&mut self, ctx: &egui::Context) {
        let wanted = match self.selected_server() {
            Some(server) => format!("ValhSync — {}", server.name),
            None => "ValhSync".to_string(),
        };
        if wanted != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(wanted.clone()));
            self.title = wanted;
        }
    }

    /// Ask the server again on a timer, and the moment the window regains
    /// focus: a player who alt-tabs back should see the truth, not a stale
    /// screen, and should never have to press a button for it.
    fn auto_refresh(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let regained = focused && !self.was_focused;
        self.was_focused = focused;
        if self.busy() || self.selected.is_none() || self.add_dialog.is_some() {
            return;
        }
        if regained || self.last_check.elapsed() >= AUTO_REFRESH {
            self.check();
        }
    }

    /// The offer, and the one button that takes it.
    fn update_bar(&mut self, ui: &mut egui::Ui) {
        let Some(offer) = self.update.clone() else {
            return;
        };
        let busy = self.busy();
        let source = self.selected_server().map(|s| {
            format!(
                "{} · {} · {}",
                self.t(Key::UpdateFrom),
                s.name,
                s.fingerprint()
            )
        });
        th::callout(ui, th::GOLD, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(
                    th::GOLD_LIT,
                    format!("{} — {}", self.t(Key::UpdateReady), offer.version),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new(self.t(Key::UpdateInstall)).color(th::NIGHT),
                            )
                            .fill(th::GOLD),
                        )
                        .clicked()
                    {
                        self.start_self_update();
                    }
                });
            });
            // The one click above is the whole consent gate for running a
            // binary chosen by someone else. It has to say whose.
            if let Some(source) = source {
                ui.label(RichText::new(source).small().color(th::BONE));
            }
            ui.label(
                RichText::new(self.t(Key::UpdateReplaces))
                    .small()
                    .color(th::BONE_DIM),
            );
        });
        ui.add_space(12.0);
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.paths) {
            self.notify(e.to_string(), th::BLOOD_LIT);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();
        if self.quit_after_update {
            // The replacement is already running. Two launchers on one game
            // folder is exactly the race the backup journal cannot help with.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.track_game();
        self.auto_refresh(ctx);
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some((_, _, since)) = &self.notice {
            if since.elapsed() > NOTICE_TTL {
                self.notice = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }

        chrome::clamp_to_display(ctx);
        chrome::handle_edge_resize(ctx);
        self.header(ctx);
        self.notice_bar(ctx);
        let panel = egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(th::NIGHT)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| {
                th::backdrop(ui.ctx(), ui.painter(), ui.max_rect().expand(20.0));
                if let Some(fatal) = self.fatal.clone() {
                    th::callout(ui, th::BLOOD, |ui| {
                        ui.colored_label(th::BLOOD_LIT, fatal);
                    });
                    return ui.cursor().top();
                }
                self.update_bar(ui);
                self.action_bar(ui);
                ui.add_space(12.0);
                if self.book.servers.is_empty() {
                    self.empty_state(ui);
                } else {
                    self.server_card(ui);
                    if !self.mods.is_empty() {
                        ui.add_space(12.0);
                        self.mods_card(ui);
                    }
                }
                // The cursor sits just under the last thing drawn.
                ui.cursor().top()
            })
            .inner;
        // The window is as tall as what it draws: the content, the notice bar
        // and one margin. A dialog floats above all that and needs its own
        // room, or its buttons end up past the bottom edge.
        self.wanted_height = panel + 20.0 + self.notice_height;
        if self.add_dialog.is_some()
            || self.confirm_open
            || self.settings_open
            || self.mods_open
            || self.news_open
        {
            self.wanted_height = self.wanted_height.max(600.0);
        }

        self.update_title(ctx);
        chrome::fit_to_content(
            ctx,
            self.wanted_height,
            egui::vec2(720.0, 360.0),
            egui::vec2(940.0, 900.0),
        );
        chrome::draw_border(ctx);
        self.mods_dialog(ctx);
        self.news_dialog(ctx);
        self.add_dialog(ctx);
        self.confirm_dialog(ctx);
        self.settings_dialog(ctx);
    }
}

// ---- layout pieces -------------------------------------------------------

impl App {
    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(th::NIGHT)
                    .inner_margin(egui::Margin {
                        left: 20,
                        right: 0,
                        top: 10,
                        bottom: 12,
                    })
                    .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT)),
            )
            .show(ctx, |ui| {
                chrome::draggable(ui, ui.max_rect());
                ui.horizontal(|ui| {
                    valhsync_ui::widgets::header(ui, "V A L H S Y N C");
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        chrome::window_controls(ui);
                        ui.add_space(8.0);
                        if ui.button(self.t(Key::Settings)).clicked() {
                            self.settings_open = !self.settings_open;
                        }
                        let other = self.lang.other();
                        if ui
                            .button(other.code().to_uppercase())
                            .on_hover_text(self.t(Key::Language))
                            .clicked()
                        {
                            self.lang = other;
                            self.settings.language = Some(other.code().to_string());
                            self.save_settings();
                        }
                    });
                });
            });
    }

    fn notice_bar(&mut self, ctx: &egui::Context) {
        let Some((message, color, _)) = self.notice.clone() else {
            self.notice_height = 0.0;
            return;
        };
        let bar = egui::TopBottomPanel::bottom("notice")
            .frame(
                egui::Frame::new()
                    .fill(th::PANEL)
                    .inner_margin(egui::Margin::symmetric(20, 10))
                    .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    valhsync_ui::widgets::dot(ui, color);
                    ui.label(RichText::new(message).color(th::BONE));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            self.notice = None;
                        }
                    });
                });
            });
        self.notice_height = bar.response.rect.height();
    }

    /// Pick a server, add one, drop one, and the action the whole window is
    /// built around.
    fn action_bar(&mut self, ui: &mut egui::Ui) {
        let servers: Vec<(String, String)> = self
            .book
            .servers
            .iter()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        let before = self.selected.clone();
        let mut forget = false;

        ui.horizontal(|ui| {
            if !servers.is_empty() {
                let current = servers
                    .iter()
                    .find(|(id, _)| Some(id) == self.selected.as_ref())
                    .map_or("—", |(_, name)| name.as_str())
                    .to_string();
                egui::ComboBox::from_id_salt("server")
                    .selected_text(RichText::new(current).strong())
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for (id, name) in &servers {
                            ui.selectable_value(&mut self.selected, Some(id.clone()), name);
                        }
                    });
            }
            if ui
                .add_enabled(!self.busy(), egui::Button::new(self.t(Key::AddServer)))
                .clicked()
            {
                self.add_dialog = Some(AddDialog::default());
            }
            if !servers.is_empty() {
                let bin = valhsync_ui::widgets::icon_button(
                    ui,
                    !self.busy(),
                    self.t(Key::ForgetServer),
                    th::trash,
                );
                if bin.clicked() {
                    forget = true;
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.action_button(ui);
                self.repair_button(ui);
            });
        });

        if forget {
            self.forget_selected();
        }
        if before != self.selected {
            if let Some(id) = &self.selected {
                self.book.set_default(id);
                let _ = self.book.save(&self.paths);
            }
            self.prepared = None;
            self.mods.clear();
            self.check();
        }
    }

    /// PLAY, UPDATE, or what the game is doing right now.
    fn action_button(&mut self, ui: &mut egui::Ui) {
        let pending = self.prepared.as_ref().is_some_and(|p| !p.is_up_to_date());
        let ready = matches!(self.status, Status::Ready) && !self.busy();

        let label = match self.game_state {
            GameState::Launching(since) => {
                // Three dots that fill and clear once a second.
                let dots = ".".repeat(1 + (since.elapsed().as_millis() / 400 % 3) as usize);
                format!("{}{dots}", self.t(Key::Launching2))
            }
            GameState::Running => self.t(Key::Launching2).to_string(),
            GameState::Idle if pending => self.t(Key::Update).to_string(),
            GameState::Idle => self.t(Key::Play).to_string(),
        };
        let enabled = ready && self.game_state == GameState::Idle;
        let (ink, plate) = if enabled {
            (th::NIGHT, th::GOLD)
        } else {
            (th::BONE_DIM, th::LEATHER)
        };
        let button = ui.add_enabled(
            enabled,
            egui::Button::new(
                RichText::new(label)
                    .font(th::display_font(18.0))
                    .strong()
                    .color(ink),
            )
            .fill(plate)
            .corner_radius(4)
            .min_size(egui::vec2(220.0, 44.0)),
        );
        if enabled {
            let heat = if button.hovered() { 0.30 } else { 0.16 };
            th::glow(ui.painter(), button.rect, th::GOLD.gamma_multiply(heat));
            th::brackets(ui.painter(), button.rect.expand(4.0), th::EDGE);
        }
        if button.clicked() {
            self.on_action();
        }
    }

    /// The anvil: put everything back the way the server has it.
    fn repair_button(&mut self, ui: &mut egui::Ui) {
        let enabled = matches!(self.status, Status::Ready)
            && !self.busy()
            && self.game_state == GameState::Idle;
        let tooltip = format!("{} — {}", self.t(Key::Repair), self.t(Key::RepairHint));
        let response = valhsync_ui::widgets::icon_button(ui, enabled, &tooltip, th::anvil);
        if enabled && response.clicked() {
            self.start_repair();
        }
    }

    /// Re-plan with every file enforced, then apply it.
    fn start_repair(&mut self) {
        let Some(server) = self.selected_server() else {
            return;
        };
        self.status = Status::Checking;
        self.last_check = Instant::now();
        self.start_job(Job::Sync, move |mut rep| {
            let result = (|| {
                let mut ctx = Context::discover()?;
                ctx.repair = true;
                let prepared = engine::prepare(&ctx, &server, &mut rep)?;
                engine::apply(&ctx, &prepared, &mut rep)
            })()
            .map_err(|e| e.to_string())
            .map(|applied| (applied, None));
            rep.send(Msg::Applied(result));
        });
    }

    fn forget_selected(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if let Ok(removed) = self.book.remove(&id) {
            let _ = self.book.save(&self.paths);
            self.notify(
                format!("{}: {}", self.t(Key::Forget), removed.name),
                th::GOLD_LIT,
            );
        }
        self.selected = self.book.resolve(None).ok().map(|s| s.id.clone());
        self.prepared = None;
        self.mods.clear();
        self.status = Status::NoServer;
        self.check();
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(self.t(Key::NoServerHint)).color(th::BONE_DIM));
                ui.add_space(14.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(self.t(Key::AddServer))
                                .size(18.0)
                                .strong()
                                .color(th::NIGHT),
                        )
                        .fill(th::GOLD)
                        .min_size(egui::vec2(240.0, 46.0)),
                    )
                    .clicked()
                {
                    self.add_dialog = Some(AddDialog::default());
                }
            });
            ui.add_space(20.0);
        });
    }

    #[allow(clippy::too_many_lines)] // one card, read top to bottom
    fn server_card(&mut self, ui: &mut egui::Ui) {
        let Some(server) = self.selected_server() else {
            return;
        };
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // A lamp beside the name: lit brass when the pack answers,
                // dull metal when it does not. No word needed.
                let lamp = match &self.status {
                    Status::Ready => th::GOLD_LIT,
                    Status::Checking => th::GOLD.gamma_multiply(0.55),
                    // Unreachable is the dullest: the server said nothing at
                    // all. Any other failure means it answered.
                    Status::Error(failure) if failure.offline => th::GOLD.gamma_multiply(0.18),
                    _ => th::GOLD.gamma_multiply(0.40),
                };
                valhsync_ui::widgets::lamp(ui, lamp, 26.0);
                ui.label(
                    RichText::new(&server.name)
                        .font(th::display_font(19.0))
                        .strong()
                        .color(th::GOLD_LIT),
                );
                if let Some((colour, line)) = self.ready_summary() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(line)
                                .text_style(th::label_style())
                                .color(th::BONE),
                        );
                        valhsync_ui::widgets::dot(ui, colour);
                    });
                }
            });
            ui.label(RichText::new(&server.url).small().color(th::BONE_DIM));
            ui.add_space(10.0);
            self.whats_new_block(ui);
            if let Some((mine, theirs)) = self.prepared.as_ref().and_then(|p| p.version_gap) {
                valhsync_ui::widgets::notice(
                    ui,
                    th::BLOOD_LIT,
                    &format!("{} ({mine} / {theirs})", self.t(Key::VersionGap)),
                );
                ui.add_space(8.0);
            }
            self.status_block(ui);
            let set_aside = self
                .prepared
                .as_ref()
                .map(|p| engine::quarantine_dir(&p.install.root))
                .filter(|q| q.is_dir());
            if let Some(folder) = set_aside {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    valhsync_ui::widgets::dot(ui, th::RUNE);
                    ui.label(
                        RichText::new(self.t(Key::SetAsideHint))
                            .small()
                            .color(th::BONE_DIM),
                    );
                });
                if ui
                    .small_button(self.t(Key::SetAside))
                    .on_hover_text(folder.display().to_string())
                    .clicked()
                {
                    open_folder(&folder);
                }
            }

            if let Some(hint) = self
                .prepared
                .as_ref()
                .and_then(|p| game::bepinex_hint(&p.install))
            {
                ui.add_space(10.0);
                th::callout(ui, th::GOLD, |ui| {
                    ui.label(RichText::new(hint).small().color(th::GOLD_LIT));
                });
            }
        });
    }

    /// One row per mod, with what the next sync will do to it. Long packs
    /// are cut off: a launcher with forty mods must not become a wall.
    fn mods_card(&mut self, ui: &mut egui::Ui) {
        th::card(ui, |ui| {
            ui.set_width(ui.available_width());
            valhsync_ui::widgets::section(
                ui,
                &format!("{} ({})", self.t(Key::ServerMods), self.mods.len()),
            );
            for row in self.mods.iter().take(MODS_SHOWN) {
                mod_row(ui, row, self.lang);
            }
            if self.mods.len() > MODS_SHOWN {
                ui.add_space(4.0);
                if ui
                    .button(format!("{} ({})", self.t(Key::ShowAll), self.mods.len()))
                    .clicked()
                {
                    self.mods_open = true;
                }
            }
        });
    }

    /// Write down what the sync that just succeeded changed.
    ///
    /// Taken from the plan that was applied rather than from a fresh one: by
    /// the next check the same comparison yields nothing, because everything
    /// in it is now on disk.
    fn record_news(&mut self) {
        let Some(p) = &self.prepared else {
            return;
        };
        let entry = crate::news::Entry {
            server_id: p.server.id.clone(),
            server_name: p.server.name.clone(),
            pack_id: p.manifest.pack_id.clone(),
            at: valhsync_core::clock::now_rfc3339(),
            notes: p.manifest.notes.clone(),
            changes: valhsync_core::changes::mods_touched(&p.plan),
        };
        if self.news.record(entry)
            && let Err(e) = self.news.save(&self.paths)
        {
            // Losing the history is not worth interrupting anyone over, but
            // it should not be lost in silence either.
            self.notify(e.to_string(), th::GOLD);
        }
    }

    /// What the next press will change, in mods rather than in files, with
    /// whatever the admin wanted to say about it.
    ///
    /// Before the sync, not after: this is the moment somebody decides. The
    /// confirmation dialog only appears on a first sync with a server, and a
    /// note saying "empty your chests before this one" is worth exactly
    /// nothing once the files are on disk.
    fn whats_new_block(&mut self, ui: &mut egui::Ui) {
        let (changes, notes) = self.prepared.as_ref().map_or_else(
            || (Vec::new(), None),
            |p| {
                (
                    valhsync_core::changes::mods_touched(&p.plan),
                    p.manifest.notes.clone(),
                )
            },
        );
        let has_history = self
            .selected
            .as_deref()
            .is_some_and(|id| self.news.for_server(id).next().is_some());
        if changes.is_empty() && notes.is_none() {
            // Shown even with nothing behind it. A feature that only appears
            // once something happens cannot be told apart from one that does
            // not work, and "nothing new yet" is an answer -- silence is not.
            let label = if has_history {
                self.t(Key::WhatsNew).to_string()
            } else {
                format!("{} — {}", self.t(Key::WhatsNew), self.t(Key::NewsNone))
            };
            if ui.small_button(label).clicked() {
                self.news_open = true;
            }
            ui.add_space(10.0);
            return;
        }

        let title = self.t(Key::WhatsNew);
        let history = self.t(Key::NewsHistory);
        th::callout(ui, th::GOLD, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong().color(th::GOLD_LIT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if has_history && ui.small_button(history).clicked() {
                        self.news_open = true;
                    }
                });
            });
            if let Some(notes) = &notes {
                ui.add_space(4.0);
                ui.label(RichText::new(notes).color(th::BONE));
            }
            if !changes.is_empty() {
                ui.add_space(4.0);
                self.change_lines(ui, &changes);
            }
        });
        ui.add_space(10.0);
    }

    /// One line per kind: added, updated, removed. Named mods, not counts --
    /// "3 mods updated" tells nobody whether the one they care about moved.
    fn change_lines(&self, ui: &mut egui::Ui, changes: &[valhsync_core::ModChange]) {
        use valhsync_core::ChangeKind;
        for (kind, key, colour) in [
            (ChangeKind::Added, Key::NewsAdded, th::MOSS),
            (ChangeKind::Updated, Key::NewsUpdated, th::GOLD),
            (ChangeKind::Removed, Key::NewsRemoved, th::RUNE),
        ] {
            let names: Vec<&str> = changes
                .iter()
                .filter(|c| c.kind == kind)
                .map(|c| c.name.as_str())
                .collect();
            if names.is_empty() {
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                valhsync_ui::widgets::dot(ui, colour);
                ui.label(
                    RichText::new(format!("{} · {}", self.t(key), names.join(", ")))
                        .text_style(th::label_style())
                        .color(th::BONE),
                );
            });
        }
    }

    /// Everything this server has changed on this machine, newest first.
    fn news_dialog(&mut self, ctx: &egui::Context) {
        if !self.news_open {
            return;
        }
        let mut open = true;
        let entries: Vec<crate::news::Entry> = self
            .selected
            .as_deref()
            .map(|id| self.news.for_server(id).cloned().collect())
            .unwrap_or_default();
        egui::Window::new(self.t(Key::WhatsNew))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(520.0)
            .show(ctx, |ui| {
                if entries.is_empty() {
                    ui.label(RichText::new(self.t(Key::NewsNone)).color(th::BONE_DIM));
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(valhsync_ui::widgets::dialog_room(ui, 230.0))
                    .show(ui, |ui| {
                        for (i, entry) in entries.iter().enumerate() {
                            if i > 0 {
                                ui.add_space(10.0);
                                th::hairline(ui);
                                ui.add_space(10.0);
                            }
                            ui.label(
                                RichText::new(entry.at.get(..10).unwrap_or(&entry.at))
                                    .small()
                                    .color(th::BONE_DIM),
                            );
                            if let Some(notes) = &entry.notes {
                                ui.add_space(2.0);
                                ui.label(RichText::new(notes).color(th::BONE));
                            }
                            ui.add_space(2.0);
                            self.change_lines(ui, &entry.changes);
                        }
                    });
            });
        self.news_open = open;
    }

    /// The whole pack, when the player asks for it.
    fn mods_dialog(&mut self, ctx: &egui::Context) {
        if !self.mods_open {
            return;
        }
        let mut open = true;
        egui::Window::new(self.t(Key::ServerMods))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(valhsync_ui::widgets::dialog_room(ui, 230.0))
                    .show(ui, |ui| {
                        for row in &self.mods {
                            mod_row(ui, row, self.lang);
                        }
                    });
            });
        self.mods_open = open;
    }

    #[allow(clippy::too_many_lines)] // one screen region, read top to bottom
    fn status_block(&mut self, ui: &mut egui::Ui) {
        if let Some(p) = &self.progress {
            let label = match p.phase {
                Phase::Contacting => self.t(Key::Contacting).to_string(),
                Phase::Downloading { index, count } if count > 0 => {
                    format!("{} {index}/{count}: {}", self.t(Key::Downloading), p.detail)
                }
                Phase::Downloading { .. } => format!("{}: {}", self.t(Key::Downloading), p.detail),
                Phase::Applying => self.t(Key::Applying).to_string(),
            };
            ui.label(RichText::new(label).color(th::BONE_DIM));
            ui.add(
                egui::ProgressBar::new(p.fraction())
                    .animate(true)
                    .show_percentage(),
            );
            if p.total > 0 {
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
                let speed = human_bytes(p.speed as u64);
                let mut line = format!(
                    "{} / {} · {speed}/s",
                    human_bytes(p.done),
                    human_bytes(p.total)
                );
                if let Some(left) = p.seconds_left() {
                    let shown = if left >= 60 {
                        format!("{}m {}s", left / 60, left % 60)
                    } else {
                        format!("{left}s")
                    };
                    line = format!("{line} · {shown} {}", self.t(Key::Remaining));
                }
                ui.label(RichText::new(line).small().color(th::RUNE));
            }
            return;
        }
        match &self.status {
            // Nothing to say: no server picked, and for Ready, the line
            // beside the server's name has already said it -- only what
            // needs the full width stays under the card.
            Status::NoServer | Status::Ready => {}
            Status::Checking => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().color(th::GOLD));
                    ui.label(RichText::new(self.t(Key::Checking)).color(th::BONE_DIM));
                });
            }
            Status::Error(failure) => {
                let message = failure.message.clone();
                // A server that is simply down is the commonest thing that
                // goes wrong here, and it is not the player's problem to
                // solve: two words and the fact that it retries by itself.
                // The URL and the transport error stay one hover away, and
                // the command line still prints them in full.
                let offline = failure.offline;
                let (title, hint) = (self.t(Key::ServerDown), self.t(Key::ServerDownHint));
                th::callout(ui, th::BLOOD, |ui| {
                    if offline {
                        ui.label(RichText::new(title).strong().color(th::BLOOD_LIT))
                            .on_hover_text(&message);
                        ui.label(RichText::new(hint).small().color(th::BONE_DIM))
                            .on_hover_text(&message);
                    } else {
                        ui.label(RichText::new(message).color(th::BLOOD_LIT));
                    }
                });
            }
        }
    }

    /// What the next press would do, in one line. `None` while a job is
    /// running or the server has not answered: those have their own space.
    fn ready_summary(&self) -> Option<(Color32, String)> {
        if self.progress.is_some() || !matches!(self.status, Status::Ready) {
            return None;
        }
        let p = self.prepared.as_ref()?;
        let c = p.plan.counts();
        if p.is_up_to_date() {
            return Some((
                th::MOSS,
                format!(
                    "{} · {} {}",
                    self.t(Key::UpToDate),
                    c.keep + c.seed_kept,
                    self.t(Key::Installed)
                ),
            ));
        }
        let mut parts = vec![
            if p.needs_confirmation {
                self.t(Key::FirstSync)
            } else {
                self.t(Key::Pending)
            }
            .to_string(),
        ];
        if c.add > 0 {
            parts.push(format!("{} {}", c.add, self.t(Key::PlanInstall)));
        }
        if c.replace > 0 {
            parts.push(format!("{} {}", c.replace, self.t(Key::PlanUpdate)));
        }
        if c.remove > 0 {
            parts.push(format!("{} {}", c.remove, self.t(Key::PlanRemove)));
        }
        if c.quarantine > 0 {
            parts.push(format!("{} {}", c.quarantine, self.t(Key::PlanQuarantine)));
        }
        parts.push(format!(
            "{} {}",
            human_bytes(p.plan.download_bytes),
            self.t(Key::PlanDownload)
        ));
        Some((th::GOLD, parts.join(" · ")))
    }

    #[allow(clippy::too_many_lines)] // one screen region, read top to bottom
    fn add_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.add_dialog.take() else {
            return;
        };
        let mut keep_open = true;
        let mut submitted = false;
        let mut trusted = false;

        egui::Window::new(self.t(Key::AddServer))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(480.0)
            .show(ctx, |ui| {
                valhsync_ui::widgets::dialog_body(ui, |ui| {
                    ui.label(RichText::new(self.t(Key::InvitePrompt)).color(th::BONE_DIM));
                    ui.add_enabled(
                        dialog.pending.is_none() && !dialog.asking,
                        egui::TextEdit::multiline(&mut dialog.input)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .hint_text(self.t(Key::InviteHint)),
                    );

                    if dialog.asking {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().color(th::GOLD));
                            ui.label(RichText::new(self.t(Key::Checking2)).color(th::BONE_DIM));
                        });
                    }
                    if let Some(found) = &dialog.pending {
                        ui.add_space(8.0);
                        valhsync_ui::widgets::section(ui, self.t(Key::ConfirmKeyTitle));
                        ui.label(RichText::new(&found.invite.name).strong().color(th::BONE));
                        ui.label(
                            RichText::new(&found.invite.url)
                                .monospace()
                                .small()
                                .color(th::BONE_DIM),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(&found.fingerprint)
                                .font(th::display_font(20.0))
                                .color(th::GOLD_LIT),
                        );
                        ui.add_space(6.0);
                        valhsync_ui::widgets::notice(ui, th::GOLD, self.t(Key::ConfirmKeyBody));
                    }
                    if let Some(e) = &dialog.error {
                        valhsync_ui::widgets::notice(ui, th::BLOOD_LIT, e);
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = if dialog.pending.is_some() {
                        self.t(Key::Trust)
                    } else {
                        self.t(Key::Apply)
                    };
                    if ui
                        .add_enabled(
                            !dialog.asking,
                            egui::Button::new(RichText::new(label).strong().color(th::NIGHT))
                                .fill(th::GOLD),
                        )
                        .clicked()
                    {
                        if dialog.pending.is_some() {
                            trusted = true;
                        } else {
                            submitted = true;
                        }
                    }
                    if ui.button(self.t(Key::Cancel)).clicked() {
                        keep_open = false;
                    }
                });
            });

        if submitted {
            let text = dialog.input.trim().to_string();
            if text.contains(valhsync_core::invite::PREFIX) {
                match Invite::parse(&text) {
                    Ok(invite) => {
                        if let Err(e) = self.adopt(&invite) {
                            dialog.error = Some(e);
                        } else {
                            keep_open = false;
                        }
                    }
                    Err(e) => dialog.error = Some(e.to_string()),
                }
            } else if valhsync_core::invite::looks_like_join_code(&text) {
                dialog.error = Some(self.t(Key::JoinCodeNotAnAddress).to_string());
            } else {
                // An address: ask the server who it is, then have the player
                // confirm the fingerprint before pinning anything.
                dialog.error = None;
                dialog.asking = true;
                let address = text;
                self.start_job(Job::Discover, move |rep| {
                    let result = Context::discover()
                        .and_then(|ctx| engine::discover(&ctx, &address))
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    rep.send(Msg::Discovered(result));
                });
            }
        }
        if trusted && let Some(found) = dialog.pending.clone() {
            match self.adopt(&found.invite) {
                Ok(()) => keep_open = false,
                Err(e) => {
                    dialog.pending = None;
                    dialog.error = Some(e);
                }
            }
        }
        if keep_open {
            self.add_dialog = Some(dialog);
        }
    }

    /// Pin a server and select it.
    fn adopt(&mut self, invite: &Invite) -> Result<(), String> {
        self.book.join(invite, false).map_err(|e| e.to_string())?;
        self.book.save(&self.paths).map_err(|e| e.to_string())?;
        self.selected = self
            .book
            .resolve(Some(&invite.name))
            .ok()
            .map(|s| s.id.clone());
        if let Some(id) = self.selected.clone() {
            self.book.set_default(&id);
            let _ = self.book.save(&self.paths);
        }
        self.notify(format!("{}: {}", self.t(Key::Added), invite.name), th::MOSS);
        self.check();
        Ok(())
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_open {
            return;
        }
        let Some(p) = self.prepared.clone() else {
            self.confirm_open = false;
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new(self.t(Key::ConfirmTitle))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(self.t(Key::ConfirmBody)).color(th::BONE_DIM));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{} {}: {}",
                        self.t(Key::TrustLine),
                        self.t(Key::KeyFingerprint),
                        p.server.fingerprint()
                    ))
                    .small()
                    .color(th::RUNE),
                );
                // The admin's own words, above the file list rather than
                // under it: a warning about chests or config files is read
                // before the plan, or it is not read.
                if let Some(notes) = p.manifest.notes.clone() {
                    ui.add_space(8.0);
                    th::callout(ui, th::GOLD, |ui| {
                        ui.label(
                            RichText::new(self.t(Key::NewsFromAdmin))
                                .small()
                                .color(th::GOLD_LIT),
                        );
                        ui.label(RichText::new(notes).color(th::BONE));
                    });
                }
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(valhsync_ui::widgets::dialog_room(ui, 300.0))
                    .show(ui, |ui| {
                        for (key, action) in [
                            (Key::PlanInstall, Action::Add),
                            (Key::PlanUpdate, Action::Replace),
                            (Key::PlanRemove, Action::Remove),
                            (Key::PlanQuarantine, Action::Quarantine),
                        ] {
                            let items: Vec<_> = p.plan.with_action(action).collect();
                            if items.is_empty() {
                                continue;
                            }
                            ui.label(
                                RichText::new(format!("{} {}", items.len(), self.t(key)))
                                    .strong()
                                    .color(th::GOLD),
                            );
                            for i in items {
                                ui.label(
                                    RichText::new(format!(
                                        "  {}  ({})",
                                        i.path,
                                        human_bytes(i.size)
                                    ))
                                    .monospace()
                                    .small()
                                    .color(th::BONE_DIM),
                                );
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(self.t(Key::Apply)).strong().color(th::NIGHT),
                            )
                            .fill(th::GOLD),
                        )
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button(self.t(Key::Cancel)).clicked() {
                        cancel = true;
                    }
                });
            });
        if apply {
            self.start_sync(true);
        }
        if cancel {
            self.confirm_open = false;
        }
    }

    #[allow(clippy::too_many_lines)] // one dialog, read top to bottom
    fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut close = false;
        let mut apply_root = false;
        let mut forget = false;
        let mut reset = false;
        let server = self.selected_server();
        let install = self.prepared.as_ref().map(|p| p.install.clone());

        egui::Window::new(self.t(Key::Settings))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(540.0)
            .show(ctx, |ui| {
                valhsync_ui::widgets::dialog_body(ui, |ui| {
                    // --- where Valheim is -------------------------------------
                    valhsync_ui::widgets::section(ui, self.t(Key::GameFolder));
                    match &install {
                        Some(i) => {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("{} :", self.t(Key::GameFolderInUse)))
                                        .small()
                                        .color(th::BONE_DIM),
                                );
                                ui.label(
                                    RichText::new(i.root.display().to_string())
                                        .monospace()
                                        .small()
                                        .color(th::BONE),
                                );
                            });
                            if self.settings.game_root.is_none() {
                                ui.label(
                                    RichText::new(self.t(Key::GameFolderAuto))
                                        .small()
                                        .color(th::RUNE),
                                );
                            }
                        }
                        None => {
                            valhsync_ui::widgets::notice(
                                ui,
                                th::BLOOD_LIT,
                                &crate::SyncError::GameNotFound.to_string(),
                            );
                        }
                    }
                    ui.add_space(6.0);
                    valhsync_ui::widgets::hint(ui, self.t(Key::GameFolderHint));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.game_root_input)
                                .desired_width(340.0)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("…/steamapps/common/Valheim"),
                        );
                        if ui.button(self.t(Key::Apply)).clicked() {
                            apply_root = true;
                        }
                    });

                    // --- the selected server ----------------------------------
                    if let Some(s) = &server {
                        ui.add_space(14.0);
                        valhsync_ui::widgets::section(ui, self.t(Key::ThisServer));
                        ui.label(RichText::new(&s.name).strong().color(th::BONE));
                        ui.label(
                            RichText::new(&s.url)
                                .monospace()
                                .small()
                                .color(th::BONE_DIM),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} : {}",
                                self.t(Key::KeyFingerprint),
                                s.fingerprint()
                            ))
                            .small()
                            .color(th::RUNE),
                        );
                        ui.add_space(6.0);
                        if ui.button(self.t(Key::Forget)).clicked() {
                            forget = true;
                        }
                    }

                    // --- ValhSync itself ---------------------------------------
                    ui.add_space(14.0);
                    valhsync_ui::widgets::section(ui, self.t(Key::AboutValhSync));
                    ui.label(
                        RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(th::BONE_DIM),
                    );
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(self.paths.config_dir.display().to_string())
                                .monospace()
                                .small()
                                .color(th::BONE_DIM),
                        );
                        if ui.small_button(self.t(Key::OpenFolder)).clicked() {
                            open_folder(&self.paths.config_dir);
                        }
                    });
                    ui.add_space(6.0);
                    valhsync_ui::widgets::hint(ui, self.t(Key::ResetAllHint));
                    if ui.button(self.t(Key::ResetAll)).clicked() {
                        reset = true;
                    }
                });
                ui.add_space(14.0);
                if ui.button(self.t(Key::Close)).clicked() {
                    close = true;
                }
            });

        if reset {
            for path in [self.paths.servers_file(), self.paths.settings_file()] {
                let _ = std::fs::remove_file(path);
            }
            self.book = ServerBook::default();
            self.settings = Settings::default();
            self.game_root_input.clear();
            self.selected = None;
            self.prepared = None;
            self.status = Status::NoServer;
            let msg = self.t(Key::ResetDone).to_string();
            self.notify(msg, th::GOLD_LIT);
            self.settings_open = false;
        }

        if apply_root {
            let input = self.game_root_input.trim().to_string();
            if input.is_empty() {
                self.settings.game_root = None;
                self.save_settings();
                self.check();
            } else {
                let path = PathBuf::from(&input);
                if game::looks_like_valheim(&path) {
                    self.settings.game_root = Some(path);
                    self.save_settings();
                    self.check();
                } else {
                    self.notify(
                        crate::SyncError::NotAGameFolder(path).to_string(),
                        th::BLOOD_LIT,
                    );
                }
            }
        }
        if forget && let Some(id) = self.selected.clone() {
            if let Ok(removed) = self.book.remove(&id) {
                let _ = self.book.save(&self.paths);
                self.notify(
                    format!("{}: {}", self.t(Key::Forget), removed.name),
                    th::GOLD_LIT,
                );
            }
            self.selected = self.book.resolve(None).ok().map(|s| s.id.clone());
            self.prepared = None;
            self.settings_open = false;
            self.check();
        }
        if close {
            self.settings_open = false;
        }
    }
}

/// Show a folder in the system file manager. Best effort.
fn open_folder(path: &Path) {
    let cmd = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}
