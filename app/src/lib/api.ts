// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

// Typed wrappers around the Rust commands (src-tauri/src). Field names are camelCase (serde).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Installation {
  key: string;
  path: string;
  exists: boolean;
  version: string | null;
  timestamp: string | null;
  arch: string | null;
  lang: string | null;
  modules: string[];
  launch: string | null;
  updaterPath: string;
  updaterExists: boolean;
  diskFree: number | null;
  diskTotal: number | null;
  cfgError: string | null;
}

export interface InstallReport {
  installations: Installation[];
  savedGames: string | null;
  savedGamesDcsDirs: string[];
}

export interface LogGroup {
  level: string;
  category: string;
  message: string;
  count: number;
  source: string | null;
  mine: boolean;
  firstTime: string;
  lastTime: string;
  sample: string[];
  sampleTruncated: boolean;
}

export interface DcsLog {
  path: string;
  exists: boolean;
  size: number;
  lineCount: number;
  dcsVersion: string | null;
  groups: LogGroup[];
}

export interface Phase { name: string; seconds: number; }

export interface UpdaterRun {
  startedUtc: string;
  kind: "update" | "install" | "uninstall" | "repair" | "other";
  target: string | null;
  result: "success" | "failed" | "unknown";
  resultText: string | null;
  durationS: number;
  toDownload: number | null;
  toDownloadFiles: number | null;
  phases: Phase[];
}

export interface ProcView { pid: number; ppid: number; exitCode: number | null; }

export interface MonitorState {
  status: "idle" | "running" | "finished";
  startedByHangar: boolean;
  action: string | null;
  installPath: string | null;
  elapsedS: number;
  phase: string | null;
  windowReadable: boolean;
  /** ED's updater window: "minimized" by Hangar, "blocked" (Windows refused), "shown", or null. */
  windowControl: "minimized" | "blocked" | "shown" | null;
  source: "window" | "downloads" | "none";
  downloadedMb: number | null;
  downloadTotalMb: number | null;
  edSpeedMbs: number | null;
  unpackedMb: number | null;
  unpackedTotalMb: number | null;
  edEtaS: number | null;
  currentFile: string | null;
  etaS: number | null;
  etaLowS: number | null;
  etaHighS: number | null;
  confidence: "high" | "medium" | "low" | null;
  unpackRateMbs: number | null;
  netRateMbs: number;
  /** "updater" = DCS updater's own download; "pc" = whole PC (until the updater's figure appears) */
  netScope: "updater" | "pc";
  netHistory: number[];
  unpackHistory: number[];
  freeBytes: number | null;
  toDownload: number | null;
  logTail: string[];
  processes: ProcView[];
  result: "success" | "failed" | "unknown" | null;
  resultText: string | null;
}

// ------------------------------------------------------------------ settings (settings.rs)
export interface WallFolder { path: string; enabled: boolean; }
export interface BackupSource { key: string; path: string; enabled: boolean; }
export interface LaunchPreset { id: string; name: string; args: string[]; }
export interface ModSet { id: string; name: string; enabled: string[]; }
export interface Settings {
  wallFolders: WallFolder[];
  wallMode: "random" | "newest" | "favorite";
  wallSubfolders: boolean;
  wallFavorite: string | null;
  /** HDR pictures: "tonemap" (shown, tone-mapped to SDR) or "skip". */
  wallHdr: "tonemap" | "skip";
  backupTarget: string | null;
  backupSources: BackupSource[];
  backupExcluded: string[];
  backupBeforeUpdate: boolean;
  launchPresets: LaunchPreset[];
  defaultPreset: string | null;
  modSets: ModSet[];
  /** ETA research: record updater runs (off by default). */
  recordUpdates: boolean;
  compression: Compression;
  /** ED's updater window while Hangar follows it. */
  updaterWindow: "minimize" | "show";
}

// ------------------------------------------------------------------ batch actions, storage (commands.rs)
export interface BatchReport {
  done: string[];
  /** [id, reason] */
  skipped: [string, string][];
  /** [id, error]; the batch stops at the first failure */
  failed: [string, string][];
  freed: number;
  verified: [string, boolean][];
}
export interface Compression {
  algorithm: "zstd" | "none";
  level: number;
  threads: number;
  minSize: number;
  skipExtensions: string[];
  backups: boolean;
  vault: boolean;
  vaultMaxGb: number | null;
}
export interface StorageOverview { backupBytes: number | null; vaultBytes: number; recycleBinForVault: boolean; }
export interface CompressionEstimate { sampled: number; compressed: number; seconds: number; }

// ------------------------------------------------------------------ ETA research (share.rs)
export interface RecordingsOverview {
  enabled: boolean;
  folder: string | null;
  count: number;
  bytes: number;
  /** "YYYYMMDD-HHMMSS" (UTC) */
  oldest: string | null;
  newest: string | null;
}
export interface ExportSummary { recordings: number; bytes: number; path: string; }

// ------------------------------------------------------------------ wall (wall.rs)
export interface WallPicture { path: string; name: string; dataUrl: string; total: number; }
export interface FolderStats { path: string; exists: boolean; pictures: number; skipped: number; hdr: number; }
export interface WallThumb { path: string; dataUrl: string; }

// ------------------------------------------------------------------ backups (backup.rs, commands.rs)
export interface Verification { atUnix: number; ok: boolean; checked: number; errors: string[]; }
export interface SnapshotInfo {
  id: string; createdUnix: number; reason: string; scope: "all" | "config"; sources: string[];
  fileCount: number; totalBytes: number; addedBytes: number; pinned: boolean; verified: Verification | null; skipped: number;
}
export interface SourceView { key: string; path: string; enabled: boolean; exists: boolean; }
export interface BackupOverview {
  target: string | null; targetFree: number | null; sameDisk: boolean; storeBytes: number;
  snapshots: SnapshotInfo[]; sources: SourceView[]; excluded: string[]; beforeUpdate: boolean;
  busy: string | null; dcsRunning: boolean;
  /** The backup drive has a Recycle Bin (deleting goes there). */
  recycleBin: boolean;
}
export interface TopFolder { name: string; bytes: number; cache: boolean; }
export interface RestoreResult { restored: number; failed: string[]; beforeRestoreId: string; }
export interface FileRow { source: string; path: string; size: number; }

// ------------------------------------------------------------------ mods (mods.rs, commands.rs)
export interface ModView {
  id: string; name: string; kind: string; root: string; enabled: boolean; addedUnix: number;
  bytes: number; files: number; sampleFiles: string[]; conflicts: string[]; conflictsWith: string[]; vault: string;
}
export interface Candidate { root: string; name: string; kind: string; bytes: number; files: number; managed: string | null; }
export interface ModsOverview {
  savedGames: string | null; dcsRunning: boolean; mods: ModView[]; candidates: Candidate[]; sets: ModSet[]; busy: string | null;
}
export interface JobProgress { job: string; done: number; total: number; item: string; finished: boolean; }

export const api = {
  installations: () => invoke<InstallReport>("get_installations"),
  dcsLog: (savedGamesDir: string) => invoke<DcsLog>("get_dcs_log", { savedGamesDir }),
  updaterHistory: (installPath: string) => invoke<UpdaterRun[]>("get_updater_history", { installPath }),
  monitorState: () => invoke<MonitorState>("get_monitor_state"),
  /** Only "update" and "repair" are accepted by the backend in 0.1. */
  startUpdater: (action: "update" | "repair", installPath: string) =>
    invoke<void>("start_updater", { action, installPath }),
  onMonitor: (cb: (s: MonitorState) => void): Promise<UnlistenFn> =>
    listen<MonitorState>("updater-state", (e) => cb(e.payload)),

  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),

  wallPicture: (skip: string | null = null) => invoke<WallPicture | null>("wall_picture", { skip }),
  wallStats: (folders: WallFolder[], subfolders: boolean, hdr = true) => invoke<FolderStats[]>("wall_stats", { folders, subfolders, hdr }),
  wallThumbs: (folders: WallFolder[], subfolders: boolean, count: number, hdr = true) => invoke<WallThumb[]>("wall_thumbs", { folders, subfolders, count, hdr }),

  backupOverview: () => invoke<BackupOverview>("backup_overview"),
  backupTopFolders: (path: string) => invoke<TopFolder[]>("backup_top_folders", { path }),
  backupNow: (scope: "all" | "config") => invoke<SnapshotInfo>("backup_now", { scope }),
  backupVerify: (id: string) => invoke<Verification>("backup_verify", { id }),
  backupRestore: (id: string, scope: "all" | "config", files: [string, string][] | null = null) =>
    invoke<RestoreResult>("backup_restore", { id, scope, files }),
  backupPin: (id: string, pinned: boolean) => invoke<void>("backup_pin", { id, pinned }),
  /** To the Recycle Bin, or for good with `permanent` and the phrase "thisismyownfault". */
  backupDelete: (id: string, permanent = false, phrase: string | null = null) => invoke<number>("backup_delete", { id, permanent, phrase }),
  backupFiles: (id: string) => invoke<FileRow[]>("backup_files", { id }),

  modsOverview: () => invoke<ModsOverview>("mods_overview"),
  modAdopt: (root: string, name: string, kind: string) => invoke<void>("mod_adopt", { root, name, kind }),
  modAdd: (path: string) => invoke<void>("mod_add", { path }),
  modSetEnabled: (id: string, enabled: boolean, force = false) => invoke<void>("mod_set_enabled", { id, enabled, force }),
  /** To the Recycle Bin, or for good with `permanent` and the phrase "thisismyownfault". */
  modRemove: (id: string, permanent = false, phrase: string | null = null) => invoke<void>("mod_remove", { id, permanent, phrase }),
  backupsEstimateFreed: (ids: string[]) => invoke<number>("backups_estimate_freed", { ids }),
  backupsBatch: (action: "delete" | "verify" | "pin" | "unpin", ids: string[], opts: { permanent?: boolean; phrase?: string | null } = {}) =>
    invoke<BatchReport>("backups_batch", { action, ids, permanent: opts.permanent ?? false, phrase: opts.phrase ?? null }),
  modsBatch: (action: "enable" | "disable" | "remove" | "forget", ids: string[], opts: { force?: boolean; permanent?: boolean; phrase?: string | null } = {}) =>
    invoke<BatchReport>("mods_batch", { action, ids, force: opts.force ?? false, permanent: opts.permanent ?? false, phrase: opts.phrase ?? null }),
  showUpdaterWindow: () => invoke<void>("show_updater_window"),
  /** Installs several mod zips/folders in one job. */
  modsAddMany: (paths: string[]) => invoke<BatchReport>("mods_add_many", { paths }),
  storageOverview: () => invoke<StorageOverview>("storage_overview"),
  compressionEstimate: (cfg: Compression) => invoke<CompressionEstimate>("compression_estimate", { cfg }),
  vaultCompressAll: () => invoke<[number, number]>("vault_compress_all"),
  modForget: (id: string) => invoke<void>("mod_forget", { id }),
  modApplySet: (setId: string) => invoke<string[]>("mod_apply_set", { setId }),

  launchDcs: (installPath: string, launch: string | null, presetId: string) => invoke<void>("launch_dcs", { installPath, launch, presetId }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  recordingsOverview: () => invoke<RecordingsOverview>("recordings_overview"),
  recordingsExport: (path: string) => invoke<ExportSummary>("recordings_export", { path }),
  recordingsDeleteAll: () => invoke<number>("recordings_delete_all"),
  openProjectLink: (which: "source" | "issues" | "gpl") => invoke<void>("open_project_link", { which }),
  licenseText: () => invoke<string>("license_text"),
  thirdPartyNotices: () => invoke<string>("third_party_notices"),
  /** Opens one of ED's public pages in the default browser (fixed list, built in Rust). */
  openEdPage: (page: "changelog" | "newsletters" | "news" | "version", version: string | null = null) =>
    invoke<void>("open_ed_page", { page, version }),
  onJob: (cb: (p: JobProgress) => void): Promise<UnlistenFn> => listen<JobProgress>("job-progress", (e) => cb(e.payload)),
};
