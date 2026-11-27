<script lang="ts">
  import "@fontsource/inter/400.css";
  import "@fontsource/inter/500.css";
  import "@fontsource/inter/600.css";
  import "@fontsource/chakra-petch/500.css";
  import "@fontsource/chakra-petch/600.css";
  import "@fontsource/chakra-petch/700.css";
  import "@fontsource/jetbrains-mono/400.css";
  import "@fontsource/jetbrains-mono/500.css";
  import "$lib/styles.css";

  import { onMount } from "svelte";
  import { page } from "$app/state";
  import Icon from "$lib/components/Icon.svelte";
  import JobBar from "$lib/components/JobBar.svelte";
  import { hangar, startHangar, currentInstall } from "$lib/state.svelte";
  import { api } from "$lib/api";

  let { children } = $props();

  onMount(() => {
    void startHangar();
    if (import.meta.env.DEV) return;
    // Release build: Hangar is an app, not a web page (ROADMAP #11). No browser context menu
    // (Back, Reload, Inspect) except in text fields, where cut/copy/paste is useful, and none of
    // the browser shortcuts that reload, print, search or navigate the page.
    const editable = (t: EventTarget | null) =>
      t instanceof HTMLElement && (t.isContentEditable || t.closest("input, textarea, select") !== null);
    const onMenu = (e: MouseEvent) => {
      if (!editable(e.target)) e.preventDefault();
    };
    const onKey = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      const ctrl = e.ctrlKey || e.metaKey;
      const blocked =
        k === "f5" || k === "f3" || k === "f7" || k === "browserback" || k === "browserforward" || k === "browserrefresh" ||
        (ctrl && ["r", "p", "f", "g", "u", "s", "j"].includes(k)) ||
        (e.altKey && (k === "arrowleft" || k === "arrowright"));
      if (blocked) e.preventDefault();
    };
    window.addEventListener("contextmenu", onMenu);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("contextmenu", onMenu);
      window.removeEventListener("keydown", onKey, true);
    };
  });

  const nav = [
    { href: "/", label: "Overview", icon: "overview" },
    { href: "/update", label: "Update", icon: "update" },
    { href: "/mods", label: "Mods", icon: "mods" },
    { href: "/logs", label: "Logs", icon: "logs" },
    { href: "/backups", label: "Backups", icon: "backups" },
    { href: "/settings", label: "Settings", icon: "settings" },
  ];

  let running = $derived(hangar.monitor?.status === "running");
  let pct = $derived.by(() => {
    const m = hangar.monitor;
    if (!m?.unpackedMb || !m.unpackedTotalMb) return null;
    return Math.round((m.unpackedMb / m.unpackedTotalMb) * 100);
  });
  let install = $derived(currentInstall());
  function isActive(href: string): boolean {
    return href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href);
  }
</script>

<div class="shell">
  <nav aria-label="Main">
    <div class="brand">
      <!-- Logo E "Bay marker" (B-024) -->
      <svg viewBox="0 0 48 48" width="36" height="36" aria-hidden="true">
        <path d="M7 15V7h8M33 7h8v8M41 33v8h-8M15 41H7v-8" style="stroke: var(--ink)"></path>
        <path d="M18 15v18M30 15v18"></path>
        <path d="M21 24h6" style="stroke-linecap: butt"></path>
      </svg>
      <div>
        <div class="name">HANGAR</div>
        <div class="tag">COMMUNITY-MADE<br />UPDATER &amp; LAUNCHER</div>
      </div>
    </div>

    <div class="links">
      {#each nav as item}
        <a href={item.href} class:active={isActive(item.href)} aria-current={isActive(item.href) ? "page" : undefined}>
          <Icon name={item.icon} color={isActive(item.href) ? "var(--phosphor)" : "currentColor"} />
          {item.label}
          {#if item.href === "/update" && running}
            <span class="badge live">{pct == null ? "…" : pct + "%"}</span>
          {/if}
        </a>
      {/each}
    </div>

    <div class="news">
      <div class="label">DCS news</div>
      <button onclick={() => api.openEdPage("changelog")} title="Opens ED's changelog in your browser"><Icon name="news" />Changelog<Icon name="external" size={14} /></button>
      <button onclick={() => api.openEdPage("newsletters")} title="Opens ED's newsletters in your browser"><Icon name="mail" />Newsletters<Icon name="external" size={14} /></button>
    </div>

    {#if install}
      <div class="install">
        <div class="label">Installation</div>
        <div class="inst-card">
          <span class="dot" style="color: var(--phosphor)"></span>
          <span>
            <span class="iname">{install.key}</span>
            <span class="ipath mono">{install.path}</span>
          </span>
        </div>
      </div>
    {/if}

    <p class="disclaimer">Not affiliated with or endorsed by Eagle Dynamics SA. THIS MATERIAL IS NOT MADE OR SUPPORTED BY EAGLE DYNAMICS SA.</p>
  </nav>

  <main>
    <JobBar />
    {@render children()}
  </main>
</div>

<style>
  .shell { display: flex; min-height: 100vh; }
  nav { width: 236px; flex: none; background: var(--rail); border-right: 1px solid #20282c; padding: 24px 14px; display: flex; flex-direction: column; gap: 28px; position: sticky; top: 0; height: 100vh; overflow-y: auto; }
  .brand { display: flex; align-items: center; gap: 12px; padding: 0 10px; }
  .brand svg { fill: none; stroke: var(--phosphor); stroke-width: 3.5; stroke-linecap: round; stroke-linejoin: round; flex: none; }
  .name { font-family: var(--font-head); font-weight: 700; font-size: 21px; letter-spacing: 0.22em; line-height: 1; }
  .tag { font-family: var(--font-mono); font-size: 9.5px; letter-spacing: 0.1em; line-height: 1.45; color: var(--ink-3); margin-top: 6px; }
  .news { display: flex; flex-direction: column; gap: 2px; }
  .news .label { padding: 0 12px 6px; }
  .news button { display: flex; align-items: center; gap: 12px; min-height: 38px; padding: 0 12px; border: 0; border-radius: 8px; background: transparent; color: var(--ink-2); font: inherit; font-weight: 500; cursor: pointer; text-align: left; }
  .news button :global(svg:last-child) { margin-left: auto; opacity: 0.7; }
  .news button:hover { background: #171d20; color: var(--ink); }
  .links { display: flex; flex-direction: column; gap: 2px; }
  .links a { display: flex; align-items: center; gap: 12px; min-height: 44px; padding: 0 12px; border-radius: 8px; text-decoration: none; color: var(--ink-2); font-weight: 500; }
  .links a:hover { background: #171d20; color: var(--ink); }
  .links a.active { background: var(--raised); color: var(--ink); }
  .badge { margin-left: auto; font-family: var(--font-mono); font-size: 11px; border-radius: 10px; padding: 2px 8px; }
  .badge.live { background: var(--phosphor); color: var(--phosphor-ink); }
  .install { display: flex; flex-direction: column; gap: 10px; }
  .install .label { padding: 0 10px; }
  .inst-card { display: flex; align-items: center; gap: 10px; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 12px; }
  .inst-card > span:last-child { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .iname { font-weight: 600; }
  .ipath { font-size: 11px; color: var(--ink-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .disclaimer { margin: auto 0 0; padding: 0 10px; font-size: 11px; line-height: 1.5; color: var(--ink-3); }
  main { flex: 1; min-width: 0; padding: 28px 36px 48px; display: flex; flex-direction: column; gap: 20px; }
</style>
