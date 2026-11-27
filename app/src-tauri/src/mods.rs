// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Mod manager (B-005): a mod is a set of files in Saved Games\DCS that Hangar tracks in a
//! manifest. Switching it off moves its files into Hangar's vault, switching it on moves them
//! back. Nothing in the DCS install folder is ever touched.
//!
//!   <data>/mods/<id>.json          manifest
//!   <data>/mods/<id>.journal       moves of an operation in progress (rolled back at start)
//!   <data>/vault/<id>/<path>       files of a switched-off mod
//!   <data>/vault/<id>/.displaced/  files of other mods this mod covered while on

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compress;
use crate::settings::{Compression, now_unix, slug, write_json};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
    /// Relative to Saved Games\DCS, with '/'.
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    /// "Aircraft" | "Tech" | "Terrain" | "Service" | "Hook" | "Livery" | "Kneeboard" | "Other"
    pub kind: String,
    /// Main folder or file, relative to Saved Games\DCS.
    pub root: String,
    pub enabled: bool,
    pub added_unix: u64,
    pub files: Vec<ModFile>,
    /// Files of other mods moved aside while this one is on.
    #[serde(default)]
    pub displaced: Vec<String>,
}

/// A folder or file in Saved Games that looks like a mod.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub root: String,
    pub name: String,
    pub kind: String,
    pub bytes: u64,
    pub files: usize,
    /// Manifest id if Hangar already manages it.
    pub managed: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub root: String,
    pub enabled: bool,
    pub added_unix: u64,
    pub bytes: u64,
    pub files: usize,
    pub sample_files: Vec<String>,
    /// Paths shared with other managed mods.
    pub conflicts: Vec<String>,
    pub conflicts_with: Vec<String>,
    pub vault: String,
}

/// Where each kind of mod lives (relative to Saved Games\DCS) and how deep its root is.
const PLACES: [(&str, &str); 6] = [
    ("Mods/aircraft", "Aircraft"),
    ("Mods/tech", "Tech"),
    ("Mods/terrains", "Terrain"),
    ("Mods/services", "Service"),
    ("Kneeboard", "Kneeboard"),
    ("Scripts/Hooks", "Hook"),
];
/// Top-level folders a mod package may contain.
const PACKAGE_ROOTS: [&str; 6] = ["mods", "scripts", "liveries", "kneeboard", "missions", "config"];

pub struct ModStore {
    pub saved_games: PathBuf,
    pub data: PathBuf,
    /// Compress the files of switched-off mods in the vault (Settings > Storage, off by default).
    pub vault_compression: Option<Compression>,
}

/// How a mod is removed (B-030).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RemoveMode {
    /// Windows Recycle Bin (the default; can be undone).
    RecycleBin,
    /// Deleted for good (Shift + the confirmation phrase).
    Permanent,
}

fn mtime_of(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_))) { Some(p.to_path_buf()) } else { None }
}

fn list_files(root_abs: &Path, base: &Path) -> Vec<ModFile> {
    let mut out = Vec::new();
    let mut stack = vec![root_abs.to_path_buf()];
    if root_abs.is_file() {
        if let (Ok(md), Ok(rel)) = (std::fs::metadata(root_abs), root_abs.strip_prefix(base)) {
            out.push(ModFile { path: rel.to_string_lossy().replace('\\', "/"), size: md.len(), mtime: mtime_of(&md) });
        }
        return out;
    }
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                stack.push(e.path());
            } else if let Ok(rel) = e.path().strip_prefix(base) {
                out.push(ModFile { path: rel.to_string_lossy().replace('\\', "/"), size: md.len(), mtime: mtime_of(&md) });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Moves a file, also across volumes (copy + delete). Creates the target folder.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

/// Removes empty folders from `start` upwards, but never `stop` or above it.
fn prune_empty(start: &Path, stop: &Path) {
    let mut cur = start.to_path_buf();
    while cur.starts_with(stop) && cur != stop {
        if std::fs::remove_dir(&cur).is_err() {
            break; // not empty
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => break,
        }
    }
}

impl ModStore {
    pub fn new(saved_games: impl Into<PathBuf>, data: impl Into<PathBuf>) -> Self {
        ModStore { saved_games: saved_games.into(), data: data.into(), vault_compression: None }
    }

    pub fn with_compression(mut self, cfg: &Compression) -> Self {
        self.vault_compression = (cfg.vault && cfg.algorithm != "none").then(|| cfg.clone());
        self
    }

    /// The file of a mod in the vault: plain, or compressed (".zst"), if either exists.
    fn vault_file(&self, id: &str, rel: &Path) -> Option<PathBuf> {
        let plain = self.vault_dir(id).join(rel);
        if plain.exists() {
            return Some(plain);
        }
        let z = compress::zst_path(&plain);
        z.exists().then_some(z)
    }

    /// Compresses the vault files of a switched-off mod (if vault compression is on and the mod
    /// is within the size limit). Returns (bytes before, bytes after). Each file is written as
    /// ".zst" first and the plain one removed after, so an interruption loses nothing.
    pub fn compress_vault(&self, id: &str) -> Result<(u64, u64), String> {
        let Some(cfg) = self.vault_compression.clone() else { return Ok((0, 0)) };
        let m = self.load(id)?;
        if m.enabled {
            return Ok((0, 0));
        }
        let total: u64 = m.files.iter().map(|f| f.size).sum();
        if cfg.vault_max_gb.is_some_and(|gb| total as f64 > gb * 1e9) {
            return Ok((0, 0));
        }
        let vault = self.vault_dir(id);
        let (mut before, mut after) = (0u64, 0u64);
        for f in &m.files {
            let rel = safe_rel(&f.path).ok_or("bad path")?;
            let plain = vault.join(&rel);
            let z = compress::zst_path(&plain);
            if !plain.exists() {
                continue; // already compressed
            }
            let size = std::fs::metadata(&plain).map(|m| m.len()).unwrap_or(0);
            if !compress::worth_it(&cfg, &plain, size) {
                continue;
            }
            let packed = compress::compress_file(&cfg, &plain, &z).map_err(|e| format!("Could not compress {}: {e}", f.path))?;
            if packed >= size - size / 32 {
                let _ = std::fs::remove_file(&z); // not worth it
                continue;
            }
            std::fs::remove_file(&plain).map_err(|e| e.to_string())?;
            before += size;
            after += packed;
        }
        Ok((before, after))
    }

    /// Bytes the vault uses on disk (compressed files count as stored).
    pub fn vault_bytes(&self) -> u64 {
        crate::winutil::folder_size(&self.data.join("vault"))
    }

    fn manifest_path(&self, id: &str) -> PathBuf {
        self.data.join("mods").join(format!("{id}.json"))
    }
    fn journal_path(&self, id: &str) -> PathBuf {
        self.data.join("mods").join(format!("{id}.journal"))
    }
    pub fn vault_dir(&self, id: &str) -> PathBuf {
        self.data.join("vault").join(id)
    }

    /// Highest folder that may be removed when it becomes empty: inside the mod's own folder
    /// only, so standard folders like Mods/aircraft or Scripts/Hooks always stay.
    fn prune_stop(&self, root: &str, file: &str) -> PathBuf {
        let parent = root.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        if !parent.is_empty() && file.to_ascii_lowercase().starts_with(&format!("{}/", parent.to_ascii_lowercase())) {
            self.saved_games.join(parent)
        } else {
            // a file outside the root: keep its top-level folder and the one below
            let segs: Vec<&str> = file.split('/').collect();
            self.saved_games.join(segs[..segs.len().saturating_sub(1).min(2)].join("/"))
        }
    }

    pub fn manifests(&self) -> Vec<Manifest> {
        let mut v: Vec<Manifest> = std::fs::read_dir(self.data.join("mods"))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                    .filter_map(|e| std::fs::read(e.path()).ok())
                    .filter_map(|b| serde_json::from_slice::<Manifest>(&b).ok())
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        v
    }

    pub fn load(&self, id: &str) -> Result<Manifest, String> {
        let b = std::fs::read(self.manifest_path(id)).map_err(|e| format!("Mod {id}: {e}"))?;
        serde_json::from_slice(&b).map_err(|e| format!("Mod {id}: {e}"))
    }

    fn save(&self, m: &Manifest) -> Result<(), String> {
        write_json(&self.manifest_path(&m.id), m)
    }

    /// Folders/files in Saved Games that look like mods (read only).
    pub fn scan(&self) -> Vec<Candidate> {
        let managed: HashMap<String, String> = self.manifests().into_iter().map(|m| (m.root.to_ascii_lowercase(), m.id)).collect();
        let mut out = Vec::new();
        let mut push = |rel: String, name: String, kind: &str, abs: &Path| {
            let files = list_files(abs, &self.saved_games);
            out.push(Candidate {
                managed: managed.get(&rel.to_ascii_lowercase()).cloned(),
                bytes: files.iter().map(|f| f.size).sum(),
                files: files.len(),
                root: rel,
                name,
                kind: kind.into(),
            });
        };
        for (place, kind) in PLACES {
            let dir = self.saved_games.join(place);
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let is_dir = e.path().is_dir();
                if kind == "Hook" && (is_dir || !name.to_ascii_lowercase().ends_with(".lua")) {
                    continue;
                }
                if kind != "Hook" && !is_dir {
                    continue;
                }
                push(format!("{place}/{name}"), name.trim_end_matches(".lua").to_string(), kind, &e.path());
            }
        }
        // Liveries/<unit>: one row per unit (all its liveries), unless Hangar manages single
        // liveries of that unit (then those stay as they are)
        for unit in std::fs::read_dir(self.saved_games.join("Liveries")).into_iter().flatten().flatten() {
            if !unit.path().is_dir() {
                continue;
            }
            let u = unit.file_name().to_string_lossy().into_owned();
            let prefix = format!("liveries/{}/", u.to_ascii_lowercase());
            let single: Vec<String> = managed.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
            if single.is_empty() {
                let n = std::fs::read_dir(unit.path()).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).count();
                push(format!("Liveries/{u}"), format!("{u} liveries ({n})"), "Livery", &unit.path());
            } else {
                for liv in std::fs::read_dir(unit.path()).into_iter().flatten().flatten() {
                    if liv.path().is_dir() {
                        let l = liv.file_name().to_string_lossy().into_owned();
                        push(format!("Liveries/{u}/{l}"), format!("{l} ({u})"), "Livery", &liv.path());
                    }
                }
            }
        }
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
        out
    }

    fn new_id(&self, name: &str) -> String {
        let base = slug(name);
        let mut id = base.clone();
        let mut i = 2;
        while self.manifest_path(&id).exists() {
            id = format!("{base}-{i}");
            i += 1;
        }
        id
    }

    /// Starts tracking a folder/file that is already in Saved Games. Writes only the manifest.
    pub fn adopt(&self, root: &str, name: &str, kind: &str) -> Result<Manifest, String> {
        let rel = safe_rel(root).ok_or("Not a folder inside Saved Games.")?;
        let abs = self.saved_games.join(&rel);
        if !abs.exists() {
            return Err(format!("{root} was not found in Saved Games."));
        }
        if self.manifests().iter().any(|m| m.root.eq_ignore_ascii_case(root)) {
            return Err("Hangar already manages this mod.".into());
        }
        let files = list_files(&abs, &self.saved_games);
        if files.is_empty() {
            return Err("The folder is empty.".into());
        }
        let m = Manifest {
            id: self.new_id(name),
            name: name.to_string(),
            kind: kind.to_string(),
            root: root.replace('\\', "/"),
            enabled: true,
            added_unix: now_unix(),
            files,
            displaced: vec![],
        };
        self.save(&m)?;
        Ok(m)
    }

    /// Copies a mod package (folder or .zip) into Saved Games and tracks it. The package must
    /// contain one of the usual top folders (Mods, Scripts, Liveries, ...), possibly one level
    /// down. Refuses to overwrite existing files.
    pub fn add_package(&self, package: &Path, progress: &mut dyn FnMut(usize, usize, &str)) -> Result<Manifest, String> {
        let entries = package_entries(package)?;
        let layout = package_layout(&entries)
            .ok_or("Can't tell where this mod goes: it has no Mods, Scripts, Liveries or Kneeboard folder, and no entry.lua.")?;
        let (files, display_name): (Vec<(String, String)>, Option<String>) = match &layout {
            Layout::Rooted(base) => (
                entries
                    .iter()
                    .filter_map(|e| e.strip_prefix(base.as_str()).map(|r| (e.clone(), r.trim_start_matches('/').to_string())))
                    .filter(|(_, r)| !r.is_empty() && !r.ends_with('/'))
                    // only files under the usual DCS folders; readmes etc. stay out of Saved Games
                    .filter(|(_, r)| r.contains('/') && PACKAGE_ROOTS.contains(&r.split('/').next().unwrap_or("").to_ascii_lowercase().as_str()))
                    .collect(),
                None,
            ),
            Layout::ModFolder { prefix, folder } => {
                // a bare mod folder (entry.lua at its top, e.g. CurrentHill asset packs): the whole
                // folder goes to Mods/<aircraft|tech>/<folder>, as its readme says
                let entry = read_package_file(package, &format!("{prefix}entry.lua")).unwrap_or_default();
                let place = if entry.contains("make_flyable") { "Mods/aircraft" } else { "Mods/tech" };
                let folder = folder.clone().unwrap_or_else(|| package.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Mod".into()));
                (
                    entries
                        .iter()
                        .filter_map(|e| e.strip_prefix(prefix.as_str()).map(|r| (e.clone(), r.to_string())))
                        .filter(|(_, r)| !r.is_empty() && !r.ends_with('/'))
                        .map(|(e, r)| (e, format!("{place}/{folder}/{r}")))
                        .collect(),
                    lua_display_name(&entry),
                )
            }
        };
        if files.is_empty() {
            return Err("The package has no files.".into());
        }
        for (_, rel) in &files {
            let p = safe_rel(rel).ok_or(format!("Unsafe path in package: {rel}"))?;
            if self.saved_games.join(p).exists() {
                return Err(format!("Would overwrite an existing file: {rel}. Remove or switch off the old version first."));
            }
        }
        let root = mod_root(files.iter().map(|(_, r)| r.as_str()));
        let name = display_name.unwrap_or_else(|| root.rsplit('/').next().unwrap_or("Mod").trim_end_matches(".lua").to_string());
        let total = files.len();
        let kind = kind_of(&root);
        // journal the copies so an interruption can be cleaned up
        let id = self.new_id(&name);
        let mut journal = Journal::open(&self.journal_path(&id))?;
        let res = (|| -> Result<(), String> {
            if package.is_dir() {
                for (i, (src, rel)) in files.iter().enumerate() {
                    progress(i, total, rel);
                    let to = self.saved_games.join(safe_rel(rel).unwrap());
                    std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
                    std::fs::copy(package.join(safe_rel(src).ok_or("bad path")?), &to).map_err(|e| e.to_string())?;
                    journal.created(&to)?;
                }
            } else {
                let f = std::fs::File::open(package).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
                for (i, (src, rel)) in files.iter().enumerate() {
                    progress(i, total, rel);
                    let to = self.saved_games.join(safe_rel(rel).unwrap());
                    std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
                    let mut entry = zip.by_name(src).map_err(|e| e.to_string())?;
                    let mut out = std::fs::File::create(&to).map_err(|e| e.to_string())?;
                    std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                    journal.created(&to)?;
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            drop(journal);
            self.recover();
            return Err(e);
        }
        let mut tracked: Vec<ModFile> = files
            .iter()
            .filter_map(|(_, rel)| {
                let md = std::fs::metadata(self.saved_games.join(rel)).ok()?;
                Some(ModFile { path: rel.clone(), size: md.len(), mtime: mtime_of(&md) })
            })
            .collect();
        tracked.sort_by(|a, b| a.path.cmp(&b.path));
        let m = Manifest { id: id.clone(), name, kind, root, enabled: true, added_unix: now_unix(), files: tracked, displaced: vec![] };
        self.save(&m)?;
        journal.finish();
        Ok(m)
    }

    /// Switches a mod off: moves its files into the vault and puts back files it covered.
    /// Refuses (unless `force`) if a file changed since Hangar recorded it.
    pub fn disable(&self, id: &str, force: bool) -> Result<Manifest, String> {
        let mut m = self.load(id)?;
        if !m.enabled {
            return Ok(m);
        }
        let changed: Vec<&str> = m
            .files
            .iter()
            .filter(|f| match std::fs::metadata(self.saved_games.join(&f.path)) {
                Ok(md) => md.len() != f.size || mtime_of(&md) != f.mtime,
                Err(_) => false,
            })
            .map(|f| f.path.as_str())
            .collect();
        if !changed.is_empty() && !force {
            return Err(format!("CHANGED:{} file(s) changed since Hangar recorded this mod, e.g. {}", changed.len(), changed[0]));
        }
        let vault = self.vault_dir(id);
        let mut journal = Journal::open(&self.journal_path(id))?;
        for f in &m.files {
            let rel = safe_rel(&f.path).ok_or("bad path")?;
            let from = self.saved_games.join(&rel);
            if from.exists() {
                let to = vault.join(&rel);
                let _ = std::fs::remove_file(compress::zst_path(&to)); // stale compressed copy
                if let Err(e) = move_file(&from, &to) {
                    drop(journal);
                    self.recover();
                    return Err(format!("Could not move {}: {e}", f.path));
                }
                journal.moved(&from, &to)?;
                if let Some(parent) = from.parent() {
                    prune_empty(parent, &self.prune_stop(&m.root, &f.path));
                }
            }
        }
        for p in &m.displaced {
            let rel = safe_rel(p).ok_or("bad path")?;
            let from = vault.join(".displaced").join(&rel);
            let to = self.saved_games.join(&rel);
            if from.exists() && !to.exists() {
                move_file(&from, &to).map_err(|e| e.to_string())?;
                journal.moved(&from, &to)?;
            }
        }
        // remember current size/mtime so switching on again is not reported as a change
        for f in m.files.iter_mut() {
            if let Ok(md) = std::fs::metadata(vault.join(&f.path)) {
                f.size = md.len();
                f.mtime = mtime_of(&md);
            }
        }
        m.enabled = false;
        m.displaced.clear();
        self.save(&m)?;
        journal.finish();
        if self.vault_compression.is_some() {
            let _ = self.compress_vault(id); // best effort: the plain files stay if it fails
        }
        Ok(m)
    }

    /// Switches a mod on: moves its files back. Files of other mods in the way are kept in this
    /// mod's vault and come back when it is switched off ("switched on last wins").
    pub fn enable(&self, id: &str) -> Result<Manifest, String> {
        let mut m = self.load(id)?;
        if m.enabled {
            return Ok(m);
        }
        let vault = self.vault_dir(id);
        let missing: Vec<&str> = m
            .files
            .iter()
            .filter(|f| safe_rel(&f.path).and_then(|r| self.vault_file(id, &r)).is_none())
            .map(|f| f.path.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(format!("{} file(s) are missing from the vault, e.g. {}", missing.len(), missing[0]));
        }
        let mut journal = Journal::open(&self.journal_path(id))?;
        let mut displaced = Vec::new();
        let mut unpacked: Vec<PathBuf> = Vec::new();
        for f in &m.files {
            let rel = safe_rel(&f.path).ok_or("bad path")?;
            let target = self.saved_games.join(&rel);
            if target.exists() {
                let aside = vault.join(".displaced").join(&rel);
                if let Err(e) = move_file(&target, &aside) {
                    drop(journal);
                    self.recover();
                    return Err(format!("Could not move {} aside: {e}", f.path));
                }
                journal.moved(&target, &aside)?;
                displaced.push(f.path.clone());
            }
            let from = self.vault_file(id, &rel).unwrap_or_else(|| vault.join(&rel));
            if compress::is_zst(&from) && !vault.join(&rel).exists() {
                // compressed in the vault: unpack into place, keep the .zst until all is done
                if let Err(e) = compress::decode_to(&from, &target) {
                    drop(journal);
                    self.recover();
                    return Err(format!("Could not unpack {}: {e}", f.path));
                }
                journal.created(&target)?;
                if let Ok(file) = std::fs::File::options().write(true).open(&target) {
                    let _ = file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(f.mtime.max(0) as u64));
                }
                unpacked.push(from);
                continue;
            }
            if let Err(e) = move_file(&from, &target) {
                drop(journal);
                self.recover();
                return Err(format!("Could not move {}: {e}", f.path));
            }
            journal.moved(&from, &target)?;
            if let Some(parent) = from.parent() {
                prune_empty(parent, &vault);
            }
        }
        for f in m.files.iter_mut() {
            if let Ok(md) = std::fs::metadata(self.saved_games.join(&f.path)) {
                f.size = md.len();
                f.mtime = mtime_of(&md);
            }
        }
        m.enabled = true;
        m.displaced = displaced;
        self.save(&m)?;
        journal.finish();
        for z in unpacked {
            let _ = std::fs::remove_file(&z);
            if let Some(parent) = z.parent() {
                prune_empty(parent, &vault);
            }
        }
        Ok(m)
    }

    /// Stops tracking a mod that is on. Its files stay in Saved Games.
    pub fn forget(&self, id: &str) -> Result<(), String> {
        let m = self.load(id)?;
        if !m.enabled {
            return Err("Switch the mod on first, or remove it.".into());
        }
        std::fs::remove_file(self.manifest_path(id)).map_err(|e| e.to_string())
    }

    /// Removes a mod: puts back files it covered, then sends its files to the Recycle Bin
    /// (default) or deletes them for good, and forgets it (B-030).
    /// A mod that is on is recycled from Saved Games, so "Restore" in the Recycle Bin puts it
    /// back where it was. A mod that is off is recycled from the vault (unpacked first if it
    /// was compressed, so the files stay usable).
    pub fn remove(&self, id: &str, mode: RemoveMode) -> Result<(), String> {
        let m = self.load(id)?;
        let vault = self.vault_dir(&m.id);
        if mode == RemoveMode::RecycleBin {
            if m.enabled {
                let own: Vec<PathBuf> = m.files.iter().filter_map(|f| safe_rel(&f.path)).map(|r| self.saved_games.join(r)).filter(|p| p.exists()).collect();
                crate::trash::recycle(&own)?;
                for f in &m.files {
                    if let Some(parent) = safe_rel(&f.path).and_then(|r| self.saved_games.join(r).parent().map(Path::to_path_buf)) {
                        prune_empty(&parent, &self.prune_stop(&m.root, &f.path));
                    }
                }
                for p in &m.displaced {
                    let rel = safe_rel(p).ok_or("bad path")?;
                    let from = vault.join(".displaced").join(&rel);
                    let to = self.saved_games.join(&rel);
                    if from.exists() && !to.exists() {
                        move_file(&from, &to).map_err(|e| e.to_string())?;
                    }
                }
                let _ = std::fs::remove_dir_all(&vault);
            } else {
                for f in &m.files {
                    let Some(rel) = safe_rel(&f.path) else { continue };
                    let plain = vault.join(&rel);
                    let z = compress::zst_path(&plain);
                    if !plain.exists() && z.exists() {
                        compress::decode_to(&z, &plain).map_err(|e| format!("Could not unpack {}: {e}", f.path))?;
                        let _ = std::fs::remove_file(&z);
                    }
                }
                if vault.exists() {
                    crate::trash::recycle(&[vault.clone()])?;
                }
            }
        } else {
            let m = if m.enabled { self.disable(id, true)? } else { m };
            let vault = self.vault_dir(&m.id);
            std::fs::remove_dir_all(&vault).or_else(|e| if vault.exists() { Err(e) } else { Ok(()) }).map_err(|e| e.to_string())?;
        }
        std::fs::remove_file(self.manifest_path(id)).map_err(|e| e.to_string())
    }

    /// Rolls back any operation that was interrupted (journal left behind).
    pub fn recover(&self) -> usize {
        let mut n = 0;
        for e in std::fs::read_dir(self.data.join("mods")).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "journal") {
                n += Journal::roll_back(&e.path());
            }
        }
        n
    }

    pub fn views(&self) -> Vec<ModView> {
        let all = self.manifests();
        let mut owners: HashMap<String, Vec<String>> = HashMap::new();
        for m in &all {
            for f in &m.files {
                owners.entry(f.path.to_ascii_lowercase()).or_default().push(m.id.clone());
            }
        }
        all.iter()
            .map(|m| {
                let mut conflicts = Vec::new();
                let mut with: HashSet<String> = HashSet::new();
                for f in &m.files {
                    if let Some(o) = owners.get(&f.path.to_ascii_lowercase()) {
                        if o.len() > 1 {
                            conflicts.push(f.path.clone());
                            with.extend(o.iter().filter(|x| **x != m.id).cloned());
                        }
                    }
                }
                let names: HashMap<&str, &str> = all.iter().map(|x| (x.id.as_str(), x.name.as_str())).collect();
                ModView {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    kind: m.kind.clone(),
                    root: m.root.clone(),
                    enabled: m.enabled,
                    added_unix: m.added_unix,
                    bytes: m.files.iter().map(|f| f.size).sum(),
                    files: m.files.len(),
                    sample_files: m.files.iter().take(8).map(|f| f.path.clone()).collect(),
                    conflicts,
                    conflicts_with: with.iter().map(|i| names.get(i.as_str()).unwrap_or(&i.as_str()).to_string()).collect(),
                    vault: self.vault_dir(&m.id).to_string_lossy().into_owned(),
                }
            })
            .collect()
    }

    /// Applies a mod set: everything in `enabled` on, every other managed mod off.
    pub fn apply_set(&self, enabled: &[String]) -> Result<Vec<String>, String> {
        let want: HashSet<&str> = enabled.iter().map(String::as_str).collect();
        let all = self.manifests();
        let mut errors = Vec::new();
        for m in all.iter().filter(|m| m.enabled && !want.contains(m.id.as_str())) {
            if let Err(e) = self.disable(&m.id, false) {
                errors.push(format!("{}: {e}", m.name));
            }
        }
        for m in all.iter().filter(|m| !m.enabled && want.contains(m.id.as_str())) {
            if let Err(e) = self.enable(&m.id) {
                errors.push(format!("{}: {e}", m.name));
            }
        }
        Ok(errors)
    }
}

/// Append-only log of file moves of one operation. Deleted when the operation completes;
/// if it is still there at start, the moves are undone in reverse order.
struct Journal {
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl Journal {
    fn open(path: &Path) -> Result<Self, String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
        Ok(Journal { path: path.to_path_buf(), file: Some(file) })
    }
    fn line(&mut self, l: String) -> Result<(), String> {
        let f = self.file.as_mut().ok_or("journal closed")?;
        f.write_all(l.as_bytes()).and_then(|_| f.write_all(b"\n")).and_then(|_| f.sync_data()).map_err(|e| e.to_string())
    }
    fn moved(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        self.line(format!("M\t{}\t{}", from.display(), to.display()))
    }
    fn created(&mut self, p: &Path) -> Result<(), String> {
        self.line(format!("C\t{}", p.display()))
    }
    fn finish(mut self) {
        self.file = None;
        let _ = std::fs::remove_file(&self.path);
    }
    fn roll_back(path: &Path) -> usize {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut n = 0;
        for l in text.lines().rev() {
            let parts: Vec<&str> = l.split('\t').collect();
            match parts.as_slice() {
                ["M", from, to] => {
                    if Path::new(to).exists() && !Path::new(from).exists() && move_file(Path::new(to), Path::new(from)).is_ok() {
                        n += 1;
                    }
                }
                ["C", p] => {
                    if std::fs::remove_file(p).is_ok() {
                        n += 1;
                    }
                }
                _ => {}
            }
        }
        let _ = std::fs::remove_file(path);
        n
    }
}

/// Entry names of a folder or zip, with '/' (folders end with '/').
fn package_entries(package: &Path) -> Result<Vec<String>, String> {
    if package.is_dir() {
        let mut v: Vec<String> = list_files(package, package).into_iter().map(|f| f.path).collect();
        v.sort();
        return Ok(v);
    }
    let f = std::fs::File::open(package).map_err(|e| format!("{}: {e}", package.display()))?;
    let zip = zip::ZipArchive::new(f).map_err(|e| format!("Not a zip file: {e}"))?;
    Ok(zip.file_names().map(|s| s.replace('\\', "/")).collect())
}

/// How a package is laid out.
#[derive(Debug, PartialEq)]
enum Layout {
    /// Its DCS folders (Mods, Liveries, ...) start under this prefix ("" or "MyMod/").
    Rooted(String),
    /// A bare mod folder: entry.lua directly under `prefix` ("" = at the top of the package,
    /// folder named after the package; "X/" = folder X).
    ModFolder { prefix: String, folder: Option<String> },
}

fn package_layout(entries: &[String]) -> Option<Layout> {
    let is_root = |seg: &str| PACKAGE_ROOTS.contains(&seg.to_ascii_lowercase().as_str());
    let segs_of = |e: &String| e.split('/').map(str::to_string).collect::<Vec<String>>();
    if entries.iter().any(|e| {
        let s = segs_of(e);
        s.len() > 1 && is_root(&s[0])
    }) {
        return Some(Layout::Rooted(String::new()));
    }
    if entries.iter().any(|e| e.eq_ignore_ascii_case("entry.lua")) {
        return Some(Layout::ModFolder { prefix: String::new(), folder: None });
    }
    // "X/entry.lua" is checked before "X/Liveries/...": a mod folder can carry its own Liveries
    if let Some(e) = entries.iter().find(|e| {
        let s = segs_of(e);
        s.len() == 2 && s[1].eq_ignore_ascii_case("entry.lua")
    }) {
        let folder = e.split('/').next().unwrap_or("Mod").to_string();
        return Some(Layout::ModFolder { prefix: format!("{folder}/"), folder: Some(folder) });
    }
    for e in entries {
        let s = segs_of(e);
        if s.len() > 2 && is_root(&s[1]) {
            return Some(Layout::Rooted(format!("{}/", s[0])));
        }
    }
    // a mod folder wrapped once more ("Pack 2.0/Pack 2.0/entry.lua"): the shallowest entry.lua
    if let Some(e) = entries
        .iter()
        .filter(|e| {
            let s = segs_of(e);
            (3..=4).contains(&s.len()) && s.last().is_some_and(|l| l.eq_ignore_ascii_case("entry.lua"))
        })
        .min_by_key(|e| e.len())
    {
        let prefix = &e[..e.len() - "entry.lua".len()];
        let folder = prefix.trim_end_matches('/').rsplit('/').next().unwrap_or("Mod").to_string();
        return Some(Layout::ModFolder { prefix: prefix.to_string(), folder: Some(folder) });
    }
    None
}

/// Reads one text file from a package (folder or zip), at most 1 MB.
fn read_package_file(package: &Path, rel: &str) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    if package.is_dir() {
        std::fs::File::open(package.join(safe_rel(rel)?)).ok()?.take(1 << 20).read_to_end(&mut buf).ok()?;
    } else {
        let mut z = zip::ZipArchive::new(std::fs::File::open(package).ok()?).ok()?;
        z.by_name(rel).ok()?.take(1 << 20).read_to_end(&mut buf).ok()?;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// `displayName = _("...")` from an entry.lua, if it is plain text.
fn lua_display_name(entry: &str) -> Option<String> {
    let i = entry.find("displayName")?;
    let rest = &entry[i..];
    let q = rest.find('"')?;
    let end = rest[q + 1..].find('"')?;
    let name = rest[q + 1..q + 1 + end].trim();
    (!name.is_empty() && name.len() <= 80 && !name.contains('\n')).then(|| name.to_string())
}

/// The mod's main folder: e.g. Mods/aircraft/A-4E-C, Liveries/F-16C_50/331, Scripts/Hooks/x.lua.
fn mod_root<'a>(rels: impl Iterator<Item = &'a str>) -> String {
    let cands: Vec<String> = rels
        .map(|r| {
            let segs: Vec<&str> = r.split('/').collect();
            let l0 = segs[0].to_ascii_lowercase();
            let l1 = segs.get(1).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
            if segs.len() > 2 && (l0 == "mods" || l0 == "liveries" || (l0 == "scripts" && l1 == "hooks")) {
                segs[..3].join("/")
            } else {
                segs[..segs.len().min(2)].join("/")
            }
        })
        .collect();
    let known = cands.iter().filter(|c| kind_of(c) != "Other").min_by_key(|c| c.len());
    known.or_else(|| cands.iter().min_by_key(|c| c.len())).cloned().unwrap_or_else(|| "Mod".into())
}

fn kind_of(root: &str) -> String {
    let l = root.to_ascii_lowercase();
    for (place, kind) in PLACES {
        if l.starts_with(&format!("{}/", place.to_ascii_lowercase())) {
            return kind.into();
        }
    }
    if l.starts_with("liveries/") {
        return "Livery".into();
    }
    "Other".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hangar-mods-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn put(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    fn setup(name: &str) -> (PathBuf, ModStore) {
        let root = tmp(name);
        let sg = root.join("Saved Games").join("DCS");
        put(&sg, "Mods/aircraft/A-4E-C/entry.lua", "A4");
        put(&sg, "Mods/aircraft/A-4E-C/Cockpit/x.lua", "COCKPIT");
        put(&sg, "Scripts/Hooks/SRS-AutoConnect.lua", "SRS");
        put(&sg, "Liveries/F-16C_50/331sq/description.lua", "LIV331");
        put(&sg, "Config/options.lua", "OPTIONS");
        let store = ModStore::new(&sg, root.join("data"));
        (root, store)
    }

    #[test]
    fn scan_finds_mods_and_marks_managed() {
        let (root, store) = setup("scan");
        let c = store.scan();
        let roots: Vec<&str> = c.iter().map(|x| x.root.as_str()).collect();
        assert!(roots.contains(&"Mods/aircraft/A-4E-C"));
        assert!(roots.contains(&"Scripts/Hooks/SRS-AutoConnect.lua"));
        assert!(roots.contains(&"Liveries/F-16C_50"), "liveries are listed per unit");
        assert!(!roots.iter().any(|r| r.starts_with("Config")));
        store.adopt("Mods/aircraft/A-4E-C", "A-4E-C", "Aircraft").unwrap();
        let a4 = store.scan().into_iter().find(|x| x.root == "Mods/aircraft/A-4E-C").unwrap();
        assert_eq!(a4.managed.as_deref(), Some("a-4e-c"));
        assert_eq!(a4.files, 2);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn disable_and_enable_move_files_and_back() {
        let (root, store) = setup("toggle");
        let sg = store.saved_games.clone();
        let m = store.adopt("Mods/aircraft/A-4E-C", "A-4E-C", "Aircraft").unwrap();
        store.disable(&m.id, false).unwrap();
        assert!(!sg.join("Mods/aircraft/A-4E-C").exists(), "folder removed when empty");
        assert!(sg.join("Mods/aircraft").exists(), "parent folders stay");
        assert_eq!(std::fs::read_to_string(store.vault_dir(&m.id).join("Mods/aircraft/A-4E-C/Cockpit/x.lua")).unwrap(), "COCKPIT");
        assert!(!store.journal_path(&m.id).exists());
        store.enable(&m.id).unwrap();
        assert_eq!(std::fs::read_to_string(sg.join("Mods/aircraft/A-4E-C/Cockpit/x.lua")).unwrap(), "COCKPIT");
        assert!(store.load(&m.id).unwrap().enabled);
        assert_eq!(std::fs::read_to_string(sg.join("Config/options.lua")).unwrap(), "OPTIONS", "other files untouched");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn changed_files_need_force() {
        let (root, store) = setup("changed");
        let m = store.adopt("Scripts/Hooks/SRS-AutoConnect.lua", "SRS", "Hook").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        put(&store.saved_games, "Scripts/Hooks/SRS-AutoConnect.lua", "SRS edited by user");
        let e = store.disable(&m.id, false).unwrap_err();
        assert!(e.starts_with("CHANGED:"), "{e}");
        assert!(store.saved_games.join("Scripts/Hooks/SRS-AutoConnect.lua").exists());
        store.disable(&m.id, true).unwrap();
        assert!(!store.saved_games.join("Scripts/Hooks/SRS-AutoConnect.lua").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn conflict_last_on_wins_and_original_comes_back() {
        let (root, store) = setup("conflict");
        let sg = store.saved_games.clone();
        let a = store.adopt("Liveries/F-16C_50/331sq", "331 squadron", "Livery").unwrap();
        // a second livery pack with the same file, kept switched off in the vault
        put(&root, "pack/Liveries/F-16C_50/331sq/description.lua", "PACK VERSION");
        store.disable(&a.id, false).unwrap();
        let b = store.add_package(&root.join("pack"), &mut |_, _, _| {}).unwrap();
        store.disable(&b.id, false).unwrap();
        store.enable(&a.id).unwrap();
        // b switched on while a is on: b wins, a's file goes to b's vault
        store.enable(&b.id).unwrap();
        assert_eq!(std::fs::read_to_string(sg.join("Liveries/F-16C_50/331sq/description.lua")).unwrap(), "PACK VERSION");
        assert_eq!(store.views().iter().find(|v| v.id == a.id).unwrap().conflicts.len(), 1);
        store.disable(&b.id, false).unwrap();
        assert_eq!(std::fs::read_to_string(sg.join("Liveries/F-16C_50/331sq/description.lua")).unwrap(), "LIV331");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn add_package_from_zip_with_wrapper_folder() {
        let (root, store) = setup("zip");
        let zp = root.join("mod.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("MyMod v1.2/Mods/tech/MyTech/entry.lua", o).unwrap();
            z.write_all(b"TECH").unwrap();
            z.start_file("MyMod v1.2/README.txt", o).unwrap();
            z.write_all(b"readme").unwrap();
            z.finish().unwrap();
        }
        let m = store.add_package(&zp, &mut |_, _, _| {}).unwrap();
        assert_eq!(m.root, "Mods/tech/MyTech");
        assert_eq!(m.kind, "Tech");
        assert_eq!(m.files.len(), 1, "README outside the DCS folders is not copied: {:?}", m.files);
        assert_eq!(std::fs::read_to_string(store.saved_games.join("Mods/tech/MyTech/entry.lua")).unwrap(), "TECH");
        // adding again refuses to overwrite
        assert!(store.add_package(&zp, &mut |_, _, _| {}).unwrap_err().contains("overwrite"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_mod_folder_like_currenthill_goes_to_mods_tech() {
        let (root, store) = setup("barefolder");
        let zp = root.join("CH Pack - Full installation 1.0.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("CH Asset Pack Test 1.0/entry.lua", o).unwrap();
            z.write_all(b"declare_plugin(\"x\", { displayName = _(\"Asset Pack Test by Someone\"), })\nmount_vfs_liveries_path(current_mod_path..\"/Liveries\")").unwrap();
            z.start_file("CH Asset Pack Test 1.0/Liveries/CH_Tank/Green/description.lua", o).unwrap();
            z.write_all(b"livery").unwrap();
            z.start_file("CH Asset Pack Test 1.0/Shapes/tank.edm", o).unwrap();
            z.write_all(b"shape").unwrap();
            z.start_file("CH Asset Pack Test 1.0/README.txt", o).unwrap();
            z.write_all(b"Extract to Saved Games\\DCS\\Mods\\tech").unwrap();
            z.finish().unwrap();
        }
        let mut steps = 0;
        let m = store.add_package(&zp, &mut |_, _, _| steps += 1).unwrap();
        assert_eq!(m.root, "Mods/tech/CH Asset Pack Test 1.0");
        assert_eq!(m.kind, "Tech");
        assert_eq!(m.name, "Asset Pack Test by Someone", "the name comes from entry.lua");
        assert_eq!(m.files.len(), 4, "the whole folder, liveries and readme included");
        assert!(steps >= 4, "progress is reported per file");
        let base = store.saved_games.join("Mods/tech/CH Asset Pack Test 1.0");
        assert!(base.join("Liveries/CH_Tank/Green/description.lua").exists(), "liveries stay inside the mod folder");
        assert!(!store.saved_games.join("Liveries/CH_Tank").exists(), "nothing goes to Saved Games\\Liveries");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bare_aircraft_folder_and_unit_liveries_in_scan() {
        let (root, store) = setup("bareaircraft");
        put(&root, "pack/MyJet/entry.lua", "declare_plugin(\"MyJet\", {}) make_flyable('MyJet', nil, nil, nil)");
        put(&root, "pack/MyJet/FM/fm.lua", "fm");
        let m = store.add_package(&root.join("pack"), &mut |_, _, _| {}).unwrap();
        assert_eq!(m.root, "Mods/aircraft/MyJet");
        assert_eq!(m.kind, "Aircraft");
        // loose liveries are listed once per unit, not once per livery
        put(&store.saved_games, "Liveries/CH_Strv103/Green/description.lua", "x");
        put(&store.saved_games, "Liveries/CH_Strv103/Splinter/description.lua", "x");
        let c: Vec<_> = store.scan().into_iter().filter(|c| c.kind == "Livery" && c.root.contains("CH_")).collect();
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].root, "Liveries/CH_Strv103");
        assert_eq!(c[0].name, "CH_Strv103 liveries (2)");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn doubly_wrapped_mod_folder() {
        let (root, store) = setup("doublewrap");
        put(&root, "pack/Pack 2.0/Pack 2.0/entry.lua", "declare_plugin(\"p\", {})");
        put(&root, "pack/Pack 2.0/Pack 2.0/Shapes/a.edm", "a");
        let m = store.add_package(&root.join("pack"), &mut |_, _, _| {}).unwrap();
        assert_eq!(m.root, "Mods/tech/Pack 2.0");
        assert!(store.saved_games.join("Mods/tech/Pack 2.0/Shapes/a.edm").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn package_without_dcs_folders_is_refused() {
        let (root, store) = setup("nofolders");
        put(&root, "pack/readme.txt", "x");
        assert!(store.add_package(&root.join("pack"), &mut |_, _, _| {}).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn interrupted_operation_is_rolled_back() {
        let (root, store) = setup("journal");
        let sg = store.saved_games.clone();
        let m = store.adopt("Mods/aircraft/A-4E-C", "A-4E-C", "Aircraft").unwrap();
        // simulate a crash half-way through a disable: one file moved, journal left behind
        let from = sg.join("Mods/aircraft/A-4E-C/entry.lua");
        let to = store.vault_dir(&m.id).join("Mods/aircraft/A-4E-C/entry.lua");
        let mut j = Journal::open(&store.journal_path(&m.id)).unwrap();
        move_file(&from, &to).unwrap();
        j.moved(&from, &to).unwrap();
        drop(j);
        assert_eq!(store.recover(), 1);
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "A4");
        assert!(!store.journal_path(&m.id).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn compressed_vault_switches_on_and_off() {
        let (root, store) = setup("vaultzip");
        let store = store.with_compression(&Compression { vault: true, ..Compression::default() });
        let body = "-- a mod script line that repeats
".repeat(4000);
        put(&store.saved_games, "Mods/tech/Zipped/entry.lua", &body);
        put(&store.saved_games, "Mods/tech/Zipped/Textures/skin.dds", &"D".repeat(20000));
        let m = store.adopt("Mods/tech/Zipped", "Zipped", "Tech").unwrap();
        store.disable(&m.id, false).unwrap();
        let v = store.vault_dir(&m.id);
        assert!(v.join("Mods/tech/Zipped/entry.lua.zst").exists(), "switched-off files are compressed");
        assert!(!v.join("Mods/tech/Zipped/entry.lua").exists());
        assert!(!store.saved_games.join("Mods/tech/Zipped").exists());
        store.enable(&m.id).unwrap();
        assert_eq!(std::fs::read_to_string(store.saved_games.join("Mods/tech/Zipped/entry.lua")).unwrap(), body);
        assert!(!v.join("Mods/tech/Zipped/entry.lua.zst").exists(), "the compressed copy is removed after switching on");
        // switching on again is not reported as a change
        store.disable(&m.id, false).unwrap();
        store.enable(&m.id).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn mod_sets_and_remove() {
        let (root, store) = setup("sets");
        let a = store.adopt("Mods/aircraft/A-4E-C", "A-4E-C", "Aircraft").unwrap();
        let s = store.adopt("Scripts/Hooks/SRS-AutoConnect.lua", "SRS", "Hook").unwrap();
        // "Clean": everything off
        assert!(store.apply_set(&[]).unwrap().is_empty());
        assert!(store.manifests().iter().all(|m| !m.enabled));
        // only SRS
        assert!(store.apply_set(&[s.id.clone()]).unwrap().is_empty());
        assert!(store.load(&s.id).unwrap().enabled && !store.load(&a.id).unwrap().enabled);
        store.remove(&a.id, RemoveMode::Permanent).unwrap();
        assert!(!store.vault_dir(&a.id).exists());
        assert!(store.load(&a.id).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod real_packages {
    /// Reads only the file lists (and entry.lua) of the zips in "Saved Games\DCS Mods" and prints
    /// where Hangar would install them. Installs nothing.
    /// `cargo test --lib real_package_layouts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_package_layouts() {
        let Some(sg) = crate::winutil::saved_games_dir() else { return };
        let dir = sg.join("DCS Mods");
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
                continue;
            }
            let entries = super::package_entries(&p).unwrap();
            let layout = super::package_layout(&entries);
            let (place, name) = match &layout {
                Some(super::Layout::ModFolder { prefix, folder }) => {
                    let entry = super::read_package_file(&p, &format!("{prefix}entry.lua")).unwrap_or_default();
                    (format!("{}/{}", if entry.contains("make_flyable") { "Mods/aircraft" } else { "Mods/tech" }, folder.clone().unwrap_or_default()), super::lua_display_name(&entry))
                }
                other => (format!("{other:?}"), None),
            };
            println!("{} -> {} ({:?}, {} entries)", p.file_name().unwrap().to_string_lossy(), place, name, entries.len());
            assert!(matches!(layout, Some(super::Layout::ModFolder { .. })));
        }
    }
}
