// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Tauri commands for Backups, Mods and launching DCS. Every writing command runs only on
//! the user's click in the UI. Long operations run as a background job (one at a time) and
//! report progress with the "job-progress" event.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::backup::{RestoreResult, SnapshotInfo, SourceRef, Store, TopFolder};
use crate::mods::{Candidate, ModStore, ModView};
use crate::settings::{self, ModSet, Settings, data_dir, dcs_running};
use crate::winutil;

// ------------------------------------------------------------------ jobs

static BUSY: Mutex<Option<String>> = Mutex::new(None);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct JobProgress {
    job: String,
    done: usize,
    total: usize,
    item: String,
    finished: bool,
}

fn busy() -> Option<String> {
    BUSY.lock().ok().and_then(|b| b.clone())
}

/// Runs `f` on a worker thread as the only job, emitting throttled progress.
async fn job<T: Send + 'static>(
    app: AppHandle,
    name: &str,
    f: impl FnOnce(&mut dyn FnMut(usize, usize, &str)) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    {
        let mut b = BUSY.lock().map_err(|e| e.to_string())?;
        if let Some(other) = b.as_ref() {
            return Err(format!("Wait until \"{other}\" is done."));
        }
        *b = Some(name.to_string());
    }
    let job_name = name.to_string();
    let app2 = app.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        let mut last = Instant::now();
        let mut report = |done: usize, total: usize, item: &str| {
            if last.elapsed().as_millis() >= 120 || done >= total {
                last = Instant::now();
                let _ = app2.emit("job-progress", JobProgress { job: job_name.clone(), done, total, item: item.to_string(), finished: false });
            }
        };
        f(&mut report)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    if let Ok(mut b) = BUSY.lock() {
        *b = None;
    }
    let _ = app.emit("job-progress", JobProgress { job: name.to_string(), done: 1, total: 1, item: String::new(), finished: true });
    res
}

fn refuse_if_dcs_running(what: &str) -> Result<(), String> {
    if dcs_running() {
        return Err(format!("Close DCS before {what}."));
    }
    Ok(())
}

/// Saved Games\DCS (or DCS.openbeta).
pub fn main_saved_games() -> Option<PathBuf> {
    let sg = winutil::saved_games_dir()?;
    ["DCS", "DCS.openbeta"].iter().map(|n| sg.join(n)).find(|p| p.is_dir())
}

fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_ascii_uppercase());
    root(a).is_some() && root(a) == root(b)
}

// ------------------------------------------------------------------ backups

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub key: String,
    pub path: String,
    pub enabled: bool,
    pub exists: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupOverview {
    pub target: Option<String>,
    pub target_free: Option<u64>,
    /// The destination is on the same drive as Saved Games.
    pub same_disk: bool,
    pub store_bytes: u64,
    pub snapshots: Vec<SnapshotInfo>,
    pub sources: Vec<SourceView>,
    pub excluded: Vec<String>,
    pub before_update: bool,
    pub busy: Option<String>,
    pub dcs_running: bool,
}

fn sources_of(s: &Settings) -> Vec<SourceRef> {
    s.backup_sources
        .iter()
        .filter(|b| b.enabled && Path::new(&b.path).is_dir())
        .map(|b| SourceRef { key: b.key.clone(), root: b.path.clone() })
        .collect()
}

fn store_of(s: &Settings) -> Result<Store, String> {
    s.backup_target.as_ref().map(Store::new).ok_or_else(|| "Choose a backup destination first.".to_string())
}

#[tauri::command]
pub async fn backup_overview() -> Result<BackupOverview, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let s = settings::load();
        let (snapshots, store_bytes, free) = match &s.backup_target {
            Some(t) => {
                let st = Store::new(t);
                (st.list().iter().map(SnapshotInfo::from).collect(), st.store_bytes(), winutil::disk_space(Path::new(t)).map(|x| x.0))
            }
            None => (vec![], 0, None),
        };
        let same_disk = match (&s.backup_target, winutil::saved_games_dir()) {
            (Some(t), Some(sg)) => same_volume(Path::new(t), &sg),
            _ => false,
        };
        BackupOverview {
            target: s.backup_target.clone(),
            target_free: free,
            same_disk,
            store_bytes,
            snapshots,
            sources: s.backup_sources.iter().map(|b| SourceView { key: b.key.clone(), path: b.path.clone(), enabled: b.enabled, exists: Path::new(&b.path).is_dir() }).collect(),
            excluded: s.backup_excluded.clone(),
            before_update: s.backup_before_update,
            busy: busy(),
            dcs_running: dcs_running(),
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn backup_top_folders(path: String) -> Result<Vec<TopFolder>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::backup::top_folders(Path::new(&path))).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn backup_now(app: AppHandle, scope: String) -> Result<SnapshotInfo, String> {
    let s = settings::load();
    let store = store_of(&s)?;
    let sources = sources_of(&s);
    if sources.is_empty() {
        return Err("Nothing selected to back up.".into());
    }
    let scope = if scope == "config" { "config" } else { "all" }.to_string();
    job(app, "Backing up", move |p| {
        let snap = store.snapshot(&sources, &scope, "Manual", &s.backup_excluded, p)?;
        store.prune();
        Ok(SnapshotInfo::from(&snap))
    })
    .await
}

#[tauri::command]
pub async fn backup_verify(app: AppHandle, id: String) -> Result<crate::backup::Verification, String> {
    let store = store_of(&settings::load())?;
    job(app, "Verifying", move |p| store.verify(&id, p)).await
}

#[tauri::command]
pub async fn backup_restore(app: AppHandle, id: String, scope: String, files: Option<Vec<(String, String)>>) -> Result<RestoreResult, String> {
    refuse_if_dcs_running("restoring")?;
    let s = settings::load();
    let store = store_of(&s)?;
    job(app, "Restoring", move |p| store.restore(&id, &scope, files.as_deref(), &s.backup_excluded, p)).await
}

#[tauri::command]
pub fn backup_pin(id: String, pinned: bool) -> Result<(), String> {
    store_of(&settings::load())?.set_pinned(&id, pinned)
}

#[tauri::command]
pub fn backup_delete(id: String) -> Result<u64, String> {
    if busy().is_some() {
        return Err("Wait until the current job is done.".into());
    }
    store_of(&settings::load())?.delete(&id)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileRow {
    pub source: String,
    pub path: String,
    pub size: u64,
}

#[tauri::command]
pub async fn backup_files(id: String) -> Result<Vec<FileRow>, String> {
    let store = store_of(&settings::load())?;
    tauri::async_runtime::spawn_blocking(move || {
        store.load(&id).map(|s| s.files.into_iter().map(|f| FileRow { source: f.source, path: f.path, size: f.size }).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Called by start_updater: a Config snapshot before an update, if the user wants one.
pub fn backup_before_update() -> Result<Option<String>, String> {
    let s = settings::load();
    if !s.backup_before_update || s.backup_target.is_none() {
        return Ok(None);
    }
    let sources = sources_of(&s);
    if sources.is_empty() {
        return Ok(None);
    }
    let store = store_of(&s)?;
    let snap = store.snapshot(&sources, "config", "Before update", &s.backup_excluded, &mut |_, _, _| {})?;
    store.prune();
    Ok(Some(snap.id))
}

// ------------------------------------------------------------------ mods

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModsOverview {
    pub saved_games: Option<String>,
    pub dcs_running: bool,
    pub mods: Vec<ModView>,
    /// Mod-like folders Hangar does not manage yet.
    pub candidates: Vec<Candidate>,
    pub sets: Vec<ModSet>,
    pub busy: Option<String>,
}

fn mod_store() -> Result<ModStore, String> {
    let sg = main_saved_games().ok_or("Saved Games\\DCS was not found.")?;
    Ok(ModStore::new(sg, data_dir()))
}

#[tauri::command]
pub async fn mods_overview() -> Result<ModsOverview, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let s = settings::load();
        match mod_store() {
            Ok(st) => ModsOverview {
                saved_games: Some(st.saved_games.to_string_lossy().into_owned()),
                dcs_running: dcs_running(),
                mods: st.views(),
                candidates: st.scan().into_iter().filter(|c| c.managed.is_none()).collect(),
                sets: s.mod_sets,
                busy: busy(),
            },
            Err(_) => ModsOverview { saved_games: None, dcs_running: dcs_running(), mods: vec![], candidates: vec![], sets: s.mod_sets, busy: busy() },
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mod_adopt(app: AppHandle, root: String, name: String, kind: String) -> Result<(), String> {
    let st = mod_store()?;
    job(app, "Adding mod", move |_| st.adopt(&root, &name, &kind).map(|_| ())).await
}

#[tauri::command]
pub async fn mod_add(app: AppHandle, path: String) -> Result<(), String> {
    refuse_if_dcs_running("adding mods")?;
    let st = mod_store()?;
    job(app, "Installing mod", move |_| st.add_package(Path::new(&path)).map(|_| ())).await
}

#[tauri::command]
pub async fn mod_set_enabled(app: AppHandle, id: String, enabled: bool, force: bool) -> Result<(), String> {
    refuse_if_dcs_running("switching mods")?;
    let st = mod_store()?;
    let label = if enabled { "Switching on" } else { "Switching off" };
    job(app, label, move |_| if enabled { st.enable(&id).map(|_| ()) } else { st.disable(&id, force).map(|_| ()) }).await
}

#[tauri::command]
pub async fn mod_remove(app: AppHandle, id: String) -> Result<(), String> {
    refuse_if_dcs_running("removing mods")?;
    let st = mod_store()?;
    job(app, "Removing mod", move |_| st.remove(&id)).await
}

#[tauri::command]
pub fn mod_forget(id: String) -> Result<(), String> {
    mod_store()?.forget(&id)
}

/// Applies a saved set. Returns per-mod problems (empty = all fine).
#[tauri::command]
pub async fn mod_apply_set(app: AppHandle, set_id: String) -> Result<Vec<String>, String> {
    refuse_if_dcs_running("switching mods")?;
    let set = settings::load().mod_sets.into_iter().find(|s| s.id == set_id).ok_or("Set not found.")?;
    let st = mod_store()?;
    job(app, "Applying set", move |_| st.apply_set(&set.enabled)).await
}

/// At start: undo a mod operation that was interrupted (journal left behind).
pub fn recover_mods() -> usize {
    mod_store().map(|s| s.recover()).unwrap_or(0)
}

// ------------------------------------------------------------------ launch & misc

/// Starts DCS with a launch preset (separate process, on the user's click).
#[tauri::command]
pub fn launch_dcs(install_path: String, launch: Option<String>, preset_id: String) -> Result<(), String> {
    if dcs_running() {
        return Err("DCS is already running.".into());
    }
    if !winutil::processes_named("DCS_updater.exe").is_empty() {
        return Err("Wait until the updater is done.".into());
    }
    let s = settings::load();
    let preset = s.launch_presets.iter().find(|p| p.id == preset_id).ok_or("Preset not found.")?;
    let rel = launch.unwrap_or_else(|| "bin/DCS.exe".into());
    let rel = rel.replace('/', "\\");
    if rel.contains("..") || !rel.to_ascii_lowercase().ends_with(".exe") {
        return Err("Unexpected launch path in autoupdate.cfg.".into());
    }
    let exe = Path::new(&install_path).join(&rel);
    if !exe.is_file() {
        return Err(format!("Not found: {}", exe.display()));
    }
    Command::new(&exe)
        .args(&preset.args)
        .current_dir(exe.parent().unwrap_or(Path::new(&install_path)))
        .spawn()
        .map_err(|e| format!("Could not start DCS: {e}"))?;
    Ok(())
}

/// Opens a folder (or shows a file) in Explorer.
#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    let mut c = Command::new("explorer.exe");
    if p.is_file() {
        c.arg(format!("/select,{}", p.display()));
    } else if p.is_dir() {
        c.arg(p);
    } else {
        return Err(format!("Not found: {path}"));
    }
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn current_job() -> Option<String> {
    busy()
}

/// ED's public pages that Hangar links to (ROADMAP #7, HANDOFF-juridik: link, never scrape).
/// The URL is built here from a fixed list, so the UI cannot open arbitrary addresses.
/// Hangar sends no request itself: the page opens in the user's default browser.
pub fn ed_page_url(page: &str, version: Option<&str>) -> Result<String, String> {
    const BASE: &str = "https://www.digitalcombatsimulator.com/en/news/";
    match page {
        "changelog" => Ok(format!("{BASE}changelog/")),
        "newsletters" => Ok(format!("{BASE}newsletters/")),
        "news" => Ok(BASE.to_string()),
        "version" => {
            let v = version.unwrap_or("");
            let ok = !v.is_empty() && v.len() <= 32 && v.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
            if !ok {
                return Err(format!("Not a DCS version number: {v}"));
            }
            Ok(format!("{BASE}changelog/release/{v}/"))
        }
        _ => Err(format!("Unknown page: {page}")),
    }
}

#[tauri::command]
pub fn open_ed_page(page: String, version: Option<String>) -> Result<(), String> {
    let url = ed_page_url(&page, version.as_deref())?;
    Command::new("explorer.exe").arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod ed_page_tests {
    use super::ed_page_url;

    #[test]
    fn builds_only_known_urls() {
        assert_eq!(ed_page_url("changelog", None).unwrap(), "https://www.digitalcombatsimulator.com/en/news/changelog/");
        assert_eq!(
            ed_page_url("version", Some("2.9.32.30490")).unwrap(),
            "https://www.digitalcombatsimulator.com/en/news/changelog/release/2.9.32.30490/"
        );
        assert!(ed_page_url("version", Some("2.9/../../x")).is_err());
        assert!(ed_page_url("version", Some("")).is_err());
        assert!(ed_page_url("https://evil.example", None).is_err());
    }
}

/// Hangar's own project pages (About box). Fixed list, opened in the default browser.
pub fn project_link_url(which: &str) -> Result<&'static str, String> {
    match which {
        "source" => Ok("https://github.com/ChonkieMonkie/Hangar"),
        "issues" => Ok("https://github.com/ChonkieMonkie/Hangar/issues"),
        "gpl" => Ok("https://www.gnu.org/licenses/gpl-3.0.html"),
        _ => Err(format!("Unknown link: {which}")),
    }
}

#[tauri::command]
pub fn open_project_link(which: String) -> Result<(), String> {
    let url = project_link_url(&which)?;
    Command::new("explorer.exe").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The full GPL text, built into the exe (About box, "View license").
#[tauri::command]
pub fn license_text() -> &'static str {
    include_str!("../../../LICENSE")
}

/// Licenses of the third-party code built into Hangar (generated by tools/third_party_notices.py).
#[tauri::command]
pub fn third_party_notices() -> &'static str {
    include_str!("../../../THIRD-PARTY-NOTICES.txt")
}
