//! HTTP server and folder watcher.
//!
//! Nine read-only routes, no listing, no auth: the content is not secret and
//! its integrity is guaranteed by the signature. `/files/{hash}` only ever
//! opens `store/<hash>` for a hash the current or previous manifest names, and
//! the update routes serve one signed document plus the single build whose
//! digest that document names, so neither can be talked into another path.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use notify::{RecursiveMode, Watcher};
use tokio_util::io::ReaderStream;
use valhsync_core::{Invite, Keypair};

use crate::config::Config;
use crate::pack::{self, Published};
use crate::store::Store;
use crate::update::{self, Offered};
use valhsync_core::update as update_api;

/// How long the pack must stay quiet before it is rebuilt.
const SETTLE: Duration = Duration::from_secs(2);

struct AppState {
    current: RwLock<Arc<Published>>,
    store: Store,
    invite: String,
    server_name: String,
    /// Base64url public key, so a player who only has an address can pin it.
    pubkey: String,
    /// True when this process runs beside the dedicated server, and can
    /// therefore say whether the game is up.
    colocated: bool,
    /// The launcher build found beside this publisher, signed once at startup.
    update: Option<Offered>,
}

impl AppState {
    fn current(&self) -> Arc<Published> {
        self.current.read().map_or_else(
            |poisoned| Arc::clone(&poisoned.into_inner()),
            |g| Arc::clone(&g),
        )
    }
}

/// A ready-to-serve router plus the folder watcher that keeps it fresh.
/// Dropping it stops the watcher.
pub struct Server {
    pub app: Router,
    pub published: Arc<Published>,
    watcher: Option<notify::RecommendedWatcher>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("pack_id", &self.published.manifest.pack_id)
            .field("watching", &self.watcher.is_some())
            .finish_non_exhaustive()
    }
}

/// Build the pack and the router. Must run inside a Tokio runtime: the
/// watcher rebuilds on a Tokio task. `watch = false` disables it (tests).
pub fn prepare(
    cfg: &Config,
    keypair: Keypair,
    data_dir: PathBuf,
    config_path: Option<PathBuf>,
    watch: bool,
) -> Result<Server> {
    let outcome = pack::build(cfg, &keypair, &data_dir)?;
    pack::print_summary(&outcome);

    let invite = match cfg.public_url() {
        Some(url) => Invite::new(url, &keypair.public(), cfg.server.name.trim()).encode()?,
        // Served anyway: a player who joins by address gets the key from
        // `/key`, and the admin sees the warning `print_warnings` prints.
        None => String::new(),
    };
    // Signed here, while the keypair is still ours: the watcher takes it next.
    let update = update::find(&keypair);
    if let Some(up) = &update {
        tracing::info!(
            "offering ValhSync {} to launchers ({} bytes)",
            env!("CARGO_PKG_VERSION"),
            up.size
        );
    }

    let state = Arc::new(AppState {
        current: RwLock::new(Arc::clone(&outcome.published)),
        store: Store::open(&data_dir)?,
        invite,
        server_name: cfg.server.name.trim().to_string(),
        pubkey: keypair.public().to_b64(),
        // Only a publisher sitting next to the game server can see it. One
        // publishing a copy of the pack from elsewhere must not guess.
        colocated: cfg.is_colocated(),
        update,
    });

    let watcher = if watch {
        Some(spawn_watcher(
            cfg,
            config_path,
            keypair,
            data_dir,
            Arc::clone(&state),
        )?)
    } else {
        None
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/manifest.json", get(manifest))
        .route("/manifest.sig", get(signature))
        .route("/key", get(public_key))
        .route("/files/{hash}", get(file))
        .route(update_api::OFFER_PATH, get(update_offer))
        .route(update_api::SIGNATURE_PATH, get(update_signature))
        .route(
            &format!("{}{{hash}}", update_api::BUILD_PREFIX),
            get(update_build),
        )
        .route("/health", get(health))
        .with_state(state);

    Ok(Server {
        app,
        published: outcome.published,
        watcher,
    })
}

/// Serve on the configured bind address until Ctrl+C.
pub async fn run(
    cfg: Config,
    keypair: Keypair,
    data_dir: PathBuf,
    config_path: PathBuf,
) -> Result<()> {
    let server = prepare(&cfg, keypair, data_dir, Some(config_path), true)?;
    let addr = cfg.bind_addr()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot listen on {addr}; is another valhsync-server running?"))?;
    tracing::info!(
        "listening on http://{addr}  (public URL: {})",
        cfg.public_url().as_deref().unwrap_or("not set")
    );
    tracing::info!("press Ctrl+C to stop");
    let watch_game = cfg.is_colocated() && cfg.game_server.stop_with_game;
    serve_until(listener, server, shutdown_signal(watch_game)).await
}

/// Serve `server` on `listener` until `shutdown` resolves. Used by tests and
/// by [`run`].
pub async fn serve_until(
    listener: tokio::net::TcpListener,
    server: Server,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    axum::serve(listener, server.app.clone())
        .with_graceful_shutdown(shutdown)
        .await
        .context("HTTP server failed")?;
    drop(server);
    Ok(())
}

/// How often the publisher looks for the dedicated server.
const GAME_POLL: Duration = Duration::from_secs(10);

/// Wait for Ctrl+C, or for the dedicated server to go away.
///
/// The second only applies when ValhSync runs beside the game and the admin
/// left `stop_with_game` on. It waits until it has actually seen the server
/// running before binding its own life to it, so the two can be started in
/// either order.
pub async fn stop_with_game(watch_game: bool) {
    if !watch_game {
        std::future::pending::<()>().await;
    }
    let mut seen_running = false;
    loop {
        tokio::time::sleep(GAME_POLL).await;
        let running = tokio::task::spawn_blocking(crate::gameserver::is_running)
            .await
            .unwrap_or(false);
        if running {
            seen_running = true;
        } else if seen_running {
            tracing::info!("the dedicated server stopped; stopping with it");
            return;
        }
    }
}

async fn shutdown_signal(watch_game: bool) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
    };
    if !watch_game {
        ctrl_c.await;
        return;
    }
    tokio::select! {
        () = ctrl_c => {}
        () = stop_with_game(watch_game) => {}
    }
}

/// Watch every source folder; rebuild after `SETTLE` of quiet. Events under
/// the data directory are ignored so our own writes never trigger a rebuild.
fn spawn_watcher(
    cfg: &Config,
    config_path: Option<PathBuf>,
    keypair: Keypair,
    data_dir: PathBuf,
    state: Arc<AppState>,
) -> Result<notify::RecommendedWatcher> {
    let last_event: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let data_abs = std::fs::canonicalize(&data_dir).unwrap_or_else(|_| data_dir.clone());

    let marker = Arc::clone(&last_event);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let ours = event.paths.iter().all(|p| {
                std::fs::canonicalize(p)
                    .unwrap_or_else(|_| p.clone())
                    .starts_with(&data_abs)
            });
            if !ours && let Ok(mut guard) = marker.lock() {
                *guard = Some(Instant::now());
            }
        }
    })
    .context("cannot create the folder watcher")?;

    for path in cfg.watch_paths() {
        watcher
            .watch(&path, RecursiveMode::Recursive)
            .with_context(|| format!("cannot watch {}", path.display()))?;
        tracing::info!("watching {}", path.display());
    }
    // The configuration too, not only the mods. What goes in the manifest
    // comes from both: a note for players, the server's name, the address,
    // what is excluded. Watching the pack alone meant an admin could save a
    // change, see it on disk, and have the running server go on publishing
    // the manifest it built at startup -- with nothing anywhere saying so.
    //
    // The folder rather than the file: saving writes a temporary file and
    // renames it over the old one, and a watch on the old inode sees nothing.
    if let Some(dir) = config_path.as_ref().and_then(|p| p.parent())
        && !dir.as_os_str().is_empty()
    {
        match watcher.watch(dir, RecursiveMode::NonRecursive) {
            Ok(()) => tracing::info!("watching {}", dir.display()),
            Err(e) => tracing::warn!("cannot watch {}: {e}", dir.display()),
        }
    }

    let mut cfg = cfg.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        loop {
            ticker.tick().await;
            let due = last_event
                .lock()
                .ok()
                .and_then(|mut g| match *g {
                    Some(t) if t.elapsed() >= SETTLE => {
                        *g = None;
                        Some(())
                    }
                    _ => None,
                })
                .is_some();
            if !due {
                continue;
            }
            tracing::info!("something changed, rebuilding");
            // Re-read the configuration rather than reusing the one captured
            // at startup: it is half of what the manifest says. Anything the
            // running server cannot change -- the port it is bound to, the
            // key it signs with -- is simply not read from here.
            if let Some(path) = &config_path {
                match Config::load(path) {
                    Ok(fresh) => match fresh.validate() {
                        Ok(()) => cfg = fresh,
                        Err(e) => tracing::warn!(
                            "the configuration on disk is not usable, keeping the last good one: {e:#}"
                        ),
                    },
                    Err(e) => tracing::warn!("cannot re-read {}: {e:#}", path.display()),
                }
            }
            let cfg = cfg.clone();
            let keypair = keypair.clone();
            let data_dir = data_dir.clone();
            let built =
                tokio::task::spawn_blocking(move || pack::build(&cfg, &keypair, &data_dir)).await;
            match built {
                Ok(Ok(outcome)) => {
                    if outcome.unchanged {
                        tracing::info!("no change in the pack");
                    } else {
                        pack::print_summary(&outcome);
                        if let Ok(mut guard) = state.current.write() {
                            *guard = outcome.published;
                        }
                        tracing::info!("new manifest published");
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!("rebuild failed, still serving the previous pack: {e:#}");
                }
                Err(e) => tracing::error!("rebuild task panicked: {e}"),
            }
        }
    });

    Ok(watcher)
}

async fn root(State(st): State<Arc<AppState>>) -> Response {
    let cur = st.current();
    let body = format!(
        "ValhSync server: {}\n\n\
         {} files, pack {}\n\n\
         Invite code (paste it in the ValhSync launcher, \"Add a server\"):\n\n{}\n",
        st.server_name,
        cur.manifest.files.len(),
        cur.manifest
            .pack_id
            .get(..15)
            .unwrap_or(&cur.manifest.pack_id),
        st.invite
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

async fn manifest(State(st): State<Arc<AppState>>) -> Response {
    let cur = st.current();
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        cur.manifest_bytes.clone(),
    )
        .into_response()
}

async fn signature(State(st): State<Arc<AppState>>) -> Response {
    let cur = st.current();
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        format!("{}\n", cur.signature),
    )
        .into_response()
}

/// The server's public key. Nothing secret: it is what a launcher pins, and
/// it is already inside every invite code.
async fn public_key(State(st): State<Arc<AppState>>) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        format!(
            "{}
",
            st.pubkey
        ),
    )
        .into_response()
}

async fn file(State(st): State<Arc<AppState>>, Path(hash): Path<String>) -> Response {
    let Some(path) = st.store.path_for(&hash) else {
        return (StatusCode::BAD_REQUEST, "not a blake3 digest").into_response();
    };
    // Only what a manifest names is served, even if the blob happens to exist.
    if !st.current().hashes.contains(&hash) {
        return (StatusCode::NOT_FOUND, "unknown file").into_response();
    }
    let Ok(f) = tokio::fs::File::open(&path).await else {
        tracing::warn!("blob {hash} missing from the store; rescan needed");
        return (StatusCode::NOT_FOUND, "file not in store").into_response();
    };
    let len = f.metadata().await.map(|m| m.len()).ok();
    let mut resp = Body::from_stream(ReaderStream::new(f)).into_response();
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    if let Some(len) = len
        && let Ok(v) = HeaderValue::from_str(&len.to_string())
    {
        headers.insert(header::CONTENT_LENGTH, v);
    }
    resp
}

/// The signed offer. A publisher with no launcher beside it has nothing to
/// say here, and says so rather than serving an empty document.
async fn update_offer(State(st): State<Arc<AppState>>) -> Response {
    let Some(up) = st.update.as_ref() else {
        return (StatusCode::NOT_FOUND, "no update on offer").into_response();
    };
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        up.doc.clone(),
    )
        .into_response()
}

async fn update_signature(State(st): State<Arc<AppState>>) -> Response {
    let Some(up) = st.update.as_ref() else {
        return (StatusCode::NOT_FOUND, "no update on offer").into_response();
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        format!("{}\n", up.sig),
    )
        .into_response()
}

/// The offered build's bytes. The hash in the URL is not a lookup key but a
/// check: the only path ever opened is the one the signed offer names.
async fn update_build(State(st): State<Arc<AppState>>, Path(hash): Path<String>) -> Response {
    let Some(up) = st.update.as_ref().filter(|up| up.blake3 == hash) else {
        return (StatusCode::NOT_FOUND, "unknown build").into_response();
    };
    let Ok(f) = tokio::fs::File::open(&up.path).await else {
        tracing::warn!("the launcher beside this publisher has gone; restart to offer again");
        return (StatusCode::NOT_FOUND, "build not available").into_response();
    };
    let len = f.metadata().await.map(|m| m.len()).ok();
    let mut resp = Body::from_stream(ReaderStream::new(f)).into_response();
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    if let Some(len) = len
        && let Ok(v) = HeaderValue::from_str(&len.to_string())
    {
        headers.insert(header::CONTENT_LENGTH, v);
    }
    resp
}

async fn health(State(st): State<Arc<AppState>>) -> Response {
    let cur = st.current();
    let game_server = if st.colocated {
        if crate::gameserver::is_running() {
            "running"
        } else {
            "stopped"
        }
    } else {
        "unknown"
    };
    let body = serde_json::json!({
        "status": "ok",
        "game_server": game_server,
        "server_name": cur.manifest.server_name,
        "pack_id": cur.manifest.pack_id,
        "files": cur.manifest.files.len(),
        "generated_at": cur.manifest.generated_at,
        "version": env!("CARGO_PKG_VERSION"),
    });
    axum::Json(body).into_response()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use tokio::sync::oneshot;
    use valhsync_core::{UpdateOffer, hash};

    use super::*;

    /// Smallest pack `pack::build` accepts, in a temporary folder.
    fn config(server_root: &std::path::Path) -> Config {
        let mut cfg = Config::default();
        cfg.server.name = "Test publisher".into();
        // RFC 5737 documentation address: never a real server.
        cfg.server.game_address = "203.0.113.10:2456".into();
        cfg.pack.server_root = Some(server_root.to_path_buf());
        cfg
    }

    struct Running {
        addr: SocketAddr,
        stop: Option<oneshot::Sender<()>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Running {
        async fn start(kp: &Keypair, server_root: &std::path::Path, data_dir: PathBuf) -> Self {
            let server = prepare(&config(server_root), kp.clone(), data_dir, None, false).unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (stop, rx) = oneshot::channel();
            let task = tokio::spawn(async move {
                serve_until(listener, server, async {
                    let _ = rx.await;
                })
                .await
                .unwrap();
            });
            Self {
                addr,
                stop: Some(stop),
                task,
            }
        }

        async fn get(&self, path: &str) -> reqwest::Response {
            reqwest::get(format!("http://{}{path}", self.addr))
                .await
                .unwrap()
        }

        async fn stop(mut self) {
            let _ = self.stop.take().unwrap().send(());
            self.task.await.unwrap();
        }
    }

    /// The two documents are one offer in two pieces: either both are there
    /// and the signature covers the bytes, or neither is.
    ///
    /// Which of the two it is depends on whether a launcher happens to sit
    /// beside the test binary, so the test asserts the agreement, not the
    /// presence.
    #[tokio::test]
    async fn the_update_document_and_its_signature_agree() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("winhttp.dll"), b"doorstop").unwrap();
        let data = tempfile::tempdir().unwrap();
        let kp = Keypair::generate();
        let srv = Running::start(&kp, root.path(), data.path().to_path_buf()).await;

        let doc = srv.get("/update.json").await;
        let sig = srv.get("/update.sig").await;
        let offered = doc.status().as_u16();
        assert_eq!(offered, sig.status().as_u16(), "one offer, two routes");
        assert!(offered == 200 || offered == 404, "status {offered}");

        if offered == 200 {
            let bytes = doc.bytes().await.unwrap();
            let sig_text = sig.text().await.unwrap();
            let offer = UpdateOffer::parse_verified(&bytes, sig_text.trim(), &kp.public())
                .expect("the offer verifies against the key this server publishes");

            let build = srv.get(&format!("/update/{}", offer.blake3)).await;
            assert_eq!(build.status().as_u16(), 200);
            let body = build.bytes().await.unwrap();
            assert_eq!(body.len() as u64, offer.size);
            assert_eq!(hash::to_hex(&hash::hash_bytes(&body)), offer.blake3);
        }
        srv.stop().await;
    }

    #[tokio::test]
    async fn only_the_offered_digest_serves_bytes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("winhttp.dll"), b"doorstop").unwrap();
        let data = tempfile::tempdir().unwrap();
        let srv =
            Running::start(&Keypair::generate(), root.path(), data.path().to_path_buf()).await;

        for path in [
            format!("/update/{}", "b".repeat(64)),
            "/update/not-a-digest".to_string(),
            format!("/update/{}", hash::to_hex(&hash::hash_bytes(b"doorstop"))),
        ] {
            let r = srv.get(&path).await;
            assert_eq!(r.status().as_u16(), 404, "{path} must not serve bytes");
        }
        srv.stop().await;
    }
}
