<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    api,
    type JimakuFileItem,
    type JimakuSearchResponse,
  } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { formatBytes } from "$lib/format";
  import Icon from "./Icon.svelte";

  let {
    anilistId,
    title,
    episodeNumber = null,
    epKey = "1",
    hasLocalVideo = false,
    onclose,
    ondownloaded,
  }: {
    anilistId: number;
    title: string;
    episodeNumber?: number | null;
    epKey?: string;
    hasLocalVideo?: boolean;
    onclose: () => void;
    ondownloaded?: () => void;
  } = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);
  let res = $state<JimakuSearchResponse | null>(null);
  let showAllEpisodes = $state(false);
  // svelte-ignore state_referenced_locally
  let autoRetime = $state(app.jimakuAutoRetime && hasLocalVideo);
  let downloadingUrl = $state<string | null>(null);
  let statusBanner = $state<string | null>(null);

  async function search() {
    loading = true;
    error = null;
    try {
      res = await api.jimakuSearch(anilistId, episodeNumber);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    search();
  });

  const visibleFiles = $derived.by(() => {
    if (!res) return [];
    if (showAllEpisodes || !episodeNumber) return res.files;
    const matching = res.files.filter(
      (f) => f.detectedEpisode === episodeNumber || (f.isArchive && f.detectedEpisode === null),
    );
    return matching.length > 0 ? matching : res.files;
  });

  async function downloadFile(f: JimakuFileItem) {
    if (f.isRar) {
      await openUrl(f.url);
      return;
    }
    downloadingUrl = f.url;
    statusBanner = null;
    error = null;
    try {
      const dl = await api.jimakuDownload({
        anilistId,
        epKey,
        fileName: f.name,
        fileUrl: f.url,
        autoRetime,
      });
      let msg =
        dl.linkedCount > 1
          ? `Extracted & linked ${dl.linkedCount} episodes from archive!`
          : `Downloaded & linked ${f.name}!`;
      if (dl.retimeReport) {
        const sign = dl.retimeReport.offsetMs >= 0 ? "+" : "";
        msg += ` Auto-retimed (${sign}${(dl.retimeReport.offsetMs / 1000).toFixed(2)}s across ${dl.retimeReport.segments} segment${dl.retimeReport.segments === 1 ? "" : "s"}).`;
      }
      if (dl.loadedIntoMpv) {
        msg += " Loaded into running mpv!";
      }
      statusBanner = msg;
      app.toast(msg, "info", 6000);
      ondownloaded?.();
    } catch (e) {
      error = String(e);
    } finally {
      downloadingUrl = null;
    }
  }

  function openSettings() {
    try {
      localStorage.setItem("kura.settings.category", "integrations");
    } catch {}
    onclose();
    goto("/settings");
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="jimaku-title">
  <header>
    <div>
      <h2 id="jimaku-title">
        Jimaku Japanese Subtitles
        {#if episodeNumber}<span class="ep-pill">Ep {episodeNumber}</span>{/if}
      </h2>
      <p class="faint">{title}</p>
    </div>
    <button class="btn btn-ghost btn-icon" onclick={onclose} aria-label="Close">
      <Icon name="x" />
    </button>
  </header>

  <div class="body">
    {#if loading}
      <div class="state-box">
        <div class="spinner"></div>
        <p>Searching Jimaku for <strong>{title}</strong>…</p>
      </div>
    {:else if res && !res.configured}
      <div class="state-box">
        <Icon name="subtitles" size={28} />
        <h3>Jimaku API Key Needed</h3>
        <p class="muted">
          Add your free Jimaku API key in <strong>Settings → Integrations</strong> to search and download Japanese subtitles directly by AniList ID.
        </p>
        <div class="row-actions">
          <button class="btn btn-primary" onclick={openSettings}>
            Open Settings → Integrations
          </button>
          <button class="btn" onclick={() => openUrl("https://jimaku.cc/account")}>
            <Icon name="external" size={14} /> Get API Key on Jimaku.cc
          </button>
        </div>
      </div>
    {:else if error}
      <div class="error-box">
        <Icon name="alert" size={18} />
        <span>{error}</span>
      </div>
    {:else if res && !res.entry}
      <div class="state-box">
        <Icon name="search" size={26} />
        <h3>No Jimaku Entry Found</h3>
        <p class="muted">Jimaku doesn't have an entry linked to AniList #{anilistId} yet.</p>
      </div>
    {:else if res}
      <div class="toolbar">
        <div class="entry-info">
          <span class="badge badge-ok">Jimaku #{res.entry?.id}</span>
          <strong>{res.entry?.japaneseName || res.entry?.name}</strong>
        </div>
        <div class="toolbar-toggles">
          {#if episodeNumber && res.files.length > visibleFiles.length}
            <label class="toggle-chip">
              <input type="checkbox" bind:checked={showAllEpisodes} />
              <span>Show all ({res.files.length})</span>
            </label>
          {/if}
          {#if hasLocalVideo}
            <label class="toggle-chip accent" title="Align downloaded subtitle timestamps against your video's embedded subtitle track using Kura's Alass-style dynamic programming engine">
              <input type="checkbox" bind:checked={autoRetime} />
              <span>Auto-Retime to Video</span>
            </label>
          {/if}
        </div>
      </div>

      {#if statusBanner}
        <div class="ok-banner">
          <Icon name="check" size={16} />
          <span>{statusBanner}</span>
        </div>
      {/if}

      {#if visibleFiles.length === 0}
        <div class="state-box">
          <p class="muted">No subtitle files found for this entry.</p>
        </div>
      {:else}
        <div class="file-list">
          {#each visibleFiles as f (f.url)}
            {@const busy = downloadingUrl === f.url}
            <div class="file-row" class:best={episodeNumber !== null && f.detectedEpisode === episodeNumber}>
              <div class="file-meta">
                <div class="file-name-line">
                  {#if f.detectedEpisode !== null}
                    <span class="tag-ep" class:match={f.detectedEpisode === episodeNumber}>Ep {f.detectedEpisode}</span>
                  {:else if f.isArchive}
                    <span class="tag-pack">PACK</span>
                  {/if}
                  <span class="fname" title={f.name}>{f.name}</span>
                </div>
                <div class="file-sub">
                  <span>{formatBytes(f.size)}</span>
                  {#if f.lastModified}
                    <span>· {f.lastModified.slice(0, 10)}</span>
                  {/if}
                  {#if f.isRar}
                    <span class="warn-tag">· External archive (.rar/.7z)</span>
                  {/if}
                </div>
              </div>
              <button
                class="btn btn-sm"
                class:btn-primary={episodeNumber !== null && f.detectedEpisode === episodeNumber}
                disabled={downloadingUrl !== null}
                onclick={() => downloadFile(f)}
              >
                {#if busy}
                  <span class="spinner spinner-xs"></span>
                  {autoRetime ? "Retiming…" : "Saving…"}
                {:else if f.isRar}
                  <Icon name="external" size={13} /> Download
                {:else if f.isArchive}
                  <Icon name="folder-plus" size={13} /> Extract & Match
                {:else}
                  <Icon name="check" size={13} /> {autoRetime ? "Get & Retime" : "Download"}
                {/if}
              </button>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>

  <footer>
    {#if res?.entry}
      <button
        class="btn btn-ghost btn-sm"
        onclick={() => openUrl(`https://jimaku.cc/entry/${res!.entry!.id}`)}
      >
        <Icon name="external" size={14} /> Open on Jimaku.cc
      </button>
    {/if}
    <div class="spacer"></div>
    <button class="btn" onclick={onclose}>Done</button>
  </footer>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: hsl(0 0% 0% / 0.65);
    backdrop-filter: blur(4px);
  }
  .dialog {
    position: fixed;
    z-index: 51;
    top: 8vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(680px, calc(100vw - 40px));
    max-height: 84vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-xl);
    box-shadow: 6px 6px 0 var(--coral);
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 20px 24px 16px;
    border-bottom: 1.5px solid var(--line-soft);
  }
  header h2 {
    font-size: 19px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ep-pill {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: var(--r-pill);
    background: var(--coral);
    color: var(--plum);
    font-weight: 800;
  }
  .body {
    padding: 18px 24px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .state-box {
    padding: 28px 16px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .row-actions {
    display: flex;
    gap: 10px;
    margin-top: 6px;
    flex-wrap: wrap;
    justify-content: center;
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 10px 14px;
    background: var(--bg-card);
    border: 1.5px solid var(--line-soft);
    border-radius: var(--r-md);
  }
  .entry-info {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .toolbar-toggles {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .toggle-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 700;
    padding: 4px 10px;
    border-radius: var(--r-pill);
    background: var(--bg-soft);
    border: 1.5px solid var(--line-soft);
    cursor: pointer;
    user-select: none;
  }
  .toggle-chip.accent {
    border-color: var(--coral);
  }
  .ok-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-radius: var(--r-md);
    background: hsl(145 55% 42% / 0.16);
    border: 1.5px solid var(--ok);
    color: var(--fg);
    font-size: 12.5px;
    font-weight: 600;
  }
  .error-box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    background: hsl(0 70% 50% / 0.15);
    border: 1.5px solid var(--danger);
    color: var(--danger);
    font-size: 13px;
  }
  .file-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .file-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 14px;
    background: var(--bg-card);
    border: 1.5px solid var(--line-soft);
    border-radius: var(--r-md);
  }
  .file-row.best {
    border-color: var(--coral);
    background: hsl(11 91% 64% / 0.06);
  }
  .file-meta {
    min-width: 0;
    flex: 1;
  }
  .file-name-line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .fname {
    font-size: 13px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag-ep,
  .tag-pack {
    flex-shrink: 0;
    font-size: 11px;
    font-weight: 800;
    padding: 1px 7px;
    border-radius: var(--r-pill);
    background: var(--bg-soft);
    border: 1px solid var(--line-soft);
  }
  .tag-ep.match {
    background: var(--coral);
    color: var(--plum);
    border-color: var(--line);
  }
  .file-sub {
    font-size: 11.5px;
    color: var(--fg-muted);
    margin-top: 3px;
    display: flex;
    gap: 6px;
  }
  .warn-tag {
    color: var(--amber);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 24px;
    border-top: 1.5px solid var(--line-soft);
  }
  .spacer {
    flex: 1;
  }
</style>

