// Hangar - a community-made updater and launcher for DCS World.
// Copyright (C) 2026 ChonkieMonkie
// SPDX-License-Identifier: GPL-3.0-or-later
// Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

export function gb(bytes: number | null | undefined, digits = 1): string {
  if (bytes == null) return "—";
  return (bytes / 1e9).toFixed(digits) + " GB";
}

/** Bytes in the most readable unit. */
export function bytes(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1e9) return (n / 1e9).toFixed(n >= 1e11 ? 0 : 1) + " GB";
  if (n >= 1e6) return (n / 1e6).toFixed(n >= 1e8 ? 0 : 1) + " MB";
  if (n >= 1e3) return Math.round(n / 1e3) + " kB";
  return n + " B";
}

/** "Today 14:02", "Yesterday 09:10" or "27 Nov 14:02". */
export function when(unix: number | null | undefined): string {
  if (!unix) return "—";
  const d = new Date(unix * 1000);
  const t = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(new Date()) - day(d)) / 86400000);
  if (diff === 0) return `Today ${t}`;
  if (diff === 1) return `Yesterday ${t}`;
  return `${d.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: diff > 300 ? "numeric" : undefined })} ${t}`;
}

export function mb(value: number | null | undefined, digits = 1): string {
  if (value == null) return "—";
  return value.toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits }).replace(/,/g, " ");
}

export function duration(seconds: number | null | undefined): string {
  if (seconds == null || !isFinite(seconds)) return "—";
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${String(s % 60).padStart(2, "0")} s`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} min`;
}

export function clock(seconds: number | null | undefined): string {
  if (seconds == null || !isFinite(seconds)) return "—";
  const s = Math.max(0, Math.round(seconds));
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

export function finishAt(seconds: number | null | undefined): string {
  if (seconds == null) return "—";
  const d = new Date(Date.now() + seconds * 1000);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Friendly names for module IDs from autoupdate.cfg. Unknown IDs are shown as-is. */
const MODULE_NAMES: Record<string, string> = {
  WORLD: "DCS World",
  CAUCASUS_terrain: "Caucasus",
  MARIANAISLANDS_terrain: "Marianas",
  MARIANAISLANDSWWII_terrain: "Marianas WWII",
  KOLA_terrain: "Kola",
  SYRIA_terrain: "Syria",
  GERMANYCW_terrain: "Germany Cold War",
  PERSIANGULF_terrain: "Persian Gulf",
  NEVADA_terrain: "Nevada",
  NORMANDY_terrain: "Normandy",
  SINAI_terrain: "Sinai",
  AFGHANISTAN_terrain: "Afghanistan",
  IRAQ_terrain: "Iraq",
  "F-16C": "F-16C Viper",
  "FA-18C": "F/A-18C Hornet",
  AJS37: "AJS37 Viggen",
  "ASC_C-130J": "C-130J",
  "HEATBLUR_F-14": "F-14 Tomcat",
  "SU-33": "Su-33",
  SUPERCARRIER: "Supercarrier",
  "AH-64D": "AH-64D Apache",
};

export function moduleName(id: string): string {
  return MODULE_NAMES[id] ?? id;
}

export function isTerrain(id: string): boolean {
  return /_terrain$/i.test(id);
}
