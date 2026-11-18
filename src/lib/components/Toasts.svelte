<script lang="ts">
  import { goto } from "$app/navigation";
  import { app } from "$lib/app.svelte";
  import { JA } from "$lib/ja";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";

  const KIND_JA: Record<string, string> = {
    info: JA.states.notice,
    warn: JA.states.warning,
    error: JA.states.error,
  };
</script>

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="toast glass {t.kind}"
      class:clickable={!!t.anilistId}
      onclick={() => {
        if (t.anilistId) {
          goto(`/anime/${t.anilistId}`);
          app.dismissToast(t.id);
        }
      }}
    >
      <span class="kind-icon">
        <Icon name={t.kind === "info" ? "check" : "alert"} size={15} stroke={2.5} />
      </span>
      <div class="body">
        <Ja text={KIND_JA[t.kind]} class="kind-chip" />
        <p>{t.text}</p>
      </div>
      <button
        onclick={(e) => {
          e.stopPropagation();
          app.dismissToast(t.id);
        }}
        aria-label="Dismiss"
      ><Icon name="x" size={14} stroke={2.5} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 22px;
    bottom: 22px;
    z-index: 850;
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(400px, calc(100vw - 44px));
    pointer-events: none;
  }
  .toast {
    --kind: var(--ok);
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px 10px 12px 12px;
    border-radius: var(--r-md);
    color: var(--text);
    box-shadow: var(--shadow-lg);
    pointer-events: auto;
    overflow: hidden;
    animation: fade-up var(--t-med) var(--ease) both;
  }
  .toast::before {
    content: "";
    position: absolute;
    left: 0;
    top: 10px;
    bottom: 10px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--kind);
    box-shadow: 0 0 12px var(--kind);
  }
  .toast.warn {
    --kind: var(--amber);
  }
  .toast.error {
    --kind: var(--danger);
  }
  .toast.clickable {
    cursor: pointer;
    transition:
      transform var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .toast.clickable:hover {
    transform: translateY(-2px);
    box-shadow: var(--shadow-lg), var(--neon-glow);
  }
  .kind-icon {
    flex: none;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 8px;
    color: var(--kind);
    background: color-mix(in srgb, var(--kind) 16%, transparent);
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding-top: 1px;
  }
  .body :global(.kind-chip) {
    align-self: flex-start;
    padding: 1px 7px;
    border-radius: var(--r-pill);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    color: var(--kind);
    background: color-mix(in srgb, var(--kind) 14%, transparent);
  }
  p {
    overflow-wrap: anywhere;
    font-size: 13.5px;
    line-height: 1.45;
    color: var(--text);
  }
  button {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: var(--r-pill);
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
    transition: background var(--t-fast) var(--ease), color var(--t-fast) var(--ease);
  }
  button:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  button :global(svg) {
    margin: 0 !important;
  }
  @media (prefers-reduced-motion: reduce) {
    .toast {
      animation: none;
    }
    .toast.clickable:hover {
      transform: none;
    }
  }
</style>
