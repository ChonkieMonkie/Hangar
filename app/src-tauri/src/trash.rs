// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Windows Recycle Bin (B-030): removing a mod moves its files there so a mistake can be undone.
//! Permanent deletion needs Shift and the phrase in [`PERMANENT_PHRASE`], checked here too.

use std::path::{Path, PathBuf};

pub const PERMANENT_PHRASE: &str = "thisismyownfault";

/// The phrase the user typed for a permanent delete (spaces around it are ignored).
pub fn phrase_ok(typed: Option<&str>) -> bool {
    typed.is_some_and(|t| t.trim() == PERMANENT_PHRASE)
}

/// True if the drive of `path` has a Recycle Bin (network drives usually do not).
#[cfg(windows)]
pub fn available(path: &Path) -> bool {
    use windows::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};
    use windows::core::HSTRING;
    let Some(root) = path.components().next() else { return false };
    let mut root = PathBuf::from(root.as_os_str());
    root.push("\\");
    if path.to_string_lossy().starts_with("\\\\") {
        return false; // UNC path: no Recycle Bin
    }
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe { SHQueryRecycleBinW(&HSTRING::from(root.as_os_str()), &mut info).is_ok() }
}

#[cfg(not(windows))]
pub fn available(_path: &Path) -> bool {
    false
}

/// Moves files or folders to the Recycle Bin. Runs on its own STA thread (IFileOperation).
/// Refuses if the Recycle Bin is not available, so nothing is deleted permanently by surprise.
#[cfg(windows)]
pub fn recycle(paths: &[PathBuf]) -> Result<(), String> {
    let paths: Vec<PathBuf> = paths.iter().filter(|p| p.exists()).cloned().collect();
    if paths.is_empty() {
        return Ok(());
    }
    if let Some(p) = paths.iter().find(|p| !available(p)) {
        return Err(format!("The Recycle Bin is not available for {}. Hold Shift to delete permanently.", p.display()));
    }
    std::thread::spawn(move || unsafe {
        use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoUninitialize};
        use windows::Win32::UI::Shell::*;
        use windows::core::HSTRING;
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let res = (|| -> windows::core::Result<bool> {
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            // FOF_WANTNUKEWARNING: if something is too big for the Recycle Bin, Windows asks
            // before deleting it for good instead of doing so silently
            op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_SILENT | FOF_NOERRORUI | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE)?;
            for p in &paths {
                let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(p.as_os_str()), None)?;
                op.DeleteItem(&item, None)?;
            }
            op.PerformOperations()?;
            Ok(op.GetAnyOperationsAborted()?.as_bool())
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        match res {
            Ok(false) => Ok(()),
            Ok(true) => Err("Moving to the Recycle Bin was cancelled.".to_string()),
            Err(e) => Err(format!("Could not move to the Recycle Bin: {}", e.message())),
        }
    })
    .join()
    .map_err(|_| "The Recycle Bin operation failed.".to_string())?
}

#[cfg(not(windows))]
pub fn recycle(_paths: &[PathBuf]) -> Result<(), String> {
    Err("The Recycle Bin needs Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_rules() {
        assert!(phrase_ok(Some("thisismyownfault")));
        assert!(phrase_ok(Some("  thisismyownfault ")));
        assert!(!phrase_ok(Some("ThisIsMyOwnFault")));
        assert!(!phrase_ok(Some("")));
        assert!(!phrase_ok(None));
    }

    /// Puts two tiny temp files in the real Recycle Bin, so it is ignored by default.
    /// `cargo test --lib recycle_moves_temp_files -- --ignored`
    #[test]
    #[ignore]
    fn recycle_moves_temp_files() {
        let d = std::env::temp_dir().join(format!("hangar-trash-test-{}", std::process::id()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.txt"), "hangar test file, safe to delete").unwrap();
        std::fs::write(d.join("sub").join("b.txt"), "hangar test file, safe to delete").unwrap();
        assert!(available(&d));
        recycle(&[d.join("a.txt"), d.join("sub")]).unwrap();
        assert!(!d.join("a.txt").exists() && !d.join("sub").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
