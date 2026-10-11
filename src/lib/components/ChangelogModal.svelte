<script lang="ts">
  import { changelogModal } from "$lib/changelogModal.svelte";
  import { CHANGELOG } from "$lib/changelog";
  import Icon from "./Icon.svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && changelogModal.visible) {
      changelogModal.close();
    }
  }

  function openGithub() {
    openUrl("https://github.com/wantingCat/kura/releases").catch(() => {});
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if changelogModal.visible}
  <div class="backdrop" onclick={() => changelogModal.close()} role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="changelog-modal-title">
    <header>
      <div class="header-left">
        <div class="sparkle-badge">
          <Icon name="sparkles" size={16} stroke={2.5} />
          <span>WHAT'S NEW</span>
        </div>
        <div class="header-titles">
          <h2 id="changelog-modal-title">Kura v{changelogModal.currentRelease.version}</h2>
          <span class="release-date">{changelogModal.currentRelease.date}</span>
        </div>
      </div>
      <button class="close-btn" onclick={() => changelogModal.close()} aria-label="Close changelog" title="Close">
        <Icon name="x" size={18} stroke={2.5} />
      </button>
    </header>

    <!-- Version selector tabs -->
    <div class="version-tabs" role="tablist">
      {#each CHANGELOG as rel}
        <button
          class="version-tab"
          class:active={changelogModal.activeVersion === rel.version}
          onclick={() => changelogModal.selectVersion(rel.version)}
          role="tab"
          aria-selected={changelogModal.activeVersion === rel.version}
        >
          <span>v{rel.version}</span>
          {#if rel.badge}
            <span class="tab-badge">{rel.badge}</span>
          {/if}
        </button>
      {/each}
    </div>

    <div class="content">
      <div class="summary-card">
        <h3>{changelogModal.currentRelease.title}</h3>
        <p>{changelogModal.currentRelease.summary}</p>
      </div>

      <div class="items-list">
        {#each changelogModal.currentRelease.items as item}
          <div class="change-item change-{item.type}">
            <div class="badge-col">
              {#if item.type === "new"}
                <span class="type-badge badge-new">NEW</span>
              {:else if item.type === "improvement"}
                <span class="type-badge badge-imp">IMPROVED</span>
              {:else}
                <span class="type-badge badge-fix">FIXED</span>
              {/if}
            </div>
            <div class="item-text">
              {item.text}
            </div>
          </div>
        {/each}
      </div>
    </div>

    <footer>
      <button class="btn btn-ghost btn-sm btn-github" onclick={openGithub} title="View all releases on GitHub">
        <Icon name="external" size={14} />
        <span>GitHub Releases</span>
      </button>
      <button class="btn btn-primary btn-cta" onclick={() => changelogModal.close()} id="changelog-dismiss">
        <span>Got it, let's explore!</span>
      </button>
    </footer>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(10, 6, 8, 0.78);
    backdrop-filter: blur(10px);
    z-index: 9998;
    animation: fadeIn 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(640px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    background: var(--surface, #1e1416);
    border: 1px solid rgba(247, 109, 77, 0.3);
    border-radius: 20px;
    box-shadow:
      0 24px 70px rgba(0, 0, 0, 0.75),
      0 0 0 1px rgba(255, 255, 255, 0.05),
      0 0 40px rgba(247, 109, 77, 0.15);
    z-index: 9999;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: var(--text, #f7ede2);
    animation: slideUp 0.28s cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes slideUp {
    from {
      opacity: 0;
      transform: translate(-50%, -46%) scale(0.96);
    }
    to {
      opacity: 1;
      transform: translate(-50%, -50%) scale(1);
    }
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 20px 24px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    background: linear-gradient(180deg, rgba(247, 109, 77, 0.08) 0%, transparent 100%);
  }

  .header-left {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .sparkle-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.08em;
    color: var(--accent, #f76d4d);
    text-transform: uppercase;
  }

  .header-titles {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  .header-titles h2 {
    margin: 0;
    font-size: 24px;
    font-weight: 800;
    letter-spacing: -0.02em;
    color: var(--text, #f7ede2);
    font-family: var(--font-display, inherit);
  }

  .release-date {
    font-size: 13px;
    color: var(--text-faint, rgba(247, 237, 226, 0.5));
    font-weight: 600;
  }

  .close-btn {
    background: transparent;
    border: none;
    color: var(--text-faint, rgba(247, 237, 226, 0.6));
    cursor: pointer;
    padding: 8px;
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s, color 0.15s;
  }

  .close-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text, #f7ede2);
  }

  /* Version tab bar */
  .version-tabs {
    display: flex;
    gap: 6px;
    padding: 10px 24px 6px;
    background: rgba(0, 0, 0, 0.15);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .version-tabs::-webkit-scrollbar {
    display: none;
  }

  .version-tab {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.06);
    color: var(--text-muted, rgba(247, 237, 226, 0.7));
    font-size: 12px;
    font-weight: 700;
    padding: 6px 14px;
    border-radius: 999px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    transition: all 0.15s ease;
  }

  .version-tab:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text, #f7ede2);
  }

  .version-tab.active {
    background: var(--accent, #f76d4d);
    color: #fff;
    border-color: var(--accent, #f76d4d);
    box-shadow: 0 4px 14px rgba(247, 109, 77, 0.35);
  }

  .tab-badge {
    font-size: 9px;
    padding: 2px 6px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.25);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .content {
    flex: 1;
    overflow-y: auto;
    padding: 16px 24px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.2) transparent;
  }

  .summary-card {
    background: rgba(247, 109, 77, 0.08);
    border: 1px solid rgba(247, 109, 77, 0.2);
    border-radius: 14px;
    padding: 14px 18px;
  }

  .summary-card h3 {
    margin: 0 0 4px;
    font-size: 15px;
    font-weight: 800;
    color: var(--text, #f7ede2);
  }

  .summary-card p {
    margin: 0;
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-muted, rgba(247, 237, 226, 0.8));
  }

  .items-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .change-item {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    background: rgba(255, 255, 255, 0.025);
    border: 1px solid rgba(255, 255, 255, 0.05);
    border-radius: 12px;
    padding: 10px 14px;
    font-size: 13px;
    line-height: 1.45;
    transition: background 0.15s;
  }

  .change-item:hover {
    background: rgba(255, 255, 255, 0.045);
  }

  .badge-col {
    padding-top: 1px;
    flex-shrink: 0;
  }

  .type-badge {
    font-size: 10px;
    font-weight: 800;
    padding: 3px 8px;
    border-radius: 6px;
    letter-spacing: 0.04em;
    display: inline-block;
  }

  .badge-new {
    background: rgba(247, 109, 77, 0.25);
    color: #f76d4d;
    border: 1px solid rgba(247, 109, 77, 0.4);
  }

  .badge-imp {
    background: rgba(247, 178, 103, 0.2);
    color: #f7b267;
    border: 1px solid rgba(247, 178, 103, 0.35);
  }

  .badge-fix {
    background: rgba(131, 197, 190, 0.2);
    color: #83c5be;
    border: 1px solid rgba(131, 197, 190, 0.35);
  }

  .item-text {
    flex: 1;
    color: var(--text, #f7ede2);
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 24px 18px;
    border-top: 1px solid rgba(255, 255, 255, 0.06);
    background: rgba(0, 0, 0, 0.2);
  }

  .btn-github {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted, rgba(247, 237, 226, 0.7));
    font-size: 13px;
  }

  .btn-github:hover {
    color: var(--text, #f7ede2);
  }

  .btn-cta {
    background: var(--accent, #f76d4d);
    color: #fff;
    border: none;
    font-weight: 700;
    font-size: 14px;
    padding: 9px 20px;
    border-radius: 12px;
    cursor: pointer;
    box-shadow: 0 4px 16px rgba(247, 109, 77, 0.4);
    transition: transform 0.15s, box-shadow 0.15s;
  }

  .btn-cta:hover {
    transform: translateY(-1px);
    box-shadow: 0 6px 20px rgba(247, 109, 77, 0.5);
  }

  .btn-cta:active {
    transform: translateY(0);
  }
</style>
