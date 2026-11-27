// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

//! Hangar - an unofficial updater & launcher for DCS World.
//! Not affiliated with or endorsed by Eagle Dynamics SA.
//!
//! Design rules (see docs/ and CLAUDE.md):
//! - ED's programs run as separate processes; nothing is patched, injected or linked.
//! - ED's files are only read. Hangar writes only to Saved Games and its own data
//!   (`%LOCALAPPDATA%\Hangar`; in 0.1 only run recordings, see recorder.rs).
//! - No login and no network traffic to ED's servers.
//! - The updater window is only *read* (UI Automation); Hangar never clicks or types in it.

pub mod backup;
mod commands;
pub mod compress;
pub mod eta;
pub mod hdr;
mod install;
mod logs;
pub mod mods;
pub mod recorder;
pub mod replay;
pub mod settings;
pub mod share;
pub mod trash;
mod updater;
pub mod wall;
pub mod window;
mod winutil;

/// Read-only check against the real installation on this PC:
/// `cargo test --lib real_install_read_only -- --ignored --nocapture`
#[cfg(test)]
mod smoke {
    #[test]
    #[ignore]
    fn real_install_read_only() {
        let rep = crate::install::find_installations();
        println!("saved games: {:?}, dcs dirs: {:?}", rep.saved_games, rep.saved_games_dcs_dirs);
        for i in &rep.installations {
            println!("{} | {} | v{:?} | {} modules | launch {:?} | free {:?} | cfg_error {:?}",
                i.key, i.path, i.version, i.modules.len(), i.launch, i.disk_free.map(|b| b / 1_000_000_000), i.cfg_error);
            let root = std::path::Path::new(&i.path);
            for name in ["autoupdate_log_old.txt", "autoupdate_log.txt"] {
                if let Ok(t) = crate::winutil::read_shared(&root.join(name)) {
                    for r in crate::logs::parse_updater_log(&t) {
                        println!("  run {} {} {:?} {} {:.1}s to_download={:?} phases={}",
                            r.started_utc, r.kind, r.target, r.result, r.duration_s, r.to_download, r.phases.len());
                    }
                }
            }
        }
        if let Some(sg) = &rep.saved_games {
            let log = crate::logs::parse_dcs_log(&std::path::Path::new(sg).join("DCS").join("Logs").join("dcs.log"));
            println!("dcs.log: exists={} lines={} version={:?} groups={}", log.exists, log.line_count, log.dcs_version, log.groups.len());
            for g in log.groups.iter().take(8) {
                println!("  {:>5}x {:<7} {} | mine={} src={:?}", g.count, g.level, g.category, g.mine, g.source);
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(updater::MonitorShared::default())
        .setup(|app| {
            updater::start_monitor(app.handle().clone());
            // undo a mod operation that was interrupted last time (only acts if a journal exists)
            std::thread::spawn(|| {
                commands::recover_mods();
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            install::get_installations,
            logs::get_dcs_log,
            logs::get_updater_history,
            updater::get_monitor_state,
            updater::start_updater,
            settings::get_settings,
            settings::save_settings,
            wall::wall_picture,
            wall::wall_stats,
            wall::wall_thumbs,
            commands::backup_overview,
            commands::backup_top_folders,
            commands::backup_now,
            commands::backup_verify,
            commands::backup_restore,
            commands::backup_pin,
            commands::backup_delete,
            commands::backup_files,
            commands::mods_overview,
            commands::mod_adopt,
            commands::mod_add,
            commands::mod_set_enabled,
            commands::mod_remove,
            commands::mod_forget,
            commands::mod_apply_set,
            commands::launch_dcs,
            commands::reveal_path,
            commands::open_ed_page,
            commands::open_project_link,
            commands::license_text,
            commands::third_party_notices,
            updater::show_updater_window,
            commands::backups_estimate_freed,
            commands::backups_batch,
            commands::mods_batch,
            commands::mods_add_many,
            commands::storage_overview,
            commands::compression_estimate,
            commands::vault_compress_all,
            share::recordings_overview,
            share::recordings_export,
            share::recordings_delete_all,
            commands::current_job,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hangar");
}
