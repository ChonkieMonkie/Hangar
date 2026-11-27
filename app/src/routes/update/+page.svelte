<!--
  Hangar - a community-made updater and launcher for DCS World.
  Copyright (C) 2026 ChonkieMonkie
  SPDX-License-Identifier: GPL-3.0-or-later
  Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.
-->
<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import LineChart from "$lib/components/LineChart.svelte";
  import { api } from "$lib/api";
  import { mb, gb, duration, clock, finishAt } from "$lib/format";
  import Switch from "$lib/components/Switch.svelte";
  import { hangar, currentInstall, loadSettings, saveSettings } from "$lib/state.svelte";

  async function setMinimize(on: boolean) {
    const s = structuredClone($state.snapshot(hangar.settings ?? (await loadSettings())));
    s.updaterWindow = on ? "minimize" : "show";
    await saveSettings(s);
  }

  let install = $derived(currentInstall());
  let m = $derived(hangar.monitor);
  let running = $derived(m?.status === "running");
  let finished = $derived(m?.status === "finished");
  let error = $state<string | null>(null);
  let confirmRepair = $state(false);

  let unpackPct = $derived(m?.unpackedMb != null && m.unpackedTotalMb ? (m.unpackedMb / m.unpackedTotalMb) * 100 : null);
  let dlPct = $derived(m?.downloadedMb != null && m.downloadTotalMb ? (m.downloadedMb / m.downloadTotalMb) * 100 : null);
  let installing = $derived(m?.phase?.startsWith("Installing") ?? false);
  let lowSpace = $derived(m?.toDownload != null && m.freeBytes != null && m.freeBytes < m.toDownload * 1.05);
  let lastRun = $derived(hangar.history[0] ?? null);

  async function start(action: "update" | "repair") {
    if (!install) return;
    error = null;
    try {
      await api.startUpdater(action, install.path);
    } catch (e) {
      error = String(e);
    }
  }
</script>

<header class="top">
  <div>
    <div class="bay">HANGAR {hangar.version}</div>
    <h1>{running ? "Updating DCS World" : finished ? "Update finished" : "Update"}</h1>
    {#if running || finished}<div class="mono muted sub">elapsed {duration(m?.elapsedS)}{m?.action ? ` · ${m.action}` : ""}{m && !m.startedByHangar ? " · started outside Hangar" : ""}</div>{/if}
  </div>
</header>

{#if !running && !finished}
  <section class="hero idle">
    <div>
      <div class="label">DCS World {install?.version ? `v${install.version}` : ""}</div>
      <p class="muted">Hangar starts ED's updater and follows it here. If you have chosen a backup folder, your controls and options are backed up first.</p>
    </div>
    <div class="actions">
      <button class="btn primary big" onclick={() => start("update")} disabled={!install?.updaterExists}><Icon name="update" />Update</button>
      {#if !confirmRepair}
        <button class="btn big" onclick={() => (confirmRepair = true)} disabled={!install?.updaterExists}><Icon name="repair" />Repair</button>
      {:else}
        <span class="confirm">Repair restores all official files and overwrites mods in the install folder.
          <button class="btn small" onclick={() => { confirmRepair = false; void start("repair"); }}>Start repair</button>
          <button class="btn small" onclick={() => (confirmRepair = false)}>Cancel</button>
        </span>
      {/if}
    </div>
    {#if error}<div class="err"><Icon name="warn" size={16} color="var(--warning)" />{error}</div>{/if}
    {#if hangar.settings}
      <div class="minrow">
        <Switch checked={hangar.settings.updaterWindow !== "show"} label="Minimize ED's updater window" onchange={setMinimize} />
        <span>Minimize ED's updater window while Hangar shows its progress</span>
        <span class="faint small">It comes back by itself if it asks you something.</span>
      </div>
    {/if}
    {#if lastRun}
      <div class="last faint">Last run: {lastRun.startedUtc} UTC · {lastRun.kind}{lastRun.target ? ` ${lastRun.target}` : ""} · {lastRun.result} · {duration(lastRun.durationS)}</div>
    {/if}
  </section>
{/if}

{#if m && running && m.windowControl === "minimized"}
  <div class="winnote"><Icon name="info" size={16} color="var(--sky)" /><span>ED's updater window is minimized while Hangar shows its progress.</span>
    <button class="btn small" onclick={() => api.showUpdaterWindow()}>Show updater window</button></div>
{:else if m && running && m.windowControl === "blocked"}
  <div class="winnote"><Icon name="info" size={16} color="var(--ink-2)" /><span>Windows doesn't let Hangar minimize ED's updater window, because it runs as administrator. You can minimize it yourself.</span></div>
{/if}

{#if m && (running || finished)}
  {#if finished}
    <section class="hero result {m.result}">
      <Icon name={m.result === "failed" ? "warn" : "check"} size={22} color={m.result === "failed" ? "var(--warning)" : "var(--phosphor)"} />
      <div>
        <h2>{m.result === "success" ? "Done" : m.result === "failed" ? "The updater stopped" : "The updater has closed"}</h2>
        <div class="muted">{m.resultText ?? ""} · {duration(m.elapsedS)}</div>
      </div>
      <button class="btn small" onclick={() => (hangar.monitor = null)}>Close</button>
    </section>
  {/if}

  <div class="grid2">
    <section class="hero progress">
      <div class="label">{m.phase ?? "Starting…"}</div>
      <div class="pct-row">
        <div class="pct">{unpackPct == null ? "—" : unpackPct.toFixed(0)}<span>%</span></div>
        {#if m.unpackedTotalMb}<div class="mono soft">{gb(m.unpackedMb ? m.unpackedMb * 1e6 : 0, 2)} <span class="faint">/ {gb(m.unpackedTotalMb * 1e6, 2)}</span></div>{/if}
      </div>
      {#if m.downloadTotalMb}
        <div class="rowbar">
          <div class="rowhead"><span><span class="sw" style="background: var(--sky)"></span>Downloaded</span><span class="mono faint">{mb(m.downloadedMb)} / {mb(m.downloadTotalMb)} MB</span></div>
          <div class="bar"><span style="width: {dlPct ?? 0}%; background: var(--sky)"></span></div>
        </div>
      {/if}
      <div class="rowbar">
        <div class="rowhead"><span><span class="sw" style="background: var(--phosphor)"></span>Unpacked</span><span class="mono faint">{mb(m.unpackedMb)} / {mb(m.unpackedTotalMb)} MB</span></div>
        <div class="bar tick tall"><span style="width: {unpackPct ?? 0}%; background: var(--phosphor)"></span></div>
      </div>
      {#if installing && m.currentFile}<div class="mono faint file">{m.currentFile}</div>{/if}
      {#if lowSpace}<div class="err"><Icon name="warn" size={16} color="var(--warning)" />Not enough free space: needs {gb(m.toDownload)}, {gb(m.freeBytes)} free.</div>{/if}
    </section>

    <section class="card eta">
      <div class="etahead">
        <div class="label">Time remaining</div>
        {#if m.confidence}<span class="chip {m.confidence === 'high' ? 'ok' : m.confidence === 'medium' ? 'warn' : 'bad'}">{m.confidence} confidence</span>{/if}
      </div>
      <div class="etabig">{finished ? "—" : m.etaS == null ? "Calculating…" : duration(m.etaS)}</div>
      {#if !finished && m.etaLowS != null && m.etaHighS != null}
        <div class="muted small">{clock(m.etaLowS)} – {clock(m.etaHighS)} · done ≈ {finishAt(m.etaS)}</div>
      {/if}
      {#if !finished && m.edEtaS != null}<div class="faint small">ED's updater: {clock(m.edEtaS)}</div>{/if}
    </section>
  </div>

  <div class="grid2">
    <section class="card"><LineChart values={m.netHistory} color="var(--sky)" label={m.netScope === "updater" ? "Network · DCS updater" : "Network · whole PC"} /></section>
    <section class="card"><LineChart values={m.unpackHistory} color="var(--phosphor)" label="Unpacked" /></section>
  </div>

  {#if m.logTail.length}
    <section class="card">
      <div class="label">Updater log</div>
      <div class="tail mono">{#each m.logTail as l}<div>{l}</div>{/each}</div>
    </section>
  {/if}
{/if}

<style>
  .sub { margin-top: 6px; font-size: 12px; }
  .idle { padding: 24px; display: flex; flex-direction: column; gap: 16px; }
  .idle p { margin: 8px 0 0; }
  .actions { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .big { min-height: 48px; font-size: 15px; }
  .confirm { display: inline-flex; flex-wrap: wrap; gap: 8px; align-items: center; font-size: 13px; padding: 8px 12px; border: 1px solid #4a2a2d; border-radius: 8px; background: var(--warning-bg); }
  .err { display: flex; gap: 8px; align-items: center; color: var(--warning-soft); font-size: 13px; }
  .last { font-size: 12px; }
  .result { display: flex; align-items: center; gap: 14px; padding: 16px 20px; }
  .result > div { flex: 1; }
  .grid2 { display: grid; grid-template-columns: repeat(auto-fit, minmax(380px, 1fr)); gap: 18px; }
  .progress { padding: 24px; display: flex; flex-direction: column; gap: 16px; }
  .pct-row { display: flex; align-items: baseline; gap: 18px; flex-wrap: wrap; }
  .pct { font-family: var(--font-head); font-weight: 600; font-size: 68px; line-height: 0.9; font-variant-numeric: tabular-nums; }
  .pct span { font-size: 30px; color: var(--ink-2); }
  .soft { color: var(--ink-soft); font-size: 15px; }
  .rowbar { display: flex; flex-direction: column; gap: 6px; }
  .rowhead { display: flex; justify-content: space-between; gap: 10px; font-size: 12.5px; color: var(--ink-soft); }
  .rowhead > span:first-child { display: flex; align-items: center; gap: 8px; }
  .sw { width: 10px; height: 10px; border-radius: 2px; display: inline-block; }
  .bar.tall { height: 14px; }
  .file { font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .eta { display: flex; flex-direction: column; gap: 10px; }
  .etahead { display: flex; justify-content: space-between; align-items: center; }
  .etabig { font-family: var(--font-head); font-weight: 600; font-size: 42px; line-height: 1; font-variant-numeric: tabular-nums; }
  .small { font-size: 13px; }
  .tail { margin-top: 10px; font-size: 11.5px; line-height: 1.7; color: var(--ink-2); overflow-x: auto; white-space: nowrap; }
  .minrow { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; font-size: 13px; }
  .minrow .small { font-size: 12px; }
  .winnote { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; font-size: 13px; color: var(--ink-soft); }
</style>
