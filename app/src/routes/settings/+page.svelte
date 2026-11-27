<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Icon from "$lib/components/Icon.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { api, type FolderStats, type Settings, type WallThumb } from "$lib/api";
  import { hangar, loadSettings, saveSettings } from "$lib/state.svelte";

  let s = $state<Settings | null>(null);
  let stats = $state<FolderStats[]>([]);
  let thumbs = $state<WallThumb[]>([]);
  let error = $state<string | null>(null);
  let saved = $state(false);
  let section = $state<"wall" | "launch">("wall");

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
    stats = await api.wallStats(folders, s.wallSubfolders).catch(() => []);
    thumbs = await api.wallThumbs(folders, s.wallSubfolders, 8).catch(() => []);
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
  const statFor = (path: string) => stats.find((x) => x.path === path);

  const modes = [
    { id: "random", label: "Random at start", desc: "A new picture every time Hangar opens" },
    { id: "newest", label: "Newest", desc: "Your latest screenshot" },
    { id: "favorite", label: "One favorite", desc: "Click a picture below to pick it" },
  ] as const;

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
    <div class="bay">BAY 01 · SETTINGS</div>
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
                  <span class="faint small">{st ? (st.exists ? `${st.pictures} pictures${st.skipped ? ` · ${st.skipped} HDR skipped` : ""}` : "Folder not found") : "…"}</span>
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
          <div class="faint small">{total} pictures · JPG, PNG, BMP, WebP{skipped ? ` · ${skipped} HDR (.jxr) skipped` : ""}</div>
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
</style>
