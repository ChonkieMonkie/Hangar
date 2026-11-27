<!--
  Hangar - a community-made updater and launcher for DCS World.
  Copyright (C) 2026 ChonkieMonkie
  SPDX-License-Identifier: GPL-3.0-or-later
  Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import Icon from "$lib/components/Icon.svelte";
  import { api, type DcsLog } from "$lib/api";
  import { gb, moduleName, isTerrain, duration } from "$lib/format";
  import { hangar, currentInstall, savedGamesDcs, loadWallPicture } from "$lib/state.svelte";

  let install = $derived(currentInstall());
  let log = $state<DcsLog | null>(null);
  let actionError = $state<string | null>(null);
  let confirmRepair = $state(false);
  let picture = $derived(hangar.wallPicture);
  let pictureLoading = $state(false);
  let preset = $state<string>("");
  let launching = $state(false);

  async function nextPicture() {
    pictureLoading = true;
    await loadWallPicture(true);
    pictureLoading = false;
  }
  // Once per app start (B-008 "random at start"): the picture lives in the shared state, so
  // coming back to Overview shows the same one. Not an $effect: it must not re-run when the
  // picture it sets changes (that loop swapped pictures several times a second, ROADMAP #1).
  onMount(() => {
    if (!hangar.wallPictureLoaded) void loadWallPicture(false);
  });
  $effect(() => {
    const s = hangar.settings;
    if (s && !preset) preset = s.defaultPreset ?? s.launchPresets[0]?.id ?? "";
  });
  let dcsRunningHint = $state(false);

  async function launch() {
    if (!install || !preset) return;
    actionError = null;
    launching = true;
    try {
      await api.launchDcs(install.path, install.launch, preset);
      dcsRunningHint = true;
      setTimeout(() => (dcsRunningHint = false), 8000);
    } catch (e) {
      actionError = String(e);
    } finally {
      launching = false;
    }
  }

  $effect(() => {
    const sg = savedGamesDcs();
    if (sg) api.dcsLog(sg).then((l) => (log = l)).catch(() => (log = null));
  });

  let lastUpdate = $derived(hangar.history.find((r) => r.kind === "update" && r.result === "success") ?? null);
  let terrains = $derived(install?.modules.filter(isTerrain) ?? []);
  let aircraft = $derived(install?.modules.filter((m) => !isTerrain(m) && m !== "WORLD") ?? []);
  let mineErrors = $derived(log?.groups.filter((g) => g.level === "ERROR" && g.mine).length ?? 0);
  let running = $derived(hangar.monitor?.status === "running");
  let usedPct = $derived(install?.diskFree != null && install.diskTotal ? 100 - (install.diskFree / install.diskTotal) * 100 : null);

  async function start(action: "update" | "repair") {
    if (!install) return;
    actionError = null;
    try {
      await api.startUpdater(action, install.path);
      await goto("/update");
    } catch (e) {
      actionError = String(e);
    }
  }
</script>

<header>
  <div class="bay">HANGAR {hangar.version}</div>
  <h1>Overview</h1>
</header>

{#if hangar.loading}
  <p class="muted">Looking for DCS World…</p>
{:else if !install}
  <section class="hero empty">
    <h2>No DCS World installation found</h2>
    <p class="muted">Hangar looks in the registry under Eagle Dynamics. Is DCS World (standalone) installed on this PC?</p>
  </section>
{:else}
  <section class="hero main">
    <div class="info">
      <div class="chips">
        <span class="chip ok"><span class="dot"></span>Installed · Standalone</span>
        {#if install.lang}<span class="chip mono">lang: {install.lang}</span>{/if}
        {#if install.launch}<span class="chip mono">{install.launch}</span>{/if}
      </div>
      <div>
        <h2 class="title">DCS World</h2>
        <div class="mono muted sub">
          v{install.version ?? "?"} · {install.path}
          {#if install.version}
            <button class="whatsnew" onclick={() => api.openEdPage("version", install.version)} title="Opens ED's changelog for this version in your browser">What's new in this version<Icon name="external" size={13} /></button>
          {/if}
        </div>
      </div>
      {#if install.cfgError}
        <div class="notice bad"><Icon name="warn" size={16} color="var(--warning)" />{install.cfgError}</div>
      {/if}
      <div class="actions">
        <button class="btn primary big" onclick={() => start("update")} disabled={running || !install.updaterExists}>
          <Icon name="update" />{running ? "Updater running…" : "Update"}
        </button>
        {#if !confirmRepair}
          <button class="btn big" onclick={() => (confirmRepair = true)} disabled={running || !install.updaterExists}><Icon name="repair" />Repair</button>
        {:else}
          <div class="confirm">
            <span>Repair checks every file and restores the official ones. Mods inside the install folder will be overwritten.</span>
            <button class="btn small" onclick={() => { confirmRepair = false; void start("repair"); }}>Start repair</button>
            <button class="btn small" onclick={() => (confirmRepair = false)}>Cancel</button>
          </div>
        {/if}
      </div>
      {#if hangar.settings}
        <div class="launch">
          <div class="split">
            <button class="btn big" onclick={launch} disabled={running || launching || !preset}><Icon name="play" />Launch DCS</button>
            <span class="sep" aria-hidden="true"></span>
            <label class="picker">
              <span class="sr">Launch preset</span>
              <select bind:value={preset}>
                {#each hangar.settings.launchPresets as p}<option value={p.id}>{p.name}</option>{/each}
              </select>
              <Icon name="chevron" size={16} color="var(--ink-2)" />
            </label>
          </div>
          {#if dcsRunningHint}<span class="faint small">Starting DCS…</span>{/if}
        </div>
      {/if}
      {#if actionError}<div class="notice bad"><Icon name="warn" size={16} color="var(--warning)" />{actionError}</div>{/if}
    </div>
    {#if picture}
      <div class="wall" style="background-image: url({picture.dataUrl})">
        <div class="wallbar">
          <span class="mono faint small" title={picture.path}>{picture.name}</span>
          <button class="btn small ghost" onclick={nextPicture} disabled={pictureLoading} aria-label="Next picture"><Icon name="refresh" size={16} /></button>
        </div>
      </div>
    {:else}
      <div class="blueprint" aria-hidden="true">
        <svg viewBox="0 0 200 200" width="230" height="230">
          <path d="M100 12 C104 22 106 42 106 62 L106 150 L111 176 L100 171 L89 176 L94 150 L94 62 C94 42 96 22 100 12 Z"></path>
          <path d="M106 82 L172 126 L172 136 L106 122 Z"></path><path d="M94 82 L28 126 L28 136 L94 122 Z"></path>
          <path d="M106 150 L134 168 L134 176 L106 166 Z"></path><path d="M94 150 L66 168 L66 176 L94 166 Z"></path>
        </svg>
      </div>
    {/if}
  </section>

  <section class="tiles">
    <div class="card tile">
      <div class="label">Free space</div>
      <div class="big mono">{gb(install.diskFree, 0)}</div>
      {#if usedPct != null}<div class="bar"><span style="width: {usedPct}%; background: var(--ink-2)"></span></div>{/if}
      <div class="faint small">of {gb(install.diskTotal, 0)}</div>
    </div>
    <div class="card tile">
      <div class="label">Modules</div>
      <div class="big mono">{aircraft.length}<span class="faint"> + {terrains.length} maps</span></div>
    </div>
    <div class="card tile">
      <div class="label">Last update</div>
      <div class="big mono">{lastUpdate ? lastUpdate.target : "—"}</div>
      <div class="faint small">{lastUpdate ? `${lastUpdate.startedUtc} UTC · ${duration(lastUpdate.durationS)}` : "No update in the updater's logs"}</div>
    </div>
    <a class="card tile link" href="/logs">
      <div class="label">dcs.log</div>
      <div class="big mono" style="color: {mineErrors ? 'var(--warning-soft)' : 'var(--ink)'}">{mineErrors}</div>
      <div class="faint small">{mineErrors === 1 ? "error from your mods" : "errors from your mods"}</div>
    </a>
  </section>

  <section class="card">
    <h2>Installed modules</h2>
    <div class="mods">
      {#each [...aircraft, ...terrains] as m}
        <div class="mod"><span class="tag mono">{isTerrain(m) ? "MAP" : "AC"}</span><span>{moduleName(m)}</span><span class="mono faint id">{m}</span></div>
      {/each}
    </div>
  </section>
{/if}

<style>
  .main { display: flex; flex-wrap: wrap; overflow: hidden; }
  .info { flex: 1 1 460px; padding: 26px; display: flex; flex-direction: column; gap: 16px; min-width: 0; }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .title { font-size: 42px; font-weight: 700; line-height: 1.05; }
  .sub { margin-top: 8px; font-size: 13px; overflow-wrap: anywhere; }
  .actions { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .big { min-height: 48px; font-size: 15px; }
  .confirm { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 10px 12px; border: 1px solid #4a2a2d; border-radius: 8px; background: var(--warning-bg); font-size: 13px; max-width: 640px; }
  .notice { display: flex; gap: 8px; align-items: center; font-size: 13px; }
  .notice.bad { color: var(--warning-soft); }
  .launch { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  /* Launch + preset as one split button (B-024, design canvas 04) */
  .split { display: flex; border: 1px solid var(--line-2); border-radius: 9px; overflow: hidden; background: var(--raised); }
  .split .btn { border: 0; border-radius: 0; }
  .split .sep { width: 1px; background: var(--line-2); }
  .picker { position: relative; display: flex; align-items: center; }
  .picker select { appearance: none; -webkit-appearance: none; min-height: 48px; padding: 0 44px 0 16px; border: 0; background: transparent; color: var(--ink); font: inherit; font-size: 15px; font-weight: 500; cursor: pointer; }
  .picker select:hover { background: #212a2e; }
  .picker select:focus-visible { outline: 2px solid var(--phosphor); outline-offset: -2px; }
  .picker select option { background: var(--panel); }
  .picker :global(svg) { position: absolute; right: 16px; pointer-events: none; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  .whatsnew { display: inline-flex; align-items: center; gap: 5px; margin-left: 12px; padding: 0; border: 0; background: none; color: var(--phosphor-soft); font: 13px var(--font-ui); cursor: pointer; }
  .whatsnew:hover { color: var(--phosphor); text-decoration: underline; }
  .wall { flex: 1 1 380px; min-height: 300px; position: relative; background-size: cover; background-position: center; border-left: 1px solid #222a2f; }
  .wallbar { position: absolute; left: 0; right: 0; bottom: 0; display: flex; justify-content: space-between; align-items: center; gap: 10px; padding: 8px 10px 8px 14px; background: linear-gradient(transparent, rgba(8, 11, 12, 0.85)); }
  .wallbar span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink-soft); }
  .ghost { border-color: transparent; background: rgba(13, 16, 18, 0.55); }
  .blueprint { flex: 1 1 280px; min-height: 260px; display: flex; align-items: center; justify-content: center; border-left: 1px solid #222a2f; background-color: #12171a; background-image: linear-gradient(#192024 1px, transparent 1px), linear-gradient(90deg, #192024 1px, transparent 1px); background-size: 28px 28px; }
  .blueprint svg { fill: none; stroke: var(--phosphor); stroke-width: 1.2; stroke-linejoin: round; opacity: 0.85; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 14px; }
  .tile { display: flex; flex-direction: column; gap: 8px; }
  .tile .big { font-size: 24px; font-weight: 500; min-height: 0; }
  .tile .big .faint { font-size: 15px; }
  .tile.link { text-decoration: none; color: inherit; }
  .tile.link:hover { border-color: var(--line-2); }
  .small { font-size: 12px; }
  .mods { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 0 24px; margin-top: 10px; }
  .mod { display: flex; align-items: center; gap: 12px; padding: 9px 0; border-bottom: 1px solid #20282c; }
  .mod .tag { width: 34px; text-align: center; font-size: 10px; color: var(--ink-2); background: var(--raised); border-radius: 5px; padding: 4px 0; }
  .mod .id { margin-left: auto; font-size: 11px; }
  .empty { padding: 26px; }
</style>
