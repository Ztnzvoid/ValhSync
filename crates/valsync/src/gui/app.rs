//! Window state and layout. One screen: pick a server, read its status, press
//! PLAY. Everything slow runs on a worker thread and reports back through a
//! channel; the UI thread never blocks.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText};
use valsync_core::limits::human_bytes;
use valsync_core::{Action, Invite};

use super::i18n::{Key, Lang, text};
use crate::engine::{self, Applied, Context, Event, Prepared, Progress};
use crate::paths::AppPaths;
use crate::servers::{KnownServer, ServerBook};
use crate::settings::Settings;
use crate::{game, invite_file, vanilla};
use valsync_ui::frame as chrome;
use valsync_ui::theme as th;

const NOTICE_TTL: Duration = Duration::from_secs(7);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Contacting,
    Downloading { index: usize, count: usize },
    Applying,
}

#[derive(Debug)]
enum Msg {
    Progress {
        frac: f32,
        phase: Phase,
        detail: String,
    },
    Prepared(Result<Box<Prepared>, String>),
    Applied(Result<(Applied, Option<String>), String>),
    RolledBack(Result<String, String>),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Check,
    Sync,
    Rollback,
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
                frac: 0.0,
                phase: Phase::Contacting,
                detail: String::new(),
            },
            Event::Downloading {
                index, count, path, ..
            } => Msg::Progress {
                frac: -1.0,
                phase: Phase::Downloading { index, count },
                detail: path.to_string(),
            },
            Event::Progress { done, total } => Msg::Progress {
                frac: if total == 0 {
                    1.0
                } else {
                    done as f32 / total as f32
                },
                phase: Phase::Downloading { index: 0, count: 0 },
                detail: String::new(),
            },
            Event::Applying { .. } => Msg::Progress {
                frac: 1.0,
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
}

#[derive(Debug)]
struct ProgressView {
    frac: f32,
    phase: Phase,
    detail: String,
}

#[derive(Debug)]
enum Status {
    NoServer,
    Checking,
    Ready,
    Error(String),
}

pub(super) struct App {
    lang: Lang,
    paths: AppPaths,
    settings: Settings,
    book: ServerBook,
    selected: Option<String>,
    status: Status,
    prepared: Option<Prepared>,
    mods_state: Option<vanilla::ModsState>,
    job: Option<(Job, Receiver<Msg>)>,
    progress: Option<ProgressView>,
    notice: Option<(String, Color32, Instant)>,
    add_dialog: Option<AddDialog>,
    confirm_open: bool,
    settings_open: bool,
    game_root_input: String,
    fatal: Option<String>,
    egui_ctx: egui::Context,
}

impl App {
    pub(super) fn new(egui_ctx: &egui::Context) -> Self {
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
            mods_state: None,
            job: None,
            progress: None,
            notice: None,
            add_dialog: None,
            confirm_open: false,
            settings_open: false,
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
            f(reporter.clone());
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
        self.start_job(Job::Check, move |mut rep| {
            let result = Context::discover()
                .and_then(|ctx| engine::prepare(&ctx, &server, &mut rep))
                .map(Box::new)
                .map_err(|e| e.to_string());
            rep.send(Msg::Prepared(result));
        });
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

    fn start_rollback(&mut self) {
        if self.busy() {
            return;
        }
        self.start_job(Job::Rollback, move |rep| {
            let result = Context::discover()
                .and_then(|ctx| engine::rollback(&ctx))
                .map(|(stamp, _)| stamp)
                .map_err(|e| e.to_string());
            rep.send(Msg::RolledBack(result));
        });
    }

    fn on_play(&mut self) {
        let Some(p) = &self.prepared else {
            return;
        };
        if p.needs_confirmation && !p.is_up_to_date() {
            self.confirm_open = true;
        } else {
            self.start_sync(true);
        }
    }

    fn toggle_vanilla(&mut self) {
        let Some(root) = self.prepared.as_ref().map(|p| p.install.root.clone()) else {
            return;
        };
        if game::is_running() {
            self.notify(crate::SyncError::GameRunning.to_string(), th::BLOOD_LIT);
            return;
        }
        let want_on = self.mods_state == Some(vanilla::ModsState::Off);
        match vanilla::set(&root, want_on) {
            Ok(state) => {
                self.mods_state = Some(state);
                let key = if want_on {
                    Key::ModsEnabled
                } else {
                    Key::ModsDisabled
                };
                self.notify(self.t(key).to_string(), th::GOLD_LIT);
                if !want_on
                    && let Some(p) = &self.prepared
                    && let Ok(_) = game::launch(&p.install, &p.manifest.game_address)
                {
                    self.notify(self.t(Key::Launching).to_string(), th::MOSS);
                }
            }
            Err(e) => self.notify(e.to_string(), th::BLOOD_LIT),
        }
    }

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
                    frac,
                    phase,
                    detail,
                } => {
                    let view = self.progress.get_or_insert(ProgressView {
                        frac: 0.0,
                        phase,
                        detail: String::new(),
                    });
                    if frac >= 0.0 {
                        view.frac = frac;
                    }
                    if !matches!(phase, Phase::Downloading { index: 0, .. }) {
                        view.phase = phase;
                    }
                    if !detail.is_empty() {
                        view.detail = detail;
                    }
                }
                Msg::Prepared(Ok(prepared)) => {
                    self.mods_state = Some(vanilla::state(&prepared.install.root));
                    self.prepared = Some(*prepared);
                    self.status = Status::Ready;
                }
                Msg::Prepared(Err(e)) => {
                    self.prepared = None;
                    self.status = Status::Error(e);
                }
                Msg::Applied(Ok((applied, launch_error))) => {
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
                Msg::Applied(Err(e)) | Msg::RolledBack(Err(e)) => self.notify(e, th::BLOOD_LIT),
                Msg::RolledBack(Ok(stamp)) => {
                    self.notify(format!("{} ({stamp})", self.t(Key::RolledBack)), th::MOSS);
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

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.paths) {
            self.notify(e.to_string(), th::BLOOD_LIT);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();
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

        chrome::handle_edge_resize(ctx);
        chrome::paint_window(ctx);
        self.header(ctx);
        self.notice_bar(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(th::NIGHT)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| {
                if let Some(fatal) = self.fatal.clone() {
                    th::callout(ui, th::BLOOD, |ui| {
                        ui.colored_label(th::BLOOD_LIT, fatal);
                    });
                    return;
                }
                self.server_row(ui);
                ui.add_space(12.0);
                if self.book.servers.is_empty() {
                    self.empty_state(ui);
                } else {
                    self.server_card(ui);
                }
            });

        chrome::draw_border(ctx);
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
                    valsync_ui::widgets::header(ui, "V A L S Y N C");
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
            return;
        };
        egui::TopBottomPanel::bottom("notice")
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(20, 10))
                    .stroke(egui::Stroke::new(1.0, th::EDGE_SOFT)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    valsync_ui::widgets::dot(ui, color);
                    ui.label(RichText::new(message).color(th::BONE));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            self.notice = None;
                        }
                    });
                });
            });
    }

    fn server_row(&mut self, ui: &mut egui::Ui) {
        let servers: Vec<(String, String)> = self
            .book
            .servers
            .iter()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        let before = self.selected.clone();
        ui.horizontal(|ui| {
            if !servers.is_empty() {
                let current = servers
                    .iter()
                    .find(|(id, _)| Some(id) == self.selected.as_ref())
                    .map_or("—", |(_, name)| name.as_str())
                    .to_string();
                egui::ComboBox::from_id_salt("server")
                    .selected_text(RichText::new(current).strong())
                    .width(260.0)
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
            if !servers.is_empty()
                && ui
                    .add_enabled(!self.busy(), egui::Button::new(self.t(Key::Refresh)))
                    .clicked()
            {
                self.check();
            }
        });
        if before != self.selected {
            if let Some(id) = &self.selected {
                self.book.set_default(id);
                let _ = self.book.save(&self.paths);
            }
            self.prepared = None;
            self.check();
        }
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        th::card().show(ui, |ui| {
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
        th::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(&server.name)
                    .size(20.0)
                    .strong()
                    .color(th::GOLD_LIT),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(&server.url).small().color(th::BONE_DIM));
                ui.label(
                    RichText::new(format!(
                        "{}: {}",
                        self.t(Key::KeyFingerprint),
                        server.fingerprint()
                    ))
                    .small()
                    .color(th::RUNE),
                );
            });
            ui.add_space(10.0);
            self.status_block(ui);
            ui.add_space(14.0);

            let can_play = matches!(self.status, Status::Ready) && !self.busy();
            ui.vertical_centered(|ui| {
                let (ink, plate) = if can_play {
                    (th::NIGHT, th::GOLD)
                } else {
                    (th::BONE_DIM, th::LEATHER)
                };
                let play = ui.add_enabled(
                    can_play,
                    egui::Button::new(
                        RichText::new(self.t(Key::Play))
                            .font(th::display_font(22.0))
                            .strong()
                            .color(ink),
                    )
                    .fill(plate)
                    .corner_radius(4)
                    .min_size(egui::vec2(280.0, 56.0)),
                );
                if can_play {
                    // A brass plate under torchlight: brighter as the pointer
                    // comes to rest on it.
                    let heat = if play.hovered() { 0.30 } else { 0.16 };
                    th::glow(ui.painter(), play.rect, th::GOLD.gamma_multiply(heat));
                    th::brackets(ui.painter(), play.rect.expand(5.0), th::EDGE);
                }
                if play.clicked() {
                    self.on_play();
                }
            });
            ui.add_space(12.0);

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(can_play, egui::Button::new(self.t(Key::Rollback)))
                    .clicked()
                {
                    self.start_rollback();
                }
                let quarantine = self
                    .prepared
                    .as_ref()
                    .map(|p| engine::quarantine_dir(&p.install.root))
                    .filter(|q| q.is_dir());
                let has_quarantine = quarantine.is_some();
                if ui
                    .add_enabled(has_quarantine, egui::Button::new(self.t(Key::SetAside)))
                    .on_hover_text(self.t(Key::SetAsideHint))
                    .on_disabled_hover_text(self.t(Key::SetAsideNone))
                    .clicked()
                    && let Some(q) = quarantine
                {
                    open_folder(&q);
                }
                let vanilla_label = match self.mods_state {
                    Some(vanilla::ModsState::Off) => self.t(Key::ModsEnabled).trim_end_matches('.'),
                    _ => self.t(Key::PlayVanilla),
                };
                if ui
                    .add_enabled(
                        can_play && self.mods_state.is_some(),
                        egui::Button::new(vanilla_label),
                    )
                    .clicked()
                {
                    self.toggle_vanilla();
                }
            });

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
                egui::ProgressBar::new(p.frac)
                    .animate(true)
                    .show_percentage(),
            );
            return;
        }
        match &self.status {
            Status::NoServer => {}
            Status::Checking => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().color(th::GOLD));
                    ui.label(RichText::new(self.t(Key::Checking)).color(th::BONE_DIM));
                });
            }
            Status::Error(e) => {
                let e = e.clone();
                th::callout(ui, th::BLOOD, |ui| {
                    ui.label(RichText::new(e).color(th::BLOOD_LIT));
                });
            }
            Status::Ready => {
                let Some(p) = &self.prepared else {
                    return;
                };
                let c = p.plan.counts();
                if p.is_up_to_date() {
                    ui.horizontal(|ui| {
                        valsync_ui::widgets::dot(ui, th::MOSS);
                        ui.label(
                            RichText::new(format!(
                                "{} · {} {}",
                                self.t(Key::UpToDate),
                                c.keep + c.seed_kept,
                                self.t(Key::Installed)
                            ))
                            .strong()
                            .color(th::BONE),
                        );
                    });
                } else {
                    let title = if p.needs_confirmation {
                        self.t(Key::FirstSync)
                    } else {
                        self.t(Key::Pending)
                    };
                    ui.horizontal(|ui| {
                        valsync_ui::widgets::dot(ui, th::GOLD);
                        ui.label(RichText::new(title).strong().color(th::BONE));
                    });
                    let mut parts = Vec::new();
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
                    ui.label(RichText::new(parts.join(" · ")).color(th::BONE_DIM));
                }
            }
        }
    }

    fn add_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.add_dialog.take() else {
            return;
        };
        let mut keep_open = true;
        let mut submitted = false;
        egui::Window::new(self.t(Key::AddServer))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(self.t(Key::InvitePrompt)).color(th::BONE_DIM));
                ui.add(
                    egui::TextEdit::multiline(&mut dialog.input)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(self.t(Key::InviteHint)),
                );
                if let Some(e) = &dialog.error {
                    th::callout(ui, th::BLOOD, |ui| {
                        ui.label(RichText::new(e).color(th::BLOOD_LIT));
                    });
                }
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new(self.t(Key::Apply)).color(th::NIGHT))
                                .fill(th::GOLD),
                        )
                        .clicked()
                    {
                        submitted = true;
                    }
                    if ui.button(self.t(Key::Cancel)).clicked() {
                        keep_open = false;
                    }
                });
            });

        if submitted {
            let line = dialog
                .input
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with("valsync1:"))
                .unwrap_or_else(|| dialog.input.trim())
                .to_string();
            let result = Invite::parse(&line)
                .map_err(|e| e.to_string())
                .and_then(|invite| {
                    self.book
                        .join(&invite, false)
                        .map(|_| invite)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(invite) => {
                    let _ = self.book.save(&self.paths);
                    self.selected = self
                        .book
                        .resolve(Some(&invite.name))
                        .ok()
                        .map(|s| s.id.clone());
                    if let Some(id) = &self.selected {
                        self.book.set_default(id);
                        let _ = self.book.save(&self.paths);
                    }
                    self.notify(format!("{}: {}", self.t(Key::Added), invite.name), th::MOSS);
                    self.check();
                    keep_open = false;
                }
                Err(e) => dialog.error = Some(e),
            }
        }
        if keep_open {
            self.add_dialog = Some(dialog);
        }
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
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
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
                // --- where Valheim is -------------------------------------
                valsync_ui::widgets::section(ui, self.t(Key::GameFolder));
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
                        valsync_ui::widgets::notice(
                            ui,
                            th::BLOOD_LIT,
                            &crate::SyncError::GameNotFound.to_string(),
                        );
                    }
                }
                ui.add_space(6.0);
                valsync_ui::widgets::hint(ui, self.t(Key::GameFolderHint));
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
                    valsync_ui::widgets::section(ui, self.t(Key::ThisServer));
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

                // --- ValSync itself ---------------------------------------
                ui.add_space(14.0);
                valsync_ui::widgets::section(ui, self.t(Key::ValsyncItself));
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
                valsync_ui::widgets::hint(ui, self.t(Key::ResetAllHint));
                if ui.button(self.t(Key::ResetAll)).clicked() {
                    reset = true;
                }

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
