// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Offline replay of recorded updater runs: feeds each recorded window reading to every ETA
//! estimator (causally, one tick at a time) and scores the answers against what actually
//! happened. Used by `examples/eta_replay.rs`. See docs/ETA.md.

use std::path::Path;

use regex::Regex;
use serde_json::Value;

use crate::eta::{self, Estimator, Obs, Pred};
use crate::window::{WindowReading, best_reading, parse_window_texts};

#[derive(Clone, Debug, Default)]
pub struct Tick {
    pub t: f64,
    pub reading: WindowReading,
    /// What Hangar showed live (whole run, incl. install), if recorded.
    pub live: Option<Pred>,
    pub procs: usize,
    /// Whole-PC received bytes (network counter).
    pub net_rx: Option<u64>,
    /// Size of _downloads (every second tick).
    pub dl_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub t: f64,
    pub file: String,
    pub line: String,
}

#[derive(Clone, Debug, Default)]
pub struct Recording {
    pub name: String,
    pub action: Option<String>,
    pub ticks: Vec<Tick>,
    pub logs: Vec<LogLine>,
    pub end_t: Option<f64>,
    /// Tail Hangar added to its live ETA when this was recorded (0.1: 3 s).
    pub install_seconds: f64,
}

// ------------------------------------------------------------------ loading

/// Reads a Hangar recording (`run-*.ndjson`, see recorder.rs).
pub fn load_recording(path: &Path) -> Result<Recording, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_recording(&text, path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Reads an anonymized export from ETA research (share.rs): a zip of format-2 recordings.
/// Each recording is named `<exportId>/<NN>` so runs from one sender can be grouped.
pub fn load_zip(path: &Path) -> Result<Vec<Recording>, String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Ok(mut m) = z.by_name("manifest.json") {
        let mut t = String::new();
        if m.read_to_string(&mut t).is_ok() {
            if let Some(x) = serde_json::from_str::<Value>(&t).ok().and_then(|v| v["exportId"].as_str().map(str::to_string)) {
                id = x;
            }
        }
    }
    let mut out = Vec::new();
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let name = e.name().to_string();
        if !(name.starts_with("recordings/") && name.ends_with(".ndjson")) || e.size() > 64 * 1024 * 1024 {
            continue;
        }
        let mut t = String::new();
        e.read_to_string(&mut t).map_err(|e| format!("{name}: {e}"))?;
        let stem = name.trim_start_matches("recordings/").trim_end_matches(".ndjson");
        out.push(parse_recording(&t, format!("{id}/{stem}"))?);
    }
    Ok(out)
}

fn parse_recording(text: &str, name: String) -> Result<Recording, String> {
    let path = std::path::PathBuf::from(&name);
    let mut rec = Recording { name, ..Default::default() };
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) if i + 1 == text.lines().count() => break, // torn last line (app closed mid-write)
            Err(e) => return Err(format!("{}:{}: {e}", path.display(), i + 1)),
        };
        let t = v["t"].as_f64().unwrap_or(0.0);
        match v["type"].as_str() {
            Some("header") => {
                rec.action = v["action"].as_str().map(str::to_string);
                rec.install_seconds = v["installSeconds"].as_f64().unwrap_or(eta::V01_INSTALL_SECONDS);
            }
            Some("tick") if v.get("reading").is_some_and(Value::is_object) => {
                // format 2 (anonymized export): the window reading is already parsed
                let r = &v["reading"];
                let num = |a: &[Value], i: usize| a.get(i).and_then(Value::as_f64);
                let reading = WindowReading {
                    downloaded: r["downloaded"].as_array().and_then(|a| Some((num(a, 0)?, num(a, 1)?, num(a, 2)?))),
                    unpacking: r["unpacking"].as_array().and_then(|a| Some((num(a, 0)?, num(a, 1)?, num(a, 2)))),
                    status: r["status"].as_str().map(str::to_string),
                    current_file: None,
                };
                let h = &v["hangar"];
                rec.ticks.push(Tick {
                    t,
                    reading,
                    live: h["eta"].as_f64().map(|e| Pred { remaining: e, low: h["low"].as_f64(), high: h["high"].as_f64() }),
                    procs: v["procs"].as_u64().unwrap_or(0) as usize,
                    net_rx: v["netRx"].as_u64(),
                    dl_bytes: v["dlBytes"].as_u64(),
                });
            }
            Some("tick") => {
                let readings = v["windows"]
                    .as_array()
                    .map(|ws| {
                        ws.iter()
                            .map(|w| {
                                let mut texts: Vec<String> = Vec::new();
                                for key in ["texts", "win32", "uia"] {
                                    for x in w[key].as_array().into_iter().flatten() {
                                        if let Some(t) = x.as_str() {
                                            if !texts.iter().any(|e| e == t) {
                                                texts.push(t.to_string());
                                            }
                                        }
                                    }
                                }
                                parse_window_texts(&texts)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let h = &v["hangar"];
                let live = h["eta"].as_f64().map(|e| Pred { remaining: e, low: h["low"].as_f64(), high: h["high"].as_f64() });
                let procs = v["procs"].as_array().map(|a| a.len()).unwrap_or(0);
                rec.ticks.push(Tick {
                    t,
                    reading: best_reading(readings),
                    live,
                    procs,
                    net_rx: v["netRx"].as_u64(),
                    dl_bytes: v["dlBytes"].as_u64(),
                });
            }
            Some("log") => rec.logs.push(LogLine {
                t,
                file: v["file"].as_str().unwrap_or("").to_string(),
                line: v["line"].as_str().unwrap_or("").to_string(),
            }),
            Some("end") => rec.end_t = Some(t),
            _ => {}
        }
    }
    Ok(rec)
}

/// Imports a run recorded by `tools/watch-updater.ps1` (the `-uia.txt` dump plus the events
/// in `-summary.md`). Coarser (window read every ~2 s) but useful for testing.
pub fn import_watcher(uia_path: &Path, summary_path: &Path) -> Result<Recording, String> {
    let uia = std::fs::read_to_string(uia_path).map_err(|e| format!("{}: {e}", uia_path.display()))?;
    let summary = std::fs::read_to_string(summary_path).unwrap_or_default();
    let head = Regex::new(r"^--- t=([\d.]+) pid=(\d+)").unwrap();
    let name = Regex::new(r#"name="(.*?)" \|"#).unwrap();
    let mut rec = Recording { name: uia_path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(), ..Default::default() };
    // group blocks by t (several windows can share a timestamp)
    let mut blocks: Vec<(f64, Vec<Vec<String>>)> = Vec::new();
    for line in uia.lines() {
        if let Some(c) = head.captures(line) {
            let t: f64 = c[1].parse().unwrap_or(0.0);
            match blocks.last_mut() {
                Some((bt, ws)) if (*bt - t).abs() < 1e-9 => ws.push(Vec::new()),
                _ => blocks.push((t, vec![Vec::new()])),
            }
        } else if let (Some(c), Some((_, ws))) = (name.captures(line), blocks.last_mut()) {
            if let Some(w) = ws.last_mut() {
                w.push(c[1].to_string());
            }
        }
    }
    for (t, ws) in blocks {
        let readings = ws.iter().map(|w| parse_window_texts(w)).collect();
        rec.ticks.push(Tick { t, reading: best_reading(readings), procs: 1, ..Default::default() });
    }
    let ev = Regex::new(r"^\s*([\d]+[,.]\d+)s (\S+)\s+(.*)$").unwrap();
    for line in summary.lines() {
        if let Some(c) = ev.captures(line) {
            let t: f64 = c[1].replace(',', ".").parse().unwrap_or(0.0);
            let (src, rest) = (c[2].to_string(), c[3].trim().to_string());
            if src == "process-exit" {
                rec.end_t = Some(rec.end_t.map_or(t, |e: f64| e.max(t)));
            } else if src.ends_with(".txt") {
                rec.logs.push(LogLine { t, file: src, line: rest });
            }
        }
    }
    // ticks after the last process exit have no processes
    if let Some(e) = rec.end_t {
        for tk in rec.ticks.iter_mut() {
            if tk.t >= e {
                tk.procs = 0;
            }
        }
        rec.ticks.push(Tick { t: e + 0.01, ..Default::default() });
    }
    Ok(rec)
}

// ------------------------------------------------------------------ ground truth

#[derive(Clone, Debug)]
pub struct Truth {
    /// First observation with progress.
    pub p_start: f64,
    /// When the download phase ended (unpacking done, "Installing new files").
    pub d_end: f64,
    pub d_end_source: &'static str,
    /// When the updater processes were gone.
    pub r_end: Option<f64>,
    pub comp_total: Option<f64>,
    pub unp_total: Option<f64>,
    /// Progress was seen again after d_end (several download phases): only the first is scored.
    pub multi_phase: bool,
    /// "window" = read from ED's window; "reconstructed" = window texts missing, compressed
    /// progress rebuilt from the network counter and unpacked progress from _downloads.
    pub source: &'static str,
}

fn log_rel_seconds(line: &str) -> Option<f64> {
    line.split_whitespace().next()?.parse().ok()
}

/// Maps log lines of autoupdate_log.txt to run time precisely: a line's own relative stamp plus
/// the smallest (tick time - stamp) seen in that log session (lines are read at most one tick late).
fn log_times(rec: &Recording) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    let mut session: Vec<(f64, f64, String)> = Vec::new(); // (tick t, rel, msg)
    let flush = |session: &mut Vec<(f64, f64, String)>, out: &mut Vec<(f64, String)>| {
        if let Some(off) = session.iter().map(|(t, rel, _)| t - rel).reduce(f64::min) {
            for (_, rel, msg) in session.drain(..) {
                out.push((rel + off, msg));
            }
        }
    };
    for l in rec.logs.iter().filter(|l| l.file == "autoupdate_log.txt") {
        if l.line.contains("=== Log opened") {
            flush(&mut session, &mut out);
        }
        if let Some(rel) = log_rel_seconds(&l.line) {
            let msg = l.line.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
            session.push((l.t, rel, msg));
        }
    }
    flush(&mut session, &mut out);
    out
}

fn status_of(msg: &str) -> Option<&str> {
    msg.strip_prefix("STATUS:").map(str::trim)
}

pub fn obs_of(tick: &Tick) -> Option<Obs> {
    let r = &tick.reading;
    if !r.has_progress() {
        return None;
    }
    Some(Obs {
        t: tick.t,
        comp: r.downloaded.map(|d| (d.0, d.1)),
        unp: r.unpacking.map(|u| (u.0, u.1)),
        ed_speed: r.downloaded.map(|d| d.2),
        ed_eta: r.unpacking.and_then(|u| u.2),
    })
}

/// Observations of the download phase (with what Hangar showed live) plus the ground truth.
pub struct Prepared {
    pub ticks: Vec<(Obs, Option<Pred>)>,
    pub truth: Truth,
}

fn interp_rx(ticks: &[Tick], t: f64) -> Option<f64> {
    let pts: Vec<(f64, f64)> = ticks.iter().filter_map(|k| k.net_rx.map(|r| (k.t, r as f64))).collect();
    let i = pts.iter().position(|p| p.0 >= t)?;
    if i == 0 {
        return Some(pts[0].1);
    }
    let (a, b) = (pts[i - 1], pts[i]);
    Some(a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0).max(1e-9))
}

pub fn prepare(rec: &Recording) -> Option<Prepared> {
    let logs = log_times(rec);
    let first_prog = rec.ticks.iter().find(|t| t.reading.has_progress()).map(|t| t.t);
    let dl_start = logs
        .iter()
        .find(|(_, m)| status_of(m).is_some_and(|s| s.starts_with("Downloading update")))
        .map(|(t, _)| *t);
    let after = first_prog.or(dl_start)?;
    let install_log = logs
        .iter()
        .find(|(t, m)| *t >= after - 0.5 && status_of(m).is_some_and(|s| s.starts_with("Installing")))
        .map(|(t, _)| *t);

    let (ticks, d_end, d_end_source, source): (Vec<(Obs, Option<Pred>)>, f64, &'static str, &'static str) =
        if first_prog.is_some() {
            let from_window = rec.ticks.iter().zip(rec.ticks.iter().skip(1)).find_map(|(a, b)| {
                (a.reading.has_progress() && !b.reading.has_progress()).then(|| (a.t + b.t) / 2.0)
            });
            let (d_end, src) = match (install_log, from_window) {
                (Some(l), _) => (l, "log"),
                (None, Some(w)) => (w, "window"),
                _ => return None,
            };
            let ticks = rec.ticks.iter().filter(|t| t.t < d_end).filter_map(|t| obs_of(t).map(|o| (o, t.live))).collect();
            (ticks, d_end, src, "window")
        } else {
            // No window texts: rebuild. Compressed done = bytes received since "Downloading update";
            // compressed total = bytes received by the end of the phase (what the window would show).
            let (start, d_end) = (dl_start?, install_log?);
            let rx0 = interp_rx(&rec.ticks, start)?;
            let comp_total = (interp_rx(&rec.ticks, d_end)? - rx0) / 1e6;
            let to_download = logs
                .iter()
                .filter(|(t, _)| *t <= start)
                .filter_map(|(_, m)| m.split("To download: ").nth(1).and_then(|r| r.split(' ').next()).and_then(|n| n.parse::<f64>().ok()))
                .next()
                .map(|b| b / 1e6);
            let ticks = rec
                .ticks
                .iter()
                .filter(|t| t.t > start && t.t < d_end)
                .filter_map(|t| {
                    let comp = t.net_rx.map(|r| (((r as f64 - rx0) / 1e6).clamp(0.0, comp_total), comp_total));
                    let unp = match (t.dl_bytes, to_download) {
                        (Some(b), Some(tot)) => Some(((b as f64 / 1e6).min(tot), tot)),
                        _ => None,
                    };
                    comp.map(|c| (Obs { t: t.t, comp: Some(c), unp, ed_speed: None, ed_eta: None }, t.live))
                })
                .collect();
            (ticks, d_end, "log", "reconstructed")
        };
    let first_t = ticks.first()?.0.t;
    let r_end = rec
        .ticks
        .iter()
        .find(|t| t.t > d_end && t.procs == 0)
        .map(|t| t.t)
        .or(rec.end_t.map(|e| e - 3.0))
        .map(|e| e.max(d_end));
    let multi_phase = rec.ticks.iter().any(|t| t.t > d_end + 2.0 && t.reading.has_progress());
    let comp_total = ticks.iter().rev().find_map(|(o, _)| o.comp.map(|c| c.1));
    let unp_total = ticks.iter().rev().find_map(|(o, _)| o.unp.map(|c| c.1));
    Some(Prepared {
        truth: Truth { p_start: first_t, d_end, d_end_source, r_end, comp_total, unp_total, multi_phase, source },
        ticks,
    })
}

pub fn truth(rec: &Recording) -> Option<Truth> {
    prepare(rec).map(|p| p.truth)
}

// ------------------------------------------------------------------ scoring

pub const BINS: [(f64, f64); 6] = [(0.0, 0.1), (0.1, 0.25), (0.25, 0.5), (0.5, 0.75), (0.75, 0.9), (0.9, 1.0)];
pub const BIN_LABELS: [&str; 6] = ["0-10%", "10-25%", "25-50%", "50-75%", "75-90%", "90-100%"];

#[derive(Clone, Debug, Default)]
pub struct Score {
    pub name: String,
    /// Share of download-phase ticks with an answer.
    pub availability: f64,
    /// Seconds from progress start to the first answer.
    pub first_answer_s: Option<f64>,
    /// Mean |error| / actual over ticks in 10-90 % of the phase.
    pub mape_10_90: Option<f64>,
    /// Mean |error| in seconds over ticks in 10-90 %.
    pub mae_10_90: Option<f64>,
    /// Median signed relative error 10-90 % (+ = too pessimistic).
    pub bias_10_90: Option<f64>,
    /// Per bin: median |relative error|.
    pub bins_mdape: [Option<f64>; 6],
    /// Per bin: median |error| in seconds.
    pub bins_mdae: [Option<f64>; 6],
    /// Median |change of predicted finish time| between ticks (seconds). Lower = calmer.
    pub jitter_s: Option<f64>,
    /// Share of ticks where the actual remaining time was inside [low, high].
    pub coverage: Option<f64>,
    /// Signed relative errors per tick with phase fraction (for range calibration).
    pub samples: Vec<(f64, f64)>,
}

fn median(v: &mut [f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Scores one series of answers (aligned with `phase_ticks`) against the truth.
pub fn score(name: &str, phase_ticks: &[f64], preds: &[Option<Pred>], tr: &Truth) -> Score {
    let dur = (tr.d_end - tr.p_start).max(1e-6);
    let mut s = Score { name: name.to_string(), ..Default::default() };
    let mut rel_mid = Vec::new();
    let mut abs_mid = Vec::new();
    let mut signed_mid = Vec::new();
    let mut bins_rel: [Vec<f64>; 6] = Default::default();
    let mut bins_abs: [Vec<f64>; 6] = Default::default();
    let mut finish_jumps = Vec::new();
    let mut last_finish: Option<f64> = None;
    let (mut inside, mut with_range, mut answered) = (0usize, 0usize, 0usize);
    for (t, p) in phase_ticks.iter().zip(preds) {
        let Some(p) = p else {
            last_finish = None;
            continue;
        };
        answered += 1;
        s.first_answer_s.get_or_insert(t - tr.p_start);
        let actual = tr.d_end - t;
        let f = ((t - tr.p_start) / dur).clamp(0.0, 0.9999);
        let err = p.remaining - actual;
        let rel = err / actual.max(1.0);
        s.samples.push((f, rel));
        let bi = BINS.iter().position(|(a, b)| f >= *a && f < *b).unwrap_or(5);
        bins_rel[bi].push(rel.abs());
        bins_abs[bi].push(err.abs());
        if (0.1..0.9).contains(&f) {
            rel_mid.push(rel.abs());
            abs_mid.push(err.abs());
            signed_mid.push(rel);
        }
        let finish = t + p.remaining;
        if let Some(lf) = last_finish {
            finish_jumps.push((finish - lf).abs());
        }
        last_finish = Some(finish);
        if let (Some(lo), Some(hi)) = (p.low, p.high) {
            with_range += 1;
            if actual >= lo.min(hi) - 0.5 && actual <= hi.max(lo) + 0.5 {
                inside += 1;
            }
        }
    }
    s.availability = if phase_ticks.is_empty() { 0.0 } else { answered as f64 / phase_ticks.len() as f64 };
    s.mape_10_90 = mean(&rel_mid);
    s.mae_10_90 = mean(&abs_mid);
    s.bias_10_90 = median(&mut signed_mid);
    for i in 0..6 {
        s.bins_mdape[i] = median(&mut bins_rel[i]);
        s.bins_mdae[i] = median(&mut bins_abs[i]);
    }
    s.jitter_s = median(&mut finish_jumps);
    s.coverage = (with_range > 0).then(|| inside as f64 / with_range as f64);
    s
}

#[derive(Clone, Debug)]
pub struct RunResult {
    pub name: String,
    pub action: Option<String>,
    pub truth: Truth,
    pub phase_ticks: usize,
    pub scores: Vec<Score>,
    /// Per tick: t, actual remaining, then one answer per estimator (same order as scores).
    pub table: Vec<(f64, f64, Vec<Option<f64>>)>,
}

/// Answers are held between ticks where an estimator's signal is missing (e.g. _downloads is
/// measured every second tick), counting down, as a display would.
fn hold(preds: &mut [Option<Pred>], ts: &[f64]) {
    let mut last: Option<(f64, Pred)> = None;
    for (p, t) in preds.iter_mut().zip(ts) {
        match (*p, last) {
            (Some(x), _) => last = Some((*t, x)),
            (None, Some((lt, lp))) if t - lt <= 2.5 => {
                let d = t - lt;
                *p = Some(Pred {
                    remaining: (lp.remaining - d).max(0.0),
                    low: lp.low.map(|l| (l - d).max(0.0)),
                    high: lp.high.map(|h| (h - d).max(0.0)),
                });
            }
            _ => {}
        }
    }
}

/// Runs every candidate (plus the recorded live value) over one recording.
pub fn run_all(rec: &Recording, mut estimators: Vec<Box<dyn Estimator>>) -> Option<RunResult> {
    let prep = prepare(rec)?;
    let tr = prep.truth;
    let phase_t: Vec<f64> = prep.ticks.iter().map(|(o, _)| o.t).collect();
    let mut preds: Vec<Vec<Option<Pred>>> = vec![Vec::new(); estimators.len()];
    let mut live: Vec<Option<Pred>> = Vec::new();
    for (o, lv) in &prep.ticks {
        for (i, e) in estimators.iter_mut().enumerate() {
            preds[i].push(e.observe(o));
        }
        // the live figure included the install tail; compare on the download phase only
        let tail = rec.install_seconds;
        let v01 = tail == eta::V01_INSTALL_SECONDS;
        live.push(lv.map(|p| Pred {
            remaining: (p.remaining - tail).max(0.0),
            low: p.low.map(|l| (l - if v01 { tail * 0.5 } else { tail }).max(0.0)),
            high: p.high.map(|h| (h - if v01 { tail * 1.5 } else { tail }).max(0.0)),
        }));
    }
    for p in preds.iter_mut() {
        hold(p, &phase_t);
    }
    let mut scores: Vec<Score> = estimators.iter().zip(&preds).map(|(e, p)| score(&e.name(), &phase_t, p, &tr)).collect();
    let has_live = live.iter().any(|p| p.is_some());
    if has_live {
        let label = if rec.install_seconds == eta::V01_INSTALL_SECONDS { "v0.1 live (recorded in app)" } else { "live (recorded in app)" };
        scores.push(score(label, &phase_t, &live, &tr));
    }
    let table = phase_t
        .iter()
        .enumerate()
        .map(|(k, t)| {
            let mut row: Vec<Option<f64>> = preds.iter().map(|p| p[k].map(|x| x.remaining)).collect();
            if has_live {
                row.push(live[k].map(|x| x.remaining));
            }
            (*t, tr.d_end - t, row)
        })
        .collect();
    Some(RunResult { name: rec.name.clone(), action: rec.action.clone(), truth: tr, phase_ticks: phase_t.len(), scores, table })
}

pub fn run_candidates(rec: &Recording) -> Option<RunResult> {
    run_all(rec, eta::candidates())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(t: f64, texts: &[&str], procs: usize) -> Tick {
        let v: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        Tick { t, reading: parse_window_texts(&v), procs, ..Default::default() }
    }

    /// A synthetic run: 20 MB/s compressed for 50 s, install at 50.3 s, gone at 53 s.
    fn synthetic() -> Recording {
        let mut rec = Recording { name: "synthetic".into(), ..Default::default() };
        rec.ticks.push(tick(0.0, &["Waiting for operation to complete..."], 1));
        for i in 1..=50 {
            let t = i as f64;
            let d = format!("Downloaded: {:.1} of 1000.0 MBytes    Speed: 20.0 MB/s", 20.0 * (t - 1.0));
            let u = format!("Unpacking {:.1} of 1500.0 MBytes (ETA 0:00:{:02})", 30.0 * (t - 1.0), (50 - i).max(0));
            rec.ticks.push(tick(t, &["Downloading the update...", &d, &u], 2));
        }
        rec.ticks.push(tick(51.0, &["Installing new files..."], 2));
        rec.ticks.push(tick(54.0, &[], 0));
        rec.logs.push(LogLine { t: 2.0, file: "autoupdate_log.txt".into(), line: "00000.000 === Log opened UTC 2026-11-26 22:55:24".into() });
        rec.logs.push(LogLine { t: 2.0, file: "autoupdate_log.txt".into(), line: "00000.500 STATUS: Downloading update...".into() });
        rec.logs.push(LogLine { t: 51.0, file: "autoupdate_log.txt".into(), line: "00049.300 STATUS: Installing new files...".into() });
        rec
    }

    #[test]
    fn truth_uses_precise_log_time() {
        let tr = truth(&synthetic()).unwrap();
        assert_eq!(tr.d_end_source, "log");
        // log session offset = min(2.0-0, 2.0-0.5, 51-49.3) = 1.5 → 49.3 + 1.5 = 50.8
        assert!((tr.d_end - 50.8).abs() < 1e-9, "{}", tr.d_end);
        assert_eq!(tr.p_start, 1.0);
        assert_eq!(tr.r_end, Some(54.0));
        assert!(!tr.multi_phase);
    }

    #[test]
    fn constant_rate_scores_near_zero() {
        let res = run_candidates(&synthetic()).unwrap();
        let ewma = res.scores.iter().find(|s| s.name == "EWMA comp tau 15 s").unwrap();
        // predicts finish at t=51 (1000 MB at 20 MB/s from t=1), truth 50.8
        assert!(ewma.mae_10_90.unwrap() < 0.5, "{:?}", ewma.mae_10_90);
        assert!(ewma.jitter_s.unwrap() < 0.05);
        assert!(ewma.availability > 0.9);
    }

    #[test]
    fn loads_recording_format() {
        let dir = std::env::temp_dir().join(format!("hangar-replay-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("run-test.ndjson");
        let lines = [
            r#"{"type":"header","format":1,"action":"update"}"#,
            r#"{"type":"tick","t":1.0,"windows":[{"pid":1,"texts":["Downloading the update...","Downloaded: 10.0 of 100.0 MBytes Speed: 10.0 MB/s","Unpacking 15.0 of 150.0 MBytes (ETA 0:00:09)"]}],"procs":[1,2],"hangar":{"eta":12.0,"low":10.0,"high":15.0}}"#,
            r#"{"type":"log","t":1.0,"file":"autoupdate_log.txt","line":"00000.100 STATUS: Downloading update..."}"#,
            r#"{"type":"end","t":20.0,"result":"success"}"#,
            r#"{"type":"tick","t":2.0,"win"#, // torn last line
        ];
        std::fs::write(&p, lines.join("\n")).unwrap();
        let rec = load_recording(&p).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(rec.action.as_deref(), Some("update"));
        assert_eq!(rec.ticks.len(), 1);
        assert_eq!(rec.ticks[0].reading.unpacking, Some((15.0, 150.0, Some(9.0))));
        assert_eq!(rec.ticks[0].live.unwrap().remaining, 12.0);
        assert_eq!(rec.ticks[0].procs, 2);
        assert_eq!(rec.logs.len(), 1);
        assert_eq!(rec.end_t, Some(20.0));
    }
}
