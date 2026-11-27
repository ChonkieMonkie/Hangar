// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Prints Hangar's ETA tick by tick for one recording, next to ED's and the truth.
//!   cargo run --release --example eta_trace -- <run-*.ndjson> [first N ticks]

use hangar_lib::eta::HangarEta;
use hangar_lib::replay;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = std::path::PathBuf::from(args.first().expect("recording path"));
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let rec = replay::load_recording(&path).expect("load");
    let prep = replay::prepare(&rec).expect("no download phase");
    let mut e = HangarEta::default();
    println!("{:>8} {:>9} {:>9} {:>8} {:>8} {:>8} {:>7} {:>6}", "t", "comp", "unp", "actual", "hangar", "ED", "conf", "live");
    for (o, live) in prep.ticks.iter().take(n) {
        let a = e.answer(o);
        println!(
            "{:>8.2} {:>9} {:>9} {:>8.1} {:>8} {:>8} {:>7} {:>6}",
            o.t,
            o.comp.map(|c| format!("{:.1}", c.0)).unwrap_or("-".into()),
            o.unp.map(|c| format!("{:.1}", c.0)).unwrap_or("-".into()),
            prep.truth.d_end - o.t,
            a.map(|a| format!("{:.1}", a.pred.remaining)).unwrap_or("-".into()),
            o.ed_eta.map(|x| format!("{x:.0}")).unwrap_or("-".into()),
            a.map(|a| a.confidence).unwrap_or("-"),
            live.map(|l| format!("{:.1}", l.remaining)).unwrap_or("-".into()),
        );
    }
}
