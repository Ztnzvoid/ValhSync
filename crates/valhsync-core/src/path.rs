//! Validation of manifest paths.
//!
//! This is the most security-critical code in the project. A manifest comes
//! from the network; every `path` in it ends up as a filesystem write on the
//! player's machine. The rules are strict and the same on both sides:
//!
//! - relative, `/`-separated, no `.` or `..`, no empty component
//! - no drive letter, no UNC prefix, no backslash
//! - no character that Windows refuses, no control character
//! - no component ending in a dot or space (Windows silently strips them)
//! - no reserved device name (`CON`, `NUL`, `COM1`, ...), even with an extension
//! - must sit under an explicitly allowed root

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// Maximum length of a whole relative path, in bytes.
pub const MAX_PATH_LEN: usize = 240;
/// Maximum length of a single component, in bytes.
pub const MAX_COMPONENT_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("path is empty")]
    Empty,
    #[error("absolute paths are not allowed")]
    Absolute,
    #[error("backslashes are not allowed, use '/'")]
    Backslash,
    #[error("drive letters and UNC paths are not allowed")]
    DriveOrUnc,
    #[error("empty path component (double slash or trailing slash)")]
    EmptyComponent,
    #[error("'.' and '..' components are not allowed")]
    DotComponent,
    #[error("forbidden character {0:?}")]
    ForbiddenChar(char),
    #[error("control characters are not allowed")]
    ControlChar,
    #[error("component ends with a dot or a space, which Windows silently strips")]
    TrailingDotOrSpace,
    #[error("{0:?} is a reserved device name on Windows")]
    ReservedName(String),
    #[error("path or component is too long")]
    TooLong,
    #[error("path is outside the allowed roots")]
    OutsideAllowedRoots,
}

const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Structural checks only; does not look at allowed roots.
pub fn check_syntax(path: &str) -> std::result::Result<(), PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > MAX_PATH_LEN {
        return Err(PathError::TooLong);
    }
    if path.contains('\\') {
        return Err(PathError::Backslash);
    }
    if path.starts_with('/') {
        return Err(PathError::Absolute);
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(PathError::DriveOrUnc);
    }
    for c in path.chars() {
        if c.is_control() {
            return Err(PathError::ControlChar);
        }
        if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
            return Err(PathError::ForbiddenChar(c));
        }
    }
    for comp in path.split('/') {
        if comp.is_empty() {
            return Err(PathError::EmptyComponent);
        }
        if comp == "." || comp == ".." {
            return Err(PathError::DotComponent);
        }
        if comp.len() > MAX_COMPONENT_LEN {
            return Err(PathError::TooLong);
        }
        if comp.ends_with('.') || comp.ends_with(' ') {
            return Err(PathError::TrailingDotOrSpace);
        }
        // Windows reserves the device names even with an extension: `CON.txt`.
        let stem = comp.split('.').next().unwrap_or(comp).trim();
        let upper = stem.to_uppercase();
        if RESERVED_NAMES.contains(&upper.as_str()) {
            return Err(PathError::ReservedName(comp.to_string()));
        }
    }
    Ok(())
}

/// The set of locations a manifest may write to, relative to the game root.
///
/// `files` are exact root-level file names (`winhttp.dll`). `dirs` are
/// directory prefixes under which anything is accepted (`BepInEx`). Matching is
/// case-sensitive on purpose: a manifest that spells `bepinex/` would create a
/// second directory on Linux, so it is rejected everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedRoots {
    files: BTreeSet<String>,
    dirs: BTreeSet<String>,
}

impl AllowedRoots {
    /// Nothing allowed. Useful as a starting point for tests and custom setups.
    pub fn empty() -> Self {
        Self {
            files: BTreeSet::new(),
            dirs: BTreeSet::new(),
        }
    }

    /// What BepInExPack_Valheim drops at the game root, and nothing else.
    /// `valheim.exe`, `valheim_Data/`, `UnityPlayer.dll`, `steam_api64.dll`
    /// are not here and can therefore never be touched.
    pub fn bepinex() -> Self {
        let mut roots = Self::empty();
        for f in [
            "winhttp.dll",
            "doorstop_config.ini",
            ".doorstop_version",
            "changelog.txt",
            "start_game_bepinex.sh",
            "start_server_bepinex.sh",
            "run_bepinex.sh",
        ] {
            roots.allow_file(f);
        }
        for d in ["BepInEx", "unstripped_corlib", "doorstop_libs"] {
            roots.allow_dir(d);
        }
        roots
    }

    pub fn allow_file(&mut self, name: &str) -> &mut Self {
        self.files.insert(name.to_string());
        self
    }

    pub fn allow_dir(&mut self, name: &str) -> &mut Self {
        self.dirs.insert(name.trim_end_matches('/').to_string());
        self
    }

    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(String::as_str)
    }

    pub fn dirs(&self) -> impl Iterator<Item = &str> {
        self.dirs.iter().map(String::as_str)
    }

    /// Does `path` (already syntax-checked) fall under an allowed root?
    pub fn allows(&self, path: &str) -> bool {
        if self.files.contains(path) {
            return true;
        }
        self.dirs.iter().any(|d| {
            path.len() > d.len() + 1
                && path.starts_with(d.as_str())
                && path.as_bytes()[d.len()] == b'/'
        })
    }
}

/// Full validation of a manifest path: syntax, then allowed roots.
pub fn validate(path: &str, roots: &AllowedRoots) -> std::result::Result<(), PathError> {
    check_syntax(path)?;
    if roots.allows(path) {
        Ok(())
    } else {
        Err(PathError::OutsideAllowedRoots)
    }
}

/// Same as [`validate`], wrapped in a [`CoreError`] carrying the path.
pub fn validate_or_err(path: &str, roots: &AllowedRoots) -> Result<()> {
    validate(path, roots).map_err(|reason| CoreError::InvalidPath {
        path: path.to_string(),
        reason,
    })
}

/// Join a validated relative path onto an OS root, component by component.
pub fn to_os_path(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for comp in rel.split('/') {
        out.push(comp);
    }
    out
}

/// Resolve `rel` under `root` while refusing to cross any symbolic link or
/// junction. Components that do not exist yet are fine: nothing can be
/// followed through them. Returns the full OS path.
pub fn ensure_within_root(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in rel.split('/') {
        cur.push(comp);
        match std::fs::symlink_metadata(&cur) {
            Ok(md) => {
                if md.file_type().is_symlink() {
                    return Err(CoreError::SymlinkRefused(cur));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => return Err(CoreError::io(cur, e)),
        }
    }
    Ok(to_os_path(root, rel))
}

/// Turn an OS path under `root` back into a manifest-style relative path.
/// Returns `None` if the path is not under `root` or contains non-UTF-8 names.
pub fn to_rel_path(root: &Path, full: &Path) -> Option<String> {
    let rel = full.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for comp in rel.components() {
        match comp {
            std::path::Component::Normal(s) => parts.push(s.to_str()?),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> AllowedRoots {
        AllowedRoots::bepinex()
    }

    #[test]
    fn accepts_legitimate_paths() {
        for p in [
            "winhttp.dll",
            "doorstop_config.ini",
            ".doorstop_version",
            "BepInEx/core/BepInEx.dll",
            "BepInEx/plugins/AzuCraftyBoxes/AzuCraftyBoxes.dll",
            "BepInEx/config/Azumatt.AzuCraftyBoxes.cfg",
            "BepInEx/plugins/Mod With Spaces/mod.dll",
            "BepInEx/plugins/Éclair/données.json",
            "unstripped_corlib/mscorlib.dll",
        ] {
            assert_eq!(validate(p, &roots()), Ok(()), "{p}");
        }
    }

    #[test]
    fn rejects_malicious_paths() {
        let cases: &[(&str, PathError)] = &[
            ("", PathError::Empty),
            ("../evil.dll", PathError::DotComponent),
            ("BepInEx/../../evil.dll", PathError::DotComponent),
            ("BepInEx/./x.dll", PathError::DotComponent),
            ("..\\evil.dll", PathError::Backslash),
            ("BepInEx\\plugins\\x.dll", PathError::Backslash),
            ("/etc/passwd", PathError::Absolute),
            ("//server/share/x.dll", PathError::Absolute),
            ("C:/Windows/x.dll", PathError::DriveOrUnc),
            ("c:x.dll", PathError::DriveOrUnc),
            ("BepInEx//x.dll", PathError::EmptyComponent),
            ("BepInEx/plugins/", PathError::EmptyComponent),
            ("BepInEx/plugins/CON", PathError::ReservedName("CON".into())),
            (
                "BepInEx/plugins/con.dll",
                PathError::ReservedName("con.dll".into()),
            ),
            (
                "BepInEx/plugins/NUL.txt",
                PathError::ReservedName("NUL.txt".into()),
            ),
            (
                "BepInEx/plugins/lpt1",
                PathError::ReservedName("lpt1".into()),
            ),
            ("BepInEx/plugins/a.dll ", PathError::TrailingDotOrSpace),
            ("BepInEx/plugins/a.", PathError::TrailingDotOrSpace),
            ("BepInEx/plugins/a<b.dll", PathError::ForbiddenChar('<')),
            ("BepInEx/plugins/a|b.dll", PathError::ForbiddenChar('|')),
            ("BepInEx/plugins/a?.dll", PathError::ForbiddenChar('?')),
            ("BepInEx/plugins/a*.dll", PathError::ForbiddenChar('*')),
            ("BepInEx/plugins/a\"b.dll", PathError::ForbiddenChar('"')),
            ("BepInEx/plugins/a\0b.dll", PathError::ControlChar),
            ("BepInEx/plugins/a\nb.dll", PathError::ControlChar),
        ];
        for (p, expected) in cases {
            assert_eq!(validate(p, &roots()), Err(expected.clone()), "{p:?}");
        }
    }

    #[test]
    fn rejects_paths_outside_allowed_roots() {
        for p in [
            "valheim.exe",
            "UnityPlayer.dll",
            "steam_api64.dll",
            "valheim_Data/Managed/assembly_valheim.dll",
            "MonoBleedingEdge/x.dll",
            "bepinex/plugins/x.dll",
            "BepInEx",
            "BepInExx/x.dll",
            "winhttp.dll/x",
        ] {
            assert_eq!(
                validate(p, &roots()),
                Err(PathError::OutsideAllowedRoots),
                "{p}"
            );
        }
    }

    #[test]
    fn rejects_overlong_paths() {
        let long = format!("BepInEx/plugins/{}", "a".repeat(MAX_PATH_LEN));
        assert_eq!(check_syntax(&long), Err(PathError::TooLong));
        let long_comp = format!("BepInEx/{}/x", "b".repeat(MAX_COMPONENT_LEN + 1));
        assert_eq!(check_syntax(&long_comp), Err(PathError::TooLong));
    }

    #[test]
    fn extra_roots_can_be_added() {
        let mut r = AllowedRoots::empty();
        r.allow_dir("Custom/");
        assert!(r.allows("Custom/x.txt"));
        assert!(!r.allows("Custom"));
        assert!(!r.allows("BepInEx/x.dll"));
    }

    #[test]
    fn os_path_roundtrip() {
        let root = Path::new("root");
        let os = to_os_path(root, "BepInEx/plugins/x.dll");
        assert_eq!(
            to_rel_path(root, &os).as_deref(),
            Some("BepInEx/plugins/x.dll")
        );
        assert_eq!(to_rel_path(root, Path::new("elsewhere/x")), None);
    }

    #[test]
    fn ensure_within_root_allows_missing_components() {
        let tmp = tempfile::tempdir().unwrap();
        let p = ensure_within_root(tmp.path(), "BepInEx/plugins/new.dll").unwrap();
        assert!(p.starts_with(tmp.path()));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_within_root_refuses_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx")).unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("BepInEx/plugins")).unwrap();
        let err = ensure_within_root(tmp.path(), "BepInEx/plugins/x.dll").unwrap_err();
        assert!(matches!(err, CoreError::SymlinkRefused(_)));
    }

    #[cfg(windows)]
    #[test]
    fn ensure_within_root_refuses_junctions_when_creatable() {
        // Directory symlinks need a privilege on Windows; junctions do not.
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("BepInEx")).unwrap();
        let link = tmp.path().join("BepInEx").join("plugins");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output();
        if !status.is_ok_and(|o| o.status.success()) {
            eprintln!("mklink /J unavailable, skipping");
            return;
        }
        let err = ensure_within_root(tmp.path(), "BepInEx/plugins/x.dll").unwrap_err();
        assert!(matches!(err, CoreError::SymlinkRefused(_)), "{err}");
    }
}
