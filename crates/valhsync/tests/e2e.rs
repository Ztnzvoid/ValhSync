//! End-to-end: a real `valhsync-server` router on a local port, a fake game
//! folder, and the launcher engine driving a full life cycle.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use valhsync::backup::Backup;
use valhsync::engine::{self, Context, Silent};
use valhsync::error::SyncError;
use valhsync::paths::AppPaths;
use valhsync::servers::ServerBook;
use valhsync::settings::Settings;
use valhsync::vanilla;
use valhsync_core::path::to_os_path;
use valhsync_core::{Action, Invite, Keypair, hash};
use valhsync_server::config::Config;
use valhsync_server::serve;

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
                let server = serve::prepare(&cfg, kp, data_dir, None, false).unwrap();
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

fn join(w: &World, url: &str, kp: &Keypair) -> valhsync::servers::KnownServer {
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
    assert!(!exists(&g, valhsync::TMP_DIR_NAME), "temp dir cleaned");
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
    let newest = &Backup::list(&w.ctx.paths).unwrap()[0];
    assert_eq!(newest.record.stamp, stamp);
    assert!(newest.record.restored_at.is_some(), "flagged as restored");
    // Older backups stay available: a second `rollback` would step back once more.
    let next = Backup::latest_restorable(&w.ctx.paths).unwrap().unwrap();
    assert!(next.record.stamp < stamp);

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
    assert!(!exists(&g, valhsync::TMP_DIR_NAME));

    // --- a different key for the same URL: refused before anything else ---
    let impostor = Keypair::generate();
    let wrong = join(&w, &ts.url(), &impostor);
    let err = engine::prepare(&w.ctx, &wrong, &mut Silent).unwrap_err();
    assert!(matches!(err, SyncError::KeyMismatch { .. }), "{err}");
}

#[test]
fn adding_a_server_by_address_never_panics() {
    // What the window does when someone types an address into "Add a server":
    // ask the server who it is, pin it, then plan the first sync. Every step
    // here has run on someone else's machine and taken the window down with
    // it, so the whole path is walked rather than its pieces.
    let w = world();
    let server = TestServer::start(&w.cfg, &w.kp, &w.data_dir);
    let url = server.url();

    let found = engine::discover(&w.ctx, &url).expect("the address should resolve");
    assert_eq!(found.invite.name, "E2E");
    assert_eq!(found.fingerprint, w.kp.public().fingerprint());
    assert!(found.files > 0);

    // The address typed as `host:port`, the way a player is given it.
    let bare = url.trim_start_matches("http://").to_string();
    assert_eq!(
        engine::discover(&w.ctx, &bare).unwrap().invite.url,
        found.invite.url
    );

    // Pinned, then planned: this is what the window does on Trust.
    let mut book = ServerBook::load(&w.ctx.paths).unwrap();
    book.join(&found.invite, false).unwrap();
    book.save(&w.ctx.paths).unwrap();
    let known = book.resolve(None).unwrap().clone();

    let prepared = engine::prepare(&w.ctx, &known, &mut Silent).expect("planning the first sync");
    assert!(prepared.needs_confirmation, "a server never seen before");
    assert!(!prepared.plan.is_noop());
    // Neither side has a Valheim log in this fixture, so nothing is claimed.
    assert_eq!(prepared.version_gap, None);
}

#[test]
fn an_address_that_is_not_one_is_refused_quietly() {
    let w = world();
    for text in [
        "236486",         // Valheim's own join code
        "",               //
        "   ",            //
        "user:pass@host", //
        "http://ho st",   //
        "not a server",   //
    ] {
        let result = engine::discover(&w.ctx, text);
        assert!(result.is_err(), "{text:?} should not resolve");
    }
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

#[test]
fn older_manifest_is_refused_as_replay() {
    let w = world();
    let ts = TestServer::start(&w.cfg, &w.kp, &w.data_dir);
    let server = join(&w, &ts.url(), &w.kp);
    // Pretend a newer manifest was already applied from this server.
    let mut book = ServerBook::load(&w.ctx.paths).unwrap();
    book.note_pack(&server.id, "b3:newer", "2999-01-01T00:00:00Z");
    book.save(&w.ctx.paths).unwrap();
    let server = book.resolve(None).unwrap().clone();

    let err = engine::prepare(&w.ctx, &server, &mut Silent).unwrap_err();
    assert!(matches!(err, SyncError::OlderManifest { .. }), "{err}");

    let mut ctx = Context::with(w.ctx.paths.clone(), w.ctx.settings.clone()).unwrap();
    ctx.skip_process_check = true;
    ctx.allow_older = true;
    engine::prepare(&ctx, &server, &mut Silent).unwrap();
}

// ── The update channel, over real HTTP ──────────────────────────────────
//
// The one path in the project that ends in executing a downloaded binary, and
// the only one with no end-to-end coverage until now: the server half and the
// launcher half were each unit-tested against their own idea of the contract.
// This runs the launcher's real client against a real listener, so it also
// runs on Linux in CI, where nobody has ever opened a window.

struct OfferServer {
    addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl OfferServer {
    /// Serve one signed offer and one build. `build` is what the bytes route
    /// actually returns, which is not always what the offer promises.
    fn start(doc: Vec<u8>, sig: String, hash: String, build: Vec<u8>) -> Self {
        use axum::routing::get;
        use valhsync_core::update;

        let (addr_tx, addr_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let app = axum::Router::new()
                    .route(update::OFFER_PATH, get(async move || doc))
                    .route(update::SIGNATURE_PATH, get(async move || sig))
                    .route(
                        &format!("{}{{hash}}", update::BUILD_PREFIX),
                        get(
                            async move |axum::extract::Path(asked): axum::extract::Path<String>| {
                                if asked == hash {
                                    Ok(build)
                                } else {
                                    Err(axum::http::StatusCode::NOT_FOUND)
                                }
                            },
                        ),
                    );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                addr_tx.send(listener.local_addr().unwrap()).unwrap();
                let shutdown = async {
                    let _ = stop_rx.await;
                };
                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown)
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

impl Drop for OfferServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A signed offer for `build`, as a publisher would produce it.
fn sign_offer(kp: &Keypair, build: &[u8], version: &str) -> (Vec<u8>, String, String) {
    use valhsync_core::hash;
    let digest = hash::to_hex(&hash::hash_bytes(build));
    let offer = valhsync_core::UpdateOffer {
        format: valhsync_core::update::UPDATE_FORMAT,
        version: version.to_string(),
        target: valhsync::selfupdate::current_target().to_string(),
        exe: "valhsync-test".to_string(),
        size: build.len() as u64,
        blake3: digest.clone(),
        generated_at: valhsync_core::clock::now_rfc3339(),
    };
    let doc = serde_json::to_vec(&offer).unwrap();
    let sig = valhsync_core::sign::encode_signature(&kp.sign(&doc));
    (doc, sig, digest)
}

#[test]
fn an_offered_build_is_verified_then_written() {
    let kp = Keypair::generate();
    let build = b"a newer launcher, honestly".to_vec();
    let (doc, sig, hash) = sign_offer(&kp, &build, "99.0.0");
    let srv = OfferServer::start(doc, sig, hash, build.clone());

    let client = valhsync::http::Client::new().unwrap();
    let offer = client
        .fetch_update_offer(&srv.url(), &kp.public())
        .unwrap()
        .expect("the server is offering one");
    assert!(
        valhsync::selfupdate::wanted(&offer),
        "99.0.0 for this target should be wanted"
    );

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("staged");
    client
        .download_update(&srv.url(), &offer, &dest, &mut |_| {})
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), build, "byte for byte");
}

#[test]
fn a_build_that_does_not_match_its_digest_is_not_written() {
    let kp = Keypair::generate();
    let promised = b"the build that was signed".to_vec();
    let (doc, sig, hash) = sign_offer(&kp, &promised, "99.0.0");
    // Same length, different bytes: the size check passes and only the digest
    // stands between the offer and something else entirely.
    let served = b"the build that was sent!!".to_vec();
    assert_eq!(promised.len(), served.len());
    let srv = OfferServer::start(doc, sig, hash, served);

    let client = valhsync::http::Client::new().unwrap();
    let offer = client
        .fetch_update_offer(&srv.url(), &kp.public())
        .unwrap()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("staged");
    assert!(
        client
            .download_update(&srv.url(), &offer, &dest, &mut |_| {})
            .is_err()
    );
    assert!(!dest.exists(), "a build that failed its digest is removed");
}

#[test]
fn an_offer_from_another_key_is_not_an_offer() {
    let theirs = Keypair::generate();
    let build = b"someone else's launcher".to_vec();
    let (doc, sig, hash) = sign_offer(&theirs, &build, "99.0.0");
    let srv = OfferServer::start(doc, sig, hash, build);

    let ours = Keypair::generate();
    let client = valhsync::http::Client::new().unwrap();
    assert!(
        client
            .fetch_update_offer(&srv.url(), &ours.public())
            .is_err(),
        "a signature from a key we did not pin must not read as an offer"
    );
}

/// An empty router: every path is a 404, which is what a publisher with no
/// launcher beside it, or one built before the update channel existed, looks
/// like from here. Most servers will look like this, so it must not read as a
/// failure -- a player would see an error on a server that is working.
struct SilentServer {
    addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl SilentServer {
    fn start() -> Self {
        let (addr_tx, addr_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                addr_tx.send(listener.local_addr().unwrap()).unwrap();
                let shutdown = async {
                    let _ = stop_rx.await;
                };
                axum::serve(listener, axum::Router::new())
                    .with_graceful_shutdown(shutdown)
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

impl Drop for SilentServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[test]
fn a_server_offering_nothing_is_not_an_error() {
    let srv = SilentServer::start();
    let client = valhsync::http::Client::new().unwrap();
    assert_eq!(
        client
            .fetch_update_offer(&srv.url(), &Keypair::generate().public())
            .expect("404 is an answer, not a failure"),
        None
    );
}
