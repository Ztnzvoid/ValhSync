//! Putting a mod the admin dropped on the window where the server loads it.
//!
//! The manual version of this is: download a Thunderstore zip, work out
//! whether its files go at the root or under `plugins/`, make a folder with
//! the right name, remember to delete the old version's files first, and not
//! typo the path. Every one of those is a way to end up with two versions of
//! a mod loaded at once, which BepInEx resolves by refusing both.
//!
//! What arrives here is untrusted in the ordinary sense -- an archive from the
//! internet -- so every entry is checked before it is written, and nothing
//! lands outside the plugins folder whatever the archive says its paths are.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

/// What we can actually decompress. Stored and deflate are all a mod archive
/// has ever needed; deflate64 is what 7-Zip writes for a large one. Growing
/// this list means growing the `zip` features in the workspace manifest, and
/// only with decoders that both exist in pure Rust and actually work -- see
/// the note there about LZMA.
const READS: [zip::CompressionMethod; 3] = [
    zip::CompressionMethod::Stored,
    zip::CompressionMethod::Deflated,
    zip::CompressionMethod::Deflate64,
];

/// No mod is anywhere near this large; the ceiling is what stops an archive
/// that unpacks to a terabyte from filling the disk before anyone notices.
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
/// Likewise for entry count: BepInEx mods are dozens of files, not thousands.
const MAX_ENTRIES: usize = 4000;

/// A file to write: its path relative to the mod's folder, and its bytes.
type Files = Vec<(String, Vec<u8>)>;

/// What was installed, for the window to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Folder it went into, under `BepInEx/plugins`.
    pub name: String,
    pub files: usize,
    /// A version of it was already there and has been replaced.
    pub replaced: bool,
}

/// Where a mod goes when the admin turns it off.
///
/// Outside `BepInEx`, deliberately. BepInEx loads every `.dll` it finds under
/// `plugins`, recursively, so renaming the folder does not stop it; and
/// anything left inside `BepInEx` would still be picked up by the pack and
/// sent to every player. A sibling folder is the one place that is neither
/// loaded nor published.
pub const DISABLED_DIR: &str = "valhsync-disabled";

/// Where a removed mod goes.
///
/// Not `remove_dir_all`. The launcher has never deleted anything on a
/// player's machine -- what it does not recognise is moved aside -- and the
/// admin's side should not be harsher than the players'. An admin who removes
/// the wrong mod at eleven at night can walk it back out of this folder; one
/// who really wants it gone empties it themselves.
pub const REMOVED_DIR: &str = "valhsync-removed";

/// A mod that is installed but turned off.
#[must_use]
pub fn disabled(server_root: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(server_root.join(DISABLED_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    out.sort();
    out
}

/// Turn a mod off: move it out of `plugins`, keeping it installed.
///
/// `loose` marks a plugin that is a single `.dll` sitting directly in
/// `plugins` rather than a folder of its own.
pub fn disable(server_root: &Path, name: &str, loose: bool) -> Result<PathBuf> {
    let name = sane_folder(name)?;
    let from = plugin_path(server_root, &name, loose);
    let into = server_root.join(DISABLED_DIR);
    move_aside(&from, &into, &name)
}

/// Turn it back on.
pub fn enable(server_root: &Path, name: &str) -> Result<PathBuf> {
    let name = sane_folder(name)?;
    let from = server_root.join(DISABLED_DIR).join(&name);
    if !from.exists() {
        bail!("{name} is not in {DISABLED_DIR}");
    }
    let plugins = server_root.join("BepInEx").join("plugins");
    let to = plugins.join(&name);
    if to.exists() {
        bail!("{name} is already in the plugins folder; remove one of the two first");
    }
    fs::create_dir_all(&plugins).with_context(|| format!("cannot create {}", plugins.display()))?;
    rename_or_copy(&from, &to)?;
    Ok(to)
}

/// Take a mod out of the server, from wherever it currently sits.
///
/// Returns where it went, so the window can say it rather than leaving an
/// admin wondering whether a click deleted something for good.
pub fn remove(server_root: &Path, name: &str, loose: bool) -> Result<PathBuf> {
    let name = sane_folder(name)?;
    let active = plugin_path(server_root, &name, loose);
    let from = if active.exists() {
        active
    } else {
        server_root.join(DISABLED_DIR).join(&name)
    };
    let into = server_root.join(REMOVED_DIR);
    move_aside(&from, &into, &name)
}

/// A plugin under `plugins`: a folder of its own, or the single `.dll` a
/// loose plugin is. The name already carries the extension in that case, so
/// the two are the same join -- `loose` is kept in the signature because the
/// caller has it and a future difference belongs here, not at every call
/// site.
fn plugin_path(server_root: &Path, name: &str, _loose: bool) -> PathBuf {
    server_root.join("BepInEx").join("plugins").join(name)
}

/// Move one mod into a folder ValhSync owns, without ever overwriting what is
/// already there: a second copy gets the date on the end, because the first
/// one is somebody's way back.
fn move_aside(from: &Path, into: &Path, name: &str) -> Result<PathBuf> {
    if !from.exists() {
        bail!("{} is not there", from.display());
    }
    fs::create_dir_all(into).with_context(|| format!("cannot create {}", into.display()))?;
    let mut to = into.join(name);
    if to.exists() {
        let stamp = valhsync_core::clock::dir_stamp();
        to = into.join(format!("{name}-{stamp}"));
    }
    rename_or_copy(from, &to)?;
    Ok(to)
}

/// Rename, falling back to a copy when the two are on different volumes --
/// which they are whenever the server folder is a junction or a mounted
/// drive, and a rename across those fails with a bare "os error 17".
fn rename_or_copy(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_tree(from, to)?;
        fs::remove_dir_all(from)
            .with_context(|| format!("copied, but cannot remove {}", from.display()))?;
    } else {
        fs::copy(from, to)
            .with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))?;
        fs::remove_file(from)
            .with_context(|| format!("copied, but cannot remove {}", from.display()))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("cannot create {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("cannot read {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .with_context(|| format!("cannot copy to {}", target.display()))?;
        }
    }
    Ok(())
}

/// Install a dropped mod into the server's plugins folder.
///
/// Accepts a Thunderstore-style zip, a folder laid out like one, or a single
/// `.dll`. Returns what it did, or why it would not.
pub fn install(server_root: &Path, source: &Path) -> Result<Installed> {
    let plugins = server_root.join("BepInEx").join("plugins");
    if !plugins.is_dir() {
        bail!(
            "{} does not exist. Is {} really the dedicated server's folder, with BepInEx installed?",
            plugins.display(),
            server_root.display()
        );
    }
    let meta = fs::metadata(source).with_context(|| format!("cannot read {}", source.display()))?;

    if meta.is_dir() {
        let name = folder_name(source)?;
        return place(&plugins, &name, &collect_folder(source)?);
    }
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "zip" => {
            let name = folder_name(source)?;
            let (name, files) = read_zip(source, &name)?;
            place(&plugins, &name, &files)
        }
        "dll" => {
            // On its own rather than loose among the others: a mod in its own
            // folder can be replaced or removed as one thing later, and
            // BepInEx looks in subfolders anyway.
            let name = folder_name(source)?;
            let file = source
                .file_name()
                .context("that file has no name")?
                .to_string_lossy()
                .to_string();
            let bytes = fs::read(source)?;
            place(&plugins, &name, &[(file, bytes)])
        }
        // Named rather than shrugged at: somebody who dropped a .7z wants
        // to be told to extract it, not that their file is not a mod.
        "7z" | "rar" | "tar" | "gz" => bail!(
            "ValhSync reads .zip archives, not .{ext}. Extract {} first, then drop the folder.",
            source.display()
        ),
        _ => bail!(
            "{} is not a mod. Drop a .zip from anywhere, a mod folder, or a .dll",
            source.display()
        ),
    }
}

/// The folder a dropped thing should become, from its own name.
fn folder_name(source: &Path) -> Result<String> {
    let stem = source
        .file_stem()
        .context("that path has no name")?
        .to_string_lossy()
        .to_string();
    sane_folder(&stem)
}

/// A single path component, safe to create, recognisable to a human.
fn sane_folder(raw: &str) -> Result<String> {
    let name = raw.trim().trim_matches('.').trim();
    if name.is_empty() || name.len() > 100 {
        bail!("{raw:?} is not a usable folder name");
    }
    if name
        .chars()
        .any(|c| valhsync_core::manifest::is_deceptive(c) || "/\\:*?\"<>|".contains(c))
    {
        bail!("{raw:?} holds characters a folder name cannot");
    }
    Ok(name.to_string())
}

/// Everything under a folder, as (relative path, bytes).
fn collect_folder(root: &Path) -> Result<Files> {
    let mut out = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            // Not followed: a link here would copy from anywhere on the disk.
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            total += meta.len();
            if total > MAX_UNPACKED_BYTES || out.len() >= MAX_ENTRIES {
                bail!("that folder is far larger than a mod; refusing it");
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, fs::read(&path)?));
        }
    }
    if out.is_empty() {
        bail!("there is nothing in that folder");
    }
    Ok(out)
}

/// Read a mod archive, returning the folder to use and the files to write.
///
/// Thunderstore packages put the plugin at the root beside a `manifest.json`,
/// or under `plugins/`; some authors ship a whole `BepInEx/plugins/...` tree
/// instead. All three are unwrapped to the same thing: the files that belong
/// in one mod folder.
fn read_zip(source: &Path, fallback_name: &str) -> Result<(String, Files)> {
    let file = fs::File::open(source)?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("{} is not a readable zip", source.display()))?;
    if zip.len() > MAX_ENTRIES {
        bail!("that archive holds {} entries; refusing it", zip.len());
    }

    let mut entries: Files = Vec::new();
    let mut manifest_name = None;
    let mut total = 0u64;

    for i in 0..zip.len() {
        // What the archive is compressed with, read before asking for the
        // decompressed bytes. The crate refuses a method it lacks with
        // "unsupported compression method" and nothing more, which leaves an
        // admin with nothing to act on.
        {
            let raw = zip.by_index_raw(i)?;
            let method = raw.compression();
            if !raw.is_dir() && !READS.contains(&method) {
                bail!(
                    "{} in that archive is compressed with {}, which ValhSync cannot read. Extract the archive yourself, then drop the folder instead",
                    raw.name(),
                    method_name(method)
                );
            }
        }
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        // `enclosed_name` is the crate's own refusal of anything absolute or
        // climbing out; ours below refuses the rest.
        let Some(path) = entry.enclosed_name() else {
            bail!("that archive has an entry whose path escapes it; refusing it");
        };
        let rel = safe_relative(&path)?;
        total += entry.size();
        if total > MAX_UNPACKED_BYTES {
            bail!("that archive unpacks to more than a mod ever should; refusing it");
        }
        let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
        entry
            .read_to_end(&mut bytes)
            .with_context(|| format!("cannot read {rel} out of that archive"))?;
        if rel == "manifest.json" {
            manifest_name = thunderstore_name(&bytes);
        }
        entries.push((rel, bytes));
    }
    if entries.is_empty() {
        bail!("that archive is empty");
    }

    let files = unwrap_layout(entries);
    let name = match manifest_name {
        Some(n) => sane_folder(&n)?,
        None => fallback_name.to_string(),
    };
    Ok((name, files))
}

/// One relative path, with every component checked.
fn safe_relative(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part == ".." || part.chars().any(valhsync_core::manifest::is_deceptive) {
                    bail!("that archive has an entry named {part:?}; refusing it");
                }
                parts.push(part.to_string());
            }
            // Anything else is a root, a prefix, or a climb: not a mod file.
            Component::CurDir => {}
            _ => bail!("that archive has an entry outside itself; refusing it"),
        }
    }
    if parts.is_empty() {
        bail!("that archive has an entry with no name");
    }
    Ok(parts.join("/"))
}

/// A Thunderstore `manifest.json` names the mod; nothing else in the archive
/// does as reliably.
fn thunderstore_name(bytes: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let name = value.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    match value.get("version_number").and_then(|v| v.as_str()) {
        Some(version) if !version.trim().is_empty() => Some(format!("{name}-{}", version.trim())),
        _ => Some(name.to_string()),
    }
}

/// Strip whatever wrapper an archive happens to use, so what is left is the
/// mod.
///
/// There is no standard here. Thunderstore puts the plugin at the root beside
/// a `manifest.json`; some authors ship `plugins/`, some a whole `BepInEx/`
/// tree, and a great many zip a single folder named after the mod. All of
/// them are somebody's idea of tidy, and all of them have to end up as the
/// files that belong in one mod folder.
fn unwrap_layout(entries: Files) -> Files {
    for prefix in ["BepInEx/plugins/", "plugins/"] {
        let inside: Files = entries
            .iter()
            .filter(|(p, _)| p.starts_with(prefix))
            .map(|(p, b)| (p[prefix.len()..].to_string(), b.clone()))
            .collect();
        if !inside.is_empty() {
            return strip_wrapper(inside);
        }
    }
    // Packaging metadata belongs to the site the archive came from, not to
    // the server: it is what Thunderstore reads, and what BepInEx ignores.
    let files: Files = entries
        .into_iter()
        .filter(|(p, _)| {
            !matches!(
                p.to_ascii_lowercase().as_str(),
                "manifest.json" | "icon.png" | "readme.md" | "changelog.md"
            )
        })
        .collect();
    strip_wrapper(files)
}

/// Drop a single folder that holds everything else.
///
/// A zip of `MyMod-1.2.3/MyMod.dll` would otherwise install as
/// `plugins/MyMod-1.2.3/MyMod-1.2.3/MyMod.dll`. BepInEx would still find it --
/// it looks in subfolders -- but the admin reading that list would not
/// recognise what they just dropped.
fn strip_wrapper(files: Files) -> Files {
    let mut prefix: Option<String> = None;
    for (path, _) in &files {
        let Some((head, _)) = path.split_once('/') else {
            return files; // something sits at the root: no single wrapper
        };
        match &prefix {
            Some(seen) if seen != head => return files,
            Some(_) => {}
            None => prefix = Some(head.to_string()),
        }
    }
    let Some(prefix) = prefix else {
        return files;
    };
    let cut = prefix.len() + 1;
    let inner: Files = files
        .into_iter()
        .map(|(p, b)| (p[cut..].to_string(), b))
        .collect();
    // Wrappers nest: `Mod/Mod/Mod.dll` happens more than it should.
    strip_wrapper(inner)
}

/// Write the files into `plugins/<name>/`, replacing what was there.
fn place(plugins: &Path, name: &str, files: &[(String, Vec<u8>)]) -> Result<Installed> {
    if files.is_empty() {
        bail!("there was nothing to install in that");
    }
    let target = plugins.join(name);
    // Refuse anything that did not end up under the plugins folder, whatever
    // the name looked like. Belt over the braces in `sane_folder`.
    if !target.starts_with(plugins) || target == plugins {
        bail!("{name:?} would not land inside the plugins folder");
    }

    let replaced = target.exists();
    if replaced {
        // An update is a replacement, not a merge: files the new version
        // dropped would otherwise stay behind and still be loaded.
        fs::remove_dir_all(&target)
            .with_context(|| format!("cannot replace {}", target.display()))?;
    }
    for (rel, bytes) in files {
        let dest = target.join(rel);
        if !dest.starts_with(&target) {
            bail!("{rel:?} would land outside the mod's folder");
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, bytes).with_context(|| format!("cannot write {}", dest.display()))?;
    }
    Ok(Installed {
        name: name.to_string(),
        files: files.len(),
        replaced,
    })
}

/// A name an admin can search for. The crate prints `Unsupported(12)` for a
/// method it was not built with, which tells nobody anything.
fn method_name(method: zip::CompressionMethod) -> String {
    use zip::CompressionMethod as M;
    let known = if method == M::BZIP2 {
        "bzip2"
    } else if method == M::LZMA {
        "LZMA"
    } else if method == M::XZ {
        "xz"
    } else if method == M::ZSTD || method == M::ZSTD_DEPRECATED {
        "zstandard"
    } else if method == M::PPMD {
        "PPMd"
    } else if method == M::AES {
        "AES encryption"
    } else if method == M::IMPLODE || method == M::PKWARE_IMPLODE {
        "implode"
    } else {
        return format!("{method}");
    };
    known.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn server(root: &Path) -> PathBuf {
        let plugins = root.join("BepInEx").join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        plugins
    }

    fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        for (name, bytes) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(bytes).unwrap();
        }
        w.finish().unwrap();
    }

    /// Rewrite every "this entry is Stored" to some other method, without
    /// touching the bytes. What comes back is an archive claiming a
    /// compression we do not have -- which is exactly what 7-Zip hands an
    /// admin who picked bzip2 from the dropdown, and all we need to prove is
    /// that we say so instead of failing somewhere deep in a decoder.
    fn relabel_method(path: &Path, method: u16) {
        let mut bytes = fs::read(path).unwrap();
        let tag = method.to_le_bytes();
        let mut i = 0;
        while i + 12 < bytes.len() {
            // Local file header, then central directory entry: the method
            // sits at a different offset in each.
            let at = match &bytes[i..i + 4] {
                b"PK" => Some(i + 8),
                b"PK" => Some(i + 10),
                _ => None,
            };
            if let Some(at) = at {
                bytes[at..at + 2].copy_from_slice(&tag);
            }
            i += 1;
        }
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn an_archive_we_cannot_decompress_says_which_compression() {
        let dir = tempfile::tempdir().unwrap();
        server(dir.path());
        let zip_path = dir.path().join("Mod.zip");
        zip_with(&zip_path, &[("Mod.dll", b"the mod")]);
        relabel_method(&zip_path, 12);

        let err = install(dir.path(), &zip_path).unwrap_err().to_string();
        // Named, so it can be searched for, and answered so the admin has
        // something to do next.
        assert!(err.contains("bzip2"), "{err}");
        assert!(err.contains("Extract the archive"), "{err}");
    }

    #[test]
    fn an_unnamed_compression_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        server(dir.path());
        let zip_path = dir.path().join("Mod.zip");
        zip_with(&zip_path, &[("Mod.dll", b"the mod")]);
        relabel_method(&zip_path, 777);

        let err = install(dir.path(), &zip_path).unwrap_err().to_string();
        assert!(err.contains("Extract the archive"), "{err}");
    }

    #[test]
    fn a_stored_archive_installs() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let zip_path = dir.path().join("Mod.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        w.start_file("Mod.dll", opts).unwrap();
        w.write_all(b"the mod").unwrap();
        w.finish().unwrap();

        let done = install(dir.path(), &zip_path).unwrap();
        assert_eq!(done.files, 1);
        assert_eq!(fs::read(plugins.join("Mod/Mod.dll")).unwrap(), b"the mod");
    }

    #[test]
    fn a_disabled_mod_leaves_bepinex_entirely() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        fs::create_dir_all(plugins.join("Seasonality")).unwrap();
        fs::write(plugins.join("Seasonality/Seasonality.dll"), b"mod").unwrap();

        let to = disable(dir.path(), "Seasonality", false).unwrap();
        // Not under BepInEx: it would still be loaded from anywhere in there,
        // and it would still travel to every player in the pack.
        assert!(
            !to.starts_with(dir.path().join("BepInEx")),
            "{}",
            to.display()
        );
        assert!(!plugins.join("Seasonality").exists());
        assert_eq!(fs::read(to.join("Seasonality.dll")).unwrap(), b"mod");
        assert_eq!(disabled(dir.path()), ["Seasonality"]);
    }

    #[test]
    fn enabling_puts_it_back_where_bepinex_looks() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        fs::create_dir_all(plugins.join("Mod")).unwrap();
        fs::write(plugins.join("Mod/Mod.dll"), b"mod").unwrap();

        disable(dir.path(), "Mod", false).unwrap();
        let back = enable(dir.path(), "Mod").unwrap();
        assert_eq!(back, plugins.join("Mod"));
        assert_eq!(fs::read(plugins.join("Mod/Mod.dll")).unwrap(), b"mod");
        assert!(disabled(dir.path()).is_empty());
    }

    #[test]
    fn a_loose_dll_disables_too() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        fs::write(plugins.join("Loose.dll"), b"mod").unwrap();

        disable(dir.path(), "Loose.dll", true).unwrap();
        assert!(!plugins.join("Loose.dll").exists());
        assert_eq!(disabled(dir.path()), ["Loose.dll"]);
    }

    #[test]
    fn removing_moves_aside_rather_than_deleting() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        fs::create_dir_all(plugins.join("Mod")).unwrap();
        fs::write(plugins.join("Mod/Mod.dll"), b"mod").unwrap();

        let gone = remove(dir.path(), "Mod", false).unwrap();
        assert!(!plugins.join("Mod").exists());
        // An admin who removed the wrong one at eleven at night can walk it
        // back. That is the whole point of not calling remove_dir_all.
        assert_eq!(fs::read(gone.join("Mod.dll")).unwrap(), b"mod");
    }

    #[test]
    fn a_disabled_mod_can_be_removed_from_where_it_sits() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        fs::create_dir_all(plugins.join("Mod")).unwrap();
        fs::write(plugins.join("Mod/Mod.dll"), b"mod").unwrap();

        disable(dir.path(), "Mod", false).unwrap();
        let gone = remove(dir.path(), "Mod", false).unwrap();
        assert!(gone.exists());
        assert!(disabled(dir.path()).is_empty());
    }

    #[test]
    fn removing_twice_does_not_overwrite_the_first_one() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        for body in [b"first".as_slice(), b"second".as_slice()] {
            fs::create_dir_all(plugins.join("Mod")).unwrap();
            fs::write(plugins.join("Mod/Mod.dll"), body).unwrap();
            remove(dir.path(), "Mod", false).unwrap();
        }
        let kept: Vec<_> = fs::read_dir(dir.path().join(REMOVED_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            kept.len(),
            2,
            "the second removal overwrote the first: {kept:?}"
        );
    }

    #[test]
    fn a_name_that_climbs_out_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        server(dir.path());
        for bad in ["..", "../Valheim", "a/b"] {
            assert!(disable(dir.path(), bad, false).is_err(), "{bad}");
            assert!(remove(dir.path(), bad, false).is_err(), "{bad}");
            assert!(enable(dir.path(), bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn enabling_over_a_mod_that_is_already_there_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        fs::create_dir_all(plugins.join("Mod")).unwrap();
        fs::write(plugins.join("Mod/Mod.dll"), b"one").unwrap();
        disable(dir.path(), "Mod", false).unwrap();
        // Somebody reinstalled it in the meantime. Two copies of one mod is
        // what BepInEx settles by refusing both, so say so instead.
        fs::create_dir_all(plugins.join("Mod")).unwrap();
        fs::write(plugins.join("Mod/Mod.dll"), b"two").unwrap();

        let err = enable(dir.path(), "Mod").unwrap_err().to_string();
        assert!(err.contains("already in the plugins folder"), "{err}");
    }

    #[test]
    fn a_thunderstore_zip_is_named_by_its_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let zip_path = dir.path().join("download.zip");
        zip_with(
            &zip_path,
            &[
                (
                    "manifest.json",
                    br#"{"name":"Seasonality","version_number":"3.8.1"}"#,
                ),
                ("icon.png", b"png"),
                ("README.md", b"read me"),
                ("Seasonality.dll", b"the mod"),
            ],
        );

        let done = install(dir.path(), &zip_path).unwrap();
        assert_eq!(done.name, "Seasonality-3.8.1");
        assert!(!done.replaced);
        assert_eq!(
            fs::read(plugins.join("Seasonality-3.8.1/Seasonality.dll")).unwrap(),
            b"the mod"
        );
        // The packaging metadata is Thunderstore's business, not the server's.
        assert!(!plugins.join("Seasonality-3.8.1/manifest.json").exists());
        assert!(!plugins.join("Seasonality-3.8.1/icon.png").exists());
    }

    #[test]
    fn a_plugins_prefix_is_unwrapped() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let zip_path = dir.path().join("Mod.zip");
        zip_with(
            &zip_path,
            &[
                ("plugins/Mod/Mod.dll", b"dll"),
                ("plugins/Mod/assets/a.bundle", b"bundle"),
            ],
        );
        install(dir.path(), &zip_path).unwrap();
        // `plugins/` goes, and so does the `Mod/` inside it: what is left is
        // the mod, under one folder named for the archive.
        assert!(plugins.join("Mod/Mod.dll").exists());
        assert!(plugins.join("Mod/assets/a.bundle").exists());
    }

    /// The commonest shape outside Thunderstore: a zip holding one folder
    /// named after the mod. Nothing in it says what it is, and it must not
    /// install as a folder inside a folder of the same name.
    #[test]
    fn a_wrapper_folder_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let zip_path = dir.path().join("CoolMod-1.2.3.zip");
        zip_with(
            &zip_path,
            &[
                ("CoolMod-1.2.3/CoolMod.dll", b"dll"),
                ("CoolMod-1.2.3/data/x.bin", b"bin"),
            ],
        );
        install(dir.path(), &zip_path).unwrap();
        assert!(plugins.join("CoolMod-1.2.3/CoolMod.dll").exists());
        assert!(plugins.join("CoolMod-1.2.3/data/x.bin").exists());
        assert!(
            !plugins.join("CoolMod-1.2.3/CoolMod-1.2.3").exists(),
            "the wrapper folder was kept"
        );
    }

    /// A zip with no manifest, no wrapper and no prefix -- a dll and its
    /// config, straight from a release page. Nothing to unwrap, nothing to
    /// name it by but the archive.
    #[test]
    fn a_bare_release_zip_installs_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let zip_path = dir.path().join("SomeMod.zip");
        zip_with(
            &zip_path,
            &[("SomeMod.dll", b"dll"), ("SomeMod.cfg", b"cfg")],
        );
        let done = install(dir.path(), &zip_path).unwrap();
        assert_eq!(done.name, "SomeMod");
        assert!(plugins.join("SomeMod/SomeMod.dll").exists());
        assert!(plugins.join("SomeMod/SomeMod.cfg").exists());
    }

    #[test]
    fn an_archive_we_cannot_read_says_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        server(dir.path());
        let seven = dir.path().join("Mod.7z");
        fs::write(&seven, b"not really").unwrap();
        let err = install(dir.path(), &seven).unwrap_err().to_string();
        assert!(err.contains("Extract"), "{err}");
    }

    #[test]
    fn a_whole_bepinex_tree_is_unwrapped_too() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let zip_path = dir.path().join("Big.zip");
        zip_with(
            &zip_path,
            &[
                ("BepInEx/plugins/Big/Big.dll", b"dll"),
                ("BepInEx/config/Big.cfg", b"cfg"),
            ],
        );
        install(dir.path(), &zip_path).unwrap();
        assert!(plugins.join("Big/Big.dll").exists());
        // Only the plugins subtree travels: a config belongs to whoever runs
        // the server, and the launcher seeds players' own rather than
        // overwriting them.
        assert!(!plugins.join("Big/Big.cfg").exists());
    }

    /// An update replaces: files the new version dropped must not stay behind
    /// and go on being loaded.
    #[test]
    fn installing_over_a_mod_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let old = plugins.join("Mod");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("Mod.dll"), b"v1").unwrap();
        fs::write(old.join("gone-in-v2.dll"), b"stale").unwrap();

        let zip_path = dir.path().join("Mod.zip");
        zip_with(&zip_path, &[("Mod.dll", b"v2")]);
        let done = install(dir.path(), &zip_path).unwrap();

        assert!(done.replaced);
        assert_eq!(fs::read(old.join("Mod.dll")).unwrap(), b"v2");
        assert!(
            !old.join("gone-in-v2.dll").exists(),
            "stale file left behind"
        );
    }

    #[test]
    fn a_loose_dll_gets_a_folder_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let dll = dir.path().join("Solo.dll");
        fs::write(&dll, b"solo").unwrap();
        let done = install(dir.path(), &dll).unwrap();
        assert_eq!(done.name, "Solo");
        assert_eq!(fs::read(plugins.join("Solo/Solo.dll")).unwrap(), b"solo");
    }

    /// The archive comes off the internet. An entry that climbs out of it is
    /// the oldest trick there is, and it must not reach the disk.
    #[test]
    fn an_entry_that_climbs_out_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        server(dir.path());
        let zip_path = dir.path().join("evil.zip");
        zip_with(&zip_path, &[("../../../evil.dll", b"nope")]);
        let err = install(dir.path(), &zip_path).unwrap_err().to_string();
        assert!(err.contains("refusing it"), "{err}");
        assert!(!dir.path().parent().unwrap().join("evil.dll").exists());
    }

    #[test]
    fn a_folder_is_taken_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = server(dir.path());
        let src = dir.path().join("MyMod");
        fs::create_dir_all(src.join("assets")).unwrap();
        fs::write(src.join("MyMod.dll"), b"dll").unwrap();
        fs::write(src.join("assets/a.bin"), b"bin").unwrap();
        let done = install(dir.path(), &src).unwrap();
        assert_eq!(done.name, "MyMod");
        assert_eq!(done.files, 2);
        assert!(plugins.join("MyMod/assets/a.bin").exists());
    }

    #[test]
    fn something_that_is_not_a_mod_says_so() {
        let dir = tempfile::tempdir().unwrap();
        server(dir.path());
        let txt = dir.path().join("notes.txt");
        fs::write(&txt, b"hello").unwrap();
        let err = install(dir.path(), &txt).unwrap_err().to_string();
        assert!(err.contains("not a mod"), "{err}");
    }

    #[test]
    fn a_server_without_bepinex_is_named_in_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let dll = dir.path().join("Mod.dll");
        fs::write(&dll, b"x").unwrap();
        let err = install(dir.path(), &dll).unwrap_err().to_string();
        assert!(err.contains("BepInEx"), "{err}");
    }
}
