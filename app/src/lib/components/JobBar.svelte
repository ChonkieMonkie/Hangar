<!--
  Hangar - a community-made updater and launcher for DCS World.
  Copyright (C) 2026 ChonkieMonkie
  SPDX-License-Identifier: GPL-3.0-or-later
  Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.
-->
<script lang="ts">
  // Progress of the background job (backup, verify, restore, mod moves). Shown on every screen.
  import { hangar } from "$lib/state.svelte";
  let j = $derived(hangar.job);
  let pct = $derived(j && j.total > 0 ? Math.min(100, Math.round((j.done / j.total) * 100)) : null);
</script>

{#if j}
  <div class="job" role="status" aria-live="polite">
    <span class="name">{j.job}…</span>
    <span class="bar tick"><span style="width: {pct ?? 4}%; background: var(--phosphor)"></span></span>
    <span class="mono faint">{j.total > 1 ? `${j.done} / ${j.total}` : ""}</span>
  </div>
{/if}

<style>
  .job { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 14px; align-items: center; padding: 10px 14px; border: 1px solid var(--line); border-radius: 8px; background: var(--panel); font-size: 13px; }
  .name { font-weight: 500; }
  .mono { font-size: 12px; }
</style>
