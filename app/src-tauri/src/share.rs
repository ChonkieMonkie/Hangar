// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! ETA research (ROADMAP #14): the user's own recordings (recorder.rs) and an anonymized export
//! they can send to the developer by hand. Hangar never sends anything itself.
//!
//! The export is a zip with `manifest.json` and one `recordings/NN.ndjson` per run in format 2.
//! It keeps only what replay.rs needs to score ETA algorithms:
//! relative time, the parsed numbers from the updater's window, network and disk as differences,
//! process count, phase, Hangar's own ETA and a few log lines rebuilt from known patterns.
//! It leaves out clock times, paths, names, process ids, raw window text, file names and module
//! names. Raw recordings on this PC are never changed.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::recorder::recordings_dir;
use crate::window::{best_reading, parse_window_texts};

pub const EXPORT_FORMAT: u32 = 2;
/// Keep at most this many recordings / bytes; the oldest go first (Hangar's own data only).
pub const MAX_RECORDINGS: usize = 50;
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;

/// Status phrases that may pass through to the export. Anything else becomes "STATUS: other".
const STATUS_PHRASES: [&str; 14] = [
    "Downloading update",
    "Downloading the update",
    "Waiting for operation",
    "Installing new files",
    "Installing",
    "Cleaning up",
    "Successfully",
    "Verifying",
    "Checking",
    "Preparing",
    "Unpacking",
    "Uninstalling",
    "Removing",
    "Done",
];
const LOG_FILES: [&str; 2] = ["autoupdate_log.txt", "autoupdate_templog.txt"];

fn recording_files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut v: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("run-") && n.ends_with(".ndjson")
        })
        .map(|e| (e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)))
        .collect();
    v.sort(); // names are run-YYYYMMDD-HHMMSS: oldest first
    v
}

/// Deletes the oldest recordings beyond the limits. `keep` (the one being written) is never removed.
pub fn prune(dir: &Path, keep: Option<&Path>) -> usize {
    let files = recording_files(dir);
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    let mut count = files.len();
    let mut removed = 0;
    for (p, len) in files {
        if count <= MAX_RECORDINGS && total <= MAX_BYTES {
            break;
        }
        if keep.is_some_and(|k| k == p) {
            continue;
        }
        if std::fs::remove_file(&p).is_ok() {
            count -= 1;
            total = total.saturating_sub(len);
            removed += 1;
        }
    }
    removed
}

// ------------------------------------------------------------------ anonymizing

/// Rebuilds an updater log line from known patterns, or drops it (None).
pub fn anonymize_log_line(line: &str) -> Option<String> {
    let mut parts = line.splitn(2, ' ');
    let stamp = parts.next()?;
    let rest = parts.next().unwrap_or("").trim();
    let stamp_ok = !stamp.is_empty() && stamp.chars().all(|c| c.is_ascii_digit() || c == '.') && stamp.contains('.');
    if !stamp_ok {
        return None;
    }
    let msg = if rest.starts_with("=== Log opened") {
        "=== Log opened".to_string()
    } else if let Some(s) = rest.strip_prefix("STATUS:") {
        let s = s.trim();
        let phrase = STATUS_PHRASES.iter().find(|p| s.starts_with(**p)).copied().unwrap_or("other");
        format!("STATUS: {phrase}")
    } else if let Some(r) = rest.split("To download: ").nth(1) {
        // "To download: N bytes in M files"
        let mut w = r.split_whitespace();
        let bytes: u64 = w.next()?.parse().ok()?;
        let files = r.split(" in ").nth(1).and_then(|x| x.split_whitespace().next()).and_then(|x| x.parse::<u64>().ok());
        match files {
            Some(f) => format!("To download: {bytes} bytes in {f} files"),
            None => format!("To download: {bytes} bytes"),
        }
    } else if rest.to_ascii_lowercase().contains("error") {
        "ERROR".to_string()
    } else {
        return None;
    };
    Some(format!("{stamp} {msg}"))
}

fn status_word(s: &str) -> Option<String> {
    STATUS_PHRASES.iter().find(|p| s.trim().starts_with(**p)).map(|p| p.to_string())
}

/// Turns one raw recording (format 1, NDJSON) into the anonymized format 2.
pub fn anonymize_recording(text: &str) -> String {
    let mut out = String::new();
    let mut rx0: Option<u64> = None;
    let mut free0: Option<u64> = None;
    let push = |v: Value, out: &mut String| {
        out.push_str(&v.to_string());
        out.push('\n');
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let t = v["t"].as_f64().unwrap_or(0.0);
        match v["type"].as_str() {
            Some("header") => push(
                json!({
                    "type": "header",
                    "format": EXPORT_FORMAT,
                    "anonymized": true,
                    "app": v["app"],
                    "action": v["action"].as_str().filter(|a| ["update", "repair", "install", "uninstall"].contains(a)),
                    "byHangar": v["byHangar"].as_bool(),
                    "installSeconds": v["installSeconds"],
                }),
                &mut out,
            ),
            Some("tick") => {
                let readings = v["windows"]
                    .as_array()
                    .map(|ws| {
                        ws.iter()
                            .map(|w| {
                                let mut texts: Vec<String> = Vec::new();
                                for key in ["texts", "win32", "uia"] {
                                    for x in w[key].as_array().into_iter().flatten() {
                                        if let Some(s) = x.as_str() {
                                            if !texts.iter().any(|e| e == s) {
                                                texts.push(s.to_string());
                                            }
                                        }
                                    }
                                }
                                parse_window_texts(&texts)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let r = best_reading(readings);
                let rx = v["netRx"].as_u64();
                if rx0.is_none() {
                    rx0 = rx;
                }
                let free = v["free"].as_u64();
                if free0.is_none() {
                    free0 = free;
                }
                let procs = v["procs"].as_array().map(|a| a.len() as u64).or(v["procs"].as_u64()).unwrap_or(0);
                let h = &v["hangar"];
                push(
                    json!({
                        "type": "tick",
                        "t": t,
                        "reading": {
                            "downloaded": r.downloaded.map(|d| [d.0, d.1, d.2]),
                            "unpacking": r.unpacking.map(|u| json!([u.0, u.1, u.2])),
                            "status": r.status.as_deref().and_then(status_word),
                        },
                        "netRx": match (rx, rx0) { (Some(a), Some(b)) => Some(a.saturating_sub(b)), _ => None },
                        "netScope": v["netScope"].as_str().filter(|s| ["updater", "pc"].contains(s)),
                        "dlBytes": v["dlBytes"].as_u64(),
                        "freeDelta": match (free, free0) { (Some(a), Some(b)) => Some(a as i64 - b as i64), _ => None },
                        "procs": procs,
                        "phase": v["phase"].as_str().and_then(status_word),
                        "hangar": { "eta": h["eta"].as_f64(), "low": h["low"].as_f64(), "high": h["high"].as_f64(), "confidence": h["confidence"].as_str().filter(|c| ["low", "medium", "high"].contains(c)) },
                    }),
                    &mut out,
                );
            }
            Some("log") => {
                let file = v["file"].as_str().unwrap_or("");
                if let (Some(f), Some(l)) = (LOG_FILES.iter().find(|n| **n == file), v["line"].as_str().and_then(anonymize_log_line)) {
                    push(json!({ "type": "log", "t": t, "file": f, "line": l }), &mut out);
                }
            }
            Some("end") => push(
                json!({
                    "type": "end",
                    "t": t,
                    "result": v["result"].as_str().filter(|r| ["success", "failed", "unknown", "cancelled"].contains(r)),
                    "exitCodes": v["exitCodes"].as_array().map(|a| a.iter().map(|c| c["code"].clone()).collect::<Vec<_>>()),
                }),
                &mut out,
            ),
            _ => {}
        }
    }
    out
}

fn export_id() -> String {
    let mut h = Sha256::new();
    h.update(format!("{:?}{}", std::time::SystemTime::now(), std::process::id()).as_bytes());
    h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub recordings: usize,
    pub bytes: u64,
    pub path: String,
}

/// Writes the anonymized zip. Only reads the recordings folder; writes only `out`.
pub fn export(dir: &Path, out: &Path) -> Result<ExportSummary, String> {
    let files = recording_files(dir);
    if files.is_empty() {
        return Err("There are no recordings to export.".into());
    }
    let tmp = out.with_extension("zip.part");
    let f = std::fs::File::create(&tmp).map_err(|e| format!("Could not create the file: {e}"))?;
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut n = 0;
    for (p, _) in &files {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let anon = anonymize_recording(&text);
        if !anon.contains("\"type\":\"tick\"") {
            continue;
        }
        n += 1;
        z.start_file(format!("recordings/{n:03}.ndjson"), o).map_err(|e| e.to_string())?;
        z.write_all(anon.as_bytes()).map_err(|e| e.to_string())?;
    }
    let manifest = json!({
        "format": EXPORT_FORMAT,
        "anonymized": true,
        "app": env!("CARGO_PKG_VERSION"),
        "exportId": export_id(),
        "recordings": n,
        "about": "Hangar ETA research recordings. Contains no clock times, paths, names or module names.",
    });
    z.start_file("manifest.json", o).map_err(|e| e.to_string())?;
    z.write_all(serde_json::to_string_pretty(&manifest).unwrap_or_default().as_bytes()).map_err(|e| e.to_string())?;
    z.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out).map_err(|e| format!("Could not save the file: {e}"))?;
    let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    Ok(ExportSummary { recordings: n, bytes, path: out.to_string_lossy().into_owned() })
}

// ------------------------------------------------------------------ commands

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecordingsOverview {
    pub enabled: bool,
    pub folder: Option<String>,
    pub count: usize,
    pub bytes: u64,
    /// "YYYYMMDD-HHMMSS" (UTC) from the file names.
    pub oldest: Option<String>,
    pub newest: Option<String>,
}

fn stamp_of(p: &Path) -> Option<String> {
    // "run-YYYYMMDD-HHMMSS" or, for a second recording in the same second, "...-2"
    let stem = p.file_stem()?.to_string_lossy().into_owned();
    stem.strip_prefix("run-").map(|s| s.chars().take(15).collect())
}

#[tauri::command]
pub fn recordings_overview() -> RecordingsOverview {
    let enabled = crate::settings::load().record_updates;
    let Some(dir) = recordings_dir() else { return RecordingsOverview { enabled, ..Default::default() } };
    let files = recording_files(&dir);
    RecordingsOverview {
        enabled,
        folder: dir.is_dir().then(|| dir.to_string_lossy().into_owned()),
        count: files.len(),
        bytes: files.iter().map(|f| f.1).sum(),
        oldest: files.first().and_then(|f| stamp_of(&f.0)),
        newest: files.last().and_then(|f| stamp_of(&f.0)),
    }
}

#[tauri::command]
pub async fn recordings_export(path: String) -> Result<ExportSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = recordings_dir().ok_or("No recordings folder.")?;
        let mut out = PathBuf::from(&path);
        if !out.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
            out.set_extension("zip");
        }
        export(&dir, &out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deletes Hangar's own recordings (run-*.ndjson). A recording that is being written is skipped.
#[tauri::command]
pub fn recordings_delete_all() -> usize {
    let Some(dir) = recordings_dir() else { return 0 };
    recording_files(&dir).into_iter().filter(|(p, _)| std::fs::remove_file(p).is_ok()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = r#"{"type":"header","format":1,"app":"0.2.1","startedUnix":1795809411,"action":"update","byHangar":true,"installSeconds":1.5}
{"type":"log","t":1.0,"file":"autoupdate_log.txt","line":"00000.000 === Log opened UTC 2026-11-27 19:56:51"}
{"type":"log","t":1.0,"file":"autoupdate_log.txt","line":"00000.131 --- Log file: C:\\Users\\jdoe\\AppData\\Local\\Temp\\DCS\\autoupdate_log.txt"}
{"type":"log","t":1.0,"file":"autoupdate_log.txt","line":"00001.500 INFO: install dir E:\\Games\\DCS World on PC-JDOE"}
{"type":"log","t":1.1,"file":"autoupdate_log.txt","line":"00002.000 To download: 123456789 bytes in 42 files (HEATBLUR_F-14, KOLA_terrain)"}
{"type":"log","t":1.2,"file":"autoupdate_log.txt","line":"00002.100 STATUS: Downloading update for KOLA_terrain"}
{"type":"log","t":1.2,"file":"C:\\Users\\jdoe\\secret.txt","line":"00002.100 STATUS: Downloading update"}
{"type":"tick","t":2.0,"windows":[{"pid":4242,"win32":["DCS Updater 2.17.3.11","Downloading the update...","Downloaded: 10.0 of 100.0 MBytes    Speed: 5.0 MB/s","Unpacking 20.0 of 200.0 MBytes (ETA 0:00:36)","Mods/terrains/Kola/data.zip"],"uia":[]}],"netRx":5000000000,"netScope":"updater","dlBytes":1000,"free":270000000000,"procs":[4242,4343],"phase":"Downloading update","hangar":{"eta":30.0,"low":28.0,"high":36.0,"confidence":"high"}}
{"type":"tick","t":3.0,"windows":[],"netRx":5010000000,"netScope":"updater","dlBytes":2000,"free":269990000000,"procs":[4242],"phase":null,"hangar":{"eta":null,"low":null,"high":null,"confidence":null}}
{"type":"end","t":4.0,"result":"success","resultText":"Successfully updated to 2.9.32.30490 for jdoe","exitCodes":[{"pid":4242,"code":0}]}
"#;

    #[test]
    fn export_leaves_out_private_data() {
        let anon = anonymize_recording(RAW);
        for bad in ["jdoe", "JDOE", "Users", "E:\\", "Games", "HEATBLUR", "KOLA", "Kola", "1795809411", "19:56", "2026-11-27", "4242", "4343", "270000000000", "secret", "resultText"] {
            assert!(!anon.contains(bad), "export still contains {bad:?}:\n{anon}");
        }
        assert!(anon.contains("\"anonymized\":true"));
        assert!(anon.contains("00002.000 To download: 123456789 bytes in 42 files"));
        assert!(anon.contains("00002.100 STATUS: Downloading update"));
        assert!(anon.contains("\"procs\":2"));
        assert!(anon.contains("\"freeDelta\":-10000000"));
        assert!(anon.contains("\"downloaded\":[10.0,100.0,5.0]"), "{anon}");
        assert!(anon.contains("\"status\":\"Downloading the update\""));
    }

    #[test]
    fn export_still_replays() {
        let dir = std::env::temp_dir().join(format!("hangar-share-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("run-20261127-195651.ndjson"), RAW).unwrap();
        let out = dir.join("export.zip");
        let s = export(&dir, &out).unwrap();
        assert_eq!(s.recordings, 1);
        let recs = crate::replay::load_zip(&out).unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.ticks.len(), 2);
        assert_eq!(r.ticks[0].reading.downloaded, Some((10.0, 100.0, 5.0)));
        assert_eq!(r.ticks[0].procs, 2);
        assert_eq!(r.ticks[0].net_rx, Some(0));
        assert!(r.logs.iter().any(|l| l.line.contains("STATUS: Downloading update")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prune_keeps_newest_and_current() {
        let dir = std::env::temp_dir().join(format!("hangar-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..(MAX_RECORDINGS + 3) {
            std::fs::write(dir.join(format!("run-20260101-{i:06}.ndjson")), "x").unwrap();
        }
        let current = dir.join("run-20260101-000000.ndjson"); // the oldest name, but being written
        assert_eq!(prune(&dir, Some(&current)), 3);
        assert!(current.exists());
        assert_eq!(recording_files(&dir).len(), MAX_RECORDINGS);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod real {
    /// Exports the real recordings on this PC to a temp zip and checks that no user name,
    /// computer name or drive path is left. Read-only on Hangar's own data; writes only to %TEMP%.
    /// `cargo test --lib real_recordings_export_is_clean -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_recordings_export_is_clean() {
        use std::io::Read;
        let dir = crate::recorder::recordings_dir().unwrap();
        let out = std::env::temp_dir().join("hangar-real-export-check.zip");
        let s = super::export(&dir, &out).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut all = String::new();
        for i in 0..z.len() {
            z.by_index(i).unwrap().read_to_string(&mut all).unwrap();
        }
        let mut bad: Vec<String> = ["USERNAME", "COMPUTERNAME"].iter().filter_map(|k| std::env::var(k).ok()).collect();
        bad.extend([":\\", ":/", "Users", "Saved Games", "terrain", "Mods/", "UTC 20"].map(String::from));
        for b in &bad {
            assert!(!all.to_lowercase().contains(&b.to_lowercase()), "found {b:?}");
        }
        let recs = crate::replay::load_zip(&out).unwrap();
        let scored = recs.iter().filter(|r| crate::replay::run_candidates(r).is_some()).count();
        let raw = super::recording_files(&dir)
            .iter()
            .filter_map(|(p, _)| crate::replay::load_recording(p).ok())
            .filter(|r| crate::replay::run_candidates(r).is_some())
            .count();
        println!("{} recordings exported, {} bytes, {} replayable (raw: {raw})", s.recordings, s.bytes, scored);
        assert_eq!(scored, raw, "the export must replay as well as the raw recordings");
        if std::env::var_os("HANGAR_KEEP_EXPORT").is_none() {
            std::fs::remove_file(&out).unwrap();
        }
    }
}
