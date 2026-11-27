<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Icon from "$lib/components/Icon.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { api, type BackupOverview, type FileRow, type Settings, type SnapshotInfo, type TopFolder } from "$lib/api";
  import { bytes, when } from "$lib/format";
  import { hangar, loadSettings, saveSettings } from "$lib/state.svelte";

  let ov = $state<BackupOverview | null>(null);
  let error = $state<string | null>(null);
  let message = $state<string | null>(null);
  let selId = $state<string | null>(null);
  let restoreScope = $state<"config" | "all" | "files">("config");
  let confirm = $state<"restore" | "delete" | null>(null);
  let files = $state<FileRow[] | null>(null);
  let fileQuery = $state("");
  let picked = $state<Set<string>>(new Set());
  let tops = $state<Record<string, TopFolder[]>>({});
  let openSource = $state<string | null>(null);

  async function load() {
    try {
      ov = await api.backupOverview();
      error = null;
    } catch (e) {
      error = String(e);
    }
  }
  $effect(() => {
    void load();
  });
  // reload when a job finishes
  let lastJob: string | null = null;
  $effect(() => {
    const j = hangar.job?.job ?? null;
    if (lastJob && !j) void load();
    lastJob = j;
  });

  let sel = $derived(ov?.snapshots.find((x) => x.id === selId) ?? ov?.snapshots[0] ?? null);
  let lastVerified = $derived(ov?.snapshots.find((x) => x.verified?.ok) ?? null);
  let busy = $derived(!!hangar.job || !!ov?.busy);

  async function settingsCopy(): Promise<Settings> {
    const s = hangar.settings ?? (await loadSettings());
    return structuredClone($state.snapshot(s)) as Settings;
  }
  async function updateSettings(fn: (s: Settings) => void) {
    const s = await settingsCopy();
    fn(s);
    await saveSettings(s);
    await load();
  }

  async function run<T>(label: string, f: () => Promise<T>, done?: (r: T) => string) {
    error = null;
    message = null;
    try {
      const r = await f();
      message = done ? done(r) : null;
    } catch (e) {
      error = String(e);
    } finally {
      confirm = null;
      await load();
    }
  }

  async function chooseTarget() {
    const dir = await open({ directory: true, title: "Where should backups go?" });
    if (typeof dir === "string") await updateSettings((s) => (s.backupTarget = dir));
  }
  async function addSource() {
    const dir = await open({ directory: true, title: "Add a folder to back up" });
    if (typeof dir !== "string") return;
    const name = dir.split(/[\\/]/).filter(Boolean).pop() ?? "Folder";
    await updateSettings((s) => {
      if (!s.backupSources.some((b) => b.path.toLowerCase() === dir.toLowerCase()))
        s.backupSources.push({ key: "extra-" + name.toLowerCase().replace(/[^a-z0-9]+/g, "-"), path: dir, enabled: true });
    });
  }
  async function toggleTop(key: string, folder: string, include: boolean) {
    const id = `${key}/${folder}`;
    await updateSettings((s) => {
      s.backupExcluded = s.backupExcluded.filter((x) => x.toLowerCase() !== id.toLowerCase());
      if (!include) s.backupExcluded.push(id);
    });
  }
  async function expand(key: string, path: string) {
    openSource = openSource === key ? null : key;
    if (openSource && !tops[key]) tops[key] = await api.backupTopFolders(path).catch(() => []);
  }
  const excluded = (key: string, folder: string) => !!ov?.excluded.some((x) => x.toLowerCase() === `${key}/${folder}`.toLowerCase());

  async function loadFiles() {
    if (!sel) return;
    files = await api.backupFiles(sel.id);
    picked = new Set();
  }
  let shownFiles = $derived.by(() => {
    if (!files) return [];
    const q = fileQuery.trim().toLowerCase();
    return (q ? files.filter((f) => `${f.source}/${f.path}`.toLowerCase().includes(q)) : files).slice(0, 400);
  });
  function togglePick(f: FileRow) {
    const k = `${f.source}\u0000${f.path}`;
    const n = new Set(picked);
    if (n.has(k)) n.delete(k);
    else n.add(k);
    picked = n;
  }

  async function doRestore() {
    if (!sel) return;
    const id = sel.id;
    const list = restoreScope === "files" ? [...picked].map((k) => k.split("\u0000") as [string, string]) : null;
    await run("Restoring", () => api.backupRestore(id, restoreScope === "config" ? "config" : "all", list), (r) =>
      r.failed.length ? `Restored ${r.restored} files, ${r.failed.length} failed: ${r.failed[0]}` : `Restored ${r.restored} files. The files you had before are in the snapshot "Before restore".`);
  }

  const check = (s: SnapshotInfo) => (s.verified ? (s.verified.ok ? "Verified" : "Damaged") : "Not checked");
</script>

<header class="top">
  <div>
    <div class="bay">BAY 01 · BACKUPS</div>
    <h1>Backups</h1>
  </div>
  <div class="actions">
    <button class="btn" onclick={() => run("Backing up", () => api.backupNow("config"), () => "Controls and options backed up.")} disabled={busy || !ov?.target}>Controls &amp; options only</button>
    <button class="btn primary" onclick={() => run("Backing up", () => api.backupNow("all"), (s) => `Backed up ${s.fileCount} files, ${bytes(s.addedBytes)} new.`)} disabled={busy || !ov?.target}>
      <Icon name="backups" />Back up now
    </button>
  </div>
</header>

{#if error}<div class="notice bad"><Icon name="warn" size={16} color="var(--warning)" />{error}</div>{/if}
{#if message}<div class="notice ok"><Icon name="check" size={16} color="var(--phosphor)" />{message}</div>{/if}

{#if ov}
  {#if !ov.target}
    <section class="hero setup">
      <h2>Choose where backups go</h2>
      <p class="muted">Pick a folder, ideally on another drive than Saved Games.</p>
      <div><button class="btn primary" onclick={chooseTarget}><Icon name="folder" />Choose folder…</button></div>
    </section>
  {:else}
    <section class="tiles">
      <div class="card tile">
        <div class="label">Status</div>
        {#if lastVerified}
          <div class="big ok"><Icon name="shield" color="var(--phosphor)" />Protected</div>
          <div class="faint small">Last verified snapshot {when(lastVerified.createdUnix)}</div>
        {:else if ov.snapshots.length}
          <div class="big">Not verified</div>
          <div class="faint small">Run "Verify now" on a snapshot</div>
        {:else}
          <div class="big warn">No backup yet</div>
          <div class="faint small">Press "Back up now"</div>
        {/if}
      </div>
      <div class="card tile">
        <div class="label">Destination</div>
        <div class="big mono">{bytes(ov.storeBytes)}</div>
        <div class="faint small">{ov.snapshots.length} snapshots · {bytes(ov.targetFree)} free</div>
      </div>
      <div class="card tile">
        <div class="label">Automatic</div>
        <div class="row-sw">
          <Switch checked={ov.beforeUpdate} label="Back up controls and options before every update" onchange={(v) => updateSettings((s) => (s.backupBeforeUpdate = v))} />
          <span>Before every update</span>
        </div>
        <div class="faint small">Controls &amp; options, when Hangar starts the update</div>
      </div>
    </section>

    {#if ov.sameDisk}
      <div class="notice warn"><Icon name="warn" size={16} color="var(--caution)" />The destination is on the same drive as Saved Games. It won't survive a disk failure.</div>
    {/if}

    <div class="split">
      <section class="card list">
        <div class="lhead"><h2>Snapshots</h2><span class="faint small">Keeps the last 10, one per month for 6 months, and everything pinned</span></div>
        {#each ov.snapshots as s (s.id)}
          <button class="row" class:sel={sel?.id === s.id} onclick={() => { selId = s.id; files = null; confirm = null; }}>
            <span class="when">{when(s.createdUnix)}</span>
            <span class="reason">{s.reason}{s.scope === "config" ? " · controls" : ""}</span>
            <span class="mono faint">+{bytes(s.addedBytes)}</span>
            <span class="chk" class:ok={s.verified?.ok} class:bad={s.verified && !s.verified.ok}>{check(s)}</span>
            {#if s.pinned}<Icon name="pin" size={15} color="var(--sky)" />{/if}
          </button>
        {:else}
          <div class="muted pad">No snapshots yet.</div>
        {/each}
      </section>

      {#if sel}
        <aside class="hero detail">
          <div class="label">Selected snapshot</div>
          <h2>{when(sel.createdUnix)} · {sel.reason}</h2>
          <div class="muted small">{sel.fileCount} files · full size {bytes(sel.totalBytes)} · {sel.sources.join(", ")}{sel.skipped ? ` · ${sel.skipped} files could not be read` : ""}</div>
          {#if sel.verified && !sel.verified.ok}
            <div class="notice bad"><Icon name="warn" size={16} color="var(--warning)" />{sel.verified.errors.length} damaged or missing files. They won't be restored.</div>
          {/if}

          <div class="label">Restore</div>
          <div class="scopes">
            <label class:on={restoreScope === "config"}><input type="radio" bind:group={restoreScope} value="config" />Controls &amp; options</label>
            <label class:on={restoreScope === "all"}><input type="radio" bind:group={restoreScope} value="all" disabled={sel.scope === "config"} />Everything</label>
            <label class:on={restoreScope === "files"}><input type="radio" bind:group={restoreScope} value="files" onchange={loadFiles} />Choose files</label>
          </div>
          {#if restoreScope === "files"}
            <input type="search" placeholder="Search files" bind:value={fileQuery} aria-label="Search files" />
            <div class="files">
              {#each shownFiles as f}
                <label class="file"><input type="checkbox" checked={picked.has(`${f.source}\u0000${f.path}`)} onchange={() => togglePick(f)} /><span class="mono">{f.source}/{f.path}</span><span class="faint">{bytes(f.size)}</span></label>
              {:else}
                <div class="muted pad">{files ? "No files match." : "Loading…"}</div>
              {/each}
            </div>
          {/if}

          {#if confirm === "restore"}
            <div class="confirm">
              <span>Your current files are snapshotted first, so this can be undone. DCS must be closed.</span>
              <button class="btn small" onclick={doRestore}>Restore</button>
              <button class="btn small" onclick={() => (confirm = null)}>Cancel</button>
            </div>
          {:else if confirm === "delete"}
            <div class="confirm">
              <span>Delete this snapshot? Files only it contains are removed from the backup.</span>
              <button class="btn small danger" onclick={() => run("Deleting", () => api.backupDelete(sel!.id), (b) => `Deleted. ${bytes(b)} freed.`)}>Delete</button>
              <button class="btn small" onclick={() => (confirm = null)}>Cancel</button>
            </div>
          {:else}
            <div class="dactions">
              <button class="btn primary" disabled={busy || ov.dcsRunning || (restoreScope === "files" && picked.size === 0)} onclick={() => (confirm = "restore")}>Restore…</button>
              <button class="btn" disabled={busy} onclick={() => run("Verifying", () => api.backupVerify(sel!.id), (v) => v.ok ? `All ${v.checked} stored files check out.` : `${v.errors.length} problems found.`)}>Verify now</button>
              <button class="btn" onclick={() => run("", () => api.backupPin(sel!.id, !sel!.pinned))}><Icon name="pin" size={16} />{sel.pinned ? "Unpin" : "Pin"}</button>
              <button class="btn danger" disabled={busy || sel.pinned} onclick={() => (confirm = "delete")}>Delete…</button>
            </div>
            {#if ov.dcsRunning}<div class="faint small">Close DCS to restore.</div>{/if}
          {/if}
        </aside>
      {/if}
    </div>
  {/if}

  <section class="card">
    <div class="lhead">
      <h2>What to back up</h2>
      <div class="dactions">
        {#if ov.target}<button class="btn small" onclick={chooseTarget}><Icon name="folder" size={16} />Destination: <span class="mono">{ov.target}</span></button>{/if}
        <button class="btn small" onclick={addSource}><Icon name="plus" size={16} />Add folder…</button>
      </div>
    </div>
    <div class="rows">
      {#each ov.sources as src (src.key)}
        <div class="srow">
          <Switch checked={src.enabled} disabled={!src.exists} label={`Back up ${src.key}`}
            onchange={(v) => updateSettings((s) => { const b = s.backupSources.find((x) => x.key === src.key); if (b) b.enabled = v; })} />
          <div class="spath">
            <span>{src.key}</span>
            <span class="mono faint small">{src.exists ? src.path : `${src.path} (not found)`}</span>
          </div>
          {#if src.enabled && src.exists}
            <button class="btn small" onclick={() => expand(src.key, src.path)}>{openSource === src.key ? "Hide folders" : "Folders"}</button>
          {/if}
          {#if src.key.startsWith("extra-")}
            <button class="btn small danger" aria-label="Remove folder" onclick={() => updateSettings((s) => (s.backupSources = s.backupSources.filter((x) => x.key !== src.key)))}><Icon name="cross" size={16} /></button>
          {/if}
        </div>
        {#if openSource === src.key}
          <div class="tops">
            {#each tops[src.key] ?? [] as t}
              <label class="top-f" class:cache={t.cache}>
                <input type="checkbox" checked={!t.cache && !excluded(src.key, t.name)} disabled={t.cache} onchange={(e) => toggleTop(src.key, t.name, e.currentTarget.checked)} />
                <span>{t.name}</span><span class="mono faint">{bytes(t.bytes)}</span>
                {#if t.cache}<span class="faint small">cache, rebuilt by DCS</span>{/if}
              </label>
            {:else}
              <div class="muted pad">Counting…</div>
            {/each}
          </div>
        {/if}
      {/each}
    </div>
  </section>
{/if}

<style>
  .top { display: flex; justify-content: space-between; align-items: flex-end; gap: 16px; flex-wrap: wrap; }
  .actions, .dactions { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .notice { display: flex; gap: 8px; align-items: center; font-size: 13px; }
  .notice.bad { color: var(--warning-soft); }
  .notice.ok { color: var(--phosphor-soft); }
  .notice.warn { color: var(--caution); }
  .setup { padding: 26px; display: flex; flex-direction: column; gap: 10px; }
  .setup p { margin: 0; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 14px; }
  .tile { display: flex; flex-direction: column; gap: 8px; }
  .tile .big { display: flex; align-items: center; gap: 8px; font-size: 20px; font-weight: 600; }
  .big.ok { color: var(--phosphor-soft); }
  .big.warn { color: var(--caution); }
  .row-sw { display: flex; align-items: center; gap: 10px; }
  .small { font-size: 12px; }
  .split { display: flex; flex-wrap: wrap; gap: 18px; align-items: flex-start; }
  .list { flex: 1 1 460px; min-width: 0; padding: 0; overflow: hidden; }
  .lhead { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex-wrap: wrap; padding: 16px 18px; }
  .card:not(.list) .lhead { padding: 0 0 12px; }
  .row { width: 100%; display: grid; grid-template-columns: 140px minmax(0, 1fr) 80px 96px 18px; gap: 12px; align-items: center; min-height: 48px; padding: 8px 18px; border: 0; border-top: 1px solid #1e2529; background: transparent; text-align: left; cursor: pointer; font-size: 13px; }
  .row:hover { background: #181e21; }
  .row.sel { background: #1b2226; box-shadow: inset 3px 0 0 var(--phosphor); }
  .reason { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chk { font-size: 12px; color: var(--ink-3); }
  .chk.ok { color: var(--phosphor-soft); }
  .chk.bad { color: var(--warning-soft); }
  .pad { padding: 16px 18px; }
  .detail { flex: 1 1 420px; min-width: 0; padding: 20px; display: flex; flex-direction: column; gap: 12px; }
  .scopes { display: flex; gap: 6px; flex-wrap: wrap; }
  .scopes label { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border: 1px solid var(--line-2); border-radius: 8px; font-size: 13px; cursor: pointer; }
  .scopes label.on { border-color: var(--phosphor); background: var(--phosphor-bg); }
  input[type="search"] { min-height: 38px; padding: 0 10px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--inset); color: var(--ink); font: inherit; }
  .files { max-height: 260px; overflow-y: auto; border: 1px solid var(--line); border-radius: 8px; }
  .file { display: grid; grid-template-columns: 20px minmax(0, 1fr) auto; gap: 8px; align-items: center; padding: 6px 10px; font-size: 12px; border-bottom: 1px solid #1e2529; }
  .file .mono { overflow-wrap: anywhere; }
  .confirm { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 10px 12px; border: 1px solid #4a2a2d; border-radius: 8px; background: var(--warning-bg); font-size: 13px; }
  .rows { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .srow { display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-bottom: 1px solid #1e2529; }
  .spath { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .spath .mono { overflow-wrap: anywhere; }
  .tops { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 2px 16px; padding: 8px 12px 12px 70px; border-bottom: 1px solid #1e2529; background: var(--inset); }
  .top-f { display: flex; align-items: center; gap: 8px; font-size: 13px; min-height: 30px; }
  .top-f .mono { margin-left: auto; font-size: 12px; }
  .top-f.cache { opacity: 0.6; }
</style>
