// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Small, read-only Win32 helpers: shared file reads, disk space, known folders,
//! process list, network counters, top-level windows and UI Automation texts.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{IUIAutomation, TreeScope_Subtree};
use windows::Win32::UI::Shell::{FOLDERID_SavedGames, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, InternalGetWindowText, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, ShowWindow,
};

const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4; // READ | WRITE | DELETE

/// Reads a whole text file without locking it (the updater or DCS may be writing to it).
pub fn read_shared(path: &Path) -> std::io::Result<String> {
    let mut f = OpenOptions::new().read(true).share_mode(FILE_SHARE_ALL).open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Reads bytes from `offset` to end of file (for tailing logs).
pub fn read_shared_from(path: &Path, offset: u64) -> std::io::Result<(String, u64)> {
    let mut f = OpenOptions::new().read(true).share_mode(FILE_SHARE_ALL).open(path)?;
    let len = f.metadata()?.len();
    let start = if offset > len { 0 } else { offset };
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok((String::from_utf8_lossy(&buf).into_owned(), start + buf.len() as u64))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// (free bytes available to the user, total bytes) for the volume containing `path`.
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let w = wide(&path.to_string_lossy());
    let mut free = 0u64;
    let mut total = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), Some(&mut total), None) }.ok()?;
    Some((free, total))
}

pub fn saved_games_dir() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_SavedGames, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.map(PathBuf::from)
    }
}

#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
}

/// All running processes whose exe name matches (case-insensitive).
pub fn processes_named(name: &str) -> Vec<ProcInfo> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let exe = String::from_utf16_lossy(&e.szExeFile[..len]);
                if exe.eq_ignore_ascii_case(name) {
                    out.push(ProcInfo { pid: e.th32ProcessID, ppid: e.th32ParentProcessID });
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// A handle kept open so the exit code can be read after the process ends.
/// PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE only: enough to read, nothing more.
pub struct ProcHandle(HANDLE);

impl ProcHandle {
    pub fn open(pid: u32) -> Option<Self> {
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid) }.ok().map(ProcHandle)
    }
    /// Full path of the process's exe (works for elevated processes with limited query rights).
    pub fn image_path(&self) -> Option<PathBuf> {
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()?;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
    /// None while running (STILL_ACTIVE = 259) or if it cannot be read.
    pub fn exit_code(&self) -> Option<u32> {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.0, &mut code) }.ok()?;
        if code == 259 { None } else { Some(code) }
    }
}

impl Drop for ProcHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Total bytes received on physical, connected network adapters (compressed download for the graph).
pub fn net_rx_total() -> u64 {
    let mut sum = 0u64;
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return 0;
        }
        let n = (*table).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
        for r in rows {
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            let hardware = flags & 0x1 != 0;
            let filter = flags & 0x2 != 0;
            if hardware && !filter && r.OperStatus == IfOperStatusUp {
                sum += r.InOctets;
            }
        }
        FreeMibTable(table as _);
    }
    sum
}

#[derive(Clone, Debug)]
pub struct WinInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub class: String,
    pub title: String,
    pub visible: bool,
}

struct EnumCtx {
    pids: Vec<u32>,
    out: Vec<WinInfo>,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let ctx = &mut *(lparam.0 as *mut EnumCtx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if ctx.pids.contains(&pid) {
            let mut t = [0u16; 512];
            let tl = GetWindowTextW(hwnd, &mut t) as usize;
            let mut c = [0u16; 256];
            let cl = GetClassNameW(hwnd, &mut c) as usize;
            ctx.out.push(WinInfo {
                hwnd: hwnd.0 as isize,
                pid,
                class: String::from_utf16_lossy(&c[..cl]),
                title: String::from_utf16_lossy(&t[..tl]),
                visible: IsWindowVisible(hwnd).as_bool(),
            });
        }
    }
    BOOL(1)
}

/// Top-level windows owned by the given processes.
pub fn windows_of(pids: &[u32]) -> Vec<WinInfo> {
    let mut ctx = EnumCtx { pids: pids.to_vec(), out: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut EnumCtx as isize));
    }
    ctx.out
}

unsafe extern "system" fn child_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let out = &mut *(lparam.0 as *mut Vec<String>);
        let mut t = [0u16; 512];
        // InternalGetWindowText reads the text Windows stores for the control. It sends no
        // message to the other process, so it also works on the elevated `apply` worker.
        let n = InternalGetWindowText(hwnd, &mut t).max(0) as usize;
        let s = String::from_utf16_lossy(&t[..n.min(t.len())]);
        if !s.trim().is_empty() && out.len() < 200 {
            out.push(s);
        }
    }
    BOOL(1)
}

/// Texts of a dialog's child controls (Static, Button, ...), read without sending messages.
/// Read only: nothing is clicked or typed. Starts with the window's own title.
/// Minimizes another program's window without activating it (ROADMAP #12). Plain window
/// management, as when the user clicks minimize. Returns whether it is minimized afterwards
/// (Windows may refuse for a window of a process running as administrator).
pub fn minimize_window(hwnd: isize) -> bool {
    unsafe {
        let h = HWND(hwnd as *mut core::ffi::c_void);
        let _ = ShowWindow(h, SW_SHOWMINNOACTIVE);
        std::thread::sleep(std::time::Duration::from_millis(60));
        IsIconic(h).as_bool()
    }
}

/// Shows a minimized window again without stealing focus.
pub fn restore_window(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut core::ffi::c_void);
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
    }
}

pub fn child_texts(hwnd: isize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    unsafe {
        let h = HWND(hwnd as *mut _);
        let mut t = [0u16; 512];
        let n = InternalGetWindowText(h, &mut t).max(0) as usize;
        if n > 0 {
            out.push(String::from_utf16_lossy(&t[..n.min(t.len())]));
        }
        let _ = EnumChildWindows(Some(h), Some(child_cb), LPARAM(&mut out as *mut Vec<String> as isize));
    }
    out
}

/// Names of all UI Automation elements in a window (read only: no Invoke, no input).
pub fn uia_texts(uia: &IUIAutomation, hwnd: isize) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(el) = uia.ElementFromHandle(HWND(hwnd as *mut _)) else { return out };
        let Ok(cond) = uia.CreateTrueCondition() else { return out };
        let Ok(arr) = el.FindAll(TreeScope_Subtree, &cond) else { return out };
        let n = arr.Length().unwrap_or(0);
        for i in 0..n.min(200) {
            if let Ok(e) = arr.GetElement(i) {
                if let Ok(name) = e.CurrentName() {
                    let s = name.to_string();
                    if !s.trim().is_empty() {
                        out.push(s);
                    }
                }
            }
        }
    }
    out
}

/// Size of a folder tree in bytes. Does not follow junctions/symlinks.
pub fn folder_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if let Ok(m) = entry.metadata() {
                total += m.len();
            }
        }
    }
    total
}

/// Replaces the user profile path with %USERPROFILE% (for display and forum posts).
pub fn anonymize(s: &str) -> String {
    match std::env::var("USERPROFILE") {
        Ok(home) if !home.is_empty() => {
            let a = s.replace(&home, "%USERPROFILE%");
            a.replace(&home.replace('\\', "/"), "%USERPROFILE%")
        }
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    /// Reads a message box (#32770 + Static) owned by another process, like ED's updater dialog.
    /// `cargo test --lib reads_child_texts_of_another_process -- --ignored --nocapture`
    /// Opens a MessageBox in another process (same rights as Hangar), minimizes it and shows it
    /// again. The real updater window runs as administrator: that case is tested by the owner.
    /// `cargo test --lib minimizes_window_of_another_process -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn minimizes_window_of_another_process() {
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show('Hangar window test', 'Hangar test') | Out-Null"])
            .spawn()
            .unwrap();
        let mut result = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Some(w) = super::windows_of(&[child.id()]).into_iter().find(|w| w.class == "#32770") {
                let min = super::minimize_window(w.hwnd);
                super::restore_window(w.hwnd);
                result = Some(min);
                break;
            }
        }
        let _ = child.kill();
        assert_eq!(result, Some(true), "the window was found and minimized");
    }

    #[test]
    #[ignore]
    fn reads_child_texts_of_another_process() {
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show('Downloaded: 684.6 of 1248.3 MBytes    Speed: 32.2 MB/s', 'DCS Updater test') | Out-Null"])
            .spawn()
            .unwrap();
        let mut found = Vec::new();
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let wins: Vec<_> = super::windows_of(&[child.id()]).into_iter().filter(|w| w.class == "#32770").collect();
            if let Some(w) = wins.first() {
                found = super::child_texts(w.hwnd);
                break;
            }
        }
        let _ = child.kill();
        println!("{found:?}");
        let r = crate::window::parse_window_texts(&found);
        assert_eq!(r.downloaded, Some((684.6, 1248.3, 32.2)));
    }
}
