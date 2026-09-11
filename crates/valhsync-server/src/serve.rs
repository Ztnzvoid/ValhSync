//! HTTP server and folder watcher.
//!
//! Four read-only routes, no listing, no auth: the content is not secret and
//! its integrity is guaranteed by the signature. `/files/{hash}` only ever
//! opens `store/<hash>` for a hash the current or previous manifest names.

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
pub fn prepare(cfg: &Config, keypair: Keypair, data_dir: PathBuf, watch: bool) -> Result<Server> {
    let outcome = pack::build(cfg, &keypair, &data_dir)?;
    pack::print_summary(&outcome);

    let invite =
        Invite::new(cfg.public_url(), &keypair.public(), cfg.server.name.trim()).encode()?;
    let state = Arc::new(AppState {
        current: RwLock::new(Arc::clone(&outcome.published)),
        store: Store::open(&data_dir)?,
        invite,
        server_name: cfg.server.name.trim().to_string(),
        pubkey: keypair.public().to_b64(),
        // Only a publisher sitting next to the game server can see it. One
        // publishing a copy of the pack from elsewhere must not guess.
        colocated: cfg.pack.server_root.is_some() || cfg.game_server.start_script.is_some(),
    });

    let watcher = if watch {
        Some(spawn_watcher(cfg, keypair, data_dir, Arc::clone(&state))?)
    } else {
        None
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/manifest.json", get(manifest))
        .route("/manifest.sig", get(signature))
        .route("/key", get(public_key))
        .route("/files/{hash}", get(file))
        .route("/health", get(health))
        .with_state(state);

    Ok(Server {
        app,
        published: outcome.published,
        watcher,
    })
}

/// Serve on the configured bind address until Ctrl+C.
pub async fn run(cfg: Config, keypair: Keypair, data_dir: PathBuf) -> Result<()> {
    let server = prepare(&cfg, keypair, data_dir, true)?;
    let addr = cfg.bind_addr()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot listen on {addr}; is another valhsync-server running?"))?;
    tracing::info!(
        "listening on http://{addr}  (public URL: {})",
        cfg.public_url()
    );
    tracing::info!("press Ctrl+C to stop");
    serve_until(listener, server, shutdown_signal()).await
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

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}

/// Watch every source folder; rebuild after `SETTLE` of quiet. Events under
/// the data directory are ignored so our own writes never trigger a rebuild.
fn spawn_watcher(
    cfg: &Config,
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

    let cfg = cfg.clone();
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
            tracing::info!("pack changed, rebuilding");
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
