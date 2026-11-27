// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Parses dcs.log (grouped messages) and autoupdate_log*.txt (updater run history). Read only.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use crate::winutil;

// ------------------------------------------------------------------ dcs.log

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LogGroup {
    pub level: String,
    pub category: String,
    /// First message of the group (as logged, anonymized).
    pub message: String,
    pub count: u32,
    /// Script/mod in Saved Games that the stack traces point to, if any.
    pub source: Option<String>,
    pub mine: bool,
    pub first_time: String,
    pub last_time: String,
    /// The first occurrence as logged: the message line plus its stack trace (anonymized).
    pub sample: Vec<String>,
    /// More continuation lines existed than are kept in `sample`.
    pub sample_truncated: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DcsLog {
    pub path: String,
    pub exists: bool,
    pub size: u64,
    pub line_count: usize,
    pub dcs_version: Option<String>,
    pub groups: Vec<LogGroup>,
}

/// Lines kept from the first occurrence (message + stack trace) for the detail view.
const SAMPLE_LINES: usize = 60;

fn re_head() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) (\w+)\s+(.+?) \(([^)]*)\): ?(.*)$").unwrap())
}
fn re_script() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?i)([A-Z]:[\\/][^"\]]*?Saved Games[\\/][^"\]]+?\.lua)"#).unwrap())
}
fn re_digits() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\d+").unwrap())
}
fn re_version() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"DCS/(\d+\.\d+\.\d+\.\d+)").unwrap())
}

/// Path from "Saved Games\" onwards, so the user name is not shown.
fn short_script(p: &str) -> String {
    let lower = p.to_ascii_lowercase();
    match lower.find("saved games") {
        Some(i) => p[i..].replace('/', "\\"),
        None => winutil::anonymize(p),
    }
}

#[tauri::command]
pub async fn get_dcs_log(saved_games_dir: String) -> Result<DcsLog, String> {
    let path = Path::new(&saved_games_dir).join("Logs").join("dcs.log");
    Ok(parse_dcs_log(&path))
}

pub fn parse_dcs_log(path: &Path) -> DcsLog {
    let mut out = DcsLog {
        path: winutil::anonymize(&path.to_string_lossy()),
        exists: false,
        size: 0,
        line_count: 0,
        dcs_version: None,
        groups: Vec::new(),
    };
    let Ok(text) = winutil::read_shared(path) else { return out };
    out.exists = true;
    out.size = text.len() as u64;

    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, LogGroup> = HashMap::new();
    let mut current: Option<String> = None; // key of the group the last head line belonged to
    let mut sampling = false; // the current head line is the group's first occurrence

    for line in text.lines() {
        out.line_count += 1;
        if let Some(c) = re_head().captures(line) {
            let (time, level, cat, msg) = (&c[1], &c[2], c[3].trim(), &c[5]);
            if out.dcs_version.is_none() {
                if let Some(v) = re_version().captures(msg) {
                    out.dcs_version = Some(v[1].to_string());
                }
            }
            let norm: String = re_digits().replace_all(msg, "#").chars().take(90).collect();
            let key = format!("{level}|{cat}|{norm}");
            let g = groups.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                LogGroup {
                    level: level.to_string(),
                    category: cat.to_string(),
                    message: winutil::anonymize(msg),
                    count: 0,
                    source: None,
                    mine: false,
                    first_time: time.to_string(),
                    last_time: time.to_string(),
                    sample: vec![winutil::anonymize(line)],
                    sample_truncated: false,
                }
            });
            g.count += 1;
            sampling = g.count == 1;
            g.last_time = time.to_string();
            if let Some(s) = re_script().captures(msg) {
                g.source = Some(short_script(&s[1]));
                g.mine = true;
            }
            current = Some(key);
        } else if let Some(key) = &current {
            // Continuation line (stack trace etc.) belongs to the previous message.
            if let Some(g) = groups.get_mut(key) {
                if sampling && !line.trim().is_empty() {
                    if g.sample.len() < SAMPLE_LINES {
                        g.sample.push(winutil::anonymize(line));
                    } else {
                        g.sample_truncated = true;
                    }
                }
                if g.source.is_none() {
                    if let Some(s) = re_script().captures(line) {
                        g.source = Some(short_script(&s[1]));
                        g.mine = true;
                    }
                }
            }
        }
    }

    let mut list: Vec<LogGroup> = order.into_iter().filter_map(|k| groups.remove(&k)).collect();
    let rank = |l: &str| match l {
        "ERROR" | "ALERT" => 0,
        "WARNING" => 1,
        _ => 2,
    };
    list.sort_by(|a, b| rank(&a.level).cmp(&rank(&b.level)).then(b.mine.cmp(&a.mine)).then(b.count.cmp(&a.count)));
    // Keep all errors and warnings, but only the most frequent info groups.
    let mut info_kept = 0;
    list.retain(|g| {
        if rank(&g.level) < 2 {
            true
        } else {
            info_kept += 1;
            info_kept <= 25
        }
    });
    out.groups = list;
    out
}

// ------------------------------------------------------------------ updater history

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub name: String,
    pub seconds: f64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterRun {
    pub started_utc: String,
    /// "update", "install", "uninstall", "repair" or "other".
    pub kind: String,
    pub target: Option<String>,
    pub result: String, // "success" | "failed" | "unknown"
    pub result_text: Option<String>,
    pub duration_s: f64,
    /// Unpacked bytes from the last "To download" line.
    pub to_download: Option<u64>,
    pub to_download_files: Option<u64>,
    pub phases: Vec<Phase>,
}

fn re_upd_line() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^(\d+\.\d+) (?:(\w+)\s*: (.*)|(=== Log closed\.))").unwrap())
}
fn re_to_download() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"To download: (\d+) bytes in (\d+) files").unwrap())
}

#[tauri::command]
pub async fn get_updater_history(install_path: String) -> Result<Vec<UpdaterRun>, String> {
    let root = Path::new(&install_path);
    let archive = crate::settings::data_dir().join("updater-logs");
    archive_updater_logs(root, &archive);
    let mut runs = read_archive(&archive);
    if runs.is_empty() {
        // archive not writable: show what ED still has
        for name in ["autoupdate_log_old.txt", "autoupdate_log.txt"] {
            if let Ok(text) = winutil::read_shared(&root.join(name)) {
                runs.extend(parse_updater_log(&text));
            }
        }
    }
    runs.sort_by(|a, b| b.started_utc.cmp(&a.started_utc)); // newest first
    runs.dedup_by(|a, b| a.started_utc == b.started_utc);
    Ok(runs)
}

const LOG_OPENED: &str = "=== Log opened UTC ";

/// ED keeps only two updater logs. Hangar copies every run into its own data folder
/// (`updater-logs\<start time>.txt`) so the history survives. Reads ED's files only.
pub fn archive_updater_logs(install_root: &Path, archive: &Path) -> usize {
    let mut written = 0;
    for name in ["autoupdate_log_old.txt", "autoupdate_log.txt"] {
        let Ok(text) = winutil::read_shared(&install_root.join(name)) else { continue };
        for chunk in text.split(LOG_OPENED).skip(1) {
            let started = chunk.lines().next().unwrap_or("").trim();
            if started.len() < 19 {
                continue;
            }
            let file = archive.join(format!("{}.txt", started.replace([':', ' '], "-")));
            let body = format!("{LOG_OPENED}{chunk}");
            // a run still in progress grows: replace a shorter copy
            let old_len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            if (body.len() as u64) > old_len && crate::settings::write_atomic(&file, body.as_bytes()).is_ok() {
                written += 1;
            }
        }
    }
    written
}

fn read_archive(archive: &Path) -> Vec<UpdaterRun> {
    std::fs::read_dir(archive)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .flat_map(|t| parse_updater_log(&t))
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_updater_log(text: &str) -> Vec<UpdaterRun> {
    let mut runs = Vec::new();
    for chunk in text.split("=== Log opened UTC ").skip(1) {
        let started = chunk.lines().next().unwrap_or("").trim().to_string();
        let mut run = UpdaterRun {
            started_utc: started,
            kind: "other".into(),
            target: None,
            result: "unknown".into(),
            result_text: None,
            duration_s: 0.0,
            to_download: None,
            to_download_files: None,
            phases: Vec::new(),
        };
        let mut last_status: Option<(String, f64)> = None;
        let mut end_t: f64 = 0.0;
        let mut failed = false;
        let mut succeeded = false;
        for line in chunk.lines() {
            let Some(c) = re_upd_line().captures(line) else { continue };
            let t: f64 = c[1].parse().unwrap_or(0.0);
            end_t = end_t.max(t);
            if c.get(4).is_some() {
                break; // Log closed
            }
            let level = c.get(2).map(|m| m.as_str()).unwrap_or("");
            let msg = c.get(3).map(|m| m.as_str().trim()).unwrap_or("");
            if level == "STATUS" {
                if let Some((name, t0)) = last_status.take() {
                    run.phases.push(Phase { name, seconds: (t - t0).max(0.0) });
                }
                let name = msg.trim_end_matches('.').trim().to_string();
                if let Some(rest) = name.strip_prefix("Uninstalling ") {
                    run.kind = "uninstall".into();
                    run.target = Some(rest.to_string());
                } else if let Some(rest) = name.strip_prefix("Preparing to install ") {
                    run.kind = "install".into();
                    run.target = Some(rest.trim().to_string());
                } else if let Some(rest) = name.strip_prefix("Successfully updated to ") {
                    run.kind = "update".into();
                    run.target = Some(rest.to_string());
                    succeeded = true;
                } else if name.starts_with("Downloading update") && run.kind == "other" {
                    run.kind = "update".into();
                } else if name.to_ascii_lowercase().contains("not enough disk space") {
                    failed = true;
                }
                run.result_text = Some(msg.to_string());
                last_status = Some((name, t));
            } else {
                if let Some(m) = re_to_download().captures(msg) {
                    run.to_download = m[1].parse().ok();
                    run.to_download_files = m[2].parse().ok();
                }
                if msg.starts_with("Installed ") || msg.starts_with("Uninstalled ") {
                    succeeded = true;
                }
                if level.eq_ignore_ascii_case("ERROR") {
                    failed = true;
                }
            }
        }
        if let Some((name, t0)) = last_status {
            run.phases.push(Phase { name, seconds: (end_t - t0).max(0.0) });
        }
        run.duration_s = end_t;
        run.result = if failed { "failed" } else if succeeded { "success" } else { "unknown" }.into();
        // Drop near-zero bookkeeping phases so the list stays readable.
        run.phases.retain(|p| p.seconds >= 0.05 || p.name.starts_with("Successfully"));
        runs.push(run);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_install_run() {
        let log = "00000.000 === Log opened UTC 2026-11-26 22:55:24\n\
00000.074 STATUS: Initializing...\n\
00000.577 STATUS: Preparing to install AJS37 ...\n\
00000.615 INFO : To download: 1928880784 bytes in 905 files\n\
00001.279 INFO : To download: 1892723220 bytes in 782 files\n\
00001.279 STATUS: Downloading AJS37...\n\
00002.382 STATUS: Downloading update...\n\
00041.776 STATUS: Installing new files...\n\
00042.420 INFO : Installed 905 files.\n\
00042.428 STATUS: Cleaning up...\n\
00042.691 === Log closed.\n";
        let runs = parse_updater_log(log);
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.kind, "install");
        assert_eq!(r.target.as_deref(), Some("AJS37"));
        assert_eq!(r.result, "success");
        assert_eq!(r.to_download, Some(1892723220));
        assert!((r.duration_s - 42.691).abs() < 1e-6);
        assert!(r.phases.iter().any(|p| p.name == "Downloading update" && p.seconds > 39.0));
    }

    #[test]
    fn parses_disk_space_failure() {
        let log = "00000.000 === Log opened UTC 2026-11-26 11:29:12\n\
00009.297 INFO : To download: 261974372892 bytes in 5544 files\n\
00009.297 STATUS: Not enough disk space in the download folder.\n\
00076.232 === Log closed.\n";
        let r = &parse_updater_log(log)[0];
        assert_eq!(r.result, "failed");
    }

    #[test]
    fn groups_dcs_log() {
        let dir = std::env::temp_dir().join("hangar-test-dcslog");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("dcs.log");
        std::fs::write(&p, "2026-11-26 18:14:02.118 INFO    APP (Main): DCS/2.9.32.30490 (x86_64; MT)\n\
2026-11-26 18:15:41.230 ERROR   Lua::Config (Main): Call error x\n\
stack traceback:\n\
\t[string \"C:\\Users\\x\\Saved Games\\DCS\\Scripts/TacviewGameExport.lua\"]:1: in main\n\
2026-11-26 18:15:42.230 ERROR   Lua::Config (Main): Call error x\n").unwrap();
        let l = parse_dcs_log(&p);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(l.dcs_version.as_deref(), Some("2.9.32.30490"));
        let g = &l.groups[0];
        assert_eq!(g.level, "ERROR");
        assert_eq!(g.count, 2);
        assert!(g.mine);
        assert!(g.source.as_deref().unwrap().contains("TacviewGameExport.lua"));
        // the detail shows the first occurrence with its whole stack trace, not the repeats
        assert_eq!(g.sample.len(), 3, "{:?}", g.sample);
        assert!(!g.sample_truncated);
    }

    #[test]
    fn archives_runs_and_keeps_them() {
        let dir = std::env::temp_dir().join(format!("hangar-test-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let install = dir.join("install");
        let archive = dir.join("archive");
        std::fs::create_dir_all(&install).unwrap();
        let run = |ts: &str| format!("00000.000 === Log opened UTC {ts}\n00000.100 STATUS: Uninstalling AJS37...\n00001.000 === Log closed.\n");
        std::fs::write(install.join("autoupdate_log_old.txt"), run("2026-11-26 09:00:00")).unwrap();
        std::fs::write(install.join("autoupdate_log.txt"), run("2026-11-27 10:00:00")).unwrap();
        assert_eq!(archive_updater_logs(&install, &archive), 2);
        assert_eq!(archive_updater_logs(&install, &archive), 0, "unchanged runs are not rewritten");
        // ED rotates: the oldest run disappears from its files but stays in the archive
        std::fs::write(install.join("autoupdate_log_old.txt"), run("2026-11-27 10:00:00")).unwrap();
        std::fs::write(install.join("autoupdate_log.txt"), run("2026-11-27 11:00:00")).unwrap();
        archive_updater_logs(&install, &archive);
        let runs = read_archive(&archive);
        assert_eq!(runs.len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
