// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Hangar's own settings (`%LOCALAPPDATA%\Hangar\settings.json`) and helpers for Hangar's data
//! folder. Nothing here touches DCS folders. The file is only written when the user saves.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `%LOCALAPPDATA%\Hangar`
pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Hangar")
}

/// Writes a file via a temp file + rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp-hangar");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    write_atomic(path, &bytes).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Lowercase, ASCII, dashes: for ids and file names.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "item".into() } else { out.chars().take(40).collect() }
}

pub fn dcs_running() -> bool {
    !crate::winutil::processes_named("DCS.exe").is_empty()
}

// ------------------------------------------------------------------ settings

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WallFolder {
    pub path: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupSource {
    /// "DCS", "DCS.openbeta", "Tacview" or "extra-<slug>"
    pub key: String,
    pub path: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchPreset {
    pub id: String,
    pub name: String,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModSet {
    pub id: String,
    pub name: String,
    /// Mod ids that are on in this set; every other managed mod is off.
    pub enabled: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub wall_folders: Vec<WallFolder>,
    /// "random" | "newest" | "favorite"
    pub wall_mode: String,
    pub wall_subfolders: bool,
    pub wall_favorite: Option<String>,
    /// Backup destination. None = not chosen yet.
    pub backup_target: Option<String>,
    pub backup_sources: Vec<BackupSource>,
    /// "<source key>/<top-level folder>" left out of backups (caches are always left out).
    pub backup_excluded: Vec<String>,
    /// Snapshot Config before every update started from Hangar.
    pub backup_before_update: bool,
    pub launch_presets: Vec<LaunchPreset>,
    pub default_preset: Option<String>,
    pub mod_sets: Vec<ModSet>,
    /// Record updater runs for ETA research (ROADMAP #14). Off by default.
    pub record_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wall_folders: Vec::new(),
            wall_mode: "random".into(),
            wall_subfolders: true,
            wall_favorite: None,
            backup_target: None,
            backup_sources: Vec::new(),
            backup_excluded: Vec::new(),
            backup_before_update: true,
            launch_presets: vec![
                LaunchPreset { id: "normal".into(), name: "Normal".into(), args: vec![] },
                LaunchPreset { id: "vr".into(), name: "VR (OpenXR)".into(), args: vec!["--force_enable_VR".into(), "--force_OpenXR".into()] },
                LaunchPreset { id: "flat".into(), name: "Flat screen".into(), args: vec!["--force_disable_VR".into()] },
            ],
            default_preset: Some("normal".into()),
            mod_sets: Vec::new(),
            record_updates: false,
        }
    }
}

/// Folders found on this PC that make sense as defaults (only read, never created).
pub struct Detected {
    pub saved_games: Option<PathBuf>,
    pub videos: Option<PathBuf>,
    pub documents: Option<PathBuf>,
}

impl Detected {
    pub fn this_pc() -> Self {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
        Detected {
            saved_games: crate::winutil::saved_games_dir(),
            videos: home.as_ref().map(|h| h.join("Videos")),
            documents: home.as_ref().map(|h| h.join("Documents")),
        }
    }
}

/// Fills empty lists with what exists on this PC. Pure apart from `is_dir` checks.
pub fn with_defaults(mut s: Settings, d: &Detected) -> Settings {
    if s.wall_folders.is_empty() {
        let mut cands: Vec<PathBuf> = Vec::new();
        if let Some(sg) = &d.saved_games {
            for name in ["DCS", "DCS.openbeta"] {
                cands.push(sg.join(name).join("ScreenShots"));
            }
        }
        if let Some(v) = &d.videos {
            cands.push(v.join("NVIDIA"));
            cands.push(v.join("Captures"));
        }
        s.wall_folders = cands
            .into_iter()
            .filter(|p| p.is_dir())
            .map(|p| WallFolder { path: p.to_string_lossy().into_owned(), enabled: true })
            .collect();
    }
    if s.backup_sources.is_empty() {
        if let Some(sg) = &d.saved_games {
            if let Ok(rd) = std::fs::read_dir(sg) {
                let mut names: Vec<String> = rd
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.to_ascii_uppercase().starts_with("DCS"))
                    .collect();
                names.sort();
                for n in names {
                    // the main folders on by default, other DCS* folders (module data) off
                    let on = n == "DCS" || n == "DCS.openbeta";
                    s.backup_sources.push(BackupSource { key: n.clone(), path: sg.join(&n).to_string_lossy().into_owned(), enabled: on });
                }
            }
        }
        if let Some(doc) = &d.documents {
            let tv = doc.join("Tacview");
            if tv.is_dir() {
                s.backup_sources.push(BackupSource { key: "Tacview".into(), path: tv.to_string_lossy().into_owned(), enabled: true });
            }
        }
    }
    s
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn load_from(path: &Path, d: &Detected) -> Settings {
    let s = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Settings>(&b).ok()).unwrap_or_default();
    with_defaults(s, d)
}

pub fn load() -> Settings {
    load_from(&settings_path(), &Detected::this_pc())
}

#[tauri::command]
pub fn get_settings() -> Settings {
    load()
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> Result<Settings, String> {
    write_json(&settings_path(), &settings)?;
    Ok(load())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hangar-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn defaults_from_detected_folders() {
        let root = tmp("detect");
        let sg = root.join("Saved Games");
        std::fs::create_dir_all(sg.join("DCS").join("ScreenShots")).unwrap();
        std::fs::create_dir_all(sg.join("DCS.F-14")).unwrap();
        std::fs::create_dir_all(root.join("Videos").join("NVIDIA")).unwrap();
        std::fs::create_dir_all(root.join("Documents").join("Tacview")).unwrap();
        let d = Detected { saved_games: Some(sg.clone()), videos: Some(root.join("Videos")), documents: Some(root.join("Documents")) };
        let s = load_from(&root.join("missing.json"), &d);
        assert_eq!(s.wall_folders.len(), 2, "{:?}", s.wall_folders);
        let keys: Vec<(&str, bool)> = s.backup_sources.iter().map(|b| (b.key.as_str(), b.enabled)).collect();
        assert_eq!(keys, vec![("DCS", true), ("DCS.F-14", false), ("Tacview", true)]);
        assert_eq!(s.launch_presets.len(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn saves_and_loads_round_trip() {
        let root = tmp("roundtrip");
        let p = root.join("settings.json");
        let mut s = Settings::default();
        s.wall_mode = "newest".into();
        s.mod_sets.push(ModSet { id: "clean".into(), name: "Clean".into(), enabled: vec![] });
        write_json(&p, &s).unwrap();
        let d = Detected { saved_games: None, videos: None, documents: None };
        let back = load_from(&p, &d);
        assert_eq!(back.wall_mode, "newest");
        assert_eq!(back.mod_sets.len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("A-4E Community Mod"), "a-4e-community-mod");
        assert_eq!(slug("  ##  "), "item");
    }
}
