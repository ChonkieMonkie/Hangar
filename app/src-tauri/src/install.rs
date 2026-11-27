//! Finds DCS World installations (registry) and reads autoupdate.cfg. Read only.

use std::path::{Path, PathBuf};

use serde::Serialize;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

use crate::winutil;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    /// Registry subkey name, e.g. "DCS World" or "DCS World OpenBeta".
    pub key: String,
    pub path: String,
    pub exists: bool,
    pub version: Option<String>,
    pub timestamp: Option<String>,
    pub arch: Option<String>,
    pub lang: Option<String>,
    pub modules: Vec<String>,
    /// Relative exe path from autoupdate.cfg "launch", e.g. "bin/DCS.exe".
    pub launch: Option<String>,
    pub updater_path: String,
    pub updater_exists: bool,
    pub disk_free: Option<u64>,
    pub disk_total: Option<u64>,
    pub cfg_error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub installations: Vec<Installation>,
    pub saved_games: Option<String>,
    /// Folders in Saved Games that start with "DCS" (DCS, DCS.openbeta, module data, ...).
    pub saved_games_dcs_dirs: Vec<String>,
}

#[tauri::command]
pub async fn get_installations() -> Result<InstallReport, String> {
    Ok(find_installations())
}

pub fn find_installations() -> InstallReport {
    let mut installations = Vec::new();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(ed) = hkcu.open_subkey("Software\\Eagle Dynamics") {
        for key in ed.enum_keys().flatten() {
            let Ok(k) = ed.open_subkey(&key) else { continue };
            let Ok(path) = k.get_value::<String, _>("Path") else { continue };
            installations.push(read_installation(&key, Path::new(path.trim_end_matches('\\'))));
        }
    }

    let saved_games = winutil::saved_games_dir();
    let mut dirs = Vec::new();
    if let Some(sg) = &saved_games {
        if let Ok(rd) = std::fs::read_dir(sg) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() && name.to_ascii_lowercase().starts_with("dcs") {
                    dirs.push(name);
                }
            }
        }
        dirs.sort();
    }

    InstallReport {
        installations,
        saved_games: saved_games.map(|p| p.to_string_lossy().into_owned()),
        saved_games_dcs_dirs: dirs,
    }
}

fn read_installation(key: &str, root: &Path) -> Installation {
    let updater: PathBuf = root.join("bin").join("DCS_updater.exe");
    let mut inst = Installation {
        key: key.to_string(),
        path: root.to_string_lossy().into_owned(),
        exists: root.is_dir(),
        version: None,
        timestamp: None,
        arch: None,
        lang: None,
        modules: Vec::new(),
        launch: None,
        updater_path: updater.to_string_lossy().into_owned(),
        updater_exists: updater.is_file(),
        disk_free: None,
        disk_total: None,
        cfg_error: None,
    };
    if let Some((free, total)) = winutil::disk_space(root) {
        inst.disk_free = Some(free);
        inst.disk_total = Some(total);
    }
    let cfg = root.join("autoupdate.cfg");
    match winutil::read_shared(&cfg) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => {
                let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
                inst.version = s("version");
                inst.timestamp = s("timestamp");
                inst.arch = s("arch");
                inst.lang = s("lang");
                inst.launch = s("launch");
                if let Some(arr) = v.get("modules").and_then(|m| m.as_array()) {
                    inst.modules = arr.iter().filter_map(|m| m.as_str().map(str::to_string)).collect();
                }
            }
            Err(e) => inst.cfg_error = Some(format!("autoupdate.cfg is not valid JSON: {e}")),
        },
        Err(e) => inst.cfg_error = Some(format!("autoupdate.cfg could not be read: {e}")),
    }
    inst
}
