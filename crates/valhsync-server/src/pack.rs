//! Building and publishing a pack: scan, manifest, signature, store.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use valhsync_core::limits::human_bytes;
use valhsync_core::manifest::MAX_NOTES;
use valhsync_core::scan::{self, ScanResult};
use valhsync_core::sign::encode_signature;
use valhsync_core::{AllowedRoots, Keypair, Manifest};

use crate::config::Config;
use crate::store::Store;

/// What the HTTP server hands out. Immutable; swapped as a whole on rebuild.
#[derive(Debug)]
pub struct Published {
    pub manifest: Manifest,
    pub manifest_bytes: Bytes,
    pub signature: String,
    pub hashes: HashSet<String>,
    /// Base64url public key, published next to the manifest.
    pub pubkey: String,
}

#[derive(Debug)]
pub struct BuildOutcome {
    pub published: Arc<Published>,
    pub scan: ScanResult,
    pub previous: Option<Manifest>,
    pub store_added: usize,
    pub store_removed: usize,
    /// True when the pack content did not change since the previous publish.
    pub unchanged: bool,
}

pub fn published_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("published")
}

fn read_published(data_dir: &Path) -> Option<(Vec<u8>, String)> {
    let dir = published_dir(data_dir);
    let bytes = std::fs::read(dir.join("manifest.json")).ok()?;
    let sig = std::fs::read_to_string(dir.join("manifest.sig")).ok()?;
    Some((bytes, sig.trim().to_string()))
}

/// The last manifest this server published, if any. It is our own file, so
/// it is parsed without signature verification.
pub fn load_previous(data_dir: &Path) -> Option<Manifest> {
    let (bytes, _) = read_published(data_dir)?;
    serde_json::from_slice(&bytes).ok()
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

/// Scan, sign and publish. Safe to call repeatedly; the store and the
/// Which Valheim the server last started as, from its own log.
///
/// Published with the pack so a launcher can tell a player that their game and
/// this server are on different network versions -- which Valheim refuses,
/// whatever the mods say -- instead of letting them find out at the join
/// screen.
fn server_network_version(cfg: &Config) -> Option<u32> {
    let root = cfg.pack.server_root.as_deref()?;
    let mut tail =
        crate::logs::Tail::new(crate::logs::find_sources(root, None).into_iter().next()?);
    tail.poll();
    valhsync_core::gamelog::network_version(&valhsync_core::gamelog::read_version(tail.lines())?)
}

/// The admin's word about this pack, ready for the manifest.
///
/// Whitespace alone is nothing to say: published as an empty string it would
/// still be a note, and the launcher would clear room on the plan screen for a
/// message that reads as blank.
fn pack_notes(cfg: &Config) -> Result<Option<String>> {
    let Some(notes) = cfg
        .pack
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    else {
        return Ok(None);
    };
    // Caught here rather than left to `validate`, which knows the rule but not
    // where the value came from: the admin needs to be sent back to the box
    // they typed it in, not told that the pack failed validation.
    if notes.len() > MAX_NOTES {
        bail!(
            "[pack] notes is {} bytes, more than the {MAX_NOTES} a manifest may carry; \
             shorten it in the window or in the configuration",
            notes.len()
        );
    }
    Ok(Some(notes.to_string()))
}

/// published files are only rewritten when something changed.
pub fn build(cfg: &Config, keypair: &Keypair, data_dir: &Path) -> Result<BuildOutcome> {
    let scan_cfg = cfg.scan_config()?;
    let scan = scan::scan(&scan_cfg).context("scanning the pack failed")?;
    let mut manifest = Manifest::new(
        cfg.server.name.trim(),
        cfg.server.game_address.trim(),
        cfg.pack.managed_roots.clone(),
        scan.entries(),
    )
    .with_network_version(server_network_version(cfg));
    manifest.notes = pack_notes(cfg)?;
    manifest
        .validate(&AllowedRoots::bepinex(), &cfg.limits())
        .context("the pack does not pass manifest validation")?;

    let previous = load_previous(data_dir);
    let unchanged = previous.as_ref().is_some_and(|p| {
        p.pack_id == manifest.pack_id
            && p.server_name == manifest.server_name
            && p.game_address == manifest.game_address
            && p.managed_roots == manifest.managed_roots
            && p.network_version == manifest.network_version
            // Rewriting the note is the whole point of editing it: without
            // this the old bytes would be reused and players would keep
            // reading the previous message.
            && p.notes == manifest.notes
    });

    // Reuse the previous bytes when nothing changed, so `generated_at` and the
    // signature stay stable and clients can short-circuit on identical bytes.
    let (manifest, manifest_bytes, signature) = match (unchanged, read_published(data_dir)) {
        (true, Some((bytes, sig))) if keypair.public().verify_text(&bytes, &sig).is_ok() => {
            let prev = previous.clone().context("previous manifest vanished")?;
            (prev, bytes, sig)
        }
        _ => {
            let bytes = manifest.to_canonical_bytes()?;
            let sig = encode_signature(&keypair.sign(&bytes));
            (manifest, bytes, sig)
        }
    };

    let store = Store::open(data_dir)?;
    let store_added = store.ingest(&scan.files)?;

    let dir = published_dir(data_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    write_atomic(&dir.join("manifest.json"), &manifest_bytes)?;
    write_atomic(
        &dir.join("manifest.sig"),
        format!("{signature}\n").as_bytes(),
    )?;

    // Keep the previous generation's blobs too: a launcher that fetched the
    // old manifest seconds ago must still be able to finish its download.
    let hashes: HashSet<String> = manifest.files.iter().map(|f| f.blake3.clone()).collect();
    let mut keep = hashes.clone();
    if let Some(prev) = &previous {
        keep.extend(prev.files.iter().map(|f| f.blake3.clone()));
    }
    let store_removed = store.gc(&keep)?;

    Ok(BuildOutcome {
        published: Arc::new(Published {
            manifest,
            manifest_bytes: Bytes::from(manifest_bytes),
            signature,
            hashes,
            pubkey: keypair.public().to_b64(),
        }),
        scan,
        previous,
        store_added,
        store_removed,
        unchanged,
    })
}

/// What [`export_static`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExportReport {
    pub copied: usize,
    pub removed: usize,
    pub files: usize,
}

/// Write the pack as plain static files, in the exact layout the launcher
/// fetches (`manifest.json`, `manifest.sig`, `files/<blake3>`), so it can be
/// uploaded to any web space (GitHub Pages, S3, a Nextcloud public folder...).
/// No port to open: the signature makes the host irrelevant to integrity.
/// Idempotent: blobs already present are kept, stale ones are removed.
pub fn export_static(
    published: &Published,
    data_dir: &Path,
    out_dir: &Path,
) -> Result<ExportReport> {
    let store = Store::open(data_dir)?;
    let files_dir = out_dir.join("files");
    std::fs::create_dir_all(&files_dir)
        .with_context(|| format!("cannot create {}", files_dir.display()))?;

    let mut report = ExportReport {
        files: published.hashes.len(),
        ..ExportReport::default()
    };
    for hash in &published.hashes {
        let dest = files_dir.join(hash);
        if dest.is_file() {
            continue;
        }
        let src = store
            .path_for(hash)
            .filter(|p| p.is_file())
            .with_context(|| format!("blob {hash} missing from the store; run `scan`"))?;
        let tmp = files_dir.join(format!("{hash}.tmp"));
        std::fs::copy(&src, &tmp).with_context(|| format!("cannot copy {hash}"))?;
        std::fs::rename(&tmp, &dest).with_context(|| format!("cannot finalize {hash}"))?;
        report.copied += 1;
    }
    for entry in std::fs::read_dir(&files_dir)?.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !published.hashes.contains(name) && std::fs::remove_file(entry.path()).is_ok() {
            report.removed += 1;
        }
    }
    // Manifest last: a launcher that reads it finds every blob already there.
    write_atomic(
        &out_dir.join("manifest.sig"),
        format!("{}\n", published.signature).as_bytes(),
    )?;
    write_atomic(&out_dir.join("manifest.json"), &published.manifest_bytes)?;
    Ok(report)
}

/// Paths added, changed and removed between two manifests.
pub fn diff(prev: &Manifest, next: &Manifest) -> (Vec<String>, Vec<String>, Vec<String>) {
    let before = prev.by_lower_path();
    let after = next.by_lower_path();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut removed = Vec::new();
    for (k, f) in &after {
        match before.get(k) {
            None => added.push(f.path.clone()),
            Some(p) if p.blake3 != f.blake3 || p.policy != f.policy => changed.push(f.path.clone()),
            Some(_) => {}
        }
    }
    for (k, f) in &before {
        if !after.contains_key(k) {
            removed.push(f.path.clone());
        }
    }
    added.sort();
    changed.sort();
    removed.sort();
    (added, changed, removed)
}

pub fn print_summary(outcome: &BuildOutcome) {
    let m = &outcome.published.manifest;
    println!(
        "Pack \"{}\": {} files, {} ({})",
        m.server_name,
        m.files.len(),
        human_bytes(m.total_bytes()),
        m.pack_id.get(..15).unwrap_or(&m.pack_id)
    );
    if !outcome.scan.skipped.is_empty() {
        println!("Skipped (not packaged):");
        for (p, why) in &outcome.scan.skipped {
            println!("  - {p}: {why}");
        }
    }
    match &outcome.previous {
        None => println!("First publish."),
        Some(_) if outcome.unchanged => println!("No change since the previous publish."),
        Some(prev) => {
            let (added, changed, removed) = diff(prev, m);
            for p in &added {
                println!("  + {p}");
            }
            for p in &changed {
                println!("  ~ {p}");
            }
            for p in &removed {
                println!("  - {p}");
            }
            println!(
                "{} added, {} changed, {} removed",
                added.len(),
                changed.len(),
                removed.len()
            );
        }
    }
    if outcome.store_added > 0 || outcome.store_removed > 0 {
        println!(
            "Store: {} blob(s) added, {} removed.",
            outcome.store_added, outcome.store_removed
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let p = valhsync_core::path::to_os_path(root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn export_mirrors_the_http_layout() {
        let server = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        write(server.path(), "winhttp.dll", b"doorstop");
        write(server.path(), "BepInEx/plugins/A/A.dll", b"a1");
        let mut cfg = Config::default();
        cfg.pack.server_root = Some(server.path().to_path_buf());
        let kp = Keypair::generate();

        let first = build(&cfg, &kp, data.path()).unwrap();
        let r = export_static(&first.published, data.path(), out.path()).unwrap();
        assert_eq!((r.copied, r.removed, r.files), (2, 0, 2));
        let bytes = std::fs::read(out.path().join("manifest.json")).unwrap();
        let sig = std::fs::read_to_string(out.path().join("manifest.sig")).unwrap();
        kp.public().verify_text(&bytes, sig.trim()).unwrap();
        for f in &first.published.manifest.files {
            assert!(out.path().join("files").join(&f.blake3).is_file());
        }

        // A changed file: one new blob copied, the stale one removed.
        write(server.path(), "BepInEx/plugins/A/A.dll", b"a2");
        let second = build(&cfg, &kp, data.path()).unwrap();
        let r = export_static(&second.published, data.path(), out.path()).unwrap();
        assert_eq!((r.copied, r.removed, r.files), (1, 1, 2));
    }

    #[test]
    fn build_publishes_and_is_stable() {
        let server = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        write(server.path(), "winhttp.dll", b"doorstop");
        write(server.path(), "BepInEx/plugins/A/A.dll", b"a1");
        let mut cfg = Config::default();
        cfg.pack.server_root = Some(server.path().to_path_buf());
        let kp = Keypair::generate();

        let first = build(&cfg, &kp, data.path()).unwrap();
        assert!(first.previous.is_none());
        assert_eq!(first.published.manifest.files.len(), 2);
        assert_eq!(first.store_added, 2);
        let (bytes, sig) = read_published(data.path()).unwrap();
        kp.public().verify_text(&bytes, &sig).unwrap();

        let second = build(&cfg, &kp, data.path()).unwrap();
        assert!(second.unchanged);
        assert_eq!(
            second.published.manifest_bytes,
            first.published.manifest_bytes
        );
        assert_eq!(second.published.signature, first.published.signature);

        write(server.path(), "BepInEx/plugins/A/A.dll", b"a2");
        write(server.path(), "BepInEx/plugins/B/B.dll", b"b");
        let third = build(&cfg, &kp, data.path()).unwrap();
        assert!(!third.unchanged);
        let (added, changed, removed) =
            diff(third.previous.as_ref().unwrap(), &third.published.manifest);
        assert_eq!(added, vec!["BepInEx/plugins/B/B.dll"]);
        assert_eq!(changed, vec!["BepInEx/plugins/A/A.dll"]);
        assert!(removed.is_empty());
        // old A blob is kept one generation for in-flight downloads
        assert_eq!(third.store_removed, 0);
        let fourth = build(&cfg, &kp, data.path()).unwrap();
        assert_eq!(fourth.store_removed, 1);
    }

    #[test]
    fn the_admin_note_travels_with_the_pack() {
        let server = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        write(server.path(), "winhttp.dll", b"doorstop");
        let mut cfg = Config::default();
        cfg.pack.server_root = Some(server.path().to_path_buf());
        let kp = Keypair::generate();

        assert_eq!(
            build(&cfg, &kp, data.path())
                .unwrap()
                .published
                .manifest
                .notes,
            None
        );

        cfg.pack.notes = Some("  Videz vos coffres.\nLe mod remet sa config à zéro.  ".into());
        let out = build(&cfg, &kp, data.path()).unwrap();
        assert_eq!(
            out.published.manifest.notes.as_deref(),
            Some("Videz vos coffres.\nLe mod remet sa config à zéro."),
            "trimmed at the edges, untouched in the middle"
        );
        // Only the note changed, and it still has to reach the signed bytes.
        assert!(!out.unchanged);
        assert!(
            String::from_utf8_lossy(&out.published.manifest_bytes).contains("Videz vos coffres."),
        );

        // An empty box is no note at all, not an empty one.
        cfg.pack.notes = Some("   \n  ".into());
        assert_eq!(
            build(&cfg, &kp, data.path())
                .unwrap()
                .published
                .manifest
                .notes,
            None
        );

        // Too long is refused before anything is published, and says so.
        cfg.pack.notes = Some("a".repeat(MAX_NOTES + 1));
        let err = build(&cfg, &kp, data.path()).unwrap_err().to_string();
        assert!(err.contains("notes"), "{err}");
        assert!(err.contains(&MAX_NOTES.to_string()), "{err}");
    }
}
