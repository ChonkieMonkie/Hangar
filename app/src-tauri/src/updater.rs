// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Starts DCS_updater.exe on the user's click (separate process) and observes any running
//! updater: process chain, exit codes, autoupdate_log.txt and the updater window's texts.
//!
//! Observation is read only. The window is read with UI Automation; Hangar never clicks,
//! types or sends input to it. See docs/DECISIONS.md B-010, B-012.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use regex::Regex;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

use crate::eta::{HangarAnswer, HangarEta, INSTALL_SECONDS, NetRate, Obs};
use crate::recorder::Recorder;
use crate::window::{WindowReading, best_reading, parse_window_texts};
use crate::winutil::{self, ProcHandle};

const UPDATER_EXE: &str = "DCS_updater.exe";
const HISTORY_LEN: usize = 120;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcView {
    pub pid: u32,
    pub ppid: u32,
    pub exit_code: Option<u32>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonitorState {
    /// "idle" | "running" | "finished"
    pub status: String,
    pub started_by_hangar: bool,
    pub action: Option<String>,
    pub install_path: Option<String>,
    pub elapsed_s: f64,
    pub phase: Option<String>,
    pub window_readable: bool,
    /// Where progress comes from: "window" | "downloads" | "none"
    pub source: String,
    pub downloaded_mb: Option<f64>,
    pub download_total_mb: Option<f64>,
    pub ed_speed_mbs: Option<f64>,
    pub unpacked_mb: Option<f64>,
    pub unpacked_total_mb: Option<f64>,
    pub ed_eta_s: Option<f64>,
    pub current_file: Option<String>,
    pub eta_s: Option<f64>,
    pub eta_low_s: Option<f64>,
    pub eta_high_s: Option<f64>,
    pub confidence: Option<String>,
    pub unpack_rate_mbs: Option<f64>,
    pub net_rate_mbs: f64,
    /// "updater" = DCS updater's own download rate (from its window); "pc" = whole PC
    /// (only when the window gives no figure).
    pub net_scope: String,
    pub net_history: Vec<f64>,
    pub unpack_history: Vec<f64>,
    pub free_bytes: Option<u64>,
    pub to_download: Option<u64>,
    pub log_tail: Vec<String>,
    pub processes: Vec<ProcView>,
    /// "success" | "failed" | "unknown"
    pub result: Option<String>,
    pub result_text: Option<String>,
}

impl Default for MonitorState {
    fn default() -> Self {
        MonitorState {
            status: "idle".into(),
            started_by_hangar: false,
            action: None,
            install_path: None,
            elapsed_s: 0.0,
            phase: None,
            window_readable: false,
            source: "none".into(),
            downloaded_mb: None,
            download_total_mb: None,
            ed_speed_mbs: None,
            unpacked_mb: None,
            unpacked_total_mb: None,
            ed_eta_s: None,
            current_file: None,
            eta_s: None,
            eta_low_s: None,
            eta_high_s: None,
            confidence: None,
            unpack_rate_mbs: None,
            net_rate_mbs: 0.0,
            net_scope: "pc".into(),
            net_history: Vec::new(),
            unpack_history: Vec::new(),
            free_bytes: None,
            to_download: None,
            log_tail: Vec::new(),
            processes: Vec::new(),
            result: None,
            result_text: None,
        }
    }
}

#[derive(Default)]
pub struct MonitorShared {
    pub state: Mutex<MonitorState>,
    /// The CLI process Hangar started (to reap it and read its exit code).
    pub child: Mutex<Option<Child>>,
    pub pending_action: Mutex<Option<String>>,
}

// ------------------------------------------------------------------ commands

#[tauri::command]
pub fn get_monitor_state(shared: State<'_, MonitorShared>) -> MonitorState {
    shared.state.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Starts `DCS_updater.exe <action>` as a separate process. Only "update" and "repair"
/// are allowed in 0.1. Called only from the user's click in the UI.
#[tauri::command]
pub fn start_updater(action: String, install_path: String, shared: State<'_, MonitorShared>) -> Result<(), String> {
    if action != "update" && action != "repair" {
        return Err(format!("Action not allowed: {action}"));
    }
    if !winutil::processes_named(UPDATER_EXE).is_empty() {
        return Err("DCS updater is already running.".into());
    }
    if !winutil::processes_named("DCS.exe").is_empty() {
        return Err("Close DCS before updating.".into());
    }
    let exe = Path::new(&install_path).join("bin").join(UPDATER_EXE);
    if !exe.is_file() {
        return Err(format!("Updater not found: {}", exe.display()));
    }
    // B-009: snapshot Config before every update started from Hangar (if switched on)
    crate::commands::backup_before_update()
        .map_err(|e| format!("Backup before the update failed: {e}. Fix it in Backups, or switch off \"Before every update\"."))?;
    let child = Command::new(&exe)
        .arg(&action)
        .current_dir(Path::new(&install_path))
        .spawn()
        .map_err(|e| format!("Could not start the updater: {e}"))?;
    *shared.child.lock().map_err(|e| e.to_string())? = Some(child);
    *shared.pending_action.lock().map_err(|e| e.to_string())? = Some(action);
    Ok(())
}

// ------------------------------------------------------------------ parsing helpers

fn re_to_download() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"To download: (\d+) bytes in (\d+) files").unwrap())
}
fn re_log_line() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^\d+\.\d+ (\w+)\s*: (.*)$").unwrap())
}
// ------------------------------------------------------------------ monitor loop

struct Tail {
    offset: u64,
    created: Option<SystemTime>,
    rest: String,
}

struct Run {
    t0: Instant,
    handles: HashMap<u32, ProcHandle>,
    procs: HashMap<u32, ProcView>,
    root: Option<PathBuf>,
    tails: HashMap<PathBuf, Tail>,
    last_net: (Instant, u64),
    samples: VecDeque<(f64, f64)>,
    last_unpacked: Option<(f64, f64)>,
    net_hist: VecDeque<f64>,
    unpack_hist: VecDeque<f64>,
    log_tail: VecDeque<String>,
    gone_since: Option<Instant>,
    to_download_first: Option<u64>,
    to_download: Option<u64>,
    log_phase: Option<String>,
    result: Option<String>,
    result_text: Option<String>,
    tick: u64,
    eta: HangarEta,
    last_answer: Option<(f64, HangarAnswer)>,
    /// The updater's own download rate (from its window).
    net_rate: NetRate,
    net_scope_updater: bool,
    rec: Recorder,
    /// Log lines read this tick, written to the recording as (file name, line).
    new_log: Vec<(String, String)>,
}

fn temp_logs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("dcs") {
                v.push(e.path().join("autoupdate_templog.txt"));
            }
        }
    }
    v
}

fn file_created(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok().and_then(|m| m.created().ok())
}

impl Run {
    fn new(root: Option<PathBuf>, rec: Recorder) -> Self {
        let mut run = Run {
            t0: Instant::now(),
            handles: HashMap::new(),
            procs: HashMap::new(),
            root: None,
            tails: HashMap::new(),
            last_net: (Instant::now(), winutil::net_rx_total()),
            samples: VecDeque::new(),
            last_unpacked: None,
            net_hist: VecDeque::new(),
            unpack_hist: VecDeque::new(),
            log_tail: VecDeque::new(),
            gone_since: None,
            to_download_first: None,
            to_download: None,
            log_phase: None,
            result: None,
            result_text: None,
            tick: 0,
            eta: HangarEta::default(),
            last_answer: None,
            net_rate: NetRate::default(),
            net_scope_updater: false,
            rec,
            new_log: Vec::new(),
        };
        for p in temp_logs() {
            run.start_tail_at_end(p);
        }
        if let Some(r) = root {
            run.set_root(r);
        }
        run
    }

    fn set_root(&mut self, root: PathBuf) {
        self.start_tail_at_end(root.join("autoupdate_log.txt"));
        self.root = Some(root);
    }

    /// Existing logs are read from their current end; logs that appear later from the start.
    fn start_tail_at_end(&mut self, p: PathBuf) {
        let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let created = file_created(&p);
        self.tails.insert(p, Tail { offset: len, created, rest: String::new() });
    }

    fn read_logs(&mut self) {
        let mut paths: Vec<PathBuf> = self.tails.keys().cloned().collect();
        for p in temp_logs() {
            if !self.tails.contains_key(&p) {
                paths.push(p);
            }
        }
        for p in paths {
            if !p.is_file() {
                continue;
            }
            let created = file_created(&p);
            let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            let tail = self.tails.entry(p.clone()).or_insert(Tail { offset: 0, created, rest: String::new() });
            if tail.created != created || len < tail.offset {
                tail.offset = 0; // rotated or recreated
                tail.created = created;
                tail.rest.clear();
            }
            if len == tail.offset {
                continue;
            }
            let Ok((chunk, new_off)) = winutil::read_shared_from(&p, tail.offset) else { continue };
            tail.offset = new_off;
            let text = std::mem::take(&mut tail.rest) + &chunk;
            let mut lines: Vec<&str> = text.split('\n').collect();
            let rest = lines.pop().unwrap_or("").to_string();
            let complete: Vec<String> = lines.iter().map(|l| l.trim_end_matches('\r').to_string()).collect();
            if let Some(t) = self.tails.get_mut(&p) {
                t.rest = rest;
            }
            let fname = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            for l in complete {
                self.new_log.push((fname.clone(), winutil::anonymize(&l)));
                self.handle_log_line(&l);
            }
        }
    }

    fn handle_log_line(&mut self, line: &str) {
        let Some(c) = re_log_line().captures(line) else { return };
        let (level, msg) = (c[1].to_string(), c[2].trim().to_string());
        let mut interesting = false;
        if let Some(m) = re_to_download().captures(&msg) {
            let v: u64 = m[1].parse().unwrap_or(0);
            if self.to_download_first.is_none() {
                self.to_download_first = Some(v);
            }
            self.to_download = Some(v);
            interesting = true;
        }
        if level == "STATUS" {
            let name = msg.trim_end_matches('.').trim().to_string();
            if name.starts_with("Successfully updated to") {
                self.result = Some("success".into());
            } else if name.to_ascii_lowercase().contains("not enough disk space") {
                self.result = Some("failed".into());
            }
            self.result_text = Some(msg.clone());
            self.log_phase = Some(name);
            interesting = true;
        } else if msg.starts_with("Installed ") || msg.starts_with("Uninstalled ") {
            if self.result.is_none() {
                self.result = Some("success".into());
            }
            interesting = true;
        } else if level.eq_ignore_ascii_case("ERROR") {
            self.result = Some("failed".into());
            interesting = true;
        }
        if interesting {
            self.log_tail.push_back(winutil::anonymize(line));
            while self.log_tail.len() > 10 {
                self.log_tail.pop_front();
            }
        }
    }
}

fn push_hist(q: &mut VecDeque<f64>, v: f64) {
    q.push_back(v);
    while q.len() > HISTORY_LEN {
        q.pop_front();
    }
}

pub fn start_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let uia: Option<IUIAutomation> = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        };
        let shared = app.state::<MonitorShared>();
        let mut run: Option<Run> = None;

        loop {
            let procs = winutil::processes_named(UPDATER_EXE);

            if run.is_none() && !procs.is_empty() {
                let root = procs.iter().find_map(|p| ProcHandle::open(p.pid).and_then(|h| h.image_path()))
                    .and_then(|exe| exe.parent().and_then(|bin| bin.parent()).map(Path::to_path_buf));
                let action = shared.pending_action.lock().ok().and_then(|mut a| a.take());
                let by_hangar = shared.child.lock().map(|c| c.is_some()).unwrap_or(false);
                if let Ok(mut s) = shared.state.lock() {
                    *s = MonitorState { status: "running".into(), started_by_hangar: by_hangar, action: action.clone(), ..Default::default() };
                }
                let rec = Recorder::start(action.as_deref(), by_hangar);
                run = Some(Run::new(root, rec));
            }

            if let Some(r) = run.as_mut() {
                r.tick += 1;
                let now = Instant::now();
                let t = now.duration_since(r.t0).as_secs_f64();

                // processes and exit codes
                for p in &procs {
                    r.procs.entry(p.pid).or_insert(ProcView { pid: p.pid, ppid: p.ppid, exit_code: None });
                    if !r.handles.contains_key(&p.pid) {
                        if let Some(h) = ProcHandle::open(p.pid) {
                            if r.root.is_none() {
                                if let Some(root) = h.image_path().and_then(|e| e.parent().and_then(|b| b.parent()).map(Path::to_path_buf)) {
                                    r.set_root(root);
                                }
                            }
                            r.handles.insert(p.pid, h);
                        }
                    }
                }
                for (pid, view) in r.procs.iter_mut() {
                    if view.exit_code.is_none() && !procs.iter().any(|p| p.pid == *pid) {
                        view.exit_code = r.handles.get(pid).and_then(|h| h.exit_code());
                    }
                }
                if let Ok(mut c) = shared.child.lock() {
                    if let Some(child) = c.as_mut() {
                        if let Ok(Some(_)) = child.try_wait() {
                            *c = None;
                        }
                    }
                }

                // log
                r.read_logs();

                // window (UI Automation, read only)
                let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
                let mut readable = false;
                let mut readings: Vec<WindowReading> = Vec::new();
                let mut raw_windows = Vec::new();
                if let Some(u) = &uia {
                    for w in winutil::windows_of(&pids).into_iter().filter(|w| w.visible && w.class == "#32770") {
                        let win32 = winutil::child_texts(w.hwnd);
                        let uia_t = winutil::uia_texts(u, w.hwnd);
                        let mut texts = win32.clone();
                        for t in &uia_t {
                            if !texts.contains(t) {
                                texts.push(t.clone());
                            }
                        }
                        let rd = parse_window_texts(&texts);
                        if rd.has_progress() || rd.status.is_some() {
                            readable = true;
                        }
                        readings.push(rd);
                        raw_windows.push(json!({ "pid": w.pid, "win32": win32, "uia": uia_t }));
                    }
                }
                // Prefer the window that shows progress (the elevated worker).
                let reading = best_reading(readings);

                // size of _downloads every second tick (recorded; fallback progress source)
                let dl_bytes = match (r.tick % 2 == 0, r.root.as_ref()) {
                    (true, Some(root)) if root.join("_downloads").is_dir() => Some(winutil::folder_size(&root.join("_downloads"))),
                    _ => None,
                };

                // unpacked progress: window first, else _downloads vs first "To download"
                let mut source = "none";
                let mut unpacked: Option<(f64, f64)> = None;
                if let Some((done, total, _)) = reading.unpacking {
                    unpacked = Some((done, total));
                    source = "window";
                } else if let Some(first) = r.to_download_first {
                    unpacked = match dl_bytes {
                        Some(b) => Some((b as f64 / 1e6, first as f64 / 1e6)),
                        None => r.last_unpacked,
                    };
                    source = "downloads";
                }

                // network: the updater's own download rate, from the compressed figure in its
                // window (median of the last 5 per-tick rates: ED's counter adds ~30 MB in one
                // step about once a minute without matching traffic, ETA.md). The whole-PC
                // counter is shown only until that figure first appears.
                let rx = winutil::net_rx_total();
                let dt = now.duration_since(r.last_net.0).as_secs_f64().max(0.2);
                let pc_rate = rx.saturating_sub(r.last_net.1) as f64 / dt / 1e6;
                r.last_net = (now, rx);
                let mut updater_rate = None;
                if let Some((done, _, _)) = reading.downloaded {
                    if !r.net_scope_updater {
                        r.net_scope_updater = true;
                        r.net_hist.clear();
                    }
                    updater_rate = r.net_rate.push(t, done);
                }
                let net_rate = if r.net_scope_updater { updater_rate.unwrap_or(0.0) } else { pc_rate };
                push_hist(&mut r.net_hist, net_rate);

                let mut unpack_rate = None;
                if let Some((done, _)) = unpacked {
                    if let Some((pt, pdone)) = r.samples.back().copied() {
                        if t - pt > 0.2 {
                            unpack_rate = Some(((done - pdone) / (t - pt)).max(0.0));
                        }
                    }
                    r.samples.push_back((t, done));
                    while r.samples.len() > 61 {
                        r.samples.pop_front();
                    }
                    push_hist(&mut r.unpack_hist, unpack_rate.unwrap_or(0.0));
                }
                r.last_unpacked = unpacked;

                // phase: the worker's status while it shows progress, otherwise the log
                // (the CLI window keeps saying "Waiting for operation to complete").
                let phase = if reading.has_progress() {
                    reading.status.clone().or_else(|| r.log_phase.clone())
                } else {
                    r.log_phase.clone().or_else(|| reading.status.clone())
                };
                let after_download = phase
                    .as_deref()
                    .map(|p| p.starts_with("Installing") || p.starts_with("Cleaning up") || p.starts_with("Successfully"))
                    .unwrap_or(false);

                // ETA (eta.rs, chosen with the replay; docs/ETA.md)
                let obs = if reading.has_progress() {
                    Some(Obs {
                        t,
                        comp: reading.downloaded.map(|d| (d.0, d.1)),
                        unp: reading.unpacking.map(|u| (u.0, u.1)),
                        ed_speed: reading.downloaded.map(|d| d.2),
                        ed_eta: reading.unpacking.and_then(|u| u.2),
                    })
                } else if source == "downloads" && dl_bytes.is_some() && !after_download {
                    unpacked.map(|u| Obs { t, unp: Some(u), ..Default::default() })
                } else {
                    None
                };
                if let Some(a) = obs.and_then(|o| r.eta.answer(&o)) {
                    r.last_answer = Some((t, a));
                }
                // hold the last answer briefly (counting down) between readings
                let answer = match r.last_answer {
                    Some((at, a)) if !after_download && t - at <= 2.5 => {
                        let d = t - at;
                        Some(HangarAnswer {
                            pred: crate::eta::Pred {
                                remaining: (a.pred.remaining - d).max(0.0),
                                low: a.pred.low.map(|l| (l - d).max(0.0)),
                                high: a.pred.high.map(|h| (h - d).max(0.0)),
                            },
                            ..a
                        })
                    }
                    _ => None,
                };

                // finished?
                let mut finished = false;
                if procs.is_empty() {
                    let since = *r.gone_since.get_or_insert(now);
                    if now.duration_since(since) >= Duration::from_secs(3) {
                        finished = true;
                    }
                } else {
                    r.gone_since = None;
                }

                if let Ok(mut s) = shared.state.lock() {
                    s.status = if finished { "finished".into() } else { "running".into() };
                    s.install_path = r.root.as_ref().map(|p| p.to_string_lossy().into_owned());
                    s.elapsed_s = t;
                    s.phase = phase;
                    s.window_readable = readable || s.window_readable;
                    s.source = source.into();
                    if let Some((d, tot, sp)) = reading.downloaded {
                        s.downloaded_mb = Some(d);
                        s.download_total_mb = Some(tot);
                        s.ed_speed_mbs = Some(sp);
                    }
                    if let Some((d, tot)) = unpacked {
                        s.unpacked_mb = Some(d);
                        s.unpacked_total_mb = Some(tot);
                    }
                    if let Some((_, _, eta)) = reading.unpacking {
                        s.ed_eta_s = eta;
                    }
                    if reading.current_file.is_some() {
                        s.current_file = reading.current_file.clone();
                    }
                    if after_download {
                        s.eta_s = Some(INSTALL_SECONDS);
                        s.eta_low_s = None;
                        s.eta_high_s = None;
                        s.confidence = None;
                    } else if let Some(a) = &answer {
                        s.eta_s = Some(a.pred.remaining + INSTALL_SECONDS);
                        s.eta_low_s = a.pred.low.map(|l| l + INSTALL_SECONDS);
                        s.eta_high_s = a.pred.high.map(|h| h + INSTALL_SECONDS);
                        s.confidence = Some(a.confidence.into());
                    } else {
                        // before the download phase (or no progress source): no number
                        s.eta_s = None;
                        s.eta_low_s = None;
                        s.eta_high_s = None;
                        s.confidence = None;
                    }
                    if let Some(u) = unpack_rate {
                        s.unpack_rate_mbs = Some(u);
                    }
                    s.net_rate_mbs = net_rate;
                    s.net_scope = if r.net_scope_updater { "updater".into() } else { "pc".into() };
                    s.net_history = r.net_hist.iter().copied().collect();
                    s.unpack_history = r.unpack_hist.iter().copied().collect();
                    s.free_bytes = r.root.as_ref().and_then(|p| winutil::disk_space(p)).map(|x| x.0);
                    s.to_download = r.to_download;
                    s.log_tail = r.log_tail.iter().cloned().collect();
                    let mut pv: Vec<ProcView> = r.procs.values().cloned().collect();
                    pv.sort_by_key(|p| p.pid);
                    s.processes = pv;
                    if finished {
                        s.eta_s = None;
                        s.eta_low_s = None;
                        s.eta_high_s = None;
                        let codes_ok = !s.processes.is_empty() && s.processes.iter().all(|p| p.exit_code == Some(0));
                        s.result = r.result.clone().or_else(|| Some(if codes_ok { "success".into() } else { "unknown".into() }));
                        s.result_text = r.result_text.clone();
                    }
                    let _ = app.emit("updater-state", s.clone());
                    // recording (Hangar's own data; see recorder.rs)
                    for (file, line) in r.new_log.drain(..) {
                        r.rec.write(json!({ "type": "log", "t": t, "file": file, "line": line }));
                    }
                    r.rec.write(json!({
                        "type": "tick",
                        "t": t,
                        "windows": raw_windows,
                        "netRx": rx,
                        "netScope": s.net_scope,
                        "dlBytes": dl_bytes,
                        "free": s.free_bytes,
                        "procs": procs.iter().map(|p| p.pid).collect::<Vec<_>>(),
                        "phase": s.phase,
                        "hangar": { "eta": s.eta_s, "low": s.eta_low_s, "high": s.eta_high_s, "confidence": s.confidence },
                    }));
                    if finished {
                        r.rec.write(json!({
                            "type": "end",
                            "t": t,
                            "result": s.result,
                            "resultText": s.result_text,
                            "exitCodes": s.processes.iter().map(|p| json!({ "pid": p.pid, "code": p.exit_code })).collect::<Vec<_>>(),
                        }));
                    }
                    r.rec.flush();
                }
                if finished {
                    run = None;
                }
            }

            std::thread::sleep(Duration::from_millis(if run.is_some() { 1000 } else { 1500 }));
        }
    });
}
