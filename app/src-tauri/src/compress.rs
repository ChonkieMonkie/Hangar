// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Shared compression layer (ROADMAP #10) for backup objects (#3) and the mod vault (#9).
//! A compressed file is stored next to where the plain file would be, with ".zst" added, so a
//! store can mix old plain files and new compressed ones and nothing is ambiguous.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use crate::settings::Compression;

pub const SUFFIX: &str = "zst";

/// The ".zst" sibling of a path.
pub fn zst_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".");
    s.push(SUFFIX);
    PathBuf::from(s)
}

pub fn is_zst(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(SUFFIX))
}

/// Whether a file of this name and size is worth compressing under `cfg`.
pub fn worth_it(cfg: &Compression, name: &Path, size: u64) -> bool {
    if cfg.algorithm != "zstd" || size < cfg.min_size {
        return false;
    }
    let ext = name.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    !cfg.skip_extensions.iter().any(|x| x.trim_start_matches('.').eq_ignore_ascii_case(&ext))
}

fn threads(cfg: &Compression) -> u32 {
    if cfg.threads > 0 {
        cfg.threads
    } else {
        std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(2).clamp(1, 4)
    }
}

/// A zstd encoder over `w` with the configured level and threads.
pub fn encoder<W: Write>(cfg: &Compression, w: W) -> std::io::Result<zstd::stream::write::Encoder<'static, W>> {
    let mut e = zstd::stream::write::Encoder::new(w, cfg.level.clamp(1, 19))?;
    let t = threads(cfg);
    if t > 1 {
        let _ = e.multithread(t);
    }
    Ok(e)
}

/// Compresses `src` into `dst` (via a temp file). Returns the compressed size.
pub fn compress_file(cfg: &Compression, src: &Path, dst: &Path) -> std::io::Result<u64> {
    let tmp = zst_path(&dst.with_extension("part"));
    {
        let mut r = BufReader::with_capacity(1 << 20, File::open(src)?);
        let w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        let mut e = encoder(cfg, w)?;
        std::io::copy(&mut r, &mut e)?;
        let w = e.finish()?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }
    std::fs::rename(&tmp, dst)?;
    Ok(std::fs::metadata(dst)?.len())
}

/// Opens a file for reading its original content: decompresses if it ends in ".zst".
pub fn open_decoded(path: &Path) -> std::io::Result<Box<dyn Read>> {
    let f = BufReader::with_capacity(1 << 20, File::open(path)?);
    if is_zst(path) {
        Ok(Box::new(zstd::stream::read::Decoder::with_buffer(f)?))
    } else {
        Ok(Box::new(f))
    }
}

/// Writes the original content of `src` (plain or ".zst") to `dst` via a temp file next to it.
pub fn decode_to(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let dir = dst.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".hangar-unpack-{}", std::process::id()));
    let n = {
        let mut r = open_decoded(src)?;
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        let n = std::io::copy(&mut r, &mut w)?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        n
    };
    if let Err(e) = std::fs::rename(&tmp, dst) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(n)
}

/// Compresses an in-memory sample to estimate the ratio and speed: (input, output, seconds).
pub fn estimate(cfg: &Compression, files: &[PathBuf], budget: u64) -> (u64, u64, f64) {
    let start = std::time::Instant::now();
    let (mut input, mut output) = (0u64, 0u64);
    for p in files {
        if input >= budget {
            break;
        }
        let Ok(md) = std::fs::metadata(p) else { continue };
        let take = md.len().min(8 << 20).min(budget - input);
        let mut buf = Vec::with_capacity(take as usize);
        let Ok(f) = File::open(p) else { continue };
        if f.take(take).read_to_end(&mut buf).is_err() {
            continue;
        }
        input += buf.len() as u64;
        if worth_it(cfg, p, md.len()) {
            let out = zstd::bulk::compress(&buf, cfg.level.clamp(1, 19)).map(|v| v.len() as u64).unwrap_or(buf.len() as u64);
            output += out.min(buf.len() as u64);
        } else {
            output += buf.len() as u64;
        }
    }
    (input, output, start.elapsed().as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rules() {
        let d = std::env::temp_dir().join(format!("hangar-compress-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("options.lua");
        let text = "options = { graphics = { visibRange = \"High\" } }\n".repeat(2000);
        std::fs::write(&src, &text).unwrap();
        let cfg = Compression::default();
        assert!(worth_it(&cfg, &src, text.len() as u64));
        assert!(!worth_it(&cfg, Path::new("a.zip"), 1 << 20), "already compressed formats are skipped");
        assert!(!worth_it(&cfg, Path::new("a.lua"), 100), "tiny files are skipped");
        let dst = zst_path(&d.join("obj"));
        let n = compress_file(&cfg, &src, &dst).unwrap();
        assert!(n < text.len() as u64 / 10, "text compresses well: {n}");
        let back = d.join("back.lua");
        decode_to(&dst, &back).unwrap();
        assert_eq!(std::fs::read_to_string(&back).unwrap(), text);
        let mut s = String::new();
        open_decoded(&src).unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, text, "plain files read as they are");
        let (i, o, _) = estimate(&cfg, &[src.clone()], 1 << 20);
        assert!(o < i / 5);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
