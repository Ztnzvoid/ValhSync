//! Building a pack from the admin's folders.
//!
//! A pack is: the server's game root, filtered by include/exclude globs,
//! **plus** a client-extras folder laid out like the game root. Later sources
//! override earlier ones, so extras can replace a server-side file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::error::{CoreError, Result};
use crate::hash;
use crate::limits::{Limits, human_bytes};
use crate::manifest::{FileEntry, Policy};
use crate::path::{self, AllowedRoots};

/// One folder to pull files from.
#[derive(Debug, Clone)]
pub struct ScanSource {
    pub root: PathBuf,
    /// Globs relative to `root`. Empty means everything.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// Which policy each file gets. `enforce` patterns win over `seed` patterns,
/// which win over the default.
#[derive(Debug, Clone)]
pub struct PolicyRules {
    pub default: Policy,
    pub seed: Vec<String>,
    pub enforce: Vec<String>,
}

impl Default for PolicyRules {
    fn default() -> Self {
        Self {
            default: Policy::Enforce,
            seed: vec!["BepInEx/config/**".into()],
            enforce: vec!["BepInEx/config/BepInEx.cfg".into()],
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub sources: Vec<ScanSource>,
    pub policy: PolicyRules,
    pub limits: Limits,
    pub roots: AllowedRoots,
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub entry: FileEntry,
    /// Where the bytes live on the admin's disk.
    pub source: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
    pub total_bytes: u64,
    /// Files that matched the globs but could not be packaged, with the reason.
    pub skipped: Vec<(String, String)>,
}

impl ScanResult {
    pub fn entries(&self) -> Vec<FileEntry> {
        self.files.iter().map(|f| f.entry.clone()).collect()
    }
}

/// Compile globs so `*` never crosses a `/` and matching ignores case
/// (Windows paths do).
pub fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p).map_err(|source| CoreError::Glob {
            pattern: p.clone(),
            source,
        })?;
        let mut gb = globset::GlobBuilder::new(glob.glob());
        gb.literal_separator(true).case_insensitive(true);
        let glob = gb.build().map_err(|source| CoreError::Glob {
            pattern: p.clone(),
            source,
        })?;
        builder.add(glob);
    }
    builder.build().map_err(|source| CoreError::Glob {
        pattern: patterns.join(", "),
        source,
    })
}

struct CompiledPolicy {
    default: Policy,
    seed: GlobSet,
    enforce: GlobSet,
}

impl CompiledPolicy {
    fn new(rules: &PolicyRules) -> Result<Self> {
        Ok(Self {
            default: rules.default,
            seed: build_globset(&rules.seed)?,
            enforce: build_globset(&rules.enforce)?,
        })
    }

    fn policy_for(&self, rel: &str) -> Policy {
        if self.enforce.is_match(rel) {
            Policy::Enforce
        } else if self.seed.is_match(rel) {
            Policy::Seed
        } else {
            self.default
        }
    }
}

/// Walk every source, hash what matches, apply policies and limits.
pub fn scan(cfg: &ScanConfig) -> Result<ScanResult> {
    let policy = CompiledPolicy::new(&cfg.policy)?;
    let mut files: BTreeMap<String, ScannedFile> = BTreeMap::new();
    let mut skipped = Vec::new();

    for src in &cfg.sources {
        if !src.root.is_dir() {
            return Err(CoreError::io(
                &src.root,
                std::io::Error::new(std::io::ErrorKind::NotFound, "source folder does not exist"),
            ));
        }
        let include = build_globset(&src.include)?;
        let exclude = build_globset(&src.exclude)?;

        for entry in WalkDir::new(&src.root).follow_links(false).min_depth(1) {
            let entry = entry.map_err(|e| {
                let p = e.path().map_or_else(|| src.root.clone(), Path::to_path_buf);
                CoreError::io(p, e.into())
            })?;
            let Some(rel) = path::to_rel_path(&src.root, entry.path()) else {
                skipped.push((
                    entry.path().display().to_string(),
                    "name is not valid UTF-8".into(),
                ));
                continue;
            };
            if entry.file_type().is_symlink() {
                skipped.push((rel, "symbolic links are never packaged".into()));
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            if !include.is_empty() && !include.is_match(&rel) {
                continue;
            }
            if exclude.is_match(&rel) {
                continue;
            }
            if let Err(reason) = path::validate(&rel, &cfg.roots) {
                skipped.push((rel, reason.to_string()));
                continue;
            }
            let size = entry
                .metadata()
                .map_err(|e| CoreError::io(entry.path(), e.into()))?
                .len();
            if size > cfg.limits.max_file_bytes {
                return Err(CoreError::LimitExceeded(format!(
                    "{rel} is {}, maximum per file is {} (see [limits] in the config)",
                    human_bytes(size),
                    human_bytes(cfg.limits.max_file_bytes)
                )));
            }
            let digest = hash::to_hex(&hash::hash_file(entry.path())?);
            let scanned = ScannedFile {
                entry: FileEntry {
                    policy: policy.policy_for(&rel),
                    path: rel.clone(),
                    size,
                    blake3: digest,
                },
                source: entry.path().to_path_buf(),
            };
            // Later sources override earlier ones (extras beat server files).
            files.insert(rel.to_lowercase(), scanned);
        }
    }

    if files.len() > cfg.limits.max_files {
        return Err(CoreError::LimitExceeded(format!(
            "pack has {} files, maximum is {}",
            files.len(),
            cfg.limits.max_files
        )));
    }
    let total_bytes: u64 = files.values().map(|f| f.entry.size).sum();
    if total_bytes > cfg.limits.max_pack_bytes {
        return Err(CoreError::LimitExceeded(format!(
            "pack is {}, maximum is {}",
            human_bytes(total_bytes),
            human_bytes(cfg.limits.max_pack_bytes)
        )));
    }

    Ok(ScanResult {
        files: files.into_values().collect(),
        total_bytes,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let p = path::to_os_path(root, rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn default_include() -> Vec<String> {
        [
            "winhttp.dll",
            "doorstop_config.ini",
            "BepInEx/core/**",
            "BepInEx/plugins/**",
            "BepInEx/patchers/**",
            "BepInEx/config/**",
        ]
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    #[test]
    fn scans_filters_and_assigns_policies() {
        let server = tempfile::tempdir().unwrap();
        let extras = tempfile::tempdir().unwrap();
        let s = server.path();
        write(s, "winhttp.dll", b"doorstop");
        write(s, "doorstop_config.ini", b"ini");
        write(s, "valheim_server.exe", b"game");
        write(s, "BepInEx/core/BepInEx.dll", b"core");
        write(s, "BepInEx/plugins/Azu/Azu.dll", b"azu");
        write(s, "BepInEx/plugins/DiscordConnector/DC.dll", b"server only");
        write(s, "BepInEx/config/BepInEx.cfg", b"bep");
        write(s, "BepInEx/config/Azu.cfg", b"azucfg");
        write(s, "BepInEx/LogOutput.log", b"log");
        write(s, "BepInEx/cache/x.bin", b"cache");
        write(
            extras.path(),
            "BepInEx/plugins/Unshamed/Unshamed.dll",
            b"client only",
        );
        write(
            extras.path(),
            "BepInEx/config/Azu.cfg",
            b"overridden by extras",
        );

        let cfg = ScanConfig {
            sources: vec![
                ScanSource {
                    root: s.to_path_buf(),
                    include: default_include(),
                    exclude: vec!["BepInEx/plugins/DiscordConnector/**".into()],
                },
                ScanSource {
                    root: extras.path().to_path_buf(),
                    include: vec![],
                    exclude: vec![],
                },
            ],
            policy: PolicyRules::default(),
            limits: Limits::default(),
            roots: AllowedRoots::bepinex(),
        };
        let result = scan(&cfg).unwrap();
        let paths: Vec<&str> = result.files.iter().map(|f| f.entry.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "BepInEx/config/Azu.cfg",
                "BepInEx/config/BepInEx.cfg",
                "BepInEx/core/BepInEx.dll",
                "BepInEx/plugins/Azu/Azu.dll",
                "BepInEx/plugins/Unshamed/Unshamed.dll",
                "doorstop_config.ini",
                "winhttp.dll",
            ]
        );
        let by: BTreeMap<&str, &ScannedFile> = result
            .files
            .iter()
            .map(|f| (f.entry.path.as_str(), f))
            .collect();
        assert_eq!(by["BepInEx/config/Azu.cfg"].entry.policy, Policy::Seed);
        assert_eq!(
            by["BepInEx/config/BepInEx.cfg"].entry.policy,
            Policy::Enforce
        );
        assert_eq!(by["winhttp.dll"].entry.policy, Policy::Enforce);
        assert_eq!(
            by["BepInEx/config/Azu.cfg"].entry.size,
            b"overridden by extras".len() as u64
        );
        assert!(
            by["BepInEx/config/Azu.cfg"]
                .source
                .starts_with(extras.path())
        );
        assert_eq!(
            result.total_bytes,
            result.files.iter().map(|f| f.entry.size).sum::<u64>()
        );
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn disallowed_paths_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "BepInEx/plugins/ok.dll", b"ok");
        write(dir.path(), "valheim_server.exe", b"never packaged");
        let cfg = ScanConfig {
            sources: vec![ScanSource {
                root: dir.path().to_path_buf(),
                include: vec![],
                exclude: vec![],
            }],
            policy: PolicyRules::default(),
            limits: Limits::default(),
            roots: AllowedRoots::bepinex(),
        };
        let result = scan(&cfg).unwrap();
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].0, "valheim_server.exe");
        assert!(result.skipped[0].1.contains("outside"));
    }

    #[test]
    fn limits_stop_the_scan() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "BepInEx/plugins/a.dll", b"aaaa");
        write(dir.path(), "BepInEx/plugins/b.dll", b"bbbb");
        let mut cfg = ScanConfig {
            sources: vec![ScanSource {
                root: dir.path().to_path_buf(),
                include: vec![],
                exclude: vec![],
            }],
            policy: PolicyRules::default(),
            limits: Limits {
                max_file_bytes: 3,
                ..Limits::default()
            },
            roots: AllowedRoots::bepinex(),
        };
        assert!(matches!(scan(&cfg), Err(CoreError::LimitExceeded(_))));
        cfg.limits = Limits {
            max_files: 1,
            ..Limits::default()
        };
        assert!(matches!(scan(&cfg), Err(CoreError::LimitExceeded(_))));
        cfg.limits = Limits {
            max_pack_bytes: 7,
            ..Limits::default()
        };
        assert!(matches!(scan(&cfg), Err(CoreError::LimitExceeded(_))));
    }

    #[test]
    fn missing_source_is_an_error_and_bad_glob_too() {
        let cfg = ScanConfig {
            sources: vec![ScanSource {
                root: PathBuf::from("/definitely/not/here"),
                include: vec![],
                exclude: vec![],
            }],
            policy: PolicyRules::default(),
            limits: Limits::default(),
            roots: AllowedRoots::bepinex(),
        };
        assert!(matches!(scan(&cfg), Err(CoreError::Io { .. })));
        assert!(matches!(
            build_globset(&["[".into()]),
            Err(CoreError::Glob { .. })
        ));
    }
}
