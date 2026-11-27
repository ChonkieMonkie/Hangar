//! Records every observed updater run to `%LOCALAPPDATA%\Hangar\recordings\run-*.ndjson`
//! (Hangar's own data, never a DCS folder). One JSON object per line:
//! `header`, then `tick` (once per monitor tick) and `log` (every new updater log line), then `end`.
//! The replay tool (`examples/eta_replay.rs`, `replay.rs`) reads these to score ETA algorithms.
//! See docs/ETA.md and B-016.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

pub const FORMAT: u32 = 1;

pub fn recordings_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Hangar").join("recordings"))
}

/// UTC "YYYYMMDD-HHMMSS" without extra crates.
pub fn utc_stamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

pub struct Recorder {
    w: Option<BufWriter<File>>,
    pub path: Option<PathBuf>,
}

impl Recorder {
    pub fn disabled() -> Self {
        Recorder { w: None, path: None }
    }

    /// Opens a new recording. Failure only disables recording; it never stops monitoring.
    pub fn start(action: Option<&str>, by_hangar: bool) -> Self {
        let Some(dir) = recordings_dir() else { return Self::disabled() };
        if std::fs::create_dir_all(&dir).is_err() {
            return Self::disabled();
        }
        let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = dir.join(format!("run-{}.ndjson", utc_stamp(unix)));
        let Ok(f) = File::create(&path) else { return Self::disabled() };
        let mut r = Recorder { w: Some(BufWriter::new(f)), path: Some(path) };
        r.write(json!({
            "type": "header",
            "format": FORMAT,
            "app": env!("CARGO_PKG_VERSION"),
            "startedUnix": unix,
            "action": action,
            "byHangar": by_hangar,
            "installSeconds": crate::eta::INSTALL_SECONDS,
        }));
        r
    }

    pub fn write(&mut self, v: Value) {
        if let Some(w) = self.w.as_mut() {
            if serde_json::to_writer(&mut *w, &v).is_err() || w.write_all(b"\n").is_err() {
                self.w = None; // disk full or similar: stop recording quietly
            }
        }
    }

    pub fn flush(&mut self) {
        if let Some(w) = self.w.as_mut() {
            let _ = w.flush();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(1_798_412_461), "20261227-230101");
        assert_eq!(utc_stamp(951_782_400), "20000229-000000");
    }
}
