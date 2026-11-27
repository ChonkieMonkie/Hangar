<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import { api, type DcsLog, type LogGroup } from "$lib/api";
  import { duration, gb } from "$lib/format";
  import { hangar, savedGamesDcs } from "$lib/state.svelte";

  let tab = $state<"dcs" | "updater">("dcs");
  let log = $state<DcsLog | null>(null);
  let level = $state<"errors" | "warn" | "all">("errors");
  let onlyMine = $state(false);
  let query = $state("");
  let selKey = $state<string | null>(null);
  let runIdx = $state(0);
  let wrap = $state(true);
  let reader = $state(false);
  let copied = $state(false);

  async function load() {
    const sg = savedGamesDcs();
    log = sg ? await api.dcsLog(sg) : null;
  }
  $effect(() => {
    if (hangar.report) void load();
  });

  const key = (g: LogGroup) => `${g.level}|${g.category}|${g.message}`;
  let groups = $derived.by(() => {
    if (!log) return [];
    const allowed = level === "errors" ? ["ERROR", "ALERT"] : level === "warn" ? ["ERROR", "ALERT", "WARNING"] : null;
    const q = query.trim().toLowerCase();
    return log.groups.filter((g) =>
      (!allowed || allowed.includes(g.level)) &&
      (!onlyMine || g.mine) &&
      (!q || `${g.category} ${g.message} ${g.source ?? ""}`.toLowerCase().includes(q)));
  });
  let sel = $derived(groups.find((g) => key(g) === selKey) ?? groups[0] ?? null);
  let selText = $derived(sel ? sel.sample.join("\n") + (sel.sampleTruncated ? "\n…" : "") : "");
  let run = $derived(hangar.history[runIdx] ?? null);
  let maxPhase = $derived(Math.max(1, ...(run?.phases.map((p) => p.seconds) ?? [1])));
  const levelColor = (l: string) => (l === "ERROR" || l === "ALERT" ? "var(--warning)" : l === "WARNING" ? "var(--caution)" : "var(--ink-2)");

  async function copySel() {
    if (!sel) return;
    await navigator.clipboard.writeText(selText);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  function onKey(e: KeyboardEvent) {
    if (tab !== "dcs") return;
    if (e.key === "Escape" && reader) {
      reader = false;
      return;
    }
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const i = sel ? groups.findIndex((g) => key(g) === key(sel!)) : -1;
      const next = groups[Math.min(groups.length - 1, Math.max(0, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) {
        selKey = key(next);
        e.preventDefault();
        document.getElementById("row-" + key(next))?.scrollIntoView({ block: "nearest" });
      }
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<header class="top">
  <div>
    <div class="bay">BAY 01 · LOGS</div>
    <h1>Logs</h1>
  </div>
  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={tab === "dcs"} class:on={tab === "dcs"} onclick={() => (tab = "dcs")}>dcs.log</button>
    <button role="tab" aria-selected={tab === "updater"} class:on={tab === "updater"} onclick={() => (tab = "updater")}>Updater</button>
  </div>
</header>

{#if tab === "dcs"}
  {#if !log?.exists}
    <section class="card muted">No dcs.log found in Saved Games.</section>
  {:else}
    <div class="toolbar">
      <input type="search" placeholder="Search" bind:value={query} aria-label="Search log" />
      <div class="seg">
        <button class:on={level === "errors"} onclick={() => (level = "errors")}>Errors</button>
        <button class:on={level === "warn"} onclick={() => (level = "warn")}>+ Warnings</button>
        <button class:on={level === "all"} onclick={() => (level = "all")}>Everything</button>
      </div>
      <label class="mine"><input type="checkbox" bind:checked={onlyMine} />Only from my mods</label>
      <span class="faint small">{log.lineCount} lines{log.dcsVersion ? ` · DCS ${log.dcsVersion}` : ""}</span>
    </div>

    <div class="split">
      <section class="card list">
        {#each groups as g}
          <button id={"row-" + key(g)} class="row" class:sel={sel && key(sel) === key(g)} onclick={() => (selKey = key(g))}>
            <span class="count mono">{g.count}×</span>
            <span class="lvl mono" style="color: {levelColor(g.level)}">{g.level}</span>
            <span class="msg">
              <span class="mono text2">{g.category}: {g.message}</span>
              <span class="src" class:mine={g.mine}>{g.mine ? g.source : "DCS"}</span>
            </span>
          </button>
        {:else}
          <div class="muted pad">Nothing matches.</div>
        {/each}
      </section>

      {#if sel}
        <aside class="card detail sticky">
          <div class="dhead">
            <div class="label">{sel.level} · {sel.count}× · {sel.firstTime.slice(11, 19)}–{sel.lastTime.slice(11, 19)}</div>
            <div class="dtools">
              <button class="btn small" class:on={wrap} aria-pressed={wrap} onclick={() => (wrap = !wrap)}>Wrap</button>
              <button class="btn small" onclick={copySel}>{copied ? "Copied" : "Copy"}</button>
              <button class="btn small" onclick={() => (reader = true)}>Expand</button>
            </div>
          </div>
          <h2>{sel.category}</h2>
          <p class="fullmsg mono">{sel.message}</p>
          <pre class="mono" class:wrap>{selText}</pre>
          {#if sel.mine}
            <div class="hint"><Icon name="info" size={16} color="var(--sky)" />From your script <span class="mono">{sel.source}</span>. Turning that mod off will tell you if it's the cause.</div>
          {:else}
            <div class="hint faint"><Icon name="info" size={16} />Logged by DCS itself, not by your mods.</div>
          {/if}
        </aside>
      {/if}
    </div>

    {#if reader && sel}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="reader" role="dialog" aria-modal="true" aria-label="Log message" tabindex="-1"
        onclick={(e) => { if (e.target === e.currentTarget) reader = false; }}>
        <div class="readerbox card">
          <div class="dhead">
            <div>
              <div class="label">{sel.level} · {sel.count}× · {sel.mine ? sel.source : "DCS"}</div>
              <h2>{sel.category}</h2>
            </div>
            <div class="dtools">
              <button class="btn small" class:on={wrap} aria-pressed={wrap} onclick={() => (wrap = !wrap)}>Wrap</button>
              <button class="btn small" onclick={copySel}>{copied ? "Copied" : "Copy"}</button>
              <button class="btn small" onclick={() => (reader = false)}>Close</button>
            </div>
          </div>
          <pre class="mono big" class:wrap>{selText}</pre>
        </div>
      </div>
    {/if}
  {/if}
{:else}
  {#if !hangar.history.length}
    <section class="card muted">No updater runs in autoupdate_log.txt.</section>
  {:else}
    <div class="split">
      <section class="card list">
        {#each hangar.history as r, i}
          <button class="row" class:sel={i === runIdx} onclick={() => (runIdx = i)}>
            <Icon name={r.result === "failed" ? "cross" : r.result === "success" ? "check" : "info"} size={16}
              color={r.result === "failed" ? "var(--warning)" : r.result === "success" ? "var(--phosphor)" : "var(--ink-2)"} />
            <span class="mono when">{r.startedUtc}</span>
            <span class="msg"><span>{r.kind}{r.target ? ` ${r.target}` : ""}</span><span class="src">{r.toDownload ? gb(r.toDownload, 2) + " unpacked" : r.resultText ?? ""}</span></span>
            <span class="mono faint">{duration(r.durationS)}</span>
          </button>
        {/each}
      </section>
      {#if run}
        <aside class="hero detail">
          <div class="label">Phases</div>
          <h2>{run.kind}{run.target ? ` ${run.target}` : ""}</h2>
          <div class="phases">
            {#each run.phases as p}
              <div class="ph">
                <span>{p.name}</span>
                <span class="pbar"><span style="width: {Math.max(2, (Math.log(1 + p.seconds) / Math.log(1 + maxPhase)) * 100)}%; background: {p.name.startsWith('Downloading update') ? 'var(--phosphor)' : 'var(--sky)'}"></span></span>
                <span class="mono faint">{duration(p.seconds)}</span>
              </div>
            {/each}
          </div>
        </aside>
      {/if}
    </div>
  {/if}
{/if}

<style>
  .top { display: flex; justify-content: space-between; align-items: flex-end; gap: 16px; flex-wrap: wrap; }
  .tabs, .seg { display: flex; gap: 4px; padding: 4px; border-radius: 10px; background: var(--inset); border: 1px solid #222a2f; }
  .tabs button, .seg button { min-height: 38px; padding: 0 14px; border: 0; border-radius: 7px; background: transparent; color: var(--ink-2); cursor: pointer; font-weight: 500; }
  .tabs button.on, .seg button.on { background: var(--line); color: var(--ink); }
  .toolbar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  input[type="search"] { flex: 1 1 240px; min-height: 44px; padding: 0 12px; border: 1px solid var(--line-2); border-radius: 8px; background: var(--inset); color: var(--ink); font: inherit; }
  .mine { display: flex; align-items: center; gap: 8px; font-size: 13px; }
  .small { font-size: 12px; }
  .split { display: flex; flex-wrap: wrap; gap: 18px; align-items: flex-start; }
  .list { flex: 1 1 440px; min-width: 0; padding: 0; overflow: hidden; }
  .row { width: 100%; display: flex; align-items: center; gap: 14px; min-height: 56px; padding: 10px 18px; border: 0; border-bottom: 1px solid #1e2529; background: transparent; text-align: left; cursor: pointer; }
  .row:hover { background: #181e21; }
  .row.sel { background: #1b2226; box-shadow: inset 3px 0 0 var(--phosphor); }
  .count { width: 52px; text-align: right; flex: none; }
  .lvl { width: 64px; flex: none; font-size: 12px; }
  .when { width: 150px; flex: none; font-size: 12px; }
  .msg { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .msg > span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; }
  .msg > span.text2:first-child { white-space: normal; text-overflow: clip; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; word-break: break-word; line-height: 1.45; }
  .src { font-size: 12px; color: var(--ink-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .src.mine { color: var(--warning-soft); }
  .pad { padding: 18px; }
  .detail { flex: 1.3 1 460px; min-width: 0; padding: 20px; display: flex; flex-direction: column; gap: 12px; }
  .sticky { position: sticky; top: 16px; max-height: calc(100vh - 40px); overflow-y: auto; }
  .dhead { display: flex; justify-content: space-between; align-items: center; gap: 10px; flex-wrap: wrap; }
  .dtools { display: flex; gap: 6px; }
  .dtools .btn.on { border-color: var(--phosphor); color: var(--phosphor-soft); }
  .fullmsg { margin: 0; font-size: 13.5px; line-height: 1.6; color: var(--ink); white-space: pre-wrap; word-break: break-word; user-select: text; }
  pre { margin: 0; background: var(--well); border: 1px solid #20282c; border-radius: 8px; padding: 12px 14px; font-size: 12.5px; line-height: 1.6; color: var(--ink-soft); overflow-x: auto; white-space: pre; user-select: text; }
  pre.wrap { white-space: pre-wrap; word-break: break-word; overflow-x: hidden; }
  pre.big { font-size: 14px; flex: 1; min-height: 0; overflow: auto; }
  .hint { display: flex; gap: 8px; align-items: flex-start; font-size: 13px; line-height: 1.5; }
  .reader { position: fixed; inset: 0; z-index: 50; background: rgba(5, 8, 9, 0.72); display: flex; align-items: center; justify-content: center; padding: 32px; }
  .readerbox { width: min(1200px, 100%); height: min(860px, 100%); display: flex; flex-direction: column; gap: 14px; }
  .phases { display: flex; flex-direction: column; gap: 8px; }
  .ph { display: grid; grid-template-columns: 170px minmax(0, 1fr) 90px; gap: 10px; align-items: center; font-size: 12.5px; }
  .ph .mono { text-align: right; }
  .pbar { height: 8px; border-radius: 4px; background: var(--well); overflow: hidden; }
  .pbar > span { display: block; height: 100%; border-radius: 4px; }
</style>
