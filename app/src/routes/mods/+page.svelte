<!--
  Hangar - a community-made updater and launcher for DCS World.
  Copyright (C) 2026 ChonkieMonkie
  SPDX-License-Identifier: GPL-3.0-or-later
  Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.
-->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Icon from "$lib/components/Icon.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { api, type ModsOverview, type ModView, type Settings } from "$lib/api";
  import { bytes, when } from "$lib/format";
  import { hangar, loadSettings, saveSettings } from "$lib/state.svelte";

  let ov = $state<ModsOverview | null>(null);
  let error = $state<string | null>(null);
  let message = $state<string | null>(null);
  let query = $state("");
  let selId = $state<string | null>(null);
  let confirm = $state<"remove" | null>(null);
  let changed = $state<{ id: string; text: string } | null>(null);
  let newSetName = $state<string | null>(null);
  let editSet = $state<string | null>(null);
  let showFound = $state(false);

  async function load() {
    try {
      ov = await api.modsOverview();
      error = null;
    } catch (e) {
      error = String(e);
    }
  }
  $effect(() => {
    void load();
  });

  let mods = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return (ov?.mods ?? []).filter((m) => !q || `${m.name} ${m.kind} ${m.root}`.toLowerCase().includes(q));
  });
  let sel = $derived(ov?.mods.find((m) => m.id === selId) ?? mods[0] ?? null);
  let active = $derived(ov?.mods.filter((m) => m.enabled).length ?? 0);
  let conflicts = $derived(ov?.mods.filter((m) => m.enabled && m.conflicts.length).length ?? 0);
  let locked = $derived(!!ov?.dcsRunning || !!hangar.job);
  let currentOn = $derived((ov?.mods ?? []).filter((m) => m.enabled).map((m) => m.id).sort());
  let matchingSet = $derived(ov?.sets.find((s) => [...s.enabled].sort().join() === currentOn.join()) ?? null);

  async function act(f: () => Promise<unknown>, done?: string) {
    error = null;
    message = null;
    try {
      await f();
      message = done ?? null;
    } catch (e) {
      error = String(e);
    } finally {
      await load();
    }
  }

  async function toggle(m: ModView, on: boolean) {
    error = null;
    changed = null;
    try {
      await api.modSetEnabled(m.id, on);
    } catch (e) {
      const t = String(e);
      if (t.includes("CHANGED:")) changed = { id: m.id, text: t.split("CHANGED:")[1] };
      else error = t;
    }
    await load();
  }

  async function addFrom(kind: "zip" | "folder") {
    const p = kind === "zip"
      ? await open({ title: "Choose a mod (.zip)", filters: [{ name: "Zip", extensions: ["zip"] }] })
      : await open({ directory: true, title: "Choose a mod folder" });
    if (typeof p === "string") await act(() => api.modAdd(p), "Mod installed and switched on.");
  }

  async function settingsCopy(): Promise<Settings> {
    return structuredClone($state.snapshot(hangar.settings ?? (await loadSettings()))) as Settings;
  }
  async function saveSet(name: string) {
    const s = await settingsCopy();
    const id = "set-" + Date.now().toString(36);
    s.modSets.push({ id, name: name.trim() || "My set", enabled: currentOn });
    await saveSettings(s);
    newSetName = null;
    await load();
  }
  async function updateSet(id: string, fn: (s: Settings) => void) {
    const s = await settingsCopy();
    fn(s);
    await saveSettings(s);
    await load();
  }
  async function applySet(id: string) {
    await act(async () => {
      const problems = await api.modApplySet(id);
      if (problems.length) throw new Error(problems.join("\n"));
    }, "Set applied.");
  }
</script>

<header class="top">
  <div>
    <div class="bay">HANGAR {hangar.version}</div>
    <h1>Mods</h1>
    <div class="faint small">{active} active · {(ov?.mods.length ?? 0) - active} off{conflicts ? ` · ${conflicts} with conflicts` : ""}</div>
  </div>
  <div class="actions">
    <input type="search" placeholder="Search mods" bind:value={query} aria-label="Search mods" />
    <button class="btn" disabled={locked} onclick={() => addFrom("folder")}><Icon name="folder" size={16} />Add folder…</button>
    <button class="btn primary" disabled={locked} onclick={() => addFrom("zip")}><Icon name="plus" size={16} />Add zip…</button>
  </div>
</header>

{#if ov?.dcsRunning}<div class="notice warn"><Icon name="warn" size={16} color="var(--caution)" />DCS is running. Close it to switch mods.</div>{/if}
{#if error}<div class="notice bad"><Icon name="warn" size={16} color="var(--warning)" /><span class="pre">{error}</span></div>{/if}
{#if message}<div class="notice ok"><Icon name="check" size={16} color="var(--phosphor)" />{message}</div>{/if}
{#if changed}
  <div class="confirm">
    <span>{changed.text}. Switching off keeps your edited version in the vault.</span>
    <button class="btn small" onclick={() => { const id = changed!.id; changed = null; void act(() => api.modSetEnabled(id, false, true)); }}>Switch off anyway</button>
    <button class="btn small" onclick={() => (changed = null)}>Cancel</button>
  </div>
{/if}

{#if ov && !ov.savedGames}
  <section class="card muted">Saved Games\DCS was not found.</section>
{:else if ov}
  <section class="card sets">
    <div class="label">Mod sets</div>
    <div class="chips">
      {#each ov.sets as st (st.id)}
        {#if editSet === st.id}
          <span class="setedit">
            <input value={st.name} aria-label="Set name" onchange={(e) => updateSet(st.id, (s) => { const x = s.modSets.find((y) => y.id === st.id); if (x) x.name = e.currentTarget.value; })} />
            <button class="btn small" onclick={() => updateSet(st.id, (s) => { const x = s.modSets.find((y) => y.id === st.id); if (x) x.enabled = currentOn; })}>Save current mix</button>
            <button class="btn small danger" onclick={() => { editSet = null; void updateSet(st.id, (s) => (s.modSets = s.modSets.filter((y) => y.id !== st.id))); }}>Delete</button>
            <button class="btn small" onclick={() => (editSet = null)}>Done</button>
          </span>
        {:else}
          <span class="set" class:on={matchingSet?.id === st.id}>
            <button class="setname" disabled={locked} onclick={() => applySet(st.id)} title="Apply this set">{st.name}<span class="faint"> · {st.enabled.length}</span></button>
            <button class="setmore" onclick={() => (editSet = st.id)} aria-label={`Edit ${st.name}`}>⋯</button>
          </span>
        {/if}
      {/each}
      {#if newSetName !== null}
        <span class="setedit">
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={newSetName} placeholder="Name, e.g. Clean" aria-label="New set name" autofocus onkeydown={(e) => e.key === "Enter" && saveSet(newSetName ?? "")} />
          <button class="btn small primary" onclick={() => saveSet(newSetName ?? "")}>Save</button>
          <button class="btn small" onclick={() => (newSetName = null)}>Cancel</button>
        </span>
      {:else}
        <button class="btn small" onclick={() => (newSetName = "")}><Icon name="plus" size={14} />Save current mix as a set</button>
      {/if}
    </div>
  </section>

  <div class="split">
    <section class="card list">
      {#each mods as m (m.id)}
        <div class="row" class:sel={sel?.id === m.id}>
          <Switch checked={m.enabled} disabled={locked} label={`${m.name} on`} onchange={(v) => toggle(m, v)} />
          <button class="rowbtn" onclick={() => { selId = m.id; confirm = null; }}>
            <span class="mname">{m.name}{#if m.enabled && m.conflicts.length}<span class="chip warn tiny">conflict</span>{/if}</span>
            <span class="mono faint">{m.root}</span>
          </button>
          <span class="kind">{m.kind}</span>
          <span class="mono faint size">{bytes(m.bytes)}</span>
        </div>
      {:else}
        <div class="muted pad">{ov.mods.length ? "Nothing matches." : "No mods managed yet. Add one, or pick from what Hangar found in Saved Games below."}</div>
      {/each}
    </section>

    {#if sel}
      <aside class="hero detail">
        <div class="label">Selected</div>
        <h2>{sel.name}</h2>
        <div class="chips"><span class="chip">{sel.kind}</span><span class="chip" class:ok={sel.enabled}>{sel.enabled ? "On" : "Off, in the vault"}</span></div>
        <dl>
          <dt>Size</dt><dd>{bytes(sel.bytes)} · {sel.files} files</dd>
          <dt>When on</dt><dd class="mono">Saved Games\DCS\{sel.root.replaceAll("/", "\\")}</dd>
          <dt>When off</dt><dd class="mono">{sel.vault}</dd>
          <dt>Added</dt><dd>{when(sel.addedUnix)}</dd>
        </dl>
        {#if sel.conflicts.length}
          <div class="notice warn"><Icon name="warn" size={16} color="var(--caution)" />{sel.conflicts.length} file(s) shared with {sel.conflictsWith.join(", ")}. The mod switched on last wins.</div>
        {/if}
        <div class="label">Tracked files</div>
        <div class="tracked mono">
          {#each sel.sampleFiles as f}<div>{f}</div>{/each}
          {#if sel.files > sel.sampleFiles.length}<div class="faint">+ {sel.files - sel.sampleFiles.length} more</div>{/if}
        </div>
        {#if confirm === "remove"}
          <div class="confirm">
            <span>Delete this mod's files from Saved Games and the vault?</span>
            <button class="btn small danger" onclick={() => { confirm = null; void act(() => api.modRemove(sel!.id), "Mod removed."); }}>Delete</button>
            <button class="btn small" onclick={() => (confirm = null)}>Cancel</button>
          </div>
        {:else}
          <div class="dactions">
            <button class="btn" onclick={() => api.revealPath(sel!.enabled ? `${ov!.savedGames}\\${sel!.root.replaceAll("/", "\\")}` : sel!.vault)}><Icon name="folder" size={16} />Open folder</button>
            {#if sel.enabled}<button class="btn" onclick={() => act(() => api.modForget(sel!.id), "Hangar no longer tracks this mod. Its files stay where they are.")}>Stop tracking</button>{/if}
            <button class="btn danger" disabled={locked} onclick={() => (confirm = "remove")}>Remove…</button>
          </div>
        {/if}
      </aside>
    {/if}
  </div>

  {#if ov.candidates.length}
    <section class="card">
      <button class="found" onclick={() => (showFound = !showFound)} aria-expanded={showFound}>
        <h2>Found in Saved Games</h2>
        <span class="faint small">{ov.candidates.length} not managed by Hangar · {showFound ? "hide" : "show"}</span>
      </button>
      {#if showFound}
        <div class="rows">
          {#each ov.candidates as c (c.root)}
            <div class="crow">
              <div class="cname"><span>{c.name}</span><span class="mono faint small">{c.root}</span></div>
              <span class="kind">{c.kind}</span>
              <span class="mono faint size">{bytes(c.bytes)}</span>
              <button class="btn small" disabled={!!hangar.job} onclick={() => act(() => api.modAdopt(c.root, c.name, c.kind), `Hangar now manages ${c.name}.`)}>Manage</button>
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}
{/if}

<style>
  .top { display: flex; justify-content: space-between; align-items: flex-end; gap: 16px; flex-wrap: wrap; }
  .actions, .dactions, .chips { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  input[type="search"] { min-height: 44px; width: 240px; padding: 0 12px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--inset); color: var(--ink); font: inherit; }
  .small { font-size: 12px; }
  .notice { display: flex; gap: 8px; align-items: flex-start; font-size: 13px; }
  .notice.bad { color: var(--warning-soft); }
  .notice.ok { color: var(--phosphor-soft); }
  .notice.warn { color: var(--caution); }
  .pre { white-space: pre-wrap; }
  .confirm { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 10px 12px; border: 1px solid #4a2a2d; border-radius: 8px; background: var(--warning-bg); font-size: 13px; }
  .sets { display: flex; flex-direction: column; gap: 10px; padding: 14px 18px; }
  .set { display: inline-flex; border: 1px solid var(--line-2); border-radius: 8px; overflow: hidden; }
  .set.on { border-color: var(--phosphor); background: var(--phosphor-bg); }
  .setname, .setmore { border: 0; background: transparent; cursor: pointer; min-height: 34px; padding: 0 12px; font-size: 13px; }
  .setmore { border-left: 1px solid var(--line-2); padding: 0 10px; color: var(--ink-2); }
  .setedit { display: inline-flex; gap: 6px; align-items: center; }
  .setedit input { min-height: 34px; padding: 0 10px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--inset); color: var(--ink); font: inherit; width: 180px; }
  .split { display: flex; flex-wrap: wrap; gap: 18px; align-items: flex-start; }
  .list { flex: 1 1 520px; min-width: 0; padding: 0; overflow: hidden; }
  .row { display: flex; align-items: center; gap: 14px; padding: 8px 18px; border-bottom: 1px solid #1e2529; }
  .row.sel { background: #1b2226; box-shadow: inset 3px 0 0 var(--phosphor); }
  .rowbtn { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; border: 0; background: transparent; text-align: left; cursor: pointer; padding: 6px 0; }
  .mname { display: flex; gap: 8px; align-items: center; font-weight: 500; }
  .rowbtn .mono { font-size: 11.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tiny { padding: 1px 7px; font-size: 11px; }
  .kind { width: 80px; flex: none; font-size: 12px; color: var(--ink-2); }
  .size { width: 70px; flex: none; text-align: right; font-size: 12px; }
  .pad { padding: 18px; }
  .detail { flex: 1 1 380px; min-width: 0; padding: 20px; display: flex; flex-direction: column; gap: 12px; }
  dl { display: grid; grid-template-columns: 90px minmax(0, 1fr); gap: 6px 12px; margin: 0; font-size: 13px; }
  dt { color: var(--ink-3); }
  dd { margin: 0; overflow-wrap: anywhere; }
  dd.mono { font-size: 12px; }
  .tracked { font-size: 11.5px; line-height: 1.7; color: var(--ink-2); background: var(--well); border: 1px solid #20282c; border-radius: 8px; padding: 8px 12px; overflow-wrap: anywhere; }
  .found { display: flex; justify-content: space-between; align-items: center; width: 100%; border: 0; background: transparent; cursor: pointer; padding: 0; text-align: left; }
  .rows { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; margin-top: 12px; }
  .crow { display: flex; align-items: center; gap: 14px; padding: 8px 12px; border-bottom: 1px solid #1e2529; }
  .cname { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .cname .mono { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
