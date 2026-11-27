// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Hangar wall (B-007, B-008): a picture from the user's own folders on the Overview screen.
//! Folders are only read. Pictures are scaled down in memory and sent to the UI as a data URL;
//! no copies are saved.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::settings::{Settings, WallFolder};

const EXTS: [&str; 5] = ["jpg", "jpeg", "png", "bmp", "webp"];
const MAX_FILES: usize = 20_000;

#[derive(Clone, Debug)]
pub struct Pic {
    pub path: PathBuf,
    pub mtime: i64,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FolderStats {
    pub path: String,
    pub exists: bool,
    pub pictures: usize,
    /// HDR captures (.jxr) and other files that are skipped.
    pub skipped: usize,
}

fn ext_ok(p: &Path) -> bool {
    p.extension().map(|e| EXTS.contains(&e.to_string_lossy().to_ascii_lowercase().as_str())).unwrap_or(false)
}

/// Pictures in one folder (optionally its subfolders, max 4 levels).
pub fn index_folder(dir: &Path, subfolders: bool) -> (Vec<Pic>, usize) {
    let mut pics = Vec::new();
    let mut skipped = 0;
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
            if md.is_dir() {
                if subfolders && depth < 4 && !md.file_type().is_symlink() {
                    stack.push((e.path(), depth + 1));
                }
            } else if ext_ok(&e.path()) {
                if pics.len() < MAX_FILES {
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    pics.push(Pic { path: e.path(), mtime });
                }
            } else if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("jxr")) {
                skipped += 1;
            }
        }
    }
    (pics, skipped)
}

pub fn index(folders: &[WallFolder], subfolders: bool) -> Vec<Pic> {
    folders.iter().filter(|f| f.enabled).flat_map(|f| index_folder(Path::new(&f.path), subfolders).0).collect()
}

/// Picks according to the mode. `seed` varies the random pick; `skip` avoids repeating.
pub fn pick<'a>(pics: &'a [Pic], s: &Settings, seed: u64, skip: Option<&str>) -> Option<&'a Pic> {
    if pics.is_empty() {
        return None;
    }
    match s.wall_mode.as_str() {
        "newest" => pics.iter().max_by_key(|p| p.mtime),
        "favorite" => s
            .wall_favorite
            .as_deref()
            .and_then(|f| pics.iter().find(|p| p.path.to_string_lossy().eq_ignore_ascii_case(f)))
            .or_else(|| pics.iter().max_by_key(|p| p.mtime)),
        _ => {
            let mut i = (seed % pics.len() as u64) as usize;
            if pics.len() > 1 && skip.is_some_and(|k| pics[i].path.to_string_lossy() == k) {
                i = (i + 1) % pics.len();
            }
            pics.get(i)
        }
    }
}

/// Decodes, scales to fit `max_w`×`max_h` and encodes as JPEG. Returns a data URL.
pub fn to_data_url(path: &Path, max_w: u32, max_h: u32) -> Result<String, String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| format!("Could not read the picture: {e}"))?;
    let img = if img.width() > max_w || img.height() > max_h { img.thumbnail(max_w, max_h) } else { img };
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(img.to_rgb8())
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", base64(&buf)))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WallPicture {
    pub path: String,
    pub name: String,
    pub data_url: String,
    pub total: usize,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WallThumb {
    pub path: String,
    pub data_url: String,
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7)
}

/// The picture for Overview. `skip` = the one shown now (for "next picture").
#[tauri::command]
pub async fn wall_picture(skip: Option<String>) -> Result<Option<WallPicture>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = crate::settings::load();
        let pics = index(&s.wall_folders, s.wall_subfolders);
        // a few attempts in case a file can't be decoded
        for attempt in 0..5u64 {
            let Some(p) = pick(&pics, &s, seed().wrapping_add(attempt * 7919), skip.as_deref()) else { return Ok(None) };
            if let Ok(url) = to_data_url(&p.path, 2200, 1400) {
                return Ok(Some(WallPicture {
                    path: p.path.to_string_lossy().into_owned(),
                    name: p.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    data_url: url,
                    total: pics.len(),
                }));
            }
            if s.wall_mode != "random" {
                break;
            }
        }
        Ok(None)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn wall_stats(folders: Vec<WallFolder>, subfolders: bool) -> Result<Vec<FolderStats>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        folders
            .iter()
            .map(|f| {
                let p = Path::new(&f.path);
                let (pics, skipped) = if p.is_dir() { index_folder(p, subfolders) } else { (vec![], 0) };
                FolderStats { path: f.path.clone(), exists: p.is_dir(), pictures: pics.len(), skipped }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// The newest few pictures, as small thumbnails for Settings.
#[tauri::command]
pub async fn wall_thumbs(folders: Vec<WallFolder>, subfolders: bool, count: usize) -> Result<Vec<WallThumb>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut pics = index(&folders, subfolders);
        pics.sort_by(|a, b| b.mtime.cmp(&a.mtime));
        pics.iter()
            .take(count.min(12))
            .filter_map(|p| to_data_url(&p.path, 320, 200).ok().map(|u| WallThumb { path: p.path.to_string_lossy().into_owned(), data_url: u }))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_and_picks() {
        let d = std::env::temp_dir().join(format!("hangar-wall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        image::RgbImage::from_pixel(4000, 2000, image::Rgb([40, 200, 120])).save(d.join("big.png")).unwrap();
        image::RgbImage::from_pixel(10, 10, image::Rgb([1, 2, 3])).save(d.join("sub").join("small.jpg")).unwrap();
        std::fs::write(d.join("hdr.jxr"), b"x").unwrap();
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        let (top, skipped) = index_folder(&d, false);
        assert_eq!((top.len(), skipped), (1, 1));
        let folders = vec![WallFolder { path: d.to_string_lossy().into_owned(), enabled: true }];
        let all = index(&folders, true);
        assert_eq!(all.len(), 2);
        let mut s = Settings::default();
        s.wall_mode = "favorite".into();
        s.wall_favorite = Some(d.join("sub").join("small.jpg").to_string_lossy().into_owned());
        assert!(pick(&all, &s, 0, None).unwrap().path.ends_with("small.jpg"));
        s.wall_mode = "random".into();
        let first = pick(&all, &s, 0, None).unwrap().path.to_string_lossy().into_owned();
        assert_ne!(pick(&all, &s, 0, Some(&first)).unwrap().path.to_string_lossy(), first, "next picture differs");
        let url = to_data_url(&d.join("big.png"), 800, 600).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,/9j/"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
