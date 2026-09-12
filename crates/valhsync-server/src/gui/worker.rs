//! Everything slow the window asks for, off the UI thread: scanning and
//! signing a pack, exporting it, serving it, announcing it, and asking what
//! our public address is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use valhsync_core::plan::{Action, PlannedFile, SyncPlan};
use valhsync_core::{Keypair, Manifest, ModChange};

use crate::config::Config;
use crate::{keys, pack, serve, webhook};

/// What a finished job tells the window.
#[derive(Debug)]
pub(super) enum Msg {
    /// Pack built: file count, total bytes, invite code.
    Scanned {
        files: usize,
        bytes: u64,
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
    /// The published link was fetched as a player would fetch it.
    LinkChecked(String),
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
        self.run_then(job, |_| {});
    }

    /// Run a job, and on success do one more thing before the window is told
    /// the work is over.
    ///
    /// The ordering is not a nicety. `Msg::Idle` is what makes the window drop
    /// the channel, so anything sent after it is sent into nothing -- which is
    /// where a "Discord was not told" line would have gone.
    pub(super) fn run_then(&self, job: impl FnOnce() -> Result<Msg>, after: impl FnOnce(&Self)) {
        match job() {
            Ok(msg) => {
                self.send(msg);
                after(self);
            }
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
    let url = cfg.public_url().with_context(|| {
        format!(
            "no address to put in the code: {}",
            cfg.public_url_problem()
        )
    })?;
    valhsync_core::Invite::new(url, &kp.public(), cfg.server.name.trim())
        .encode()
        .context("cannot build the invite code")
}

/// Run the live HTTP server until `stop` fires. Blocks, so call it on a thread.
pub(super) fn serve_blocking(
    cfg: Config,
    data_dir: PathBuf,
    config_path: PathBuf,
    stop: tokio::sync::oneshot::Receiver<()>,
    rep: &Reporter,
) {
    // Copies for the announcement, taken before the runtime swallows the
    // originals: `data_dir` goes to `serve::prepare` and `cfg` into the async
    // block below.
    let announce_cfg = cfg.clone();
    let announce_dir = data_dir.clone();
    let announce_rep = rep.clone();
    let result = (|| -> Result<()> {
        let kp = load_key(&data_dir)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("cannot start the async runtime")?;
        rt.block_on(async move {
            let server = serve::prepare(&cfg, kp, data_dir, Some(config_path), true)?;
            let addr = cfg.bind_addr()?;
            let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| {
                format!("cannot listen on {addr}; is another valhsync-server running?")
            })?;
            rep.send(Msg::Serving(
                cfg.public_url().unwrap_or_else(|| addr.to_string()),
            ));
            // The pack is reachable as of this line, which is the moment worth
            // announcing: `serve::prepare` has just built and published it.
            //
            // On a thread of its own for two reasons. This one is inside the
            // Tokio runtime, where a blocking HTTP client has no business, and
            // the server has to go on serving while Discord takes its time --
            // players downloading the pack must not wait on an announcement
            // about it. Nothing is joined: it reports through the same channel
            // the live server does, which stays open for as long as it serves.
            std::thread::spawn(move || {
                announce(&announce_cfg, &announce_dir, &announce_rep);
            });
            // The window's Stop button, or the dedicated server going away.
            let watch_game = cfg.is_colocated() && cfg.game_server.stop_with_game;
            serve::serve_until(listener, server, async move {
                tokio::select! {
                    _ = stop => {}
                    () = serve::stop_with_game(watch_game) => {}
                }
            })
            .await
        })
    })();
    rep.send(Msg::ServeStopped(result.err().map(|e| format!("{e:#}"))));
    rep.send(Msg::Idle);
}

/// Fetch the published pack exactly as a launcher would: the key, then the
/// manifest and its signature, checked against the key we hold. It is the only
/// answer worth giving to "will the invite code work?" -- everything else is a
/// guess about someone else's network.
pub(super) fn check_link(cfg: &Config, data_dir: &Path) -> Result<Msg> {
    let kp = load_key(data_dir)?;
    let base = cfg
        .public_url()
        .with_context(|| format!("nothing to test: {}", cfg.public_url_problem()))?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("valhsync-server/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let get = |suffix: &str| -> Result<Vec<u8>> {
        let url = format!("{base}{suffix}");
        let body = client
            .get(&url)
            .send()
            .with_context(|| format!("cannot reach {url}"))?
            .error_for_status()
            .with_context(|| format!("{url} answered with an error"))?
            .bytes()
            .with_context(|| format!("cannot read {url}"))?;
        Ok(body.to_vec())
    };

    let published_key = String::from_utf8(get("/key")?)
        .context("/key did not answer with a key")?
        .trim()
        .to_string();
    let ours = kp.public();
    if published_key != ours.to_b64() {
        anyhow::bail!("{base} is publishing a different server's key");
    }
    let manifest = get("/manifest.json")?;
    let signature = String::from_utf8(get("/manifest.sig")?)
        .context("/manifest.sig did not answer with a signature")?;
    // The same checks a launcher runs: the signature, then the paths and the
    // limits, so a pack that no player could apply is reported here first.
    let manifest = valhsync_core::Manifest::parse_verified(
        &manifest,
        signature.trim(),
        &ours,
        &valhsync_core::AllowedRoots::bepinex(),
        &valhsync_core::Limits::default(),
    )
    .context("the published manifest is not what this server signed")?;
    Ok(Msg::LinkChecked(format!(
        "{base} -> {} files, pack {}",
        manifest.files.len(),
        manifest.pack_id
    )))
}

/// Where a mod lives, and so the only part of a pack an announcement talks
/// about. The same prefix [`valhsync_core::changes`] groups by: BepInEx itself
/// and the doorstop files change for reasons that mean nothing to a player.
const PLUGINS: &str = "BepInEx/plugins/";

/// The file, in the data directory, holding what the channel has been told.
const ANNOUNCED: &str = "announced.json";

/// The pack the Discord channel was last told about.
///
/// It exists so that an announcement fires once per pack rather than once per
/// publish. The window rebuilds a pack every time the admin presses Scan, the
/// watcher rebuilds it whenever a file moves in the mod folder, and a pack
/// whose contents did not change is not news -- a channel that gets a message
/// for each of those is a channel everybody mutes, which is the one failure
/// this feature cannot recover from.
///
/// It lives on disk rather than in the window because the window is closed
/// and reopened all day, and a record that died with it would announce the
/// same pack again every morning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Announced {
    /// The identity of the contents, straight from the manifest.
    pack_id: String,
    /// The admin's word to the players, which is half of what was said and
    /// can change on its own: republishing the same files with a new note is
    /// a genuine announcement.
    notes: Option<String>,
    /// Every file under [`PLUGINS`] of that pack, by path and digest.
    ///
    /// Kept because an announcement says what *changed*, and by the time we
    /// post, `published/manifest.json` already holds the new pack -- the
    /// manifest it replaced is gone. This is the smallest thing to keep that
    /// still lets the grouping in [`valhsync_core::changes`] do its work.
    plugins: BTreeMap<String, String>,
}

fn announced_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ANNOUNCED)
}

/// What the channel was last told, or nothing when it has never been told
/// anything.
///
/// A file that cannot be read or parsed counts as nothing: the worst that
/// causes is one announcement too many, which is a far better outcome than a
/// publish that fails over a bookkeeping file.
fn last_announced(data_dir: &Path) -> Option<Announced> {
    let bytes = std::fs::read(announced_path(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write it through a temp file and a rename, so a window killed mid-write
/// leaves the previous record rather than half of the new one.
fn remember_announced(data_dir: &Path, announced: &Announced) -> Result<()> {
    let path = announced_path(data_dir);
    let bytes = serde_json::to_vec_pretty(announced).context("cannot record what was announced")?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

/// Every file of a pack that belongs to a mod, by path and digest.
fn plugin_files(manifest: &Manifest) -> BTreeMap<String, String> {
    manifest
        .files
        .iter()
        .filter(|f| f.path.starts_with(PLUGINS))
        .map(|f| (f.path.clone(), f.blake3.clone()))
        .collect()
}

/// Is this pack worth a message, given what the channel already heard?
///
/// The pack id is the identity of the contents, so a rebuild that changed
/// nothing says nothing. The note is the other half, because republishing the
/// same files with a new word to the players is exactly the case the feature
/// exists for. Nothing else counts: a server renamed, a port moved or a
/// network version read out of the game's log must not fill the channel.
fn worth_announcing(last: Option<&Announced>, manifest: &Manifest) -> bool {
    match last {
        // Nothing was ever announced, so whatever is published now is news --
        // including the first pack after an admin pastes a webhook in, which
        // is also the only proof they get that the address works.
        None => true,
        Some(last) => last.pack_id != manifest.pack_id || last.notes != manifest.notes,
    }
}

/// What changed since the pack the channel last heard about.
///
/// It goes through [`valhsync_core::changes::mods_touched`] rather than
/// grouping by hand: what counts as one mod, what makes a partly-new mod an
/// update rather than an arrival, and the order the three kinds read in all
/// live there, tested, and are what the launcher shows the players. Saying it
/// differently in Discord than in the launcher would be worse than saying
/// nothing. Two file lists are not a plan, so the difference is dressed as
/// one -- that is the only shape the function takes.
fn changes_since(last: Option<&BTreeMap<String, String>>, manifest: &Manifest) -> Vec<ModChange> {
    let nothing = BTreeMap::new();
    let before = last.unwrap_or(&nothing);
    let after = plugin_files(manifest);
    let mut items = Vec::new();
    for (path, digest) in &after {
        let action = match before.get(path) {
            None => Action::Add,
            Some(old) if old != digest => Action::Replace,
            // The same file as last time: nothing to say about it.
            Some(_) => continue,
        };
        items.push(planned(path, action));
    }
    for path in before.keys().filter(|p| !after.contains_key(*p)) {
        items.push(planned(path, Action::Remove));
    }
    valhsync_core::changes::mods_touched(&SyncPlan {
        pack_id: manifest.pack_id.clone(),
        items,
        download_bytes: 0,
    })
}

/// One line of that pretend plan. Only the path and the action are read.
fn planned(path: &str, action: Action) -> PlannedFile {
    PlannedFile {
        path: path.to_string(),
        action,
        size: 0,
        blake3: None,
        policy: None,
    }
}

/// The head of a pack id, which is how the window and the CLI name a pack.
/// The whole digest is 64 characters of hex nobody reads.
fn short_id(pack_id: &str) -> &str {
    pack_id.get(..15).unwrap_or(pack_id)
}

/// Tell the admin's Discord channel that a new pack is out.
///
/// Call it after a publish, on a worker thread, with the data directory whose
/// `published/manifest.json` was just written. It works out on its own whether
/// there is anything to say: no webhook configured, nothing published yet, or
/// the same pack and the same note as last time all mean silence.
///
/// It returns nothing, and that is the contract. Announcing is not a step of
/// publishing: the pack is signed, stored and reachable before this is called,
/// so a webhook that was deleted, rate limited or simply unreachable must not
/// be able to fail anything. The two outcomes are reported like this:
///
/// * A post that went out is logged with the host it reached and nothing else
///   -- never the URL, which is a credential -- and the window stays quiet.
///   The admin sees the message appear in Discord; a second confirmation in
///   the window would be noise.
/// * A post that failed sends [`Msg::Error`], which the window shows as one
///   red line in its log without stopping or undoing anything. The text leads
///   with the pack being published, because that is the part the admin needs
///   to believe before they start hunting for a problem that is not there.
///
/// A failure also leaves the record untouched, so the next publish tries the
/// same announcement again rather than skipping a pack nobody was told about.
pub(super) fn announce(cfg: &Config, data_dir: &Path, rep: &Reporter) {
    match announce_published(cfg, data_dir) {
        Ok(Some(host)) => tracing::info!("new pack announced on {host}"),
        Ok(None) => {}
        Err(e) => rep.send(Msg::Error(format!(
            "the pack is published; Discord was not told: {e:#}"
        ))),
    }
}

/// The work behind [`announce`]. `Ok(None)` is "nothing to say", `Ok(Some)`
/// the host a message reached.
fn announce_published(cfg: &Config, data_dir: &Path) -> Result<Option<String>> {
    let Some(hook) = cfg.discord_hook()? else {
        return Ok(None);
    };
    // The manifest in `published/`, which is the pack players can fetch right
    // now. `load_previous` is named for its other caller, the build that is
    // about to replace it.
    let Some(manifest) = pack::load_previous(data_dir) else {
        return Ok(None);
    };
    let last = last_announced(data_dir);
    if !worth_announcing(last.as_ref(), &manifest) {
        return Ok(None);
    }
    let changes = changes_since(last.as_ref().map(|l| &l.plugins), &manifest);
    webhook::post(
        &hook,
        &manifest.server_name,
        short_id(&manifest.pack_id),
        &changes,
        manifest.notes.as_deref().unwrap_or_default(),
    )?;
    remember_announced(
        data_dir,
        &Announced {
            pack_id: manifest.pack_id.clone(),
            notes: manifest.notes.clone(),
            plugins: plugin_files(&manifest),
        },
    )
    .context(
        "the announcement went out, but recording it failed, so the next publish may repeat it",
    )?;
    Ok(Some(hook.host().to_string()))
}

/// Ask a public echo service what address the internet sees us as.
///
/// An outbound call to a third party, repeated on a timer by the window: an
/// admin who had to press a button for it ended up publishing a stale address
/// after a reboot. It sends nothing but the request, and the service is named
/// in the interface next to what it answered.
///
/// More than one of them, tried in order, because a single hard-coded host is
/// a single point of failure for the one field an admin cannot work out for
/// themselves. One being down, blocked by a network, or blocked by a DNS
/// filter should not leave the address empty with no explanation. They are
/// plain-text echo services that answer with an address and nothing else.
pub(super) const IP_ECHO_SERVICES: &[&str] = &[
    "https://api.ipify.org",
    "https://ifconfig.me/ip",
    "https://icanhazip.com",
];

/// The one named in the interface: what answered, or the first we would try.
pub(super) const IP_ECHO_SERVICE: &str = IP_ECHO_SERVICES[0];

pub(super) fn public_ip() -> Result<Msg> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("valhsync-server/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut last = None;
    for service in IP_ECHO_SERVICES {
        match ask_echo(&client, service) {
            Ok(ip) => return Ok(Msg::PublicIp(ip)),
            // Keep going, and keep the first failure: it is the one about the
            // service the interface names, so it is the one worth reporting.
            Err(e) => last.get_or_insert(e),
        };
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no address service to ask")))
}

fn ask_echo(client: &reqwest::blocking::Client, service: &str) -> Result<String> {
    let text = client
        .get(service)
        .send()
        .with_context(|| format!("cannot reach {service}"))?
        .error_for_status()?
        .text()?;
    let ip = text.trim();
    ip.parse::<std::net::IpAddr>()
        .with_context(|| format!("{service} did not answer with an address"))?;
    Ok(ip.to_string())
}

#[cfg(test)]
mod announce_tests {
    use std::sync::mpsc;

    use valhsync_core::{ChangeKind, FileEntry, Policy};

    use super::*;

    fn manifest(files: &[(&str, &str)]) -> Manifest {
        Manifest::new(
            "Frostveil",
            "203.0.113.10:2456",
            Vec::new(),
            files
                .iter()
                .map(|(path, digest)| FileEntry {
                    path: (*path).to_string(),
                    size: 1,
                    blake3: (*digest).to_string(),
                    policy: Policy::Enforce,
                })
                .collect(),
        )
    }

    fn record(manifest: &Manifest) -> Announced {
        Announced {
            pack_id: manifest.pack_id.clone(),
            notes: manifest.notes.clone(),
            plugins: plugin_files(manifest),
        }
    }

    fn named(changes: &[ModChange]) -> Vec<(&str, ChangeKind)> {
        changes.iter().map(|c| (c.name.as_str(), c.kind)).collect()
    }

    /// The whole point of the record: publishing is not announcing. A pack
    /// scanned four times is one message, not four.
    #[test]
    fn the_same_pack_is_announced_once() {
        let m = manifest(&[("BepInEx/plugins/Seasonality/Seasonality.dll", "a1")]);
        assert!(worth_announcing(None, &m), "never announced anything yet");
        assert!(!worth_announcing(Some(&record(&m)), &m));

        // A server renamed, a port moved, a network version read out of the
        // log: the contents did not change, so there is nothing to say.
        let mut renamed = m.clone();
        renamed.server_name = "Frostveil II".into();
        renamed.network_version = Some(37);
        assert!(!worth_announcing(Some(&record(&m)), &renamed));

        // New contents, and a new word about the same contents, both are.
        let changed = manifest(&[("BepInEx/plugins/Seasonality/Seasonality.dll", "a2")]);
        assert!(worth_announcing(Some(&record(&m)), &changed));
        let mut renoted = m.clone();
        renoted.notes = Some("Empty your chests first.".into());
        assert!(worth_announcing(Some(&record(&m)), &renoted));
    }

    /// What the message lists has to be what the launcher lists, so it is
    /// worked out by the same grouping: one line per mod, whatever the file
    /// count, and a mod partly replaced is an update rather than an arrival.
    #[test]
    fn the_message_lists_what_the_launcher_would() {
        let before = manifest(&[
            ("BepInEx/plugins/EpicLoot/EpicLoot.dll", "e1"),
            ("BepInEx/plugins/EpicLoot/assets/a.bundle", "e2"),
            ("BepInEx/plugins/OldChests/OldChests.dll", "o1"),
            // Not a mod: it must never reach the channel.
            ("winhttp.dll", "w1"),
        ]);
        let after = manifest(&[
            ("BepInEx/plugins/EpicLoot/EpicLoot.dll", "e9"),
            ("BepInEx/plugins/EpicLoot/assets/a.bundle", "e2"),
            ("BepInEx/plugins/Seasonality/Seasonality.dll", "s1"),
            ("BepInEx/plugins/Seasonality/assets/b.bundle", "s2"),
            ("winhttp.dll", "w9"),
        ]);
        let changes = changes_since(Some(&plugin_files(&before)), &after);
        assert_eq!(
            named(&changes),
            vec![
                ("Seasonality", ChangeKind::Added),
                ("EpicLoot", ChangeKind::Updated),
                ("OldChests", ChangeKind::Removed),
            ]
        );
    }

    /// The first message a channel gets is the pack arriving, which is also
    /// the admin's only proof that the address they pasted works.
    #[test]
    fn the_first_announcement_is_the_whole_pack() {
        let m = manifest(&[
            ("BepInEx/plugins/A/a.dll", "a1"),
            ("BepInEx/plugins/B/b.dll", "b1"),
            ("BepInEx/core/BepInEx.dll", "c1"),
        ]);
        assert_eq!(
            named(&changes_since(None, &m)),
            vec![("A", ChangeKind::Added), ("B", ChangeKind::Added)]
        );
    }

    /// A pack whose mods are untouched still gets a message when the note
    /// changed, and the message says so rather than inventing changes.
    #[test]
    fn a_note_only_republish_lists_nothing() {
        let m = manifest(&[("BepInEx/plugins/A/a.dll", "a1")]);
        assert!(changes_since(Some(&plugin_files(&m)), &m).is_empty());
    }

    #[test]
    fn the_record_survives_a_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let m = manifest(&[("BepInEx/plugins/A/a.dll", "a1")]);
        let mut written = record(&m);
        written.notes = Some("Videz vos coffres.".into());

        assert!(last_announced(tmp.path()).is_none(), "nothing yet");
        remember_announced(tmp.path(), &written).unwrap();
        let back = last_announced(tmp.path()).expect("written and read back");
        assert_eq!(back.pack_id, written.pack_id);
        assert_eq!(back.notes, written.notes);
        assert_eq!(back.plugins, written.plugins);

        // A scratch file somebody truncated is not a reason to stop
        // publishing: it reads as "nothing was ever announced".
        std::fs::write(announced_path(tmp.path()), b"{ not json").unwrap();
        assert!(last_announced(tmp.path()).is_none());
    }

    /// Nothing configured means nothing contacted. The test is here because
    /// the opposite -- a build that posts somewhere by default -- is the kind
    /// of bug nobody finds until it is in somebody's channel.
    #[test]
    fn without_a_webhook_nothing_is_posted_and_nothing_is_said() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let rep = Reporter {
            tx,
            ctx: eframe::egui::Context::default(),
        };
        let cfg = Config::default();
        assert!(cfg.server.discord_webhook.is_none(), "off by default");

        announce(&cfg, tmp.path(), &rep);
        assert!(rx.try_recv().is_err(), "the window was told nothing");
        assert!(!announced_path(tmp.path()).exists(), "nothing recorded");
    }

    /// A webhook is set, but nothing has been published from this data
    /// directory yet: there is no pack to announce, and no network call to
    /// make. Reached in the window when the admin pastes the address before
    /// their first scan.
    #[test]
    fn with_nothing_published_there_is_nothing_to_announce() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.server.discord_webhook = Some("https://discord.com/api/webhooks/1/token".into());
        assert!(announce_published(&cfg, tmp.path()).unwrap().is_none());
    }

    #[test]
    fn a_pack_id_is_shown_by_its_head() {
        assert_eq!(short_id("b3:0123456789abcdef0123"), "b3:0123456789ab");
        assert_eq!(short_id("short"), "short");
    }
}
