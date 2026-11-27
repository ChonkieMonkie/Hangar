// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Scores ETA estimators against recorded updater runs. Reads only recordings, never DCS files.
//!
//!   cargo run --release --example eta_replay -- [paths...] [--watcher <x-uia.txt>]... [--out report.md] [--csv dir]
//!
//! Paths may be recording files, ETA research exports (`*.zip`, see share.rs) or folders (all
//! `run-*.ndjson` and `*.zip`). Runs from an export are named `<exportId>/<NN>` and are also
//! scored per sender, so one fast connection does not dominate. Without paths:
//! `%LOCALAPPDATA%\Hangar\recordings`. See docs/ETA.md.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use hangar_lib::replay::{self, BIN_LABELS, RunResult, Score};

fn pct(v: Option<f64>) -> String {
    v.map(|x| format!("{:.1} %", x * 100.0)).unwrap_or_else(|| "–".into())
}
fn secs(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.1} s")).unwrap_or_else(|| "–".into())
}
fn signed_pct(v: Option<f64>) -> String {
    v.map(|x| format!("{:+.1} %", x * 100.0)).unwrap_or_else(|| "–".into())
}

fn mean_opt(v: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    let xs: Vec<f64> = v.flatten().collect();
    (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64)
}

fn quantile(v: &mut Vec<f64>, q: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((v.len() - 1) as f64 * q).round() as usize;
    Some(v[i])
}

/// Mean of per-run scores (each run counts equally).
struct Agg {
    name: String,
    runs: usize,
    mape: Option<f64>,
    mae: Option<f64>,
    bias: Option<f64>,
    jitter: Option<f64>,
    first: Option<f64>,
    coverage: Option<f64>,
    avail: f64,
    bins: [Option<f64>; 6],
    worst_run_mape: Option<f64>,
}

fn aggregate(results: &[RunResult]) -> Vec<Agg> {
    let mut by: BTreeMap<String, Vec<&Score>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for r in results {
        for s in &r.scores {
            if !by.contains_key(&s.name) {
                order.push(s.name.clone());
            }
            by.entry(s.name.clone()).or_default().push(s);
        }
    }
    order
        .into_iter()
        .map(|name| {
            let ss = &by[&name];
            let mut bins = [None; 6];
            for (i, b) in bins.iter_mut().enumerate() {
                *b = mean_opt(ss.iter().map(|s| s.bins_mdape[i]));
            }
            Agg {
                runs: ss.len(),
                mape: mean_opt(ss.iter().map(|s| s.mape_10_90)),
                mae: mean_opt(ss.iter().map(|s| s.mae_10_90)),
                bias: mean_opt(ss.iter().map(|s| s.bias_10_90)),
                jitter: mean_opt(ss.iter().map(|s| s.jitter_s)),
                first: mean_opt(ss.iter().map(|s| s.first_answer_s)),
                coverage: mean_opt(ss.iter().map(|s| s.coverage)),
                avail: ss.iter().map(|s| s.availability).sum::<f64>() / ss.len() as f64,
                bins,
                worst_run_mape: ss.iter().filter_map(|s| s.mape_10_90).reduce(f64::max),
                name,
            }
        })
        .collect()
}

fn collect_inputs(args: &[String]) -> (Vec<PathBuf>, Vec<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let (mut files, mut watchers, mut out, mut csv) = (Vec::new(), Vec::new(), None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--watcher" => watchers.extend(it.next().map(PathBuf::from)),
            "--out" => out = it.next().map(PathBuf::from),
            "--csv" => csv = it.next().map(PathBuf::from),
            p => files.push(PathBuf::from(p)),
        }
    }
    if files.is_empty() && watchers.is_empty() {
        files.extend(hangar_lib::recorder::recordings_dir());
    }
    let mut expanded = Vec::new();
    for f in files {
        if f.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&f)
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    (n.starts_with("run-") && n.ends_with(".ndjson")) || n.to_ascii_lowercase().ends_with(".zip")
                }).collect())
                .unwrap_or_default();
            v.sort();
            expanded.extend(v);
        } else {
            expanded.push(f);
        }
    }
    (expanded, watchers, out, csv)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (files, watchers, out, csv) = collect_inputs(&args);
    let mut recs = Vec::new();
    for f in &files {
        if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
            match replay::load_zip(f) {
                Ok(v) => recs.extend(v),
                Err(e) => eprintln!("skip: {e}"),
            }
            continue;
        }
        match replay::load_recording(f) {
            Ok(r) => recs.push(r),
            Err(e) => eprintln!("skip: {e}"),
        }
    }
    for w in &watchers {
        let summary = PathBuf::from(w.to_string_lossy().replace("-uia.txt", "-summary.md"));
        match replay::import_watcher(w, &summary) {
            Ok(r) => recs.push(r),
            Err(e) => eprintln!("skip: {e}"),
        }
    }
    let mut results = Vec::new();
    let mut skipped = Vec::new();
    for r in &recs {
        match replay::run_candidates(r) {
            Some(res) if res.phase_ticks >= 5 => results.push(res),
            Some(res) => skipped.push(format!("{} (only {} ticks with progress)", r.name, res.phase_ticks)),
            None => skipped.push(format!("{} (no completed download phase)", r.name)),
        }
    }
    if results.is_empty() {
        eprintln!("No scorable runs. Inputs: {} recordings. Skipped: {:?}", recs.len(), skipped);
        std::process::exit(1);
    }

    let mut md = String::new();
    let _ = writeln!(md, "# ETA replay\n");
    let _ = writeln!(md, "Target: seconds until the download phase ends (\"Installing new files\"). Errors are relative to the actual remaining time (min 1 s). Each run counts equally.\n");
    let _ = writeln!(md, "## Runs\n");
    let _ = writeln!(md, "| Run | Action | Download phase | End from | Tail (install etc.) | Compressed | Unpacked | Ticks | Note |");
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|");
    for r in &results {
        let t = &r.truth;
        let _ = writeln!(md, "| {} | {} | {:.1} s | {} | {} | {} | {} | {} | {} |",
            r.name, r.action.as_deref().unwrap_or("?"), t.d_end - t.p_start, t.d_end_source,
            secs(t.r_end.map(|e| e - t.d_end)),
            t.comp_total.map(|x| format!("{x:.0} MB")).unwrap_or("–".into()),
            t.unp_total.map(|x| format!("{x:.0} MB")).unwrap_or("–".into()),
            r.phase_ticks,
            format!("{}{}", t.source, if t.multi_phase { "; several download phases, first scored" } else { "" }));
    }
    for s in &skipped {
        let _ = writeln!(md, "\nSkipped: {s}");
    }

    let mut agg = aggregate(&results);
    agg.sort_by(|a, b| a.mape.unwrap_or(f64::MAX).partial_cmp(&b.mape.unwrap_or(f64::MAX)).unwrap());
    let _ = writeln!(md, "\n## Ranking (10–90 % of the phase)\n");
    let _ = writeln!(md, "MAPE = mean |error| / actual. Bias + = too pessimistic. Jitter = median jump of the predicted finish time per tick. Coverage = share of ticks where reality fell inside the shown range.\n");
    let _ = writeln!(md, "| # | Estimator | MAPE | Worst run | MAE | Bias | Jitter | First answer | Range coverage | Answers |");
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|---|");
    for (i, a) in agg.iter().enumerate() {
        let _ = writeln!(md, "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.0} % ({} runs) |",
            i + 1, a.name, pct(a.mape), pct(a.worst_run_mape), secs(a.mae), signed_pct(a.bias), secs(a.jitter),
            secs(a.first), pct(a.coverage), a.avail * 100.0, a.runs);
    }

    // per sender (exports are named "<exportId>/<NN>", local recordings count as "local")
    let sender = |name: &str| name.split_once('/').map(|(a, _)| a.to_string()).unwrap_or_else(|| "local".into());
    let mut senders: Vec<String> = results.iter().map(|r| sender(&r.name)).collect();
    senders.sort();
    senders.dedup();
    if senders.len() > 1 {
        let shown: Vec<String> = agg
            .iter()
            .take(3)
            .map(|a| a.name.clone())
            .chain(agg.iter().skip(3).filter(|a| a.name.starts_with("ED (") || a.name.starts_with("v0.1")).map(|a| a.name.clone()))
            .collect();
        let _ = writeln!(md, "\n## Per sender\n");
        let _ = writeln!(md, "MAPE per sender. The last row weighs every sender equally.\n");
        let _ = writeln!(md, "| Sender | Runs | {} |", shown.join(" | "));
        let _ = writeln!(md, "|---|---|{}", "---|".repeat(shown.len()));
        let mut per_est: Vec<Vec<f64>> = vec![Vec::new(); shown.len()];
        for snd in &senders {
            let subset: Vec<&RunResult> = results.iter().filter(|r| &sender(&r.name) == snd).collect();
            let owned: Vec<RunResult> = subset.iter().map(|r| (*r).clone()).collect();
            let ag = aggregate(&owned);
            let cells: Vec<String> = shown
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let m = ag.iter().find(|a| &a.name == n).and_then(|a| a.mape);
                    if let Some(x) = m {
                        per_est[i].push(x);
                    }
                    pct(m)
                })
                .collect();
            let _ = writeln!(md, "| {} | {} | {} |", snd, subset.len(), cells.join(" | "));
        }
        let eq: Vec<String> = per_est.iter().map(|v| pct((!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64))).collect();
        let _ = writeln!(md, "| **all senders equal** | {} | {} |", results.len(), eq.join(" | "));
    }

    let _ = writeln!(md, "\n## Median |relative error| per stage of the phase\n");
    let _ = writeln!(md, "| Estimator | {} |", BIN_LABELS.join(" | "));
    let _ = writeln!(md, "|---|{}", "---|".repeat(6));
    for a in &agg {
        let cells: Vec<String> = a.bins.iter().map(|b| pct(*b)).collect();
        let _ = writeln!(md, "| {} | {} |", a.name, cells.join(" | "));
    }

    // per run, top 5 plus references
    let _ = writeln!(md, "\n## Per run\n");
    let top: Vec<&str> = agg.iter().take(5).map(|a| a.name.as_str()).collect();
    for r in &results {
        let _ = writeln!(md, "### {}\n", r.name);
        let _ = writeln!(md, "| Estimator | MAPE | MAE | Bias | Jitter | {} |", BIN_LABELS.join(" | "));
        let _ = writeln!(md, "|---|---|---|---|---|{}", "---|".repeat(6));
        for s in r.scores.iter().filter(|s| top.contains(&s.name.as_str()) || s.name.starts_with("ED (") || s.name.starts_with("v0.1")) {
            let cells: Vec<String> = s.bins_mdape.iter().map(|b| pct(*b)).collect();
            let _ = writeln!(md, "| {} | {} | {} | {} | {} | {} |", s.name, pct(s.mape_10_90), secs(s.mae_10_90), signed_pct(s.bias_10_90), secs(s.jitter_s), cells.join(" | "));
        }
        let _ = writeln!(md);
    }

    // calibration of a range for the best estimator: quantiles of signed relative error per stage
    if let Some(best) = agg.first() {
        let _ = writeln!(md, "## Range calibration for \"{}\"\n", best.name);
        let _ = writeln!(md, "Signed relative error quantiles per stage over all runs. A shown range of remaining × [1 + q10, 1 + q90]... would have held reality ~80 % of the time.\n");
        let _ = writeln!(md, "| Stage | n | q10 | median | q90 |");
        let _ = writeln!(md, "|---|---|---|---|---|");
        let mut per_bin: Vec<Vec<f64>> = vec![Vec::new(); 6];
        for r in &results {
            if let Some(s) = r.scores.iter().find(|s| s.name == best.name) {
                for (f, rel) in &s.samples {
                    let bi = replay::BINS.iter().position(|(a, b)| f >= a && f < b).unwrap_or(5);
                    per_bin[bi].push(*rel);
                }
            }
        }
        for (i, v) in per_bin.iter_mut().enumerate() {
            let n = v.len();
            let _ = writeln!(md, "| {} | {} | {} | {} | {} |", BIN_LABELS[i], n,
                signed_pct(quantile(v, 0.1)), signed_pct(quantile(v, 0.5)), signed_pct(quantile(v, 0.9)));
        }
    }

    if let Some(dir) = &csv {
        let _ = std::fs::create_dir_all(dir);
        for r in &results {
            let mut c = String::from("t,phase_fraction,actual");
            for s in &r.scores {
                c.push(',');
                c.push('"');
                c.push_str(&s.name.replace('"', "'"));
                c.push('"');
            }
            c.push('\n');
            let dur = r.truth.d_end - r.truth.p_start;
            for (t, actual, row) in &r.table {
                let _ = write!(c, "{t:.2},{:.4},{actual:.2}", (t - r.truth.p_start) / dur);
                for v in row {
                    c.push(',');
                    if let Some(x) = v {
                        let _ = write!(c, "{x:.2}");
                    }
                }
                c.push('\n');
            }
            let stem = Path::new(&r.name.replace('/', "-")).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let _ = std::fs::write(dir.join(format!("{stem}.csv")), c);
        }
    }

    match &out {
        Some(p) => {
            std::fs::write(p, &md).expect("write report");
            println!("Report: {} ({} runs)", p.display(), results.len());
        }
        None => println!("{md}"),
    }
}
