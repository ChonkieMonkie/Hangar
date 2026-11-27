<!--
  Hangar - a community-made updater and launcher for DCS World.
  Copyright (C) 2026 ChonkieMonkie
  SPDX-License-Identifier: GPL-3.0-or-later
  Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.
-->
<script lang="ts">
  // One series per chart (never two y-axes). Thin line + soft area, hover crosshair.
  let {
    values,
    color,
    label,
    unit = "MB/s",
    seconds = 120,
  }: { values: number[]; color: string; label: string; unit?: string; seconds?: number } = $props();

  const W = 600;
  const H = 160;
  let hover = $state<number | null>(null);

  // Pad on the left so the newest sample is always at the right edge.
  let series = $derived(values.length >= seconds ? values.slice(-seconds) : [...Array(seconds - values.length).fill(null), ...values]);
  let maxVal = $derived(niceMax(Math.max(1, ...values.filter((v) => v != null))));

  function niceMax(v: number): number {
    const steps = [5, 10, 20, 30, 40, 60, 80, 100, 120, 160, 200, 300, 400, 600, 800, 1000];
    return steps.find((s) => s >= v * 1.1) ?? Math.ceil(v / 500) * 500;
  }

  let line = $derived.by(() => {
    let d = "";
    series.forEach((v, i) => {
      if (v == null) return;
      const x = (i / (series.length - 1)) * W;
      const y = H - (Math.min(v, maxVal) / maxVal) * (H - 4);
      d += (d ? "L" : "M") + x.toFixed(1) + " " + y.toFixed(1) + " ";
    });
    return d;
  });
  let firstX = $derived(((series.findIndex((v) => v != null) || 0) / (series.length - 1)) * W);
  let area = $derived(line ? `${line}L${W} ${H} L${firstX.toFixed(1)} ${H} Z` : "");
  let current = $derived(values.length ? values[values.length - 1] : null);

  function move(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const i = Math.round(((e.clientX - r.left) / r.width) * (series.length - 1));
    hover = series[i] == null ? null : i;
  }
</script>

<div class="chart">
  <div class="head">
    <h2><span class="sw" style="background: {color}"></span>{label}</h2>
    <div class="now mono">{current == null ? "—" : current.toFixed(1)} <span class="muted">{unit}</span></div>
  </div>
  <div class="plot">
    <div class="y mono faint"><span>{maxVal}</span><span>{maxVal / 2}</span><span>0</span></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="area" role="img" aria-label="{label}, now {current?.toFixed(1) ?? 'unknown'} {unit}" onmousemove={move} onmouseleave={() => (hover = null)}>
      <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
        <path d="M0 0.5 L{W} 0.5 M0 {H / 2} L{W} {H / 2} M0 {H - 0.5} L{W} {H - 0.5}" class="grid"></path>
        {#if area}<path d={area} style="fill: {color}; fill-opacity: 0.15"></path>{/if}
        {#if line}<path d={line} style="fill: none; stroke: {color}; stroke-width: 2; stroke-linejoin: round; vector-effect: non-scaling-stroke"></path>{/if}
      </svg>
      {#if hover != null}
        {@const pct = (hover / (series.length - 1)) * 100}
        <div class="cross" style="left: {pct}%"></div>
        <div class="tip" style="left: {pct > 65 ? `calc(${pct}% - 140px)` : `calc(${pct}% + 10px)`}">
          <div class="muted">{series.length - 1 - hover} s ago</div>
          <div class="mono">{series[hover]?.toFixed(1)} {unit}</div>
        </div>
      {/if}
    </div>
  </div>
  <div class="x mono faint"><span>−2 min</span><span>−1 min</span><span>now</span></div>
</div>

<style>
  .chart { display: flex; flex-direction: column; gap: 10px; }
  .head { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  h2 { display: flex; align-items: center; gap: 8px; font-size: 15px; }
  .sw { width: 10px; height: 10px; border-radius: 2px; }
  .now { font-size: 22px; }
  .now .muted { font-size: 12px; }
  .plot { display: flex; gap: 10px; }
  .y { display: flex; flex-direction: column; justify-content: space-between; height: 150px; width: 28px; text-align: right; font-size: 10px; flex: none; }
  .area { position: relative; flex: 1; min-width: 0; height: 150px; cursor: crosshair; }
  svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
  .grid { fill: none; stroke: #232b30; stroke-width: 1; vector-effect: non-scaling-stroke; }
  .cross { position: absolute; top: 0; bottom: 0; width: 1px; background: var(--ink-soft); pointer-events: none; }
  .tip { position: absolute; top: 6px; width: 130px; background: var(--ground); border: 1px solid var(--line-2); border-radius: 6px; padding: 6px 9px; font-size: 12px; pointer-events: none; }
  .x { display: flex; justify-content: space-between; font-size: 10px; padding-left: 38px; }
</style>
