//! `valhsync-server.toml`: what to publish, from where, to whom.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use valhsync_core::scan::{PolicyRules, ScanConfig, ScanSource};
use valhsync_core::{AllowedRoots, Limits, Policy};

/// ValhSync serves on the game's own port. Valheim uses it in UDP only, so
/// the two do not collide and no second router rule is needed.
pub const DEFAULT_PORT: u16 = valhsync_core::invite::DEFAULT_PORT;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Window language, `fr` or `en`. Unset means follow the system.
    ///
    /// A preference rather than server configuration, but this file is the
    /// only thing the publisher keeps, and a choice that lived only in the
    /// window was back to English on the next launch.
    #[serde(default)]
    pub language: Option<String>,
    pub server: ServerSection,
    pub pack: PackSection,
    pub policy: PolicySection,
    pub limits: LimitsSection,
    pub game_server: GameServerSection,
}

/// How ValhSync may start the Valheim dedicated server. Optional: leave it out
/// and ValhSync never touches the game server at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameServerSection {
    /// The admin's own start script (`.bat` / `.sh`).
    pub start_script: Option<PathBuf>,
    /// Stop publishing when the dedicated server stops. Only applies when
    /// ValhSync runs on the same machine and can see it.
    pub stop_with_game: bool,
}

impl Default for GameServerSection {
    fn default() -> Self {
        Self {
            start_script: None,
            stop_with_game: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSection {
    /// Shown to players in the launcher.
    pub name: String,
    /// TCP address `valhsync-server` listens on.
    pub bind: String,
    /// URL players reach this server at. Goes into the invite code.
    pub public_url: Option<String>,
    /// `host:port` the launcher hands to Valheim.
    pub game_address: String,
    /// Publish by serving from this machine, rather than by exporting a folder
    /// to upload. `None` means the admin has not chosen, and
    /// [`Config::publishes_live`] decides.
    ///
    /// It belongs here rather than in the window: whether publishing follows
    /// the game server hangs off it, and a choice that lived only in the tab
    /// strip was back to its default on the next launch, with the admin left
    /// wondering why nothing came online.
    pub publish_live: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PackSection {
    /// Game root of the dedicated server, or any folder laid out like one.
    /// Optional: a hosted server (G-Portal, Nitrado...) has no local root; use
    /// `client_extras` alone with a local copy of the pack.
    pub server_root: Option<PathBuf>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Client-only files, same layout as the game root. Overrides server files.
    pub client_extras: Option<PathBuf>,
    /// Folders where unknown files get quarantined on the player's side.
    pub managed_roots: Vec<String>,
    /// Where `export` writes the folder to upload. Unset means beside the
    /// configuration, in `pack-site`. Kept here for the same reason as the
    /// publish mode: a path the admin typed once should still be there on the
    /// next launch.
    pub export_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicySection {
    pub default: String,
    pub seed: Vec<String>,
    pub enforce: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_field_names)] // mirrors the TOML keys
pub struct LimitsSection {
    pub max_file_mb: u64,
    pub max_pack_mb: u64,
    pub max_files: usize,
}

/// What gets published unless the admin says otherwise.
///
/// The whole `BepInEx` tree, not a list of its subfolders: which ones exist
/// depends on the BepInEx version (`monomod`, `unity-libs`, `interop`...) and
/// mods drop files wherever they like inside it. Enumerating would quietly
/// miss whatever the list did not foresee.
pub fn default_include() -> Vec<String> {
    [
        // Doorstop, which is what loads BepInEx at all.
        "winhttp.dll",
        "doorstop_config.ini",
        ".doorstop_version",
        "doorstop_libs/**",
        "start_game_bepinex.sh",
        // Everything BepInEx and its mods use.
        "BepInEx/**",
        // Shipped by older packs; the glob matches nothing when absent.
        "unstripped_corlib/**",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// Noise that lives inside the tree and must not travel: logs, caches and the
/// backups editors leave behind.
pub fn default_exclude() -> Vec<String> {
    [
        "BepInEx/**/*.log",
        "BepInEx/cache/**",
        "BepInEx/**/*.bak",
        "BepInEx/**/*.old",
        "BepInEx/**/*.tmp",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

pub fn default_managed_roots() -> Vec<String> {
    vec!["BepInEx/plugins".into(), "BepInEx/patchers".into()]
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            name: "My Valheim server".into(),
            bind: format!("0.0.0.0:{DEFAULT_PORT}"),
            public_url: None,
            game_address: "valheim.example.org:2456".into(),
            publish_live: None,
        }
    }
}

impl Default for PackSection {
    fn default() -> Self {
        Self {
            server_root: None,
            include: default_include(),
            exclude: default_exclude(),
            client_extras: None,
            managed_roots: default_managed_roots(),
            export_dir: None,
        }
    }
}

impl Default for PolicySection {
    fn default() -> Self {
        let rules = PolicyRules::default();
        Self {
            default: rules.default.as_str().into(),
            seed: rules.seed,
            enforce: rules.enforce,
        }
    }
}

impl Default for LimitsSection {
    fn default() -> Self {
        Self {
            max_file_mb: 200,
            max_pack_mb: 2048,
            max_files: 5000,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!(
                "cannot read {}; run `valhsync-server init` first or pass --config",
                path.display()
            )
        })?;
        let cfg: Self = toml::from_str(&text)
            .with_context(|| format!("{} is not a valid configuration", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.server.name.trim().is_empty() {
            bail!("[server] name must not be empty");
        }
        self.bind_addr()?;
        if !valhsync_core::manifest::is_valid_game_address(self.server.game_address.trim()) {
            bail!(
                "[server] game_address must be host:port (letters, digits, '.', '-', '_', ':', '[', ']')"
            );
        }
        if self.pack.server_root.is_none() && self.pack.client_extras.is_none() {
            bail!("[pack] needs at least server_root or client_extras");
        }
        if let Some(root) = &self.pack.server_root
            && !root.is_dir()
        {
            bail!("[pack] server_root {} is not a directory", root.display());
        }
        // `client_extras` is for the handful of mods that run on players
        // only. Most servers have none, so a missing folder means "nothing to
        // add", not an error: the pack is the server's BepInEx tree either way.
        if self.pack.server_root.is_none()
            && self.pack.client_extras.as_ref().is_none_or(|d| !d.is_dir())
        {
            bail!("[pack] server_root is not set, and there is no client_extras folder either");
        }
        self.policy_rules()?;
        for root in &self.pack.managed_roots {
            valhsync_core::path::validate(&format!("{root}/_"), &AllowedRoots::bepinex())
                .map_err(|e| anyhow::anyhow!("[pack] managed_roots {root:?}: {e}"))?;
        }
        Ok(())
    }

    /// Things that are not errors but will bite the admin later. Printed by
    /// every command that publishes.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if valhsync_core::manifest::is_private_host(self.server.game_address.trim()) {
            out.push(format!(
                "[server] game_address is {}, a local address.\n  \
                 Players outside your network cannot use it, and a server started with \
                 -crossplay refuses local addresses even on the LAN (Iron Gate: \"it's not \
                 possible to connect using a local IP address\").\n  \
                 Use your public IP or a DNS name here.",
                self.server.game_address.trim()
            ));
        }
        if self.server.public_url.is_none() {
            out.push(
                "[server] public_url is not set, so the invite code uses this machine's LAN \
                 address and only works for players on your network."
                    .to_string(),
            );
        }
        out
    }

    /// Does this publisher run beside the dedicated server, and so know
    /// whether the game is up? A publisher holding only a copy of the pack
    /// must not guess.
    /// Does this publisher serve the pack itself?
    ///
    /// Unset means unchosen: a publisher sitting beside the dedicated server
    /// serves it, because the port is already open and that is the whole point
    /// of using the game's own. One publishing from elsewhere has nothing to
    /// be reached at and exports a folder instead.
    #[must_use]
    pub fn publishes_live(&self) -> bool {
        self.server
            .publish_live
            .unwrap_or_else(|| self.is_colocated())
    }

    pub fn is_colocated(&self) -> bool {
        self.pack.server_root.is_some() || self.game_server.start_script.is_some()
    }

    pub fn bind_addr(&self) -> Result<SocketAddr> {
        self.server
            .bind
            .parse()
            .with_context(|| format!("[server] bind {:?} is not host:port", self.server.bind))
    }

    pub fn limits(&self) -> Limits {
        Limits::from_mib(
            self.limits.max_file_mb,
            self.limits.max_pack_mb,
            self.limits.max_files,
        )
    }

    pub fn policy_rules(&self) -> Result<PolicyRules> {
        let default = match self.policy.default.as_str() {
            "enforce" => Policy::Enforce,
            "seed" => Policy::Seed,
            other => bail!("[policy] default must be \"enforce\" or \"seed\", not {other:?}"),
        };
        Ok(PolicyRules {
            default,
            seed: self.policy.seed.clone(),
            enforce: self.policy.enforce.clone(),
        })
    }

    pub fn sources(&self) -> Vec<ScanSource> {
        let mut sources = Vec::new();
        if let Some(root) = &self.pack.server_root {
            sources.push(ScanSource {
                root: root.clone(),
                include: self.pack.include.clone(),
                exclude: self.pack.exclude.clone(),
            });
        }
        // Skipped when the folder is not there: see `validate`.
        if let Some(extras) = self.pack.client_extras.as_ref().filter(|d| d.is_dir()) {
            sources.push(ScanSource {
                root: extras.clone(),
                include: Vec::new(),
                exclude: self.pack.exclude.clone(),
            });
        }
        sources
    }

    pub fn scan_config(&self) -> Result<ScanConfig> {
        Ok(ScanConfig {
            sources: self.sources(),
            policy: self.policy_rules()?,
            limits: self.limits(),
            roots: AllowedRoots::bepinex(),
        })
    }

    /// Folders the `serve` command watches for changes.
    pub fn watch_paths(&self) -> Vec<PathBuf> {
        self.sources().into_iter().map(|s| s.root).collect()
    }

    /// URL that goes into the invite code.
    /// The address players will contact.
    ///
    /// Either what the admin published, or the game server's own address on
    /// ValhSync's port -- players already have that one, and it is already
    /// routed. This machine's LAN address is never invented in its place: an
    /// invite code carrying `192.168.x.x` works for the admin and for nobody
    /// else, which is the worst way for it to fail.
    ///
    /// `None` when the configuration says nothing usable yet.
    pub fn public_url(&self) -> Option<String> {
        if let Some(url) = &self.server.public_url {
            let url = url.trim().trim_end_matches('/');
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
        let host = self.server.game_address.trim().rsplit_once(':')?.0.trim();
        if host.is_empty() || is_placeholder(host) || valhsync_core::manifest::is_private_host(host)
        {
            return None;
        }
        let port = self.bind_addr().map_or(DEFAULT_PORT, |a| a.port());
        Some(format!("http://{host}:{port}"))
    }

    /// Why there is no address to hand out, in one sentence.
    pub fn public_url_problem(&self) -> &'static str {
        let host = self
            .server
            .game_address
            .trim()
            .rsplit_once(':')
            .map_or("", |(h, _)| h.trim());
        if host.is_empty() || is_placeholder(host) {
            "the game address is still the example one"
        } else {
            "the game address is a local one, so it cannot be handed to players"
        }
    }
}

/// Values `init` fills into a fresh configuration.
#[derive(Debug, Clone)]
pub struct TemplateOptions {
    pub name: String,
    pub game_address: String,
    pub public_url: Option<String>,
    pub server_root: Option<PathBuf>,
    pub client_extras: PathBuf,
    pub start_script: Option<PathBuf>,
}

impl From<&TemplateOptions> for Config {
    fn from(o: &TemplateOptions) -> Self {
        let mut cfg = Self::default();
        cfg.server.name.clone_from(&o.name);
        cfg.server.game_address.clone_from(&o.game_address);
        // Serve on whatever port the game was given: one rule covers both.
        if let Some(port) = game_port(&o.game_address) {
            cfg.server.bind = format!("0.0.0.0:{port}");
        }
        cfg.server.public_url.clone_from(&o.public_url);
        cfg.pack.server_root.clone_from(&o.server_root);
        cfg.pack.client_extras = Some(o.client_extras.clone());
        cfg.game_server.start_script.clone_from(&o.start_script);
        cfg
    }
}

/// The hostnames the templates write as examples. They parse, they resolve to
/// nothing, and a code built on one fails at the player's end with no clue as
/// to why.
fn is_placeholder(host: &str) -> bool {
    ["valheim.example.org", "your.public.address", "example.com"]
        .iter()
        .any(|p| host.eq_ignore_ascii_case(p))
}

/// Is this `host:port` one of the template's examples? The window needs to
/// know: a placeholder parses and is not private, so nothing else catches it,
/// and publishing one sends every player to a name that resolves to nothing.
pub(crate) fn is_placeholder_address(address: &str) -> bool {
    let host = address
        .trim()
        .rsplit_once(':')
        .map_or(address.trim(), |(h, _)| h);
    is_placeholder(host.trim())
}

/// The port out of a `host:port` game address.
fn game_port(address: &str) -> Option<u16> {
    address.trim().rsplit_once(':')?.1.trim().parse().ok()
}

fn toml_str(s: &str) -> String {
    if s.contains('\'') {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{s}'")
    }
}

fn toml_list(items: &[String], indent: &str) -> String {
    items
        .iter()
        .map(|i| format!("{indent}{},", toml_str(i)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn toml_opt_path(key: &str, value: Option<&PathBuf>, example: &str) -> String {
    match value {
        Some(p) => format!("{key} = {}", toml_str(&p.display().to_string())),
        None => format!("# {key} = {example}"),
    }
}

/// Render a configuration as commented TOML.
///
/// The window and `init` both write configurations through this, so a file
/// edited in the GUI keeps its explanations instead of decaying into bare
/// key/value pairs.
#[allow(clippy::too_many_lines)] // the file it writes, read top to bottom
pub fn to_commented_toml(cfg: &Config) -> String {
    format!(
        r#"# ValhSync server configuration.
# Edit here or in the ValhSync window, then `valhsync-server scan` to check the
# result. Publish with `export` (static files, nothing to open on the router)
# or `serve` (live server on the port below).

# Window language: "fr", "en", or left out to follow the system.
{language}

[server]
# Name shown to players in the launcher.
name = {name}
# TCP address to listen on, for `serve` only. `export` needs no port at all.
bind = {bind}
# URL players reach the pack at. Goes into the invite code: the folder holding
# manifest.json for a static export, or http://your.address:2456 for `serve`.
{public_url}
# host:port the launcher hands to Valheim.
# Use your PUBLIC IP or a DNS name, not a local 192.168.x address: a server
# started with -crossplay relays through PlayFab and refuses local addresses
# even for players on the same network.
game_address = {game_address}
# Serve the pack from this machine (true) or write a folder to upload (false).
# Left out: serve it when this publisher sits beside the dedicated server,
# since the game's port is already open. The window follows this setting --
# with `true`, publishing comes online by itself whenever the game server is up.
{publish_live}

[pack]
# Game root of the dedicated server. Everything matching `include` (minus
# `exclude`) is published. Leave it out for a hosted server: put a copy of the
# pack in `client_extras` instead.
{server_root}
include = [
{include}
]
exclude = [
{exclude}
]
# Client-only files (mods that must not run on the server, e.g. Unshamed,
# ConfigManager), laid out exactly like the game root:
#   client-extras/BepInEx/plugins/Unshamed/Unshamed.dll
# Files here override files of the same path from server_root.
{client_extras}
# Folders ValhSync owns on the player's side. Unknown .dll files found there
# (leftovers of other mod packs) are moved to BepInEx/_valhsync_quarantine/.
managed_roots = [
{managed_roots}
]
# Where `export` writes the folder to upload. Left out: `pack-site`, beside
# this file.
{export_dir}

[policy]
# "enforce": always replaced when different. "seed": installed only if absent,
# then never touched (player preferences).
# Only the .cfg files BepInEx generates are seeded: what a mod ships inside
# config/ (YAML tables, texture packs) is data the players must receive.
default = {policy_default}
seed = [
{seed}
]
enforce = [
{enforce}
]

[limits]
max_file_mb = {max_file_mb}
max_pack_mb = {max_pack_mb}
max_files = {max_files}

[game_server]
# Optional: your own start script. ValhSync can launch it for you, in its own
# console window. It never stops it: Valheim only saves the world when it gets
# Ctrl+C in that window.
{start_script}
# Stop publishing once the dedicated server stops. ValhSync waits until it has
# seen the game server running before binding its own life to it, so starting
# the two in either order works. Ignored when ValhSync publishes from another
# machine and cannot see the game.
stop_with_game = {stop_with_game}
"#,
        name = toml_str(cfg.server.name.trim()),
        bind = toml_str(&cfg.server.bind),
        public_url = match &cfg.server.public_url {
            Some(u) => format!("public_url = {}", toml_str(u)),
            None => "# public_url = \"http://your.public.address:2456\"".to_string(),
        },
        game_address = toml_str(cfg.server.game_address.trim()),
        server_root = toml_opt_path(
            "server_root",
            cfg.pack.server_root.as_ref(),
            "'C:\\Program Files (x86)\\Steam\\steamapps\\common\\Valheim dedicated server'"
        ),
        include = toml_list(&cfg.pack.include, "  "),
        exclude = toml_list(&cfg.pack.exclude, "  "),
        client_extras = toml_opt_path(
            "client_extras",
            cfg.pack.client_extras.as_ref(),
            "'C:\\valhsync\\client-extras'"
        ),
        managed_roots = toml_list(&cfg.pack.managed_roots, "  "),
        language = match &cfg.language {
            Some(c) => format!("language = {}", toml_str(c)),
            None => "# language = \"fr\"".to_string(),
        },
        publish_live = match cfg.server.publish_live {
            Some(v) => format!("publish_live = {v}"),
            None => "# publish_live = true".to_string(),
        },
        export_dir = toml_opt_path(
            "export_dir",
            cfg.pack.export_dir.as_ref(),
            r"'C:\valhsync\pack-site'"
        ),
        policy_default = toml_str(&cfg.policy.default),
        seed = toml_list(&cfg.policy.seed, "  "),
        enforce = toml_list(&cfg.policy.enforce, "  "),
        max_file_mb = cfg.limits.max_file_mb,
        max_pack_mb = cfg.limits.max_pack_mb,
        max_files = cfg.limits.max_files,
        stop_with_game = cfg.game_server.stop_with_game,
        start_script = toml_opt_path(
            "start_script",
            cfg.game_server.start_script.as_ref(),
            "'C:\\...\\start_valheim_server.bat'"
        ),
    )
}

/// Write a configuration to disk through a temp file and a rename.
pub fn save(cfg: &Config, path: &Path) -> Result<()> {
    let text = to_commented_toml(cfg);
    // Never write something we could not read back.
    let _: Config = toml::from_str(&text)
        .context("internal error: the generated configuration is not valid TOML")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

/// A fresh, commented configuration for `init`.
pub fn template(opts: &TemplateOptions) -> String {
    to_commented_toml(&Config::from(opts))
}

#[cfg(test)]
mod publish_mode_tests {
    use super::*;

    /// This was UI state, reset to "export" on every launch, so publishing
    /// never followed the game server until the admin clicked the tab by hand.
    #[test]
    fn a_publisher_beside_the_game_serves_unless_told_otherwise() {
        let mut cfg = Config::default();
        assert!(!cfg.publishes_live(), "nothing local to serve from");

        cfg.pack.server_root = Some(PathBuf::from("/srv/valheim"));
        assert!(cfg.publishes_live(), "beside the game, the port is open");

        cfg.server.publish_live = Some(false);
        assert!(!cfg.publishes_live(), "an explicit choice wins");
        cfg.server.publish_live = Some(true);
        assert!(cfg.publishes_live());
    }

    #[test]
    fn the_choice_survives_a_round_trip() {
        let mut cfg = Config::default();
        cfg.server.publish_live = Some(true);
        cfg.language = Some("fr".into());
        cfg.pack.export_dir = Some(PathBuf::from("/tmp/pack-site"));
        // Through `to_commented_toml`, which is what `save` writes -- a
        // round trip via `toml::to_string` would have passed while the real
        // file silently dropped all three.
        let text = to_commented_toml(&cfg);
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.server.publish_live, Some(true));
        assert_eq!(back.language.as_deref(), Some("fr"));
        assert_eq!(back.pack.export_dir, cfg.pack.export_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commented_toml_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let extras = tmp.path().join("client-extras");
        std::fs::create_dir_all(&extras).unwrap();
        let mut cfg = Config::default();
        cfg.server.name = "A modded server".into();
        cfg.server.game_address = "203.0.113.10:2456".into();
        cfg.server.public_url = Some("https://you.github.io/pack".into());
        cfg.pack.server_root = Some(tmp.path().to_path_buf());
        cfg.pack.client_extras = Some(extras.clone());
        cfg.pack
            .exclude
            .push("BepInEx/plugins/DiscordConnector/**".into());
        cfg.game_server.start_script = Some(tmp.path().join("start_valheim_server.bat"));
        cfg.validate().unwrap();

        let text = to_commented_toml(&cfg);
        assert!(
            text.contains("# ValhSync server configuration."),
            "comments kept"
        );
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.server.name, cfg.server.name);
        assert_eq!(back.server.game_address, cfg.server.game_address);
        assert_eq!(back.server.public_url, cfg.server.public_url);
        assert_eq!(back.pack.server_root, cfg.pack.server_root);
        assert_eq!(back.pack.client_extras, cfg.pack.client_extras);
        assert_eq!(back.pack.exclude, cfg.pack.exclude);
        assert_eq!(back.pack.managed_roots, cfg.pack.managed_roots);
        assert_eq!(back.policy.seed, cfg.policy.seed);
        assert_eq!(back.limits.max_files, cfg.limits.max_files);
        assert_eq!(back.game_server.start_script, cfg.game_server.start_script);
        assert_eq!(
            back.game_server.stop_with_game,
            cfg.game_server.stop_with_game
        );

        // A second pass must be byte-identical: editing in the window twice
        // may not drift the file.
        assert_eq!(to_commented_toml(&back), text);
    }

    #[test]
    fn save_writes_readable_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("valhsync-server.toml");
        let mut cfg = Config::default();
        cfg.pack.server_root = Some(tmp.path().to_path_buf());
        cfg.server.name = "A \"quoted\" name".into();
        save(&cfg, &path).unwrap();
        let back = Config::load(&path).unwrap();
        assert_eq!(back.server.name, cfg.server.name);
    }

    #[test]
    fn template_parses_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let extras = tmp.path().join("client-extras");
        std::fs::create_dir_all(&extras).unwrap();
        let text = template(&TemplateOptions {
            name: "A modded server".into(),
            game_address: "valheim.example.org:2456".into(),
            public_url: None,
            server_root: Some(tmp.path().to_path_buf()),
            client_extras: extras.clone(),
            start_script: None,
        });
        let cfg: Config = toml::from_str(&text).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.server.name, "A modded server");
        assert_eq!(cfg.pack.include, default_include());
        assert_eq!(cfg.pack.client_extras.as_deref(), Some(extras.as_path()));
        assert_eq!(cfg.sources().len(), 2);
        assert_eq!(cfg.limits(), Limits::default());

        // A template still carrying the example address has nothing to hand
        // to players, and says so instead of building a code around it.
        assert_eq!(cfg.public_url(), None);
        assert!(cfg.public_url_problem().contains("example"));
    }

    #[test]
    fn a_missing_client_extras_folder_stops_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.server.game_address = "203.0.113.10:2456".into();
        cfg.pack.server_root = Some(tmp.path().to_path_buf());
        // Most servers run every mod on both sides and never create it.
        cfg.pack.client_extras = Some(tmp.path().join("client-extras"));

        cfg.validate().unwrap();
        assert_eq!(cfg.sources().len(), 1, "only the server's own tree");

        // Once it exists it is scanned, without anything else changing.
        std::fs::create_dir_all(tmp.path().join("client-extras")).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.sources().len(), 2);

        // What is still refused: nothing to scan at all.
        cfg.pack.server_root = None;
        cfg.pack.client_extras = Some(tmp.path().join("nowhere"));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn the_invite_address_is_the_game_address() {
        let mut cfg = Config::default();
        cfg.server.bind = "0.0.0.0:2456".into();

        // Players already have this one, and it is already routed.
        cfg.server.game_address = "203.0.113.10:2456".into();
        assert_eq!(
            cfg.public_url().as_deref(),
            Some("http://203.0.113.10:2456")
        );

        // Never this machine's own address: it works for the admin alone.
        for local in ["192.168.0.50:2456", "10.0.0.4:2456", "127.0.0.1:2456"] {
            cfg.server.game_address = local.into();
            assert_eq!(cfg.public_url(), None, "{local}");
            assert!(cfg.public_url_problem().contains("local"));
        }

        // A hosted export wins over everything.
        cfg.server.public_url = Some("https://you.github.io/pack/".into());
        assert_eq!(
            cfg.public_url().as_deref(),
            Some("https://you.github.io/pack")
        );
    }

    #[test]
    fn validation_catches_common_mistakes() {
        let mut cfg = Config::default();
        assert!(cfg.validate().is_err(), "no sources");
        let tmp = tempfile::tempdir().unwrap();
        cfg.pack.server_root = Some(tmp.path().to_path_buf());
        cfg.validate().unwrap();
        cfg.server.bind = "2470".into();
        assert!(cfg.validate().is_err());
        cfg.server.bind = "0.0.0.0:2470".into();
        cfg.policy.default = "maybe".into();
        assert!(cfg.validate().is_err());
        cfg.policy.default = "seed".into();
        cfg.pack.managed_roots = vec!["valheim_Data".into()];
        assert!(cfg.validate().is_err());
    }
}
