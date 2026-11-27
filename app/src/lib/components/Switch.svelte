<script lang="ts">
  // 46×26 switch (DESIGN.md). role="switch" + aria-checked.
  let { checked, label, disabled = false, onchange }: { checked: boolean; label: string; disabled?: boolean; onchange: (v: boolean) => void } = $props();
</script>

<button class="sw" role="switch" aria-checked={checked} aria-label={label} {disabled} onclick={() => onchange(!checked)}>
  <span class="knob"></span>
</button>

<style>
  .sw { position: relative; width: 46px; height: 26px; flex: none; border-radius: 13px; border: 1px solid var(--line-2); background: var(--well); cursor: pointer; padding: 0; transition: background 0.15s; }
  .sw[aria-checked="true"] { background: var(--phosphor); border-color: var(--phosphor); }
  .knob { position: absolute; top: 3px; left: 3px; width: 18px; height: 18px; border-radius: 50%; background: var(--ink-2); transition: transform 0.15s, background 0.15s; }
  .sw[aria-checked="true"] .knob { transform: translateX(20px); background: var(--phosphor-ink); }
  .sw:disabled { opacity: 0.45; cursor: not-allowed; }
</style>
