<script lang="ts">
  import { app } from "$lib/app.svelte";
  import Icon from "./Icon.svelte";
</script>

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast {t.kind}">
      <Icon name={t.kind === "info" ? "check" : "alert"} size={16} stroke={2.5} />
      <p>{t.text}</p>
      <button onclick={() => app.dismissToast(t.id)} aria-label="Dismiss"><Icon name="x" size={14} stroke={2.5} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 22px;
    bottom: 22px;
    z-index: 100;
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(420px, calc(100vw - 44px));
    pointer-events: none;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px 12px 12px 14px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    border-left: 8px solid var(--ok);
    background: var(--surface);
    box-shadow: var(--sticker);
    pointer-events: auto;
    animation: fade-up var(--t-med) var(--ease) both;
  }
  .toast :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .toast.info :global(svg) {
    color: var(--ok);
  }
  .toast.warn {
    border-left-color: var(--amber);
  }
  .toast.warn :global(svg) {
    color: var(--amber);
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .toast.error :global(svg) {
    color: var(--danger);
  }
  p {
    flex: 1;
    font-size: 13.5px;
    color: var(--text-2);
  }
  button {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-3);
    cursor: pointer;
  }
  button:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  button :global(svg) {
    margin: 0 !important;
  }
</style>
