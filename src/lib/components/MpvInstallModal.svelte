<script lang="ts">
  import { mpvModal } from "$lib/mpvModal.svelte";
  import Icon from "./Icon.svelte";

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !mpvModal.installing) {
      mpvModal.close();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if mpvModal.visible}
  <div class="backdrop" onclick={() => !mpvModal.installing && mpvModal.close()} role="presentation"></div>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="mpv-modal-title">
    <header>
      <div class="header-left">
        <span class="burst" aria-hidden="true"><span>MPV</span></span>
        <div>
          <span class="badge badge-accent">Recommended Player</span>
          <h2 id="mpv-modal-title">Unlock the Full Kura Experience</h2>
        </div>
      </div>
      {#if !mpvModal.installing}
        <button class="close-btn" onclick={() => mpvModal.close()} aria-label="Close dialog" title="Close">
          <Icon name="x" size={18} stroke={2.5} />
        </button>
      {/if}
    </header>

    <div class="content">
      <p class="intro">
        To use all the features of Kura — including flawless 10-bit anime playback, smart OP/ED skip, Japanese subtitle immersion, and instant resume — please install <strong>MPV Player</strong>.
      </p>

      <div class="features-grid">
        <div class="feature-item">
          <div class="feature-icon">⚡</div>
          <div class="feature-text">
            <strong>10-bit HEVC &amp; SSA/ASS Subtitles</strong>
            <small>Direct hardware decoding for high-bitrate anime without extra codec packs.</small>
          </div>
        </div>

        <div class="feature-item">
          <div class="feature-icon">⏭</div>
          <div class="feature-text">
            <strong>Smart OP/ED Skip (<kbd>Tab</kbd>)</strong>
            <small>Floating on-screen prompt with instant one-key skip past openings and endings.</small>
          </div>
        </div>

        <div class="feature-item">
          <div class="feature-icon">💬</div>
          <div class="feature-text">
            <strong>Subtitle Delay Shortcuts (<kbd>z</kbd> / <kbd>x</kbd>)</strong>
            <small>Instant on-the-fly timing correction with &plusmn;100ms manga-styled OSD feedback.</small>
          </div>
        </div>

        <div class="feature-item">
          <div class="feature-icon">🎌</div>
          <div class="feature-text">
            <strong>Remembers Your Tracks</strong>
            <small>Resume where you left off with your audio and subtitle choices kept per series.</small>
          </div>
        </div>
      </div>

      {#if mpvModal.installing}
        <div class="install-box">
          <div class="spinner"></div>
          <div class="install-text">
            <strong>{mpvModal.progressMessage ?? "Installing MPV Player…"}</strong>
            <small class="faint">If Windows asks for administrator permission, please click <strong>Yes</strong> to allow the installation.</small>
          </div>
        </div>
      {:else if mpvModal.result?.success}
        <div class="status-box success">
          <div class="status-icon">✓</div>
          <div>
            <strong>MPV is ready!</strong>
            <p class="faint">MPV has been successfully installed and configured as your default player in Kura.</p>
          </div>
        </div>
      {:else if mpvModal.result?.method === "website"}
        <div class="status-box info">
          <div class="status-icon">🌐</div>
          <div>
            <strong>Opened MPV Official Website</strong>
            <p class="faint">Download and run the installer from your browser. Once installed, click below to verify.</p>
          </div>
        </div>
      {:else if mpvModal.error}
        <div class="status-box error">
          <div class="status-icon">!</div>
          <div>
            <strong>Installation error</strong>
            <p class="faint">{mpvModal.error}</p>
          </div>
        </div>
      {/if}
    </div>

    <footer>
      <div class="footer-left">
        {#if !mpvModal.installing && !mpvModal.result?.success}
          <button class="btn-link faint" onclick={() => mpvModal.dismissPermanently()} id="mpv-dismiss-permanent">
            Don't ask again
          </button>
        {/if}
      </div>

      <div class="buttons">
        {#if mpvModal.result?.success}
          <button class="btn btn-accent" onclick={() => mpvModal.close()} id="mpv-done-btn">
            <Icon name="check" size={16} /> Start Watching
          </button>
        {:else if mpvModal.result?.method === "website"}
          <button class="btn btn-ghost" onclick={() => mpvModal.close()}>Close</button>
          <button class="btn btn-accent" onclick={() => mpvModal.check()} id="mpv-recheck-btn">
            <Icon name="refresh" size={16} /> Re-check Detection
          </button>
        {:else if mpvModal.installing}
          <button class="btn btn-ghost" disabled>Installing…</button>
        {:else}
          <button class="btn btn-ghost" onclick={() => mpvModal.close()} id="mpv-later-btn">
            Remind me later
          </button>

          {#if mpvModal.status?.hasWinget}
            <button class="btn btn-accent" onclick={() => mpvModal.install()} id="mpv-install-winget">
              <Icon name="download" size={16} /> Install MPV Automatically (winget)
            </button>
          {:else if mpvModal.status?.hasChoco}
            <button class="btn btn-accent" onclick={() => mpvModal.install()} id="mpv-install-choco">
              <Icon name="download" size={16} /> Install MPV Automatically (Chocolatey)
            </button>
          {:else}
            <button class="btn btn-accent" onclick={() => mpvModal.install()} id="mpv-download-web">
              <Icon name="external" size={16} /> Download from Website
            </button>
          {/if}
        {/if}
      </div>
    </footer>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--line) 80%, transparent);
    z-index: 100;
    animation: fade-in var(--t-med) var(--ease);
  }

  .dialog {
    position: fixed;
    z-index: 101;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(620px, calc(100vw - 36px));
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-xl);
    box-shadow: 6px 6px 0 var(--coral);
    animation: pop 320ms var(--bounce);
    overflow: hidden;
  }

  @keyframes pop {
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
    justify-content: space-between;
    align-items: flex-start;
    padding: 24px 24px 16px;
    border-bottom: var(--bw) solid var(--border);
    position: relative;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .burst {
    width: 52px;
    height: 52px;
    display: grid;
    place-items: center;
    background: var(--amber);
    clip-path: polygon(
      50% 0%, 61% 15%, 79% 7%, 80% 26%, 98% 30%, 88% 46%, 100% 61%, 82% 69%, 85% 88%, 66% 85%, 55% 100%, 44% 86%,
      26% 94%, 23% 75%, 4% 72%, 13% 55%, 0% 40%, 17% 31%, 13% 12%, 33% 14%
    );
    transform: rotate(-8deg);
    filter: drop-shadow(2px 2px 0 var(--line));
    flex-shrink: 0;
  }

  .burst span {
    font-family: var(--font-display);
    font-weight: 800;
    font-size: 13px;
    color: var(--on-coral);
  }

  h2 {
    font-size: 20px;
    margin-top: 4px;
    line-height: 1.25;
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--r-sm);
    display: grid;
    place-items: center;
    transition: color var(--t-fast), background var(--t-fast);
  }

  .close-btn:hover {
    color: var(--text);
    background: var(--surface);
  }

  .content {
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow-y: auto;
  }

  .intro {
    font-size: 14px;
    line-height: 1.5;
    color: var(--text-2);
  }

  .intro strong {
    color: var(--text);
  }

  .features-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  @media (max-width: 580px) {
    .features-grid {
      grid-template-columns: 1fr;
    }
  }

  .feature-item {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    background: var(--surface);
    border: var(--bw) solid var(--border);
    border-radius: var(--r-md);
  }

  .feature-icon {
    font-size: 18px;
    line-height: 1;
    flex-shrink: 0;
    margin-top: 2px;
  }

  .feature-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .feature-text strong {
    font-size: 13px;
    font-weight: 600;
  }

  .feature-text small {
    font-size: 11.5px;
    color: var(--text-3);
    line-height: 1.35;
  }

  kbd {
    display: inline-block;
    padding: 1px 5px;
    font-size: 10.5px;
    font-family: monospace;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    box-shadow: 1px 1px 0 var(--line);
  }

  .install-box {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    background: var(--surface-2);
    border: var(--bw) solid var(--coral);
    border-radius: var(--r-md);
    box-shadow: var(--sticker);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 3px solid color-mix(in srgb, var(--coral) 30%, transparent);
    border-top-color: var(--coral);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .install-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .install-text strong {
    font-size: 14px;
    color: var(--text);
  }

  .status-box {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 16px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
  }

  .status-box.success {
    background: color-mix(in srgb, var(--ok) 18%, var(--bg-elev));
    border-color: var(--ok);
  }

  .status-box.info {
    background: color-mix(in srgb, var(--coral) 15%, var(--bg-elev));
    border-color: var(--coral);
  }

  .status-box.error {
    background: color-mix(in srgb, var(--danger) 18%, var(--bg-elev));
    border-color: var(--danger);
  }

  .status-icon {
    font-size: 16px;
    font-weight: bold;
    margin-top: 1px;
  }

  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 24px;
    border-top: var(--bw) solid var(--border);
    background: var(--surface);
  }

  .buttons {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .btn-link {
    background: none;
    border: none;
    font-size: 12px;
    color: var(--text-3);
    cursor: pointer;
    text-decoration: underline;
    padding: 4px;
  }

  .btn-link:hover {
    color: var(--text-2);
  }
</style>
