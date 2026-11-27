<!--
  Hangar - a community-made updater and launcher for DCS World.
  Copyright (C) 2026 ChonkieMonkie
  SPDX-License-Identifier: GPL-3.0-or-later
  Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { getVersion } from "@tauri-apps/api/app";
  import Icon from "$lib/components/Icon.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { api, type FolderStats, type Settings, type WallThumb, type RecordingsOverview, type ExportSummary, type StorageOverview, type CompressionEstimate } from "$lib/api";
  import { bytes } from "$lib/format";
  import { hangar, loadSettings, saveSettings } from "$lib/state.svelte";

  let s = $state<Settings | null>(null);
  let stats = $state<FolderStats[]>([]);
  let thumbs = $state<WallThumb[]>([]);
  let error = $state<string | null>(null);
  let saved = $state(false);
  type Section = "wall" | "launch" | "storage" | "research" | "about";
  const SECTIONS: Section[] = ["wall", "launch", "storage", "research", "about"];
  let section = $state<Section>("wall");
  // /settings?section=about (the logo in the menu links here)
  $effect(() => {
    const q = page.url.searchParams.get("section") as Section | null;
    if (q && SECTIONS.includes(q)) section = q;
  });

  $effect(() => {
    if (!s) {
      if (hangar.settings) s = structuredClone($state.snapshot(hangar.settings));
      else void loadSettings().then((x) => (s = structuredClone(x)));
    }
  });

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  /** Saves right away (switches, buttons) or after typing stops (text fields). */
  function persist(delay = 0) {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      if (!s) return;
      try {
        const back = await saveSettings($state.snapshot(s) as Settings);
        s.wallFolders = back.wallFolders;
        error = null;
        saved = true;
        setTimeout(() => (saved = false), 1200);
        void refreshWall();
      } catch (e) {
        error = String(e);
      }
    }, delay);
  }

  async function refreshWall() {
    if (!s) return;
    const folders = $state.snapshot(s.wallFolders);
    const hdr = s.wallHdr !== "skip";
    stats = await api.wallStats(folders, s.wallSubfolders, hdr).catch(() => []);
    thumbs = await api.wallThumbs(folders, s.wallSubfolders, 8, hdr).catch(() => []);
  }
  let loadedOnce = false;
  $effect(() => {
    if (s && !loadedOnce) {
      loadedOnce = true;
      void refreshWall();
    }
  });

  async function addFolder() {
    const dir = await open({ directory: true, title: "Choose a picture folder" });
    if (!s || typeof dir !== "string") return;
    if (s.wallFolders.some((f) => f.path.toLowerCase() === dir.toLowerCase())) return;
    s.wallFolders.push({ path: dir, enabled: true });
    persist();
  }

  let total = $derived(stats.filter((x, i) => s?.wallFolders[i]?.enabled).reduce((a, b) => a + b.pictures, 0));
  let skipped = $derived(stats.reduce((a, b) => a + b.skipped, 0));
  let hdrCount = $derived(stats.filter((x, i) => s?.wallFolders[i]?.enabled).reduce((a, b) => a + (b.hdr ?? 0), 0));
  const statFor = (path: string) => stats.find((x) => x.path === path);

  const modes = [
    { id: "random", label: "Random at start", desc: "A new picture every time Hangar opens" },
    { id: "newest", label: "Newest", desc: "Your latest screenshot" },
    { id: "favorite", label: "One favorite", desc: "Click a picture below to pick it" },
  ] as const;

  // ---------------------------------------------------------------- ETA research
  let rec = $state<RecordingsOverview | null>(null);
  let exported = $state<ExportSummary | null>(null);
  let exporting = $state(false);
  let confirmDelete = $state(false);
  async function refreshRec() {
    rec = await api.recordingsOverview().catch(() => null);
  }
  $effect(() => {
    if (section === "research" && !rec) void refreshRec();
  });
  function stamp(st: string | null): string {
    if (!st) return "—";
    return `${st.slice(0, 4)}-${st.slice(4, 6)}-${st.slice(6, 8)} ${st.slice(9, 11)}:${st.slice(11, 13)} UTC`;
  }
  function setRecording(on: boolean) {
    if (!s) return;
    s.recordUpdates = on;
    persist();
    setTimeout(refreshRec, 300);
  }
  async function exportRecordings() {
    const today = new Date().toISOString().slice(0, 10);
    const path = await save({ title: "Save ETA recordings", defaultPath: `hangar-eta-recordings-${today}.zip`, filters: [{ name: "Zip file", extensions: ["zip"] }] });
    if (!path) return;
    exporting = true;
    error = null;
    try {
      exported = await api.recordingsExport(path);
    } catch (e) {
      error = String(e);
    } finally {
      exporting = false;
    }
  }
  async function deleteRecordings() {
    confirmDelete = false;
    exported = null;
    await api.recordingsDeleteAll().catch((e) => (error = String(e)));
    await refreshRec();
  }

  // ---------------------------------------------------------------- Storage (compression, ROADMAP #10)
  let storage = $state<StorageOverview | null>(null);
  let estimate = $state<CompressionEstimate | null>(null);
  let estimating = $state(false);
  let vaultResult = $state<string | null>(null);
  $effect(() => {
    if (section === "storage" && !storage) void api.storageOverview().then((x) => (storage = x)).catch(() => {});
  });
  const LEVELS = [
    { v: 1, label: "Fastest" },
    { v: 3, label: "Fast (recommended)" },
    { v: 9, label: "Balanced" },
    { v: 15, label: "Small" },
    { v: 19, label: "Smallest (slow)" },
  ];
  function setCompression(fn: (c: Settings["compression"]) => void, delay = 0) {
    if (!s) return;
    fn(s.compression);
    estimate = null;
    persist(delay);
  }
  async function runEstimate() {
    if (!s) return;
    estimating = true;
    estimate = await api.compressionEstimate($state.snapshot(s.compression) as Settings["compression"]).catch((e) => {
      error = String(e);
      return null;
    });
    estimating = false;
  }
  async function compressVaultNow() {
    vaultResult = null;
    try {
      const [before, after] = await api.vaultCompressAll();
      vaultResult = before ? `Compressed ${bytes(before)} to ${bytes(after)}.` : "Nothing left to compress.";
      storage = await api.storageOverview().catch(() => storage);
    } catch (e) {
      error = String(e);
    }
  }

  // ---------------------------------------------------------------- About
  let version = $state("");
  let shown = $state<"license" | "notices" | null>(null);
  let longText = $state("");
  onMount(() => {
    getVersion().then((v) => (version = v)).catch(() => {});
  });
  async function toggleText(which: "license" | "notices") {
    if (shown === which) {
      shown = null;
      return;
    }
    longText = which === "license" ? await api.licenseText() : await api.thirdPartyNotices();
    shown = which;
  }

  function addPreset() {
    if (!s) return;
    const id = "preset-" + Date.now().toString(36);
    s.launchPresets.push({ id, name: "New preset", args: [] });
    persist();
  }
  function setArgs(i: number, text: string) {
    if (!s) return;
    s.launchPresets[i].args = text.split(/\s+/).filter(Boolean);
    persist(600);
  }
</script>

<header class="top">
  <div>
    <div class="bay">HANGAR {hangar.version}</div>
    <h1>Settings</h1>
  </div>
  {#if saved}<span class="chip ok"><span class="dot"></span>Saved</span>{/if}
</header>

{#if error}<div class="notice bad"><Icon name="warn" size={16} color="var(--warning)" />{error}</div>{/if}

{#if s}
  <div class="layout">
    <nav class="sections" aria-label="Settings sections">
      <button class:on={section === "wall"} onclick={() => (section = "wall")}>Hangar wall</button>
      <button class:on={section === "launch"} onclick={() => (section = "launch")}>Launch presets</button>
      <button class:on={section === "storage"} onclick={() => (section = "storage")}>Storage</button>
      <button class:on={section === "research"} onclick={() => (section = "research")}>ETA research</button>
      <button class:on={section === "about"} onclick={() => (section = "about")}>About</button>
    </nav>

    <div class="content">
      {#if section === "wall"}
        <section class="card">
          <div class="head">
            <div>
              <h2>Hangar wall</h2>
              <p class="muted small">The picture on the Overview screen, from your own folders.</p>
            </div>
            <button class="btn" onclick={addFolder}><Icon name="plus" size={16} />Add folder…</button>
          </div>
          <div class="label">Picture folders</div>
          <div class="rows">
            {#each s.wallFolders as f, i (f.path)}
              {@const st = statFor(f.path)}
              <div class="frow">
                <Switch checked={f.enabled} label={`Use ${f.path}`} onchange={(v) => { f.enabled = v; persist(); }} />
                <div class="fpath">
                  <span class="mono">{f.path}</span>
                  <span class="faint small">{st ? (st.exists ? `${st.pictures} pictures${st.hdr ? ` · ${st.hdr} HDR` : ""}${st.skipped ? ` · ${st.skipped} HDR skipped` : ""}` : "Folder not found") : "…"}</span>
                </div>
                <button class="btn small" onclick={() => api.revealPath(f.path)} aria-label="Open folder"><Icon name="folder" size={16} /></button>
                <button class="btn small danger" onclick={() => { s!.wallFolders.splice(i, 1); persist(); }} aria-label="Remove folder"><Icon name="cross" size={16} /></button>
              </div>
            {:else}
              <div class="muted pad">No folders. Overview shows the blueprint jet instead.</div>
            {/each}
          </div>
          <label class="check"><input type="checkbox" bind:checked={s.wallSubfolders} onchange={() => persist()} />Include subfolders</label>
        </section>

        <section class="card">
          <div class="label">Which picture</div>
          <div class="modes">
            {#each modes as m}
              <button class="mode" class:on={s.wallMode === m.id} onclick={() => { s!.wallMode = m.id; persist(); }}>
                <span class="mname">{m.label}</span><span class="faint small">{m.desc}</span>
              </button>
            {/each}
          </div>
          <div class="faint small">{total} pictures · JPG, PNG, BMP, WebP, HDR (JPEG XR, HDR PNG){hdrCount ? ` · ${hdrCount} HDR, shown tone-mapped` : ""}{skipped ? ` · ${skipped} HDR skipped` : ""}</div>
          <div class="hdrrow">
            <div>
              <div class="mname">HDR screenshots</div>
              <div class="faint small">Game Bar and NVIDIA save HDR captures as JPEG XR. Hangar can't show HDR, so it converts them to normal brightness.</div>
            </div>
            <div class="seg" role="radiogroup" aria-label="HDR screenshots">
              <button role="radio" aria-checked={s.wallHdr !== "skip"} class:on={s.wallHdr !== "skip"} onclick={() => { s!.wallHdr = "tonemap"; persist(); }}>Tone-map</button>
              <button role="radio" aria-checked={s.wallHdr === "skip"} class:on={s.wallHdr === "skip"} onclick={() => { s!.wallHdr = "skip"; persist(); }}>Skip</button>
            </div>
          </div>
          {#if thumbs.length}
            <div class="thumbs">
              {#each thumbs as t}
                <button class="thumb" class:fav={s.wallMode === "favorite" && s.wallFavorite === t.path}
                  onclick={() => { s!.wallFavorite = t.path; s!.wallMode = "favorite"; persist(); }} title={t.path} aria-label="Use as favorite">
                  <img src={t.dataUrl} alt="" />
                </button>
              {/each}
            </div>
          {/if}
        </section>
      {:else if section === "storage"}
        <section class="card">
          <div>
            <h2>Storage</h2>
            <p class="muted small">How Hangar stores backups and switched-off mods. Changes apply to new backups and to mods switched off from now on. What is already stored stays readable either way.</p>
          </div>
          <div class="usage">
            <div><div class="label">Backups</div><div class="mono">{storage ? (storage.backupBytes == null ? "No destination" : bytes(storage.backupBytes)) : "…"}</div></div>
            <div><div class="label">Mod vault</div><div class="mono">{storage ? bytes(storage.vaultBytes) : "…"}</div></div>
          </div>
        </section>
        <section class="card">
          <div class="label">Compression</div>
          <div class="toggle">
            <Switch checked={s.compression.backups} label="Compress backups" onchange={(v) => setCompression((c) => (c.backups = v))} />
            <div>
              <div class="tname">Compress backups</div>
              <div class="faint small">On by default. Each file is compressed by itself, so single files can still be restored and checked.</div>
            </div>
          </div>
          <div class="toggle">
            <Switch checked={s.compression.vault} label="Compress switched-off mods" onchange={(v) => setCompression((c) => (c.vault = v))} />
            <div>
              <div class="tname">Compress switched-off mods</div>
              <div class="faint small">Off by default. Saves space in the vault, but switching a mod on takes longer because it has to be unpacked.</div>
            </div>
          </div>
          {#if s.compression.vault}
            <div class="formrow">
              <label for="vmax">Only mods up to</label>
              <input id="vmax" class="num" type="number" min="0" step="0.5" placeholder="any size" value={s.compression.vaultMaxGb ?? ""}
                oninput={(e) => setCompression((c) => (c.vaultMaxGb = e.currentTarget.value === "" ? null : Number(e.currentTarget.value)), 600)} />
              <span class="faint small">GB (leave empty for any size)</span>
              <button class="btn small" onclick={compressVaultNow} disabled={!!hangar.job}>Compress switched-off mods now</button>
              {#if vaultResult}<span class="small">{vaultResult}</span>{/if}
            </div>
          {/if}
        </section>
        <section class="card">
          <div class="label">Advanced</div>
          <div class="formgrid">
            <label for="algo">Algorithm</label>
            <select id="algo" value={s.compression.algorithm} onchange={(e) => setCompression((c) => (c.algorithm = e.currentTarget.value as "zstd" | "none"))}>
              <option value="zstd">Zstandard (zstd)</option>
              <option value="none">None (store files as they are)</option>
            </select>
            <label for="level">Effort</label>
            <div class="levelrow">
              <input id="level" type="range" min="1" max="19" value={s.compression.level} disabled={s.compression.algorithm === "none"}
                oninput={(e) => setCompression((c) => (c.level = Number(e.currentTarget.value)), 400)} />
              <span class="mono small">{s.compression.level} · {(LEVELS.filter((l) => l.v <= s!.compression.level).pop() ?? LEVELS[0]).label}</span>
            </div>
            <label for="threads">Threads</label>
            <select id="threads" value={String(s.compression.threads)} onchange={(e) => setCompression((c) => (c.threads = Number(e.currentTarget.value)))}>
              <option value="0">Automatic (up to 4)</option>
              {#each [1, 2, 4, 8] as n}<option value={String(n)}>{n}</option>{/each}
            </select>
            <label for="minsize">Smallest file to compress</label>
            <div class="levelrow">
              <input id="minsize" class="num" type="number" min="0" step="1" value={Math.round(s.compression.minSize / 1024)}
                oninput={(e) => setCompression((c) => (c.minSize = Math.max(0, Number(e.currentTarget.value)) * 1024), 600)} />
              <span class="faint small">KB</span>
            </div>
            <label for="skip">Already compressed (skipped)</label>
            <input id="skip" class="mono" value={s.compression.skipExtensions.join(", ")}
              onchange={(e) => setCompression((c) => (c.skipExtensions = e.currentTarget.value.split(/[\s,]+/).map((x) => x.replace(/^\./, "").toLowerCase()).filter(Boolean)))} />
          </div>
          <div class="formrow">
            <button class="btn small" onclick={runEstimate} disabled={estimating || s.compression.algorithm === "none"}>{estimating ? "Estimating…" : "Estimate with my files"}</button>
            {#if estimate && estimate.sampled}
              <span class="small">A sample of {bytes(estimate.sampled)} from your backup folders becomes about <b>{bytes(estimate.compressed)}</b> ({Math.round((1 - estimate.compressed / estimate.sampled) * 100)} % smaller) at about {bytes(estimate.sampled / Math.max(estimate.seconds, 0.001))}/s. This is an estimate.</span>
            {:else if estimate}
              <span class="faint small">No files to sample. Choose backup folders first.</span>
            {/if}
          </div>
        </section>
      {:else if section === "research"}
        <section class="card">
          <div>
            <h2>ETA research</h2>
            <p class="muted small">Help make Hangar's time estimate better. Hangar can record how your downloads and updates progress. You can then export the recordings as one file and send it to the developer yourself. Hangar never sends anything on its own.</p>
          </div>
          <div class="toggle">
            <Switch checked={s.recordUpdates} label="Record downloads and updates" onchange={(v) => setRecording(v)} />
            <div>
              <div class="tname">Record downloads and updates</div>
              <div class="faint small">Off by default. While it is on, every run of ED's updater on this PC is recorded.</div>
            </div>
          </div>
          <div class="recstat">
            {#if rec?.count}
              <span><b>{rec.count}</b> {rec.count === 1 ? "recording" : "recordings"} · {bytes(rec.bytes)}</span>
              <span class="faint small">{stamp(rec.oldest)} – {stamp(rec.newest)} · the oldest are removed above 50 recordings or 200 MB</span>
            {:else}
              <span class="muted">No recordings yet.</span>
            {/if}
          </div>
          <div class="actions">
            <button class="btn primary" onclick={exportRecordings} disabled={!rec?.count || exporting}><Icon name="external" size={16} />{exporting ? "Exporting…" : "Export for sharing…"}</button>
            <button class="btn" onclick={() => rec?.folder && api.revealPath(rec.folder)} disabled={!rec?.folder}><Icon name="folder" size={16} />Open folder</button>
            {#if !confirmDelete}
              <button class="btn danger" onclick={() => (confirmDelete = true)} disabled={!rec?.count}><Icon name="trash" size={16} />Delete all</button>
            {:else}
              <span class="confirm small">Delete all {rec?.count} recordings? This can't be undone.
                <button class="btn small danger" onclick={deleteRecordings}>Delete</button>
                <button class="btn small" onclick={() => (confirmDelete = false)}>Cancel</button>
              </span>
            {/if}
          </div>
          {#if exported}
            <div class="notice good"><Icon name="check" size={16} color="var(--phosphor)" />Saved {exported.recordings} {exported.recordings === 1 ? "recording" : "recordings"} ({bytes(exported.bytes)}).
              <button class="btn small" onclick={() => api.revealPath(exported!.path)}>Show file</button></div>
          {/if}
        </section>
        <section class="card">
          <div class="label">What the exported file contains</div>
          <div class="cols">
            <div>
              <div class="tname">Included</div>
              <ul class="small">
                <li>Time since the start of each run (no dates or clock times)</li>
                <li>The progress numbers from ED's updater window: downloaded, unpacked, speed, ED's estimate</li>
                <li>Network and disk use as changes during the run</li>
                <li>Hangar's own estimate, the phase and the number of updater processes</li>
                <li>A few updater log lines rebuilt from known patterns, such as "To download: … bytes"</li>
              </ul>
            </div>
            <div>
              <div class="tname">Not included</div>
              <ul class="small">
                <li>Dates, clock times, folders and paths</li>
                <li>Your user name, computer name or account</li>
                <li>File names and module names, so nobody can see what you own</li>
                <li>The updater's window text and other log lines</li>
                <li>How much free space your disk has</li>
              </ul>
            </div>
          </div>
          <p class="faint small">Exporting does not change the recordings on this PC. The file is a normal zip, so you can open it and look before you send it.</p>
        </section>
      {:else if section === "about"}
        <section class="card about">
          <div class="abouthead">
            <svg viewBox="0 0 48 48" width="56" height="56" aria-hidden="true" class="logo">
              <path d="M7 15V7h8M33 7h8v8M41 33v8h-8M15 41H7v-8" style="stroke: var(--ink)"></path>
              <path d="M18 15v18M30 15v18"></path>
              <path d="M21 24h6" style="stroke-linecap: butt"></path>
            </svg>
            <div>
              <h2>Hangar {version}</h2>
              <div class="muted small">A community-made updater and launcher for DCS World</div>
              <div class="small">Copyright (C) 2026 ChonkieMonkie</div>
            </div>
          </div>
          <p class="small warranty">This program comes with ABSOLUTELY NO WARRANTY. This is free software, and you are welcome to redistribute it under the terms of the GNU General Public License, version 3 or (at your option) any later version.</p>
          <div class="actions">
            <button class="btn" class:active={shown === "license"} onclick={() => toggleText("license")}>View license</button>
            <button class="btn" class:active={shown === "notices"} onclick={() => toggleText("notices")}>Third-party licenses</button>
            <button class="btn" onclick={() => api.openProjectLink("source")}>Source code<Icon name="external" size={14} /></button>
            <button class="btn" onclick={() => api.openProjectLink("issues")}>Report an issue<Icon name="external" size={14} /></button>
          </div>
          {#if shown}
            <pre class="longtext" aria-label={shown === "license" ? "GNU General Public License" : "Third-party licenses"}>{longText}</pre>
          {/if}
          <p class="faint small">Not affiliated with or endorsed by Eagle Dynamics SA. THIS MATERIAL IS NOT MADE OR SUPPORTED BY EAGLE DYNAMICS SA. DCS World is a trademark of Eagle Dynamics SA.</p>
        </section>
      {:else}
        <section class="card">
          <div class="head">
            <div>
              <h2>Launch presets</h2>
              <p class="muted small">Command-line options for "Launch DCS" on Overview.</p>
            </div>
            <button class="btn" onclick={addPreset}><Icon name="plus" size={16} />New preset</button>
          </div>
          <div class="rows">
            {#each s.launchPresets as p, i (p.id)}
              <div class="prow">
                <input class="pname" value={p.name} aria-label="Preset name" oninput={(e) => { p.name = e.currentTarget.value; persist(600); }} />
                <input class="pargs mono" value={p.args.join(" ")} placeholder="no options" aria-label="Options" oninput={(e) => setArgs(i, e.currentTarget.value)} />
                <label class="check small"><input type="radio" name="default" checked={s.defaultPreset === p.id} onchange={() => { s!.defaultPreset = p.id; persist(); }} />Default</label>
                <button class="btn small danger" disabled={s.launchPresets.length <= 1} onclick={() => { s!.launchPresets.splice(i, 1); persist(); }} aria-label="Delete preset"><Icon name="cross" size={16} /></button>
              </div>
            {/each}
          </div>
          <p class="faint small">Known options: <span class="mono">--force_enable_VR</span>, <span class="mono">--force_disable_VR</span>, <span class="mono">--force_OpenXR</span>, <span class="mono">-w &lt;folder&gt;</span> (other Saved Games folder).</p>
        </section>
      {/if}
    </div>
  </div>
{/if}

<style>
  .top { display: flex; justify-content: space-between; align-items: flex-end; }
  .layout { display: flex; gap: 20px; align-items: flex-start; flex-wrap: wrap; }
  .sections { display: flex; flex-direction: column; gap: 2px; width: 200px; flex: none; }
  .sections button { text-align: left; min-height: 42px; padding: 0 12px; border: 0; border-radius: 8px; background: transparent; color: var(--ink-2); cursor: pointer; font-weight: 500; text-decoration: none; display: flex; align-items: center; }
  .sections .on { background: var(--raised); color: var(--ink); }
  .content { flex: 1 1 560px; min-width: 0; display: flex; flex-direction: column; gap: 16px; }
  .card { display: flex; flex-direction: column; gap: 14px; }
  .head { display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; flex-wrap: wrap; }
  .head p { margin: 6px 0 0; }
  .small { font-size: 12px; }
  .rows { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .frow, .prow { display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-bottom: 1px solid #1e2529; }
  .frow:last-child, .prow:last-child { border-bottom: 0; }
  .fpath { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .fpath .mono { font-size: 12.5px; overflow-wrap: anywhere; }
  .pad { padding: 14px; }
  .check { display: flex; gap: 8px; align-items: center; font-size: 13px; }
  .modes { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 8px; }
  .mode { display: flex; flex-direction: column; gap: 4px; align-items: flex-start; text-align: left; padding: 12px; border: 1px solid var(--line-2); border-radius: 8px; background: transparent; cursor: pointer; }
  .mode.on { border-color: var(--phosphor); background: var(--phosphor-bg); }
  .mname { font-weight: 600; }
  .thumbs { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 8px; }
  .thumb { padding: 0; border: 2px solid transparent; border-radius: 6px; overflow: hidden; background: var(--well); cursor: pointer; aspect-ratio: 16 / 10; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .thumb.fav { border-color: var(--phosphor); }
  input:not([type]), .pname, .pargs { min-height: 38px; padding: 0 10px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--inset); color: var(--ink); font: inherit; }
  .pname { width: 180px; }
  .pargs { flex: 1; min-width: 0; font-size: 12.5px; }
  .notice { display: flex; gap: 8px; align-items: center; font-size: 13px; }
  .notice.bad { color: var(--warning-soft); }
  .notice.good { color: var(--ink); flex-wrap: wrap; }
  .hdrrow { display: flex; gap: 16px; align-items: center; justify-content: space-between; flex-wrap: wrap; padding: 12px; border: 1px solid var(--line); border-radius: 8px; }
  .hdrrow > div:first-child { flex: 1 1 280px; display: flex; flex-direction: column; gap: 4px; }
  .seg { display: inline-flex; border: 1px solid var(--line-2); border-radius: 8px; overflow: hidden; }
  .seg button { min-height: 36px; padding: 0 14px; border: 0; background: var(--raised); color: var(--ink-2); font: inherit; font-weight: 500; cursor: pointer; }
  .seg button + button { border-left: 1px solid var(--line-2); }
  .seg button.on { background: var(--phosphor-bg); color: var(--phosphor-soft); }
  .card p { margin: 6px 0 0; line-height: 1.5; }
  .toggle { display: flex; gap: 14px; align-items: center; padding: 14px; border: 1px solid var(--line); border-radius: 8px; }
  .tname { font-weight: 600; }
  .recstat { display: flex; flex-direction: column; gap: 4px; }
  .actions { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .confirm { display: inline-flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 6px 10px; border: 1px solid #4a2a2d; border-radius: 8px; background: var(--warning-bg); }
  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 16px; }
  .cols ul { margin: 8px 0 0; padding-left: 18px; line-height: 1.6; color: var(--ink-soft); }
  .abouthead { display: flex; gap: 16px; align-items: center; }
  .abouthead h2 { font-size: 22px; }
  .abouthead > div { display: flex; flex-direction: column; gap: 4px; }
  .logo { fill: none; stroke: var(--phosphor); stroke-width: 3.4; stroke-linecap: round; stroke-linejoin: round; flex: none; }
  .warranty { max-width: 680px; }
  .btn.active { border-color: var(--phosphor); }
  .usage { display: flex; gap: 32px; flex-wrap: wrap; }
  .usage .mono { font-size: 20px; margin-top: 6px; }
  .formrow { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .formgrid { display: grid; grid-template-columns: 220px minmax(0, 1fr); gap: 10px 16px; align-items: center; }
  .formgrid select, .formgrid input:not([type="range"]), .num { min-height: 36px; padding: 0 10px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--inset); color: var(--ink); font: inherit; font-size: 13px; }
  .num { width: 110px; }
  .levelrow { display: flex; gap: 12px; align-items: center; }
  .levelrow input[type="range"] { flex: 1; max-width: 320px; accent-color: var(--phosphor); }
  .longtext { max-height: 420px; overflow: auto; margin: 0; padding: 14px; border: 1px solid var(--line); border-radius: 8px; background: var(--well); font: 12px/1.5 var(--font-mono); white-space: pre-wrap; color: var(--ink-soft); }
</style>
