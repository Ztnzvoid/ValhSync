//! `valsync-server.toml`: what to publish, from where, to whom.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use valsync_core::scan::{PolicyRules, ScanConfig, ScanSource};
use valsync_core::{AllowedRoots, Limits, Policy};

pub const DEFAULT_PORT: u16 = 2470;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerSection,
    pub pack: PackSection,
    pub policy: PolicySection,
    pub limits: LimitsSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSection {
    /// Shown to players in the launcher.
    pub name: String,
    /// TCP address `valsync-server` listens on.
    pub bind: String,
    /// URL players reach this server at. Goes into the invite code.
    pub public_url: Option<String>,
    /// `host:port` the launcher hands to Valheim.
    pub game_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicySection {
    pub default: String,
    pub seed: Vec<String>,
    pub enforce: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_field_names)] // mirrors the TOML keys
pub struct LimitsSection {
    pub max_file_mb: u64,
    pub max_pack_mb: u64,
    pub max_files: usize,
}

pub fn default_include() -> Vec<String> {
    [
        "winhttp.dll",
        "doorstop_config.ini",
        ".doorstop_version",
        "BepInEx/core/**",
        "BepInEx/plugins/**",
        "BepInEx/patchers/**",
        "BepInEx/config/**",
        // Shipped by BepInExPack_Valheim for Linux players; harmless on Windows.
        "doorstop_libs/**",
        "start_game_bepinex.sh",
        // Older packs shipped a corlib; 5.4.2350 does not, the glob then matches nothing.
        "unstripped_corlib/**",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

pub fn default_exclude() -> Vec<String> {
    [
        "BepInEx/LogOutput.log",
        "BepInEx/cache/**",
        "BepInEx/config/**/*.bak",
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
                "cannot read {}; run `valsync-server init` first or pass --config",
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
        if self.server.game_address.trim().is_empty()
            || self.server.game_address.chars().any(char::is_whitespace)
        {
            bail!("[server] game_address must be host:port without spaces");
        }
        if self.pack.server_root.is_none() && self.pack.client_extras.is_none() {
            bail!("[pack] needs at least server_root or client_extras");
        }
        if let Some(root) = &self.pack.server_root
            && !root.is_dir()
        {
            bail!("[pack] server_root {} is not a directory", root.display());
        }
        if let Some(extras) = &self.pack.client_extras
            && !extras.is_dir()
        {
            bail!(
                "[pack] client_extras {} is not a directory (create it, even empty)",
                extras.display()
            );
        }
        self.policy_rules()?;
        for root in &self.pack.managed_roots {
            valsync_core::path::validate(&format!("{root}/_"), &AllowedRoots::bepinex())
                .map_err(|e| anyhow::anyhow!("[pack] managed_roots {root:?}: {e}"))?;
        }
        Ok(())
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
        if let Some(extras) = &self.pack.client_extras {
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
    pub fn public_url(&self) -> String {
        if let Some(url) = &self.server.public_url {
            return url.trim_end_matches('/').to_string();
        }
        let port = self.bind_addr().map_or(DEFAULT_PORT, |a| a.port());
        let host =
            crate::net::lan_ip().map_or_else(|| "127.0.0.1".to_string(), |ip| ip.to_string());
        format!("http://{host}:{port}")
    }
}

/// Values `init` fills into the template.
#[derive(Debug, Clone)]
pub struct TemplateOptions {
    pub name: String,
    pub game_address: String,
    pub public_url: Option<String>,
    pub server_root: Option<PathBuf>,
    pub client_extras: PathBuf,
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

/// A commented configuration file. Written once by `init`, then edited by hand.
pub fn template(opts: &TemplateOptions) -> String {
    let server_root_line = match &opts.server_root {
        Some(p) => format!("server_root = {}", toml_str(&p.display().to_string())),
        None => "# server_root = 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\Valheim dedicated server'".to_string(),
    };
    let public_url_line = match &opts.public_url {
        Some(u) => format!("public_url = {}", toml_str(u)),
        None => "# public_url = \"http://your.public.address:2470\"".to_string(),
    };
    let defaults = Config::default();
    format!(
        r#"# ValSync server configuration.
# Edit, then run `valsync-server scan` to check the result and
# `valsync-server serve` to publish. `serve` reloads the pack automatically
# whenever a file changes; restart it after editing this file.

[server]
# Name shown to players in the launcher.
name = {name}
# TCP address to listen on. Open this port in the firewall / router.
bind = "0.0.0.0:{port}"
# URL players reach this server at. Goes into the invite code. If unset, the
# LAN address of this machine is used, which only works for LAN players.
{public_url_line}
# host:port the launcher hands to Valheim (the game server, UDP 2456).
game_address = {game_address}

[pack]
# Game root of the dedicated server. Everything matching `include` (minus
# `exclude`) is published. Leave it out for a hosted server: put a copy of the
# pack in `client_extras` instead.
{server_root_line}
include = [
{include}
]
exclude = [
{exclude}
  # Server-only mods, never sent to players. Add yours:
  # 'BepInEx/plugins/DiscordConnector/**',
]
# Client-only files (mods that must not run on the server, e.g. Unshamed,
# ConfigManager), laid out exactly like the game root:
#   client-extras/BepInEx/plugins/Unshamed/Unshamed.dll
# Files here override files of the same path from server_root.
client_extras = {client_extras}
# Folders ValSync owns on the player's side. Unknown .dll files found there
# (leftovers of other mods) are moved to BepInEx/_valsync_quarantine/.
managed_roots = [
{managed_roots}
]

[policy]
# "enforce": always replaced when different. "seed": installed only if absent,
# then never touched (player preferences).
default = "enforce"
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
"#,
        name = toml_str(&opts.name),
        port = DEFAULT_PORT,
        game_address = toml_str(&opts.game_address),
        include = toml_list(&defaults.pack.include, "  "),
        exclude = toml_list(&defaults.pack.exclude, "  "),
        client_extras = toml_str(&opts.client_extras.display().to_string()),
        managed_roots = toml_list(&defaults.pack.managed_roots, "  "),
        seed = toml_list(&defaults.policy.seed, "  "),
        enforce = toml_list(&defaults.policy.enforce, "  "),
        max_file_mb = defaults.limits.max_file_mb,
        max_pack_mb = defaults.limits.max_pack_mb,
        max_files = defaults.limits.max_files,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let extras = tmp.path().join("client-extras");
        std::fs::create_dir_all(&extras).unwrap();
        let text = template(&TemplateOptions {
            name: "Ztnzvoid' server".into(),
            game_address: "valheim.example.org:2456".into(),
            public_url: None,
            server_root: Some(tmp.path().to_path_buf()),
            client_extras: extras.clone(),
        });
        let cfg: Config = toml::from_str(&text).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.server.name, "Ztnzvoid' server");
        assert_eq!(cfg.pack.include, default_include());
        assert_eq!(cfg.pack.client_extras.as_deref(), Some(extras.as_path()));
        assert_eq!(cfg.sources().len(), 2);
        assert_eq!(cfg.limits(), Limits::default());
        assert!(cfg.public_url().starts_with("http://"));
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
