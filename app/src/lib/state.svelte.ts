// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

import { getVersion } from "@tauri-apps/api/app";
import { api, type InstallReport, type MonitorState, type UpdaterRun, type Installation, type Settings, type JobProgress, type WallPicture } from "./api";

/** App-wide state shared by all screens. */
export const hangar = $state({
  report: null as InstallReport | null,
  history: [] as UpdaterRun[],
  monitor: null as MonitorState | null,
  loading: true,
  error: null as string | null,
  settings: null as Settings | null,
  /** The background job running now (backup, restore, mod move), if any. */
  job: null as JobProgress | null,
  /** The Hangar wall picture on Overview. Picked once per app start (or on "next picture"),
   *  so switching tabs does not pick a new one. Cleared when the wall settings change. */
  wallPicture: null as WallPicture | null,
  wallPictureLoaded: false,
  /** Hangar's own version (from tauri.conf.json), shown at the top of every page. */
  version: "",
});

export async function loadSettings(): Promise<Settings> {
  hangar.settings = await api.settings();
  return hangar.settings;
}

/** Saves settings and keeps the shared copy in step. */
export async function saveSettings(next: Settings): Promise<Settings> {
  const before = hangar.settings;
  hangar.settings = await api.saveSettings(next);
  if (wallKey(before) !== wallKey(hangar.settings)) {
    hangar.wallPicture = null;
    hangar.wallPictureLoaded = false;
  }
  return hangar.settings;
}

function wallKey(s: Settings | null): string {
  return s ? JSON.stringify([s.wallFolders, s.wallMode, s.wallSubfolders, s.wallFavorite]) : "";
}

/** Loads a wall picture. `next` = skip the one shown now. */
export async function loadWallPicture(next = false): Promise<void> {
  try {
    hangar.wallPicture = await api.wallPicture(next ? (hangar.wallPicture?.path ?? null) : null);
  } catch {
    hangar.wallPicture = null;
  } finally {
    hangar.wallPictureLoaded = true;
  }
}

export function currentInstall(): Installation | null {
  return hangar.report?.installations[0] ?? null;
}

/** Main Saved Games folder for DCS (DCS, else DCS.openbeta). */
export function savedGamesDcs(): string | null {
  const r = hangar.report;
  if (!r?.savedGames) return null;
  const pick = ["DCS", "DCS.openbeta"].find((d) => r.savedGamesDcsDirs.includes(d));
  return pick ? `${r.savedGames}\\${pick}` : null;
}

export async function refresh(): Promise<void> {
  try {
    hangar.report = await api.installations();
    const inst = currentInstall();
    hangar.history = inst ? await api.updaterHistory(inst.path) : [];
    hangar.error = null;
  } catch (e) {
    hangar.error = String(e);
  } finally {
    hangar.loading = false;
  }
}

let started = false;
export async function startHangar(): Promise<void> {
  if (started) return;
  started = true;
  void getVersion().then((v) => (hangar.version = v)).catch(() => {});
  await refresh();
  void loadSettings().catch(() => {});
  await api.onJob((p) => {
    hangar.job = p.finished ? null : p;
  });
  try {
    hangar.monitor = await api.monitorState();
  } catch {
    /* monitor not ready yet */
  }
  let lastStatus = hangar.monitor?.status;
  await api.onMonitor((s) => {
    hangar.monitor = s;
    if (s.status === "finished" && lastStatus !== "finished") void refresh();
    lastStatus = s.status;
  });
}
