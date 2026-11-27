<!--
  Hangar - a community-made updater and launcher for DCS World.
  Copyright (C) 2026 ChonkieMonkie
  SPDX-License-Identifier: GPL-3.0-or-later
  Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.
-->
<script lang="ts">
  // Inline stroke icons (no icon font, no network). B-024 icon system:
  // drawn on a 20 x 20 grid, 2 px stroke, straight lines on whole pixels, 2 px padding.
  // Shown at 20 px they stay sharp at 100/150/200 % Windows scaling. Other sizes work but
  // are not pixel-aligned.
  let { name, size = 20, color = "currentColor" }: { name: string; size?: number; color?: string } = $props();

  const paths: Record<string, string> = {
    overview: "M3 7V3h4M13 3h4v4M17 13v4h-4M7 17H3v-4M9 9h2v2H9z",
    update: "M10 3v10M6 9l4 4 4-4M4 17h12",
    mods: "M10 3l7 4-7 4-7-4zM3 12l7 4 7-4",
    logs: "M5 2h7l4 4v12H5zM12 2v4h4M8 10h5M8 14h3",
    backups: "M3 10A7 7 0 1 0 5.05 5.05M3 3v4h4M10 6v4l3 2",
    settings: "M3 6h7M14 6h3M3 14h3M10 14h7M10 6a2 2 0 1 0 4 0a2 2 0 1 0-4 0M6 14a2 2 0 1 0 4 0a2 2 0 1 0-4 0",
    repair: "M11.15 11.11A4.5 4.5 0 0 0 17.23 5.46L14.88 6.32 13.68 5.12 14.54 2.77A4.5 4.5 0 0 0 8.89 8.85L3.37 14.37A1.6 1.6 0 0 0 5.63 16.63Z",
    play: "M6 4v12l10-6z",
    folder: "M2 5h6l2 2h8v9H2z",
    plus: "M10 4v12M4 10h12",
    refresh: "M16 8A6.3 6.3 0 1 0 15 14M16 2v6h-6",
    pin: "M7 3h6l-1 5 3 3H5l3-3zM10 11v6",
    trash: "M3 5h14M8 5V3h4v2M5 5l1 12h8l1-12",
    shield: "M10 2l7 3v5c0 4-3 7-7 8-4-1-7-4-7-8V5zM7 10l2 2 4-4",
    check: "M4 10l4 4 8-8",
    warn: "M10 3l8 14H2zM10 8v4M10 14.5v.01",
    info: "M10 2a8 8 0 1 1 0 16a8 8 0 1 1 0-16zM10 9v5M10 6v.01",
    cross: "M5 5l10 10M15 5L5 15",
    external: "M12 3h5v5M17 3l-7 7M15 12v5H3V5h5",
    chevron: "M6 8l4 4 4-4",
    news: "M3 4h11v12H5a2 2 0 0 1-2-2zM14 8h3v6a2 2 0 0 1-2 2M6 7h5M6 10h5M6 13h3",
    mail: "M3 5h14v10H3zM3 5l7 6 7-6",
    storage: "M2 4h16v4H2zM3 8v8h14V8M8 11h4",
    graphics: "M2 3h16v11H2zM7 17h6M10 14v3M6 11a4 4 0 0 1 8 0M10 11l2-2",
  };
</script>

<svg viewBox="0 0 20 20" width={size} height={size} aria-hidden="true"
  style="fill: none; stroke: {color}; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; flex: none">
  <path d={paths[name] ?? ""}></path>
</svg>
