<script lang="ts">
  let { title, bay, text }: { title: string; bay: string; text: string } = $props();
</script>

<header>
  <div class="bay">BAY 01 · {bay}</div>
  <h1>{title}</h1>
</header>
<section class="hero soon">
  <div class="label">Coming in 0.2</div>
  <p>{text}</p>
  <p class="faint">The design is ready in the canvas. Version 0.1 only reads and monitors, it doesn't write anywhere yet.</p>
</section>

<style>
  .soon { padding: 28px; max-width: 720px; }
  p { margin: 10px 0 0; line-height: 1.55; color: var(--ink-soft); }
  p.faint { color: var(--ink-3); font-size: 13px; }
</style>
