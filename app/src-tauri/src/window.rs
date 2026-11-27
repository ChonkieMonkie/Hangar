// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Interpreting the texts of ED's updater window (read with UI Automation elsewhere).
//! Pure functions, no Windows calls, so the replay tool can use the same parser.

use std::sync::OnceLock;

use regex::Regex;

fn re_downloaded() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"Downloaded:\s*([\d.]+)\s*of\s*([\d.]+)\s*MBytes\s*Speed:\s*([\d.]+)\s*MB/s").unwrap())
}
fn re_unpacking() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"Unpacking\s*([\d.]+)\s*of\s*([\d.]+)\s*MBytes(?:\s*\(ETA\s*(\d+):(\d{2}):(\d{2})\))?").unwrap())
}
fn re_file_path() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[\w .()'\-]+(/[\w .()'\-]+)+$").unwrap())
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct WindowReading {
    /// Compressed: (done MB, total MB, ED's speed MB/s)
    pub downloaded: Option<(f64, f64, f64)>,
    /// Unpacked: (done MB, total MB, ED's ETA in seconds)
    pub unpacking: Option<(f64, f64, Option<f64>)>,
    pub status: Option<String>,
    pub current_file: Option<String>,
}

impl WindowReading {
    pub fn has_progress(&self) -> bool {
        self.downloaded.is_some() || self.unpacking.is_some()
    }
}

/// Interprets the texts of ED's updater window. Tolerant: unknown texts are ignored.
pub fn parse_window_texts(texts: &[String]) -> WindowReading {
    let mut r = WindowReading::default();
    for t in texts {
        let t = t.trim();
        if t.starts_with("DCS Updater") || t == "Cancel" {
            continue;
        }
        if let Some(c) = re_downloaded().captures(t) {
            r.downloaded = Some((c[1].parse().unwrap_or(0.0), c[2].parse().unwrap_or(0.0), c[3].parse().unwrap_or(0.0)));
        } else if let Some(c) = re_unpacking().captures(t) {
            let eta = match (c.get(3), c.get(4), c.get(5)) {
                (Some(h), Some(m), Some(s)) => Some(
                    h.as_str().parse::<f64>().unwrap_or(0.0) * 3600.0
                        + m.as_str().parse::<f64>().unwrap_or(0.0) * 60.0
                        + s.as_str().parse::<f64>().unwrap_or(0.0),
                ),
                _ => None,
            };
            r.unpacking = Some((c[1].parse().unwrap_or(0.0), c[2].parse().unwrap_or(0.0), eta));
        } else if t.ends_with("...") {
            r.status = Some(t.trim_end_matches('.').trim().to_string());
        } else if re_file_path().is_match(t) {
            r.current_file = Some(t.to_string());
        }
    }
    r
}

/// Picks the most useful reading among several updater windows: one with progress first
/// (the elevated worker), else one with a status text.
pub fn best_reading(readings: Vec<WindowReading>) -> WindowReading {
    let mut best = WindowReading::default();
    for rd in readings {
        if rd.has_progress() || (!best.has_progress() && best.status.is_none() && rd.status.is_some()) {
            best = rd;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_window_during_download() {
        let r = parse_window_texts(&s(&[
            "DCS Updater 2.17.3.11",
            "Cancel",
            "Downloading the update...",
            "Downloaded: 684.6 of 1248.3 MBytes    Speed: 32.2 MB/s",
            "Unpacking 979.4 of 1892.7 MBytes (ETA 0:00:20)",
        ]));
        assert_eq!(r.downloaded, Some((684.6, 1248.3, 32.2)));
        assert_eq!(r.unpacking, Some((979.4, 1892.7, Some(20.0))));
        assert_eq!(r.status.as_deref(), Some("Downloading the update"));
    }

    #[test]
    fn parses_window_during_install() {
        let r = parse_window_texts(&s(&["Installing new files...", "Mods/aircraft/AJS37/Missions/Single/Mozdok Attack SP.miz"]));
        assert_eq!(r.status.as_deref(), Some("Installing new files"));
        assert!(r.current_file.unwrap().ends_with(".miz"));
    }

    #[test]
    fn best_reading_prefers_progress() {
        let a = parse_window_texts(&s(&["Waiting for operation to complete..."]));
        let b = parse_window_texts(&s(&["Downloading the update...", "Downloaded: 1 of 2 MBytes Speed: 1 MB/s"]));
        assert!(best_reading(vec![a.clone(), b.clone()]).has_progress());
        assert!(best_reading(vec![b, a]).has_progress());
    }
}
