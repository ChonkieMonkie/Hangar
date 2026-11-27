// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Backups as incremental, verified snapshots (B-009).
//!
//! Store layout (in the destination the user chose):
//!   objects/ab/abcdef…   file contents named by SHA-256 (stored once)
//!   snapshots/<id>.json  what a snapshot contains
//!   tmp/                 half-written objects (renamed into place when complete)
//!
//! Sources (Saved Games\DCS*, Tacview, extra folders) are only *read* when a snapshot is taken.
//! A restore writes back into a source only on the user's click, after taking a "Before restore"
//! snapshot and checking every object's checksum.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::compress;
use crate::settings::{Compression, now_unix, write_json};

/// Caches and logs DCS rebuilds itself; never backed up.
pub const ALWAYS_EXCLUDED: [&str; 4] = ["fxo", "metashaders2", "logs", "temp"];
const KEEP_LAST: usize = 10;
const KEEP_MONTHS: usize = 6;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub key: String,
    pub root: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub source: String,
    /// Relative to the source root, with '/'.
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub at_unix: u64,
    pub ok: bool,
    pub checked: usize,
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub created_unix: u64,
    pub reason: String,
    /// "all" | "config"
    pub scope: String,
    pub sources: Vec<SourceRef>,
    pub files: Vec<FileEntry>,
    pub total_bytes: u64,
    /// Bytes of objects that were new in this snapshot.
    pub added_bytes: u64,
    pub pinned: bool,
    pub verified: Option<Verification>,
    /// Files that could not be read (locked, permission).
    pub skipped: Vec<String>,
}

/// Snapshot without its file list, for the UI.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub id: String,
    pub created_unix: u64,
    pub reason: String,
    pub scope: String,
    pub sources: Vec<String>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub added_bytes: u64,
    pub pinned: bool,
    pub verified: Option<Verification>,
    pub skipped: usize,
}

impl From<&Snapshot> for SnapshotInfo {
    fn from(s: &Snapshot) -> Self {
        SnapshotInfo {
            id: s.id.clone(),
            created_unix: s.created_unix,
            reason: s.reason.clone(),
            scope: s.scope.clone(),
            sources: s.sources.iter().map(|r| r.key.clone()).collect(),
            file_count: s.files.len(),
            total_bytes: s.total_bytes,
            added_bytes: s.added_bytes,
            pinned: s.pinned,
            verified: s.verified.clone(),
            skipped: s.skipped.len(),
        }
    }
}

pub struct Store {
    pub root: PathBuf,
    /// Compression for new objects (None = store them as they are). Objects are named by the
    /// SHA-256 of their original content; compressed ones get ".zst" added (compress.rs).
    pub compression: Option<Compression>,
}

/// Progress callback: (files done, files total, current path).
pub type Progress<'a> = &'a mut dyn FnMut(usize, usize, &str);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn mtime_of(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Rejects absolute paths and "..": restore must stay inside the source root.
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if p.components().all(|c| matches!(c, Component::Normal(_))) { Some(p.to_path_buf()) } else { None }
}

pub fn hash_file(path: &Path) -> std::io::Result<String> {
    hash_reader(BufReader::with_capacity(1 << 20, File::open(path)?))
}

/// SHA-256 of a stored object's original content (decompresses ".zst" objects).
pub fn hash_object(path: &Path) -> std::io::Result<String> {
    hash_reader(compress::open_decoded(path)?)
}

fn hash_reader(mut r: impl Read) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into(), compression: None }
    }

    /// Compress new objects with these settings (if `cfg.backups` and the algorithm is not "none").
    pub fn with_compression(mut self, cfg: &Compression) -> Self {
        self.compression = (cfg.backups && cfg.algorithm != "none").then(|| cfg.clone());
        self
    }

    fn object_plain(&self, sha: &str) -> PathBuf {
        self.root.join("objects").join(&sha[..2]).join(sha)
    }

    /// The stored file of an object: the ".zst" one if it exists, else the plain one.
    fn object_path(&self, sha: &str) -> PathBuf {
        let plain = self.object_plain(sha);
        let z = compress::zst_path(&plain);
        if z.exists() { z } else { plain }
    }

    fn snap_path(&self, id: &str) -> PathBuf {
        self.root.join("snapshots").join(format!("{id}.json"))
    }

    pub fn load(&self, id: &str) -> Result<Snapshot, String> {
        let b = std::fs::read(self.snap_path(id)).map_err(|e| format!("Snapshot {id}: {e}"))?;
        serde_json::from_slice(&b).map_err(|e| format!("Snapshot {id}: {e}"))
    }

    fn save(&self, s: &Snapshot) -> Result<(), String> {
        write_json(&self.snap_path(&s.id), s)
    }

    /// All snapshots, newest first.
    pub fn list(&self) -> Vec<Snapshot> {
        let mut v: Vec<Snapshot> = std::fs::read_dir(self.root.join("snapshots"))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                    .filter_map(|e| std::fs::read(e.path()).ok())
                    .filter_map(|b| serde_json::from_slice::<Snapshot>(&b).ok())
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then(b.id.cmp(&a.id)));
        v
    }

    /// Bytes used by stored objects.
    pub fn store_bytes(&self) -> u64 {
        crate::winutil::folder_size(&self.root.join("objects"))
    }

    /// Copies `src` into the object store while hashing it, compressed when that is worth it.
    /// Returns (sha of the original content, bytes added to the store).
    fn put_object(&self, src: &Path) -> std::io::Result<(String, u64)> {
        let tmp_dir = self.root.join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let tmp = tmp_dir.join(format!("{}-{}", std::process::id(), now_nanos()));
        let size = std::fs::metadata(src)?.len();
        let zip = self.compression.as_ref().filter(|c| compress::worth_it(c, src, size));
        let mut r = BufReader::with_capacity(1 << 20, File::open(src)?);
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut n_total = 0u64;
        let mut pump = |w: &mut dyn Write| -> std::io::Result<()> {
            loop {
                let n = r.read(&mut buf)?;
                if n == 0 {
                    return Ok(());
                }
                h.update(&buf[..n]);
                w.write_all(&buf[..n])?;
                n_total += n as u64;
            }
        };
        let file = std::io::BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        let file = match zip {
            Some(c) => {
                let mut e = compress::encoder(c, file)?;
                pump(&mut e)?;
                e.finish()?
            }
            None => {
                let mut f = file;
                pump(&mut f)?;
                f
            }
        };
        file.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        let sha = hex(&h.finalize());
        let plain = self.object_plain(&sha);
        if plain.exists() || compress::zst_path(&plain).exists() {
            let _ = std::fs::remove_file(&tmp);
            return Ok((sha, 0));
        }
        std::fs::create_dir_all(plain.parent().unwrap())?;
        let stored = std::fs::metadata(&tmp)?.len();
        if zip.is_some() && stored < n_total - n_total / 32 {
            std::fs::rename(&tmp, compress::zst_path(&plain))?;
            return Ok((sha, stored));
        }
        if zip.is_some() {
            // compression did not pay off: store the original instead
            let _ = std::fs::remove_file(&tmp);
            std::fs::copy(src, &tmp)?;
            if hash_file(&tmp)? != sha {
                let _ = std::fs::remove_file(&tmp);
                return Err(std::io::Error::other("the file changed while it was backed up"));
            }
        }
        std::fs::rename(&tmp, &plain)?;
        Ok((sha, n_total))
    }

    /// Takes a snapshot. `scope` "config" = only `Config` of each source; "all" = everything
    /// except caches and `excluded` ("<key>/<top folder>", case-insensitive).
    pub fn snapshot(&self, sources: &[SourceRef], scope: &str, reason: &str, excluded: &[String], progress: Progress) -> Result<Snapshot, String> {
        std::fs::create_dir_all(self.root.join("snapshots")).map_err(|e| format!("Backup destination: {e}"))?;
        let excluded: HashSet<String> = excluded.iter().map(|s| s.to_ascii_lowercase()).collect();
        // reuse checksums from the newest snapshot when size + mtime are unchanged
        let mut known: HashMap<(String, String), (u64, i64, String)> = HashMap::new();
        if let Some(prev) = self.list().into_iter().next() {
            for f in prev.files {
                known.insert((f.source.clone(), f.path.clone()), (f.size, f.mtime, f.sha256));
            }
        }
        // collect
        let mut todo: Vec<(String, PathBuf, String, std::fs::Metadata)> = Vec::new();
        for src in sources {
            let root = PathBuf::from(&src.root);
            let start = if scope == "config" { root.join("Config") } else { root.clone() };
            walk(&root, &start, &mut |rel, md| {
                let top = rel.split('/').next().unwrap_or("").to_ascii_lowercase();
                if ALWAYS_EXCLUDED.contains(&top.as_str()) || excluded.contains(&format!("{}/{}", src.key.to_ascii_lowercase(), top)) {
                    return;
                }
                todo.push((src.key.clone(), root.join(rel), rel.to_string(), md.clone()));
            });
        }
        let total = todo.len();
        let mut files = Vec::with_capacity(total);
        let mut skipped = Vec::new();
        let (mut total_bytes, mut added) = (0u64, 0u64);
        for (i, (key, abs, rel, md)) in todo.into_iter().enumerate() {
            progress(i, total, &rel);
            let (size, mtime) = (md.len(), mtime_of(&md));
            let sha = match known.get(&(key.clone(), rel.clone())) {
                Some((s, m, sha)) if *s == size && *m == mtime && self.object_path(sha).exists() => sha.clone(),
                _ => match self.put_object(&abs) {
                    Ok((sha, n)) => {
                        added += n;
                        sha
                    }
                    Err(e) => {
                        skipped.push(format!("{key}/{rel}: {e}"));
                        continue;
                    }
                },
            };
            total_bytes += size;
            files.push(FileEntry { source: key, path: rel, size, mtime, sha256: sha });
        }
        progress(total, total, "");
        let created = now_unix();
        let snap = Snapshot {
            id: self.unique_id(created),
            created_unix: created,
            reason: reason.to_string(),
            scope: scope.to_string(),
            sources: sources.to_vec(),
            files,
            total_bytes,
            added_bytes: added,
            pinned: false,
            verified: None,
            skipped,
        };
        self.save(&snap)?;
        Ok(snap)
    }

    /// Reads every object of a snapshot and compares its checksum. Stores the result.
    pub fn verify(&self, id: &str, progress: Progress) -> Result<Verification, String> {
        let mut snap = self.load(id)?;
        let shas: Vec<String> = snap.files.iter().map(|f| f.sha256.clone()).collect::<HashSet<_>>().into_iter().collect();
        let mut errors = Vec::new();
        for (i, sha) in shas.iter().enumerate() {
            progress(i, shas.len(), sha);
            match hash_object(&self.object_path(sha)) {
                Ok(h) if &h == sha => {}
                Ok(_) => errors.push(format!("Damaged object {sha}")),
                Err(e) => errors.push(format!("Missing object {sha}: {e}")),
            }
        }
        progress(shas.len(), shas.len(), "");
        let v = Verification { at_unix: now_unix(), ok: errors.is_empty(), checked: shas.len(), errors };
        snap.verified = Some(v.clone());
        self.save(&snap)?;
        Ok(v)
    }

    /// Writes files of a snapshot back. Takes a "Before restore" snapshot of the same sources
    /// first, checks each object's checksum, writes via temp file + rename and keeps the
    /// original modification time. Files that exist now but are not in the snapshot are left
    /// alone. `only` = restore just these (source, path) pairs.
    pub fn restore(&self, id: &str, scope: &str, only: Option<&[(String, String)]>, excluded: &[String], progress: Progress) -> Result<RestoreResult, String> {
        let snap = self.load(id)?;
        let wanted: Vec<&FileEntry> = snap
            .files
            .iter()
            .filter(|f| match (scope, only) {
                (_, Some(list)) => list.iter().any(|(s, p)| *s == f.source && *p == f.path),
                ("config", None) => f.path.to_ascii_lowercase().starts_with("config/"),
                _ => true,
            })
            .collect();
        if wanted.is_empty() {
            return Err("Nothing to restore.".into());
        }
        let before_scope = if scope == "config" && only.is_none() { "config" } else { "all" };
        let before = self.snapshot(&snap.sources, before_scope, "Before restore", excluded, &mut |_, _, _| {})?;
        let roots: HashMap<&str, &str> = snap.sources.iter().map(|s| (s.key.as_str(), s.root.as_str())).collect();
        let mut restored = 0;
        let mut failed = Vec::new();
        for (i, f) in wanted.iter().enumerate() {
            progress(i, wanted.len(), &f.path);
            let (Some(root), Some(rel)) = (roots.get(f.source.as_str()), safe_rel(&f.path)) else {
                failed.push(format!("{}/{}: unsafe path", f.source, f.path));
                continue;
            };
            match self.restore_one(f, &Path::new(root).join(rel)) {
                Ok(()) => restored += 1,
                Err(e) => failed.push(format!("{}/{}: {e}", f.source, f.path)),
            }
        }
        progress(wanted.len(), wanted.len(), "");
        Ok(RestoreResult { restored, failed, before_restore_id: before.id })
    }

    fn restore_one(&self, f: &FileEntry, dst: &Path) -> Result<(), String> {
        let obj = self.object_path(&f.sha256);
        let h = hash_object(&obj).map_err(|e| format!("object missing: {e}"))?;
        if h != f.sha256 {
            return Err("stored copy is damaged, not restored".into());
        }
        let dir = dst.parent().ok_or("no parent folder")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!(".hangar-restore-{}", now_nanos()));
        compress::decode_to(&obj, &tmp).map_err(|e| e.to_string())?;
        if hash_file(&tmp).map_err(|e| e.to_string())? != f.sha256 {
            let _ = std::fs::remove_file(&tmp);
            return Err("stored copy is damaged, not restored".into());
        }
        if let Ok(file) = File::options().write(true).open(&tmp) {
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(f.mtime.max(0) as u64);
            let _ = file.set_modified(t);
        }
        std::fs::rename(&tmp, dst).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<(), String> {
        let mut s = self.load(id)?;
        s.pinned = pinned;
        self.save(&s)
    }

    /// A snapshot id that no snapshot file uses yet. The time stamp has one-second resolution and
    /// Windows' clock only 100 ns, so two snapshots in the same second could get the same id
    /// and the second would overwrite the first. A counter makes it unique.
    fn unique_id(&self, created: u64) -> String {
        let base = format!("{}-{:03}", crate::recorder::utc_stamp(created), (now_nanos() / 1_000_000) % 1000);
        let mut id = base.clone();
        let mut n = 1;
        while self.snap_path(&id).exists() {
            n += 1;
            id = format!("{base}-{n}");
        }
        id
    }

    /// Deletes one snapshot (never a pinned one) and objects nothing points to any more.
    pub fn delete(&self, id: &str) -> Result<u64, String> {
        let s = self.load(id)?;
        if s.pinned {
            return Err("Unpin the snapshot before deleting it.".into());
        }
        std::fs::remove_file(self.snap_path(id)).map_err(|e| e.to_string())?;
        Ok(self.collect_garbage())
    }

    /// Bytes that deleting these snapshots would free: objects no other snapshot uses.
    pub fn estimate_freed(&self, ids: &[String]) -> u64 {
        let all = self.list();
        let gone: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let kept: HashSet<String> = all
            .iter()
            .filter(|s| !gone.contains(s.id.as_str()) || s.pinned)
            .flat_map(|s| s.files.iter().map(|f| f.sha256.clone()))
            .collect();
        let doomed: HashSet<String> = all
            .iter()
            .filter(|s| gone.contains(s.id.as_str()) && !s.pinned)
            .flat_map(|s| s.files.iter().map(|f| f.sha256.clone()))
            .filter(|sha| !kept.contains(sha))
            .collect();
        doomed.iter().filter_map(|sha| std::fs::metadata(self.object_path(sha)).ok()).map(|m| m.len()).sum()
    }

    /// Deletes snapshots the user picked (B-030 for backups): to the Recycle Bin by default.
    /// The snapshot files and the objects only they use are moved into one folder,
    /// `deleted/<time>/` with the store's own layout, and that folder goes to the Recycle Bin.
    /// If the user restores it, it comes back to `deleted/` and [`Store::recover_deleted`] puts
    /// it back into the store, so the snapshots reappear. Pinned snapshots are skipped.
    /// Returns (deleted ids, skipped (id, reason), bytes freed).
    pub fn delete_to_trash(&self, ids: &[String], progress: Progress) -> Result<(Vec<String>, Vec<(String, String)>, u64), String> {
        if !crate::trash::available(&self.root) {
            return Err("The Recycle Bin is not available on the backup drive. Hold Shift to delete permanently.".into());
        }
        let all = self.list();
        let mut skipped = Vec::new();
        let mut doomed_snaps = Vec::new();
        for id in ids {
            match all.iter().find(|s| &s.id == id) {
                Some(s) if s.pinned => skipped.push((id.clone(), "pinned".to_string())),
                Some(s) => doomed_snaps.push(s),
                None => skipped.push((id.clone(), "not found".to_string())),
            }
        }
        if doomed_snaps.is_empty() {
            return Ok((vec![], skipped, 0));
        }
        let gone: HashSet<&str> = doomed_snaps.iter().map(|s| s.id.as_str()).collect();
        let kept: HashSet<&str> = all.iter().filter(|s| !gone.contains(s.id.as_str())).flat_map(|s| s.files.iter().map(|f| f.sha256.as_str())).collect();
        let objects: HashSet<&str> = doomed_snaps.iter().flat_map(|s| s.files.iter().map(|f| f.sha256.as_str())).filter(|sha| !kept.contains(sha)).collect();
        let batch = self.root.join("deleted").join(format!("{}-{:03}", crate::recorder::utc_stamp(now_unix()), (now_nanos() / 1_000_000) % 1000));
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let total = doomed_snaps.len() + objects.len() + 1;
        let mut step = 0;
        let mut res = (|| -> Result<u64, String> {
            let mut freed = 0;
            for s in &doomed_snaps {
                progress(step, total, &s.id);
                step += 1;
                let from = self.snap_path(&s.id);
                let to = batch.join("snapshots").join(format!("{}.json", s.id));
                std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
                moved.push((from, to));
            }
            for sha in &objects {
                progress(step, total, "Moving stored files");
                step += 1;
                let from = self.object_path(sha);
                if !from.exists() {
                    continue;
                }
                let rel = from.strip_prefix(&self.root).map_err(|e| e.to_string())?.to_path_buf();
                let to = batch.join(rel);
                std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
                freed += std::fs::metadata(&from).map(|m| m.len()).unwrap_or(0);
                std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
                moved.push((from, to));
            }
            progress(step, total, "Moving to the Recycle Bin");
            crate::trash::recycle(&[batch.clone()])?;
            Ok(freed)
        })();
        if res.is_err() || batch.exists() {
            // put everything back: nothing may be lost if the Recycle Bin refused
            for (from, to) in moved.iter().rev() {
                if to.exists() && !from.exists() {
                    let _ = std::fs::rename(to, from);
                }
            }
            let _ = std::fs::remove_dir_all(&batch);
            if res.is_ok() {
                res = Err("Moving to the Recycle Bin was cancelled.".into());
            }
        }
        let freed = res?;
        let _ = std::fs::remove_dir(self.root.join("deleted"));
        progress(total, total, "");
        Ok((doomed_snaps.iter().map(|s| s.id.clone()).collect(), skipped, freed))
    }

    /// Puts snapshots back that the user restored from the Recycle Bin (they reappear in
    /// `deleted/<time>/`), or that an interrupted delete left there. Returns how many.
    pub fn recover_deleted(&self) -> usize {
        let dir = self.root.join("deleted");
        let mut n = 0;
        for batch in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let b = batch.path();
            if !b.is_dir() {
                continue;
            }
            for (from, rel) in files_under(&b) {
                let to = self.root.join(&rel);
                if to.exists() || (rel.starts_with("objects") && compress::zst_path(&to).exists()) {
                    let _ = std::fs::remove_file(&from);
                    continue;
                }
                if std::fs::create_dir_all(to.parent().unwrap()).is_ok() && std::fs::rename(&from, &to).is_ok() && rel.starts_with("snapshots") {
                    n += 1;
                }
            }
            let _ = std::fs::remove_dir_all(&b);
        }
        let _ = std::fs::remove_dir(&dir);
        n
    }

    /// Deletes several snapshots for good and collects garbage once. Pinned ones are skipped.
    /// Returns (deleted ids, skipped (id, reason), bytes freed).
    pub fn delete_many(&self, ids: &[String], progress: Progress) -> (Vec<String>, Vec<(String, String)>, u64) {
        let (mut deleted, mut skipped) = (Vec::new(), Vec::new());
        for (i, id) in ids.iter().enumerate() {
            progress(i, ids.len() + 1, id);
            match self.load(id) {
                Ok(s) if s.pinned => skipped.push((id.clone(), "pinned".to_string())),
                Ok(_) => match std::fs::remove_file(self.snap_path(id)) {
                    Ok(()) => deleted.push(id.clone()),
                    Err(e) => skipped.push((id.clone(), e.to_string())),
                },
                Err(e) => skipped.push((id.clone(), e)),
            }
        }
        progress(ids.len(), ids.len() + 1, "Freeing space");
        let freed = if deleted.is_empty() { 0 } else { self.collect_garbage() };
        progress(ids.len() + 1, ids.len() + 1, "");
        (deleted, skipped, freed)
    }

    /// Keeps the newest KEEP_LAST, the newest per month for KEEP_MONTHS months, all pinned and
    /// the newest verified-OK snapshot. Returns deleted ids.
    pub fn prune(&self) -> Vec<String> {
        let all = self.list();
        let keep = keep_set(&all);
        let mut deleted = Vec::new();
        for s in &all {
            if !keep.contains(&s.id) && std::fs::remove_file(self.snap_path(&s.id)).is_ok() {
                deleted.push(s.id.clone());
            }
        }
        if !deleted.is_empty() {
            self.collect_garbage();
        }
        deleted
    }

    /// Removes objects that no snapshot refers to. Returns bytes freed.
    pub fn collect_garbage(&self) -> u64 {
        let live: HashSet<String> = self.list().into_iter().flat_map(|s| s.files.into_iter().map(|f| f.sha256)).collect();
        let mut freed = 0;
        if let Ok(rd) = std::fs::read_dir(self.root.join("objects")) {
            for d in rd.flatten() {
                for o in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                    let name = o.file_name().to_string_lossy().into_owned();
                    let sha = name.strip_suffix(".zst").unwrap_or(&name);
                    if !live.contains(sha) {
                        let len = o.metadata().map(|m| m.len()).unwrap_or(0);
                        if std::fs::remove_file(o.path()).is_ok() {
                            freed += len;
                        }
                    }
                }
            }
        }
        freed
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub restored: usize,
    pub failed: Vec<String>,
    pub before_restore_id: String,
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Year*12+month of a unix time (UTC), for the monthly rule.
fn month_index(unix: u64) -> i64 {
    let s = crate::recorder::utc_stamp(unix);
    let y: i64 = s[0..4].parse().unwrap_or(0);
    let m: i64 = s[4..6].parse().unwrap_or(0);
    y * 12 + m
}

pub fn keep_set(newest_first: &[Snapshot]) -> HashSet<String> {
    let mut keep: HashSet<String> = HashSet::new();
    for s in newest_first.iter().take(KEEP_LAST) {
        keep.insert(s.id.clone());
    }
    if let Some(first) = newest_first.first() {
        let now_m = month_index(first.created_unix);
        let mut seen = HashSet::new();
        for s in newest_first {
            let m = month_index(s.created_unix);
            if now_m - m < KEEP_MONTHS as i64 && seen.insert(m) {
                keep.insert(s.id.clone());
            }
        }
    }
    for s in newest_first {
        if s.pinned {
            keep.insert(s.id.clone());
        }
    }
    if let Some(v) = newest_first.iter().find(|s| s.verified.as_ref().is_some_and(|v| v.ok)) {
        keep.insert(v.id.clone());
    }
    keep
}

/// Walks `start` (inside `root`) without following junctions or symlinks.
/// Calls `f(relative path with '/', metadata)` for each file.
/// (absolute path, path relative to `base`) of every file under `base`.
fn files_under(base: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(base) {
                out.push((p.clone(), rel.to_path_buf()));
            }
        }
    }
    out
}

/// Every file under `root` (skipping caches and links), for sampling (compression estimate).
pub fn walk_files(root: &Path, f: &mut dyn FnMut(&Path)) {
    walk(root, root, &mut |rel, _| {
        let top = rel.split('/').next().unwrap_or("").to_ascii_lowercase();
        if !ALWAYS_EXCLUDED.contains(&top.as_str()) {
            f(&root.join(rel));
        }
    });
}

fn walk(root: &Path, start: &Path, f: &mut dyn FnMut(&str, &std::fs::Metadata)) {
    let Ok(rd) = std::fs::read_dir(start) else { return };
    for e in rd.flatten() {
        let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
        if md.file_type().is_symlink() || is_reparse(&md) {
            continue;
        }
        if md.is_dir() {
            walk(root, &e.path(), f);
        } else if md.is_file() {
            if let Ok(rel) = e.path().strip_prefix(root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                f(&rel, &md);
            }
        }
    }
}

fn is_reparse(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
}

/// Sizes of the top-level folders of a source, for "What to back up".
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TopFolder {
    pub name: String,
    pub bytes: u64,
    pub cache: bool,
}

pub fn top_folders(root: &Path) -> Vec<TopFolder> {
    let mut v: Vec<TopFolder> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let cache = ALWAYS_EXCLUDED.contains(&name.to_ascii_lowercase().as_str());
                    TopFolder { bytes: crate::winutil::folder_size(&e.path()), cache, name }
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hangar-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn fake_saved_games(root: &Path) -> Vec<SourceRef> {
        let sg = root.join("Saved Games").join("DCS");
        put(&sg, "Config/options.lua", "options = { graphics = 1 }");
        put(&sg, "Config/Input/F-16C_50/joystick/stick.diff.lua", "local diff = {}");
        put(&sg, "Missions/my.miz", "MIZDATA");
        put(&sg, "fxo/cache.bin", "CACHE");
        put(&sg, "Logs/dcs.log", "LOG");
        put(&sg, "Missions/copy.miz", "MIZDATA"); // same content: one object
        vec![SourceRef { key: "DCS".into(), root: sg.to_string_lossy().into_owned() }]
    }

    #[test]
    fn snapshot_dedupes_skips_caches_and_is_incremental() {
        let root = tmp("snap");
        let src = fake_saved_games(&root);
        let store = Store::new(root.join("store"));
        let s1 = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        let paths: Vec<&str> = s1.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Config/options.lua"));
        assert!(!paths.iter().any(|p| p.starts_with("fxo") || p.starts_with("Logs")), "{paths:?}");
        assert_eq!(s1.files.len(), 4);
        // 4 files, 3 distinct contents
        let objects = walk_count(&root.join("store").join("objects"));
        assert_eq!(objects, 3);
        // nothing changed: second snapshot adds nothing
        let s2 = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        assert_eq!(s2.added_bytes, 0);
        // config scope only takes Config
        let s3 = store.snapshot(&src, "config", "Before update", &[], &mut |_, _, _| {}).unwrap();
        assert!(s3.files.iter().all(|f| f.path.starts_with("Config/")));
        // exclusion by top folder
        let s4 = store.snapshot(&src, "all", "Manual", &["DCS/Missions".into()], &mut |_, _, _| {}).unwrap();
        assert!(s4.files.iter().all(|f| !f.path.starts_with("Missions")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn walk_count(p: &Path) -> usize {
        let mut n = 0;
        walk(p, p, &mut |_, _| n += 1);
        n
    }

    #[test]
    fn verify_detects_damage() {
        let root = tmp("verify");
        let src = fake_saved_games(&root);
        let store = Store::new(root.join("store"));
        let s = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        assert!(store.verify(&s.id, &mut |_, _, _| {}).unwrap().ok);
        let sha = &s.files.iter().find(|f| f.path == "Config/options.lua").unwrap().sha256;
        std::fs::write(store.object_path(sha), "tampered").unwrap();
        let v = store.verify(&s.id, &mut |_, _, _| {}).unwrap();
        assert!(!v.ok);
        assert_eq!(v.errors.len(), 1);
        assert_eq!(store.load(&s.id).unwrap().verified.unwrap().ok, false);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn restore_takes_before_snapshot_and_can_be_undone() {
        let root = tmp("restore");
        let src = fake_saved_games(&root);
        let sg = PathBuf::from(&src[0].root);
        let store = Store::new(root.join("store"));
        let good = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        // the user breaks their bindings and adds a new file
        put(&sg, "Config/Input/F-16C_50/joystick/stick.diff.lua", "BROKEN");
        put(&sg, "Config/new.lua", "NEW");
        let r = store.restore(&good.id, "config", None, &[], &mut |_, _, _| {}).unwrap();
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(r.restored, 2);
        assert_eq!(std::fs::read_to_string(sg.join("Config/Input/F-16C_50/joystick/stick.diff.lua")).unwrap(), "local diff = {}");
        assert!(sg.join("Config/new.lua").exists(), "files not in the snapshot are left alone");
        // undo: restore the before-restore snapshot
        let r2 = store.restore(&r.before_restore_id, "config", None, &[], &mut |_, _, _| {}).unwrap();
        assert!(r2.failed.is_empty());
        assert_eq!(std::fs::read_to_string(sg.join("Config/Input/F-16C_50/joystick/stick.diff.lua")).unwrap(), "BROKEN");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn restore_refuses_damaged_objects_and_unsafe_paths() {
        let root = tmp("restore-bad");
        let src = fake_saved_games(&root);
        let sg = PathBuf::from(&src[0].root);
        let store = Store::new(root.join("store"));
        let mut s = store.snapshot(&src, "config", "Manual", &[], &mut |_, _, _| {}).unwrap();
        let opt = s.files.iter().find(|f| f.path == "Config/options.lua").unwrap().sha256.clone();
        std::fs::write(store.object_path(&opt), "tampered").unwrap();
        s.files.push(FileEntry { source: "DCS".into(), path: "../escape.lua".into(), size: 1, mtime: 0, sha256: opt.clone() });
        store.save(&s).unwrap();
        put(&sg, "Config/options.lua", "CURRENT");
        let r = store.restore(&s.id, "all", None, &[], &mut |_, _, _| {}).unwrap();
        assert_eq!(r.failed.len(), 2, "{:?}", r.failed);
        assert_eq!(std::fs::read_to_string(sg.join("Config/options.lua")).unwrap(), "CURRENT");
        assert!(!sg.parent().unwrap().join("escape.lua").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn delete_respects_pins_and_collects_garbage() {
        let root = tmp("delete");
        let src = fake_saved_games(&root);
        let sg = PathBuf::from(&src[0].root);
        let store = Store::new(root.join("store"));
        let a = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        put(&sg, "Missions/only-in-b.miz", "B ONLY CONTENT");
        let b = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        store.set_pinned(&b.id, true).unwrap();
        assert!(store.delete(&b.id).is_err());
        store.set_pinned(&b.id, false).unwrap();
        let freed = store.delete(&b.id).unwrap();
        assert_eq!(freed, "B ONLY CONTENT".len() as u64);
        assert!(store.verify(&a.id, &mut |_, _, _| {}).unwrap().ok, "objects of the other snapshot stay");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn delete_many_skips_pinned_and_estimates_right() {
        let root = tmp("many");
        let src = fake_saved_games(&root);
        let sg = PathBuf::from(&src[0].root);
        let store = Store::new(root.join("store"));
        let a = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        put(&sg, "Missions/b.miz", "ONLY IN B AND C");
        let b = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        put(&sg, "Missions/c.miz", "ONLY IN C");
        let c = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        store.set_pinned(&a.id, true).unwrap();
        let ids = vec![a.id.clone(), b.id.clone(), c.id.clone()];
        let est = store.estimate_freed(&ids);
        assert_eq!(est, ("ONLY IN B AND C".len() + "ONLY IN C".len()) as u64);
        let (deleted, skipped, freed) = store.delete_many(&ids, &mut |_, _, _| {});
        assert_eq!(deleted.len(), 2);
        assert_eq!(skipped, vec![(a.id.clone(), "pinned".to_string())]);
        assert_eq!(freed, est, "the estimate matches what was freed");
        assert!(store.verify(&a.id, &mut |_, _, _| {}).unwrap().ok);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn deleted_batch_comes_back_after_restore_from_recycle_bin() {
        // simulates what the user gets back from the Recycle Bin: the deleted/<time>/ folder
        let root = tmp("trashback");
        let src = fake_saved_games(&root);
        let sg = PathBuf::from(&src[0].root);
        let store = Store::new(root.join("store"));
        let a = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        put(&sg, "Missions/b.miz", "ONLY IN B");
        let b = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        let sha = b.files.iter().find(|f| f.path == "Missions/b.miz").unwrap().sha256.clone();
        let obj = store.object_path(&sha);
        let batch = store.root.join("deleted").join("20261128-000000-000");
        let rel = obj.strip_prefix(&store.root).unwrap().to_path_buf();
        std::fs::create_dir_all(batch.join(&rel).parent().unwrap()).unwrap();
        std::fs::create_dir_all(batch.join("snapshots")).unwrap();
        std::fs::rename(store.snap_path(&b.id), batch.join("snapshots").join(format!("{}.json", b.id))).unwrap();
        std::fs::rename(&obj, batch.join(&rel)).unwrap();
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.recover_deleted(), 1);
        assert_eq!(store.list().len(), 2, "the snapshot is back");
        assert!(store.verify(&b.id, &mut |_, _, _| {}).unwrap().ok, "with its files");
        assert!(store.verify(&a.id, &mut |_, _, _| {}).unwrap().ok);
        assert!(!store.root.join("deleted").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Moves a small temp snapshot to the real Recycle Bin, so it is ignored by default.
    /// `cargo test --lib delete_to_trash_moves_snapshot -- --ignored`
    #[test]
    #[ignore]
    fn delete_to_trash_moves_snapshot() {
        let root = tmp("trashreal");
        let src = fake_saved_games(&root);
        let sg = PathBuf::from(&src[0].root);
        let store = Store::new(root.join("store"));
        let a = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        put(&sg, "Missions/b.miz", "hangar test file, safe to delete");
        let b = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        let (done, skipped, freed) = store.delete_to_trash(&[b.id.clone()], &mut |_, _, _| {}).unwrap();
        assert_eq!((done.len(), skipped.len()), (1, 0));
        assert_eq!(freed, "hangar test file, safe to delete".len() as u64);
        assert_eq!(store.list().len(), 1);
        assert!(!store.root.join("deleted").exists(), "the batch folder went to the Recycle Bin");
        assert!(store.verify(&a.id, &mut |_, _, _| {}).unwrap().ok, "shared files stay");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn compressed_objects_verify_and_restore() {
        let root = tmp("compressed");
        let src = fake_saved_games(&root);
        let sg = PathBuf::from(&src[0].root);
        let big = "-- options\n".repeat(5000);
        put(&sg, "Config/big.lua", &big);
        put(&sg, "Missions/packed.miz", &"x".repeat(9000));
        // an old plain store first, then compression switched on: both kinds must work
        let plain = Store::new(root.join("store"));
        let a = plain.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        put(&sg, "Config/big.lua", &format!("{big}-- changed\n"));
        let store = Store::new(root.join("store")).with_compression(&Compression::default());
        let b = store.snapshot(&src, "all", "Manual", &[], &mut |_, _, _| {}).unwrap();
        let sha_big = b.files.iter().find(|f| f.path == "Config/big.lua").unwrap().sha256.clone();
        let sha_miz = b.files.iter().find(|f| f.path == "Missions/packed.miz").unwrap().sha256.clone();
        assert!(store.object_path(&sha_big).to_string_lossy().ends_with(".zst"), "text is compressed");
        assert!(!store.object_path(&sha_miz).to_string_lossy().ends_with(".zst"), ".miz is skipped");
        assert!(b.added_bytes < big.len() as u64 / 5, "added {} bytes", b.added_bytes);
        assert!(store.verify(&a.id, &mut |_, _, _| {}).unwrap().ok);
        assert!(store.verify(&b.id, &mut |_, _, _| {}).unwrap().ok);
        put(&sg, "Config/big.lua", "CURRENT");
        let r = store.restore(&b.id, "all", None, &[], &mut |_, _, _| {}).unwrap();
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert_eq!(std::fs::read_to_string(sg.join("Config/big.lua")).unwrap(), format!("{big}-- changed\n"));
        // garbage collection understands ".zst" names
        store.set_pinned(&a.id, false).unwrap();
        let r2 = store.list().into_iter().find(|s| s.reason == "Before restore").unwrap();
        store.delete(&r2.id).unwrap();
        store.delete(&b.id).unwrap();
        assert!(!store.object_path(&sha_big).exists(), "an unused compressed object is collected");
        assert!(store.verify(&a.id, &mut |_, _, _| {}).unwrap().ok);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn snapshots_in_the_same_second_get_their_own_ids() {
        let root = tmp("same-second");
        let src = fake_saved_games(&root);
        let store = Store::new(root.join("store"));
        let ids: Vec<String> = (0..5).map(|_| store.snapshot(&src, "config", "Manual", &[], &mut |_, _, _| {}).unwrap().id).collect();
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), 5, "{ids:?}");
        assert_eq!(store.list().len(), 5);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keep_rule() {
        let day = 86_400u64;
        let base = 1_798_412_461u64; // 2026-12-27
        let mk = |i: u64, pinned: bool| Snapshot {
            id: format!("s{i}"),
            created_unix: base - i * 7 * day,
            reason: String::new(),
            scope: "all".into(),
            sources: vec![],
            files: vec![],
            total_bytes: 0,
            added_bytes: 0,
            pinned,
            verified: None,
            skipped: vec![],
        };
        // weekly snapshots for a year, one old pinned
        let list: Vec<Snapshot> = (0..52).map(|i| mk(i, i == 50)).collect();
        let keep = keep_set(&list);
        for i in 0..10 {
            assert!(keep.contains(&format!("s{i}")));
        }
        assert!(keep.contains("s50"), "pinned is kept");
        assert!(!keep.contains("s40"), "old unpinned goes");
        // about 10 recent + one per month for the older months within 6 months + pinned
        assert!(keep.len() <= 10 + 6 + 1, "{}", keep.len());
    }
}
