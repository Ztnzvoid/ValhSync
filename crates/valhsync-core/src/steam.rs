//! Finding Steam libraries and installed apps without any Steam dependency.
//!
//! Shared by the server (`Valheim Dedicated Server`, app 896660) and the
//! launcher (`Valheim`, app 892970). The Windows registry lookup for the Steam
//! root lives in the launcher crate; this module only walks the filesystem and
//! parses Valve's KeyValues text format just enough for `libraryfolders.vdf`
//! and `appmanifest_*.acf`.

use std::path::{Path, PathBuf};

pub const VALHEIM_APP_ID: u32 = 892_970;
pub const VALHEIM_SERVER_APP_ID: u32 = 896_660;

/// A parsed KeyValues node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kv {
    Str(String),
    Map(Vec<(String, Kv)>),
}

impl Kv {
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Kv::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Kv::Str(s) => Some(s),
            Kv::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Kv)] {
        match self {
            Kv::Map(e) => e,
            Kv::Str(_) => &[],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => break,
                        },
                        other => s.push(other),
                    }
                }
                tokens.push(Token::Str(s));
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            other => {
                let mut s = String::from(other);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                tokens.push(Token::Str(s));
            }
        }
    }
    tokens
}

fn parse_map(tokens: &[Token], pos: &mut usize, depth: usize) -> Option<Kv> {
    if depth > 64 {
        return None;
    }
    let mut entries = Vec::new();
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Close => {
                *pos += 1;
                return Some(Kv::Map(entries));
            }
            Token::Open => return None,
            Token::Str(key) => {
                *pos += 1;
                match tokens.get(*pos)? {
                    Token::Str(v) => {
                        entries.push((key.clone(), Kv::Str(v.clone())));
                        *pos += 1;
                    }
                    Token::Open => {
                        *pos += 1;
                        let child = parse_map(tokens, pos, depth + 1)?;
                        entries.push((key.clone(), child));
                    }
                    Token::Close => return None,
                }
            }
        }
    }
    Some(Kv::Map(entries))
}

/// Parse a whole KeyValues document. The root is a map of its top-level keys.
pub fn parse_kv(text: &str) -> Option<Kv> {
    let tokens = tokenize(text);
    let mut pos = 0;
    let root = parse_map(&tokens, &mut pos, 0)?;
    if pos == tokens.len() {
        Some(root)
    } else {
        None
    }
}

/// Library folders listed in `steamapps/libraryfolders.vdf`, both the modern
/// (`"0" { "path" "..." }`) and the legacy (`"1" "C:\\..."`) layouts.
pub fn library_paths(vdf_text: &str) -> Vec<PathBuf> {
    let Some(root) = parse_kv(vdf_text) else {
        return Vec::new();
    };
    let folders = root.get("libraryfolders").unwrap_or(&root);
    let mut out = Vec::new();
    for (key, value) in folders.entries() {
        if !key.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let path = match value {
            Kv::Str(s) => Some(s.as_str()),
            Kv::Map(_) => value.get("path").and_then(Kv::as_str),
        };
        if let Some(p) = path {
            out.push(PathBuf::from(p));
        }
    }
    out
}

/// `installdir` from an `appmanifest_<id>.acf`.
pub fn acf_installdir(acf_text: &str) -> Option<String> {
    let root = parse_kv(acf_text)?;
    let state = root.get("AppState").unwrap_or(&root);
    state.get("installdir")?.as_str().map(str::to_string)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Where Steam usually lives, per platform. The launcher prepends the
/// registry value on Windows; these are the fallbacks.
pub fn default_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(pf) = std::env::var_os(var) {
                roots.push(PathBuf::from(pf).join("Steam"));
            }
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    } else if let Some(home) = home_dir() {
        if cfg!(target_os = "macos") {
            roots.push(home.join("Library/Application Support/Steam"));
        } else {
            roots.push(home.join(".steam/steam"));
            roots.push(home.join(".local/share/Steam"));
            roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
            roots.push(home.join(".steam/root"));
        }
    }
    roots.dedup();
    roots
}

/// All libraries known to a Steam root: the root itself plus what
/// `libraryfolders.vdf` lists.
pub fn libraries_of(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&vdf) {
        for p in library_paths(&text) {
            if !libs.iter().any(|l| l == &p) {
                libs.push(p);
            }
        }
    }
    libs
}

/// Install folder of `app_id` in any library of `steam_root`, if present.
pub fn find_app_in(steam_root: &Path, app_id: u32) -> Option<PathBuf> {
    for lib in libraries_of(steam_root) {
        let steamapps = lib.join("steamapps");
        let acf = steamapps.join(format!("appmanifest_{app_id}.acf"));
        let Ok(text) = std::fs::read_to_string(&acf) else {
            continue;
        };
        let Some(dir) = acf_installdir(&text) else {
            continue;
        };
        let install = steamapps.join("common").join(dir);
        if install.is_dir() {
            return Some(install);
        }
    }
    None
}

/// Look for `app_id` under `steam_roots` first, then the platform defaults.
pub fn find_app(app_id: u32, steam_roots: &[PathBuf]) -> Option<PathBuf> {
    steam_roots
        .iter()
        .cloned()
        .chain(default_steam_roots())
        .filter(|r| r.is_dir())
        .find_map(|r| find_app_in(&r, app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"512"
		}
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"
		"apps"
		{
			"892970"		"1234"
		}
	}
}
"#;

    const LEGACY: &str = r#"
"LibraryFolders"
{
	"TimeNextStatsReport"		"1"
	"ContentStatsID"		"2"
	"1"		"D:\\Games\\Steam"
}
"#;

    const ACF: &str = r#"
"AppState"
{
	"appid"		"892970"
	"name"		"Valheim"
	"installdir"		"Valheim"
	"buildid"		"1"
}
"#;

    #[test]
    fn parses_modern_and_legacy_library_files() {
        assert_eq!(
            library_paths(MODERN),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"E:\SteamLibrary")
            ]
        );
        assert_eq!(
            library_paths(LEGACY),
            vec![PathBuf::from(r"D:\Games\Steam")]
        );
        assert!(library_paths("garbage { { {").is_empty());
    }

    #[test]
    fn parses_acf() {
        assert_eq!(acf_installdir(ACF).as_deref(), Some("Valheim"));
        assert_eq!(acf_installdir("\"AppState\" { }"), None);
    }

    #[test]
    fn unquoted_tokens_and_comments() {
        let kv = parse_kv("// comment\nroot { key value  other \"quoted v\" }").unwrap();
        let root = kv.get("root").unwrap();
        assert_eq!(root.get("key").and_then(Kv::as_str), Some("value"));
        assert_eq!(root.get("other").and_then(Kv::as_str), Some("quoted v"));
    }

    #[test]
    fn finds_app_through_library_folders() {
        let steam = tempfile::tempdir().unwrap();
        let lib = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(steam.path().join("steamapps")).unwrap();
        let lib_path = lib.path().display().to_string().replace('\\', "\\\\");
        std::fs::write(
            steam.path().join("steamapps/libraryfolders.vdf"),
            format!("\"libraryfolders\" {{ \"0\" {{ \"path\" \"{lib_path}\" }} }}"),
        )
        .unwrap();
        std::fs::create_dir_all(lib.path().join("steamapps/common/Valheim")).unwrap();
        std::fs::write(lib.path().join("steamapps/appmanifest_892970.acf"), ACF).unwrap();

        let found = find_app_in(steam.path(), VALHEIM_APP_ID).unwrap();
        assert_eq!(found, lib.path().join("steamapps/common/Valheim"));
        assert_eq!(find_app_in(steam.path(), VALHEIM_SERVER_APP_ID), None);
    }
}
