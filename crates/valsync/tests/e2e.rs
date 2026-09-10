//! End-to-end: a real `valsync-server` router on a local port, a fake game
//! folder, and the launcher engine driving a full life cycle.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use valsync::backup::Backup;
use valsync::engine::{self, Context, Silent};
use valsync::error::SyncError;
use valsync::paths::AppPaths;
use valsync::servers::ServerBook;
use valsync::settings::Settings;
use valsync::vanilla;
use valsync_core::path::to_os_path;
use valsync_core::{Action, Invite, Keypair, hash};
use valsync_server::config::Config;
use valsync_server::serve;

fn write(root: &Path, rel: &str, content: &[u8]) {
    let p = to_os_path(root, rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn read(root: &Path, rel: &str) -> Vec<u8> {
    std::fs::read(to_os_path(root, rel)).unwrap()
}

fn exists(root: &Path, rel: &str) -> bool {
    to_os_path(root, rel).exists()
}

struct TestServer {
    addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    fn start(cfg: &Config, kp: &Keypair, data_dir: &Path) -> Self {
        let (addr_tx, addr_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let cfg = cfg.clone();
        let kp = kp.clone();
        let data_dir = data_dir.to_path_buf();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let server = serve::prepare(&cfg, kp, data_dir, false).unwrap();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                addr_tx.send(listener.local_addr().unwrap()).unwrap();
                serve::serve_until(listener, server, async {
                    let _ = stop_rx.await;
                })
                .await
                .unwrap();
            });
        });
        let addr = addr_rx.recv().unwrap();
        Self {
            addr,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct World {
    _home: tempfile::TempDir,
    _srv: tempfile::TempDir,
    _data: tempfile::TempDir,
    _game: tempfile::TempDir,
    server_root: PathBuf,
    data_dir: PathBuf,
    game_root: PathBuf,
    cfg: Config,
    kp: Keypair,
    ctx: Context,
}

fn world() -> World {
    let home = tempfile::tempdir().unwrap();
    let srv = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let game = tempfile::tempdir().unwrap();
    let server_root = srv.path().to_path_buf();
    let game_root = game.path().to_path_buf();

    write(&server_root, "winhttp.dll", b"doorstop");
    write(&server_root, "doorstop_config.ini", b"ini");
    write(
        &server_root,
        "valheim_server.exe",
        b"server binary, never shipped",
    );
    write(&server_root, "BepInEx/core/BepInEx.dll", b"core");
    write(&server_root, "BepInEx/plugins/Azu/Azu.dll", b"azu v1");
    write(&server_root, "BepInEx/config/Azu.cfg", b"hotkey=F1");
    write(
        &server_root,
        "BepInEx/plugins/DiscordConnector/DC.dll",
        b"server only",
    );
    write(&server_root, "BepInEx/LogOutput.log", b"log");

    let mut cfg = Config::default();
    cfg.server.name = "E2E".into();
    cfg.server.game_address = "127.0.0.1:2456".into();
    cfg.pack.server_root = Some(server_root.clone());
    cfg.pack
        .exclude
        .push("BepInEx/plugins/DiscordConnector/**".into());
    cfg.validate().unwrap();
    let kp = Keypair::generate();

    write(&game_root, "valheim.exe", b"game");
    write(
        &game_root,
        "BepInEx/plugins/EquipmentAndQuickSlots/EAQS.dll",
        b"leftover",
    );

    let paths = AppPaths::at(home.path()).unwrap();
    let settings = Settings {
        game_root: Some(game_root.clone()),
        keep_backups: 3,
        ..Settings::default()
    };
    let mut ctx = Context::with(paths, settings).unwrap();
    ctx.skip_process_check = true;

    World {
        data_dir: data.path().to_path_buf(),
        _home: home,
        _srv: srv,
        _data: data,
        _game: game,
        server_root,
        game_root,
        cfg,
        kp,
        ctx,
    }
}

fn join(w: &World, url: &str, kp: &Keypair) -> valsync::servers::KnownServer {
    let invite = Invite::new(url, &kp.public(), "E2E");
    let mut book = ServerBook::load(&w.ctx.paths).unwrap();
    book.join(&invite, true).unwrap();
    book.save(&w.ctx.paths).unwrap();
    book.resolve(None).unwrap().clone()
}

#[test]
#[allow(clippy::too_many_lines)] // one scenario, read top to bottom
fn full_life_cycle() {
    let w = world();
    let g = w.game_root.clone();

    // --- first sync: everything installed, leftover quarantined ---
    let ts = TestServer::start(&w.cfg, &w.kp, &w.data_dir);
    let server = join(&w, &ts.url(), &w.kp);
    let prepared = engine::prepare(&w.ctx, &server, &mut Silent).unwrap();
    assert!(prepared.needs_confirmation);
    let c = prepared.plan.counts();
    assert_eq!(
        (c.add, c.quarantine, c.keep),
        (5, 1, 0),
        "{:?}",
        prepared.plan
    );
    let applied = engine::apply(&w.ctx, &prepared, &mut Silent).unwrap();
    assert_eq!(applied.counts.add, 5);
    assert_eq!(read(&g, "winhttp.dll"), b"doorstop");
    assert_eq!(read(&g, "BepInEx/plugins/Azu/Azu.dll"), b"azu v1");
    assert!(
        !exists(&g, "BepInEx/plugins/DiscordConnector/DC.dll"),
        "server-only mod not shipped"
    );
    assert!(!exists(&g, "valheim_server.exe"));
    assert!(
        !exists(&g, "BepInEx/plugins/EquipmentAndQuickSlots/EAQS.dll"),
        "leftover moved"
    );
    let q = applied.quarantine_dir.clone().unwrap();
    assert_eq!(
        read(
            &g,
            &format!("{q}/BepInEx/plugins/EquipmentAndQuickSlots/EAQS.dll")
        ),
        b"leftover"
    );
    assert!(!exists(&g, valsync::TMP_DIR_NAME), "temp dir cleaned");
    let first_state = w.ctx.installed().unwrap().unwrap();
    assert_eq!(first_state.files.len(), 5);
    assert_eq!(first_state.server_id, server.id);
    let backups = Backup::list(&w.ctx.paths).unwrap();
    assert_eq!(backups.len(), 1);
    assert!(backups[0].record.completed_at.is_some());

    // --- second sync is a no-op; a player-edited seed config survives ---
    write(&g, "BepInEx/config/Azu.cfg", b"hotkey=F5");
    let prepared = engine::prepare(&w.ctx, &server, &mut Silent).unwrap();
    assert!(!prepared.needs_confirmation);
    assert!(prepared.is_up_to_date(), "{:?}", prepared.plan);
    assert_eq!(prepared.plan.counts().seed_kept, 1);
    engine::apply(&w.ctx, &prepared, &mut Silent).unwrap();
    assert_eq!(read(&g, "BepInEx/config/Azu.cfg"), b"hotkey=F5");
    assert_eq!(
        Backup::list(&w.ctx.paths).unwrap().len(),
        1,
        "no-op makes no backup"
    );

    // --- "play without mods", then the next sync restores BepInEx ---
    assert_eq!(vanilla::set(&g, false).unwrap(), vanilla::ModsState::Off);
    let prepared = engine::prepare(&w.ctx, &server, &mut Silent).unwrap();
    assert_eq!(prepared.plan.counts().add, 1);
    engine::apply(&w.ctx, &prepared, &mut Silent).unwrap();
    assert_eq!(vanilla::state(&g), vanilla::ModsState::On);
    assert!(!exists(&g, "winhttp.dll.off"));

    // --- admin updates a mod and adds one; server restarts on a new port ---
    drop(ts);
    write(&w.server_root, "BepInEx/plugins/Azu/Azu.dll", b"azu v2");
    write(&w.server_root, "BepInEx/plugins/Beta/Beta.dll", b"beta");
    let ts = TestServer::start(&w.cfg, &w.kp, &w.data_dir);
    let server = join(&w, &ts.url(), &w.kp);
    let prepared = engine::prepare(&w.ctx, &server, &mut Silent).unwrap();
    let c = prepared.plan.counts();
    assert_eq!((c.add, c.replace, c.keep), (1, 1, 3), "{:?}", prepared.plan);
    assert_eq!(
        prepared.plan.download_bytes,
        (b"azu v2".len() + b"beta".len()) as u64
    );
    let applied = engine::apply(&w.ctx, &prepared, &mut Silent).unwrap();
    assert_eq!(applied.downloaded_bytes, prepared.plan.download_bytes);
    assert_eq!(read(&g, "BepInEx/plugins/Azu/Azu.dll"), b"azu v2");
    assert_eq!(read(&g, "BepInEx/plugins/Beta/Beta.dll"), b"beta");
    let second_state = w.ctx.installed().unwrap().unwrap();
    assert_ne!(second_state.pack_id, first_state.pack_id);

    // --- rollback restores the exact previous state ---
    let (stamp, report) = engine::rollback(&w.ctx).unwrap();
    assert_eq!(stamp, applied.backup_stamp.unwrap());
    assert_eq!((report.restored, report.deleted), (1, 1));
    assert_eq!(read(&g, "BepInEx/plugins/Azu/Azu.dll"), b"azu v1");
    assert!(!exists(&g, "BepInEx/plugins/Beta/Beta.dll"));
    assert!(!exists(&g, "BepInEx/plugins/Beta"), "empty folder removed");
    let after = w.ctx.installed().unwrap().unwrap();
    assert_eq!(after.pack_id, prepared.previous.as_ref().unwrap().pack_id);
    assert!(matches!(engine::rollback(&w.ctx), Err(SyncError::NoBackup)));

    // --- a corrupted blob on the server: nothing installed, clear error ---
    let v2 = hash::to_hex(&hash::hash_bytes(b"azu v2"));
    std::fs::write(w.data_dir.join("store").join(&v2), b"corrupted in transit").unwrap();
    let prepared = engine::prepare(&w.ctx, &server, &mut Silent).unwrap();
    assert_eq!(prepared.plan.counts().replace, 1);
    let err = engine::apply(&w.ctx, &prepared, &mut Silent).unwrap_err();
    assert!(matches!(err, SyncError::Download { .. }), "{err}");
    assert!(err.to_string().contains("Azu.dll"), "{err}");
    assert_eq!(
        read(&g, "BepInEx/plugins/Azu/Azu.dll"),
        b"azu v1",
        "untouched"
    );
    assert!(!exists(&g, "BepInEx/plugins/Beta/Beta.dll"), "untouched");
    assert!(!exists(&g, valsync::TMP_DIR_NAME));

    // --- a different key for the same URL: refused before anything else ---
    let impostor = Keypair::generate();
    let wrong = join(&w, &ts.url(), &impostor);
    let err = engine::prepare(&w.ctx, &wrong, &mut Silent).unwrap_err();
    assert!(matches!(err, SyncError::KeyMismatch { .. }), "{err}");
}

#[test]
fn unreachable_server_is_reported_not_panicked() {
    let w = world();
    let server = join(&w, "http://127.0.0.1:9", &w.kp);
    let err = engine::prepare(&w.ctx, &server, &mut Silent).unwrap_err();
    assert!(matches!(err, SyncError::Unreachable { .. }), "{err}");
}

#[test]
fn hosted_server_layout_needs_only_client_extras() {
    // No local dedicated server: the admin keeps a copy of the pack in one
    // folder and publishes it. Same launcher flow.
    let w = world();
    let extras = tempfile::tempdir().unwrap();
    write(extras.path(), "winhttp.dll", b"doorstop");
    write(extras.path(), "BepInEx/plugins/Only/Only.dll", b"only");
    let mut cfg = w.cfg.clone();
    cfg.pack.server_root = None;
    cfg.pack.client_extras = Some(extras.path().to_path_buf());
    cfg.validate().unwrap();
    let data = tempfile::tempdir().unwrap();
    let ts = TestServer::start(&cfg, &w.kp, data.path());
    let server = join(&w, &ts.url(), &w.kp);
    let prepared = engine::prepare(&w.ctx, &server, &mut Silent).unwrap();
    assert_eq!(prepared.plan.counts().add, 2);
    assert!(
        prepared
            .plan
            .with_action(Action::Add)
            .any(|i| i.path == "BepInEx/plugins/Only/Only.dll")
    );
    engine::apply(&w.ctx, &prepared, &mut Silent).unwrap();
    assert_eq!(read(&w.game_root, "BepInEx/plugins/Only/Only.dll"), b"only");
}
