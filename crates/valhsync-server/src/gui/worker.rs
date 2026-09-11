//! Everything slow the window asks for, off the UI thread: scanning and
//! signing a pack, exporting it, serving it, and asking what our public
//! address is.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use anyhow::{Context, Result};
use valhsync_core::Keypair;

use crate::config::Config;
use crate::{keys, pack, serve};

/// What a finished job tells the window.
#[derive(Debug)]
pub(super) enum Msg {
    /// Pack built: file count, total bytes, invite code.
    Scanned {
        files: usize,
        bytes: u64,
        pack_id: String,
        invite: String,
        skipped: Vec<(String, String)>,
    },
    Exported {
        dir: PathBuf,
        files: usize,
        copied: usize,
    },
    /// The live server is up on this address.
    Serving(String),
    /// The live server stopped (on request, or because it failed).
    ServeStopped(Option<String>),
    PublicIp(String),
    Error(String),
    /// A job finished; the window can re-enable its buttons.
    Idle,
}

/// Sends to the window and wakes it up.
#[derive(Clone)]
pub(super) struct Reporter {
    pub(super) tx: Sender<Msg>,
    pub(super) ctx: eframe::egui::Context,
}

impl Reporter {
    pub(super) fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }

    /// Run `job`, reporting its error, then signal idleness either way.
    pub(super) fn run(&self, job: impl FnOnce() -> Result<Msg>) {
        match job() {
            Ok(msg) => self.send(msg),
            Err(e) => self.send(Msg::Error(format!("{e:#}"))),
        }
        self.send(Msg::Idle);
    }
}

/// Build the pack and report what it contains, plus the invite code.
pub(super) fn scan(cfg: &Config, data_dir: &Path) -> Result<Msg> {
    let kp = load_key(data_dir)?;
    let outcome = pack::build(cfg, &kp, data_dir)?;
    let m = &outcome.published.manifest;
    Ok(Msg::Scanned {
        files: m.files.len(),
        bytes: m.total_bytes(),
        pack_id: m.pack_id.clone(),
        invite: invite_code(cfg, &kp)?,
        skipped: outcome.scan.skipped.clone(),
    })
}

pub(super) fn export(cfg: &Config, data_dir: &Path, dir: PathBuf) -> Result<Msg> {
    let kp = load_key(data_dir)?;
    let outcome = pack::build(cfg, &kp, data_dir)?;
    let report = pack::export_static(&outcome.published, data_dir, &dir)?;
    Ok(Msg::Exported {
        dir,
        files: report.files,
        copied: report.copied,
    })
}

/// The signing key, created on first use so the window works out of the box.
pub(super) fn load_key(data_dir: &Path) -> Result<Keypair> {
    let (kp, created) = keys::load_or_create(data_dir)?;
    if created {
        tracing::info!("signing key generated in {}", data_dir.display());
    }
    Ok(kp)
}

pub(super) fn invite_code(cfg: &Config, kp: &Keypair) -> Result<String> {
    valhsync_core::Invite::new(cfg.public_url(), &kp.public(), cfg.server.name.trim())
        .encode()
        .context("cannot build the invite code")
}

/// Run the live HTTP server until `stop` fires. Blocks, so call it on a thread.
pub(super) fn serve_blocking(
    cfg: Config,
    data_dir: PathBuf,
    stop: tokio::sync::oneshot::Receiver<()>,
    rep: &Reporter,
) {
    let result = (|| -> Result<()> {
        let kp = load_key(&data_dir)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("cannot start the async runtime")?;
        rt.block_on(async move {
            let server = serve::prepare(&cfg, kp, data_dir, true)?;
            let addr = cfg.bind_addr()?;
            let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| {
                format!("cannot listen on {addr}; is another valhsync-server running?")
            })?;
            rep.send(Msg::Serving(cfg.public_url()));
            serve::serve_until(listener, server, async {
                let _ = stop.await;
            })
            .await
        })
    })();
    rep.send(Msg::ServeStopped(result.err().map(|e| format!("{e:#}"))));
    rep.send(Msg::Idle);
}

/// Ask a public echo service what address the internet sees us as.
///
/// A deliberate outbound call to a third party, so it only ever happens when
/// the admin presses the button, and the service is named in the interface.
pub(super) const IP_ECHO_SERVICE: &str = "https://api.ipify.org";

pub(super) fn public_ip() -> Result<Msg> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("valhsync-server/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let text = client
        .get(IP_ECHO_SERVICE)
        .send()
        .with_context(|| format!("cannot reach {IP_ECHO_SERVICE}"))?
        .error_for_status()?
        .text()?;
    let ip = text.trim();
    ip.parse::<std::net::IpAddr>()
        .with_context(|| format!("{IP_ECHO_SERVICE} did not answer with an address"))?;
    Ok(Msg::PublicIp(ip.to_string()))
}
