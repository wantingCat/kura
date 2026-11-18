<script lang="ts">
  import { updater } from "$lib/updater.svelte";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";

  /** Local accent vocabulary (not in ja.ts): 更新 = "update". */
  const JA_UPDATE = "更新";
</script>

{#if updater.visible}
  <div class="update glass" class:error={updater.status === "error"} role="status" id="update-banner">
    <span class="mark" aria-hidden="true">
      <Icon name={updater.status === "error" ? "alert" : "sparkles"} size={16} />
    </span>
    <div class="text">
      {#if updater.status === "error"}
        <strong>Update failed. <Ja text={JA_UPDATE} class="ja-sub" /></strong>
        <span class="muted">{updater.error ?? "Something went wrong."} You can grab it from GitHub instead.</span>
      {:else if updater.status === "downloading"}
        <strong>Downloading Kura v{updater.version}…</strong>
        <span class="neon-progress bar">
          <span
            class="fill"
            style:width={updater.percent === null ? "40%" : `${updater.percent}%`}
            class:indeterminate={updater.percent === null}
          ></span>
        </span>
      {:else if updater.status === "installing"}
        <strong>Installing — Kura will restart in a moment.</strong>
      {:else}
        <strong>Kura v{updater.version} is available! <Ja text={JA_UPDATE} class="ja-sub" /></strong>
        <span class="muted">Your library and settings stay as they are.</span>
      {/if}
    </div>
    <div class="buttons">
      {#if updater.status === "available"}
        <button class="btn btn-sm btn-cta" onclick={() => updater.install()} id="update-install">
          <Icon name="refresh" size={14} /> Update &amp; restart
        </button>
      {:else if updater.status === "error"}
        <a
          class="btn btn-sm btn-ghost-glass"
          href="https://github.com/wantingCat/kura/releases/latest"
          target="_blank"
          rel="noreferrer"
        >
          <Icon name="external" size={14} /> Open GitHub
        </a>
      {/if}
      {#if updater.status === "available" || updater.status === "error"}
        <button class="close" onclick={() => (updater.dismissed = true)} aria-label="Later" title="Later" id="update-dismiss">
          <Icon name="x" size={16} stroke={2.5} />
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* Floats just below the fixed navbar so it is never hidden underneath it. */
  .update {
    position: fixed;
    top: calc(var(--nav-h) + var(--nav-inset) + 10px);
    left: 50%;
    transform: translateX(-50%);
    z-index: 790;
    display: flex;
    align-items: center;
    gap: 14px;
    width: min(720px, calc(100vw - 2 * var(--nav-inset)));
    padding: 10px 10px 10px 12px;
    border-radius: var(--r-lg);
    color: var(--text);
    box-shadow: var(--shadow-lg), var(--neon-glow);
    animation: fade-in var(--t-med) var(--ease);
  }
  .update::before {
    /* thin neon accent edge */
    content: "";
    position: absolute;
    left: 14px;
    right: 14px;
    top: 0;
    height: 2px;
    border-radius: 2px;
    background: var(--neon);
  }
  .update.error::before {
    background: var(--danger);
  }
  .mark {
    flex: none;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--neon-violet) 22%, transparent);
    color: var(--neon-pink);
  }
  .update.error .mark {
    background: color-mix(in srgb, var(--danger) 18%, transparent);
    color: var(--danger);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  strong {
    font-family: var(--font-display);
    font-size: 14px;
    font-weight: 600;
  }
  .muted {
    color: var(--text-2);
    font-size: 12.5px;
  }
  .buttons {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .btn-ghost-glass {
    background: rgba(255, 255, 255, 0.06);
    border-color: var(--glass-hairline);
    border-radius: var(--r-pill);
    color: var(--text);
  }
  .btn-ghost-glass:hover {
    background: rgba(255, 255, 255, 0.12);
  }
  .close {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: none;
    border-radius: var(--r-pill);
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
    transition: background var(--t-fast) var(--ease), color var(--t-fast) var(--ease);
  }
  .close:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  .bar {
    display: block;
    margin-top: 6px;
    max-width: 360px;
  }
  .fill {
    transition: width var(--t-med) var(--ease);
  }
  .fill.indeterminate {
    animation: slide 1.2s var(--ease) infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(260%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .update {
      animation: none;
    }
    .fill.indeterminate {
      animation: none;
    }
  }
</style>
