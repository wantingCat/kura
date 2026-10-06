<script lang="ts">
  import { updater } from "$lib/updater.svelte";
  import Icon from "./Icon.svelte";
</script>

{#if updater.visible}
  <div class="update" role="status" id="update-banner">
    <span class="burst" aria-hidden="true"><span>NEW!</span></span>
    <div class="text">
      {#if updater.status === "error"}
        <strong>Update failed.</strong>
        <span class="muted">{updater.error ?? "Something went wrong."} You can grab it from GitHub instead.</span>
      {:else if updater.status === "downloading"}
        <strong>Downloading Kura v{updater.version}…</strong>
        <span class="bar"><span class="fill" style:width={updater.percent === null ? "40%" : `${updater.percent}%`} class:indeterminate={updater.percent === null}></span></span>
      {:else if updater.status === "installing"}
        <strong>Installing — Kura will restart in a moment.</strong>
      {:else}
        <strong>Kura v{updater.version} is available!</strong>
        <span class="muted">Your library and settings stay as they are.</span>
      {/if}
    </div>
    <div class="buttons">
      {#if updater.status === "available"}
        <button class="btn btn-sm btn-ink" onclick={() => updater.install()} id="update-install">
          <Icon name="refresh" size={14} /> Update &amp; restart
        </button>
      {:else if updater.status === "error"}
        <a class="btn btn-sm btn-ink" href="https://github.com/wantingCat/kura/releases/latest" target="_blank" rel="noreferrer">
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
  .update {
    position: relative;
    display: flex;
    align-items: center;
    gap: 16px;
    margin: 18px 36px 0;
    padding: 12px 14px 12px 74px;
    border-radius: var(--r-lg);
    border: var(--bw) solid var(--line);
    background: var(--coral);
    color: var(--on-coral);
    box-shadow: var(--sticker);
    animation: fade-in var(--t-med) var(--ease);
  }
  .burst {
    position: absolute;
    left: -10px;
    top: 50%;
    width: 74px;
    height: 74px;
    margin-top: -37px;
    display: grid;
    place-items: center;
    background: var(--amber);
    /* 12-point comic starburst */
    clip-path: polygon(
      50% 0%, 61% 15%, 79% 7%, 80% 26%, 98% 30%, 88% 46%, 100% 61%, 82% 69%, 85% 88%, 66% 85%, 55% 100%, 44% 86%,
      26% 94%, 23% 75%, 4% 72%, 13% 55%, 0% 40%, 17% 31%, 13% 12%, 33% 14%
    );
    transform: rotate(-10deg);
    filter: drop-shadow(2px 2px 0 var(--line));
  }
  .burst span {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 15px;
    color: var(--on-coral);
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
    font-size: 16px;
    font-weight: 600;
  }
  .muted {
    color: color-mix(in srgb, var(--on-coral) 75%, transparent);
    font-size: 13px;
  }
  .buttons {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .btn-ink {
    background: var(--plum);
    color: var(--cream);
  }
  .btn-ink:hover {
    background: var(--surface-3);
  }
  .close {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: none;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--on-coral);
    cursor: pointer;
  }
  .close:hover {
    background: color-mix(in srgb, var(--on-coral) 14%, transparent);
  }
  .bar {
    display: block;
    margin-top: 4px;
    height: 8px;
    max-width: 360px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--on-coral) 18%, transparent);
    border: 1.5px solid var(--line);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--cream);
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
</style>
