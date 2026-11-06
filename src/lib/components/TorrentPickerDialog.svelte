<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    type TorrentCandidate,
    type TorrentExtension,
    type TorrentSessionStatus,
  } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { formatBytes } from "$lib/format";
  import Icon from "./Icon.svelte";

  let {
    anilistId,
    title,
    epKey,
    episodeNumber,
    autoStart = false,
    onclose,
  }: {
    anilistId: number;
    title: string;
    epKey: string;
    episodeNumber: number;
    autoStart?: boolean;
    onclose: () => void;
  } = $props();

  let loading = $state(true);
  let starting = $state(false);
  let error = $state<string | null>(null);
  let candidates = $state<TorrentCandidate[]>([]);
  let customQuery = $state("");
  let manualMagnet = $state("");
  let chosenMode = $state<"stream" | "hybrid" | "download">(app.torrentAction);
  let activeSessionId = $state<string | null>(null);
  let triedKeys = $state<string[]>([]);
  let failoverCount = $state(0);

  // Modular Torrent Extensions state
  let extensions = $state<TorrentExtension[]>([]);
  let showExtensions = $state(false);
  let extInput = $state("");
  let importingExt = $state(false);

  function candidateKey(c: TorrentCandidate): string {
    return (c.infoHash || c.torrentUrl || c.magnetUri || c.title).toLowerCase();
  }

  // Prefer untried releases with reported seeders; indexer counts are often stale though
  // (DHT can still find peers), so fall back to any untried release as a last resort.
  const nextUntriedCandidate = $derived<TorrentCandidate | null>(
    candidates.find((c) => c.seeders > 0 && !triedKeys.includes(candidateKey(c))) ??
      candidates.find((c) => !triedKeys.includes(candidateKey(c))) ??
      null,
  );

  const liveSession = $derived<TorrentSessionStatus | null>(
    activeSessionId
      ? (app.torrentSessions.find((s) => s.sessionId === activeSessionId) ??
          null)
      : null,
  );

  // Auto-close once player launches in stream/hybrid mode
  $effect(() => {
    if (liveSession && liveSession.state === "streaming") {
      app.toast(`Streaming Ep ${episodeNumber} in player…`, "info", 4500);
      onclose();
    }
  });

  // Automatic failover if the active torrent has dead/unreachable seeders
  $effect(() => {
    if (
      liveSession &&
      liveSession.state === "error" &&
      failoverCount < 4 &&
      nextUntriedCandidate
    ) {
      failoverCount += 1;
      void tryNextCandidate();
    }
  });

  async function search(q?: string) {
    loading = true;
    error = null;
    triedKeys = [];
    failoverCount = 0;
    try {
      candidates = await api.searchEpisodeTorrents(
        anilistId,
        episodeNumber,
        q || null,
      );
      if (!q && !customQuery) {
        customQuery = `${title} ${String(episodeNumber).padStart(2, "0")}`;
      }
      if (autoStart && !q && app.torrentQuality !== "ask") {
        const best =
          candidates.find((c) => c.seeders >= 3 && c.score > 400) ??
          candidates.find((c) => c.seeders > 0 && c.score > 400) ??
          candidates.find((c) => c.seeders > 0) ??
          candidates[0];
        if (best) {
          await startCandidate(best);
        }
      }
    } catch (e) {
      const msg = String(e);
      if (msg.includes("NO_TORRENT_EXTENSIONS")) {
        showExtensions = true;
        error = null;
      } else {
        error = msg;
      }
    } finally {
      loading = false;
    }
  }

  async function loadExtensions() {
    try {
      extensions = await api.listTorrentExtensions();
      if (extensions.filter((e) => e.enabled).length === 0) {
        showExtensions = true;
      }
    } catch {
      // ignore
    }
  }

  async function handleImportExtension() {
    const val = extInput.trim();
    if (!val || importingExt) return;
    importingExt = true;
    error = null;
    try {
      extensions = await api.importTorrentExtension(val);
      extInput = "";
      showExtensions = false;
      app.toast(
        `Enabled ${extensions.filter((e) => e.enabled).length} torrent extension(s)`,
        "info",
      );
      await search(customQuery || undefined);
    } catch (e) {
      error = String(e);
    } finally {
      importingExt = false;
    }
  }

  async function handleToggleExtension(ext: TorrentExtension) {
    try {
      extensions = await api.toggleTorrentExtension(ext.id, !ext.enabled);
    } catch (e) {
      error = String(e);
    }
  }

  async function handleRemoveExtension(id: string) {
    try {
      extensions = await api.removeTorrentExtension(id);
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    void loadExtensions();
    void search();
  });

  async function startCandidate(c: TorrentCandidate) {
    if (starting) return;
    starting = true;
    error = null;
    const key = candidateKey(c);
    if (!triedKeys.includes(key)) {
      triedKeys = [...triedKeys, key];
    }
    try {
      if (activeSessionId) {
        await api.cancelTorrentSession(activeSessionId).catch(() => {});
        activeSessionId = null;
      }
      const status = await api.startTorrentSession({
        anilistId,
        epKey,
        episodeNum: episodeNumber,
        candidate: c,
        modeOverride: chosenMode,
      });
      activeSessionId = status.sessionId;
      if (chosenMode === "download") {
        app.toast(`Downloading ${c.title} in background…`, "info", 5000);
      }
    } catch (e) {
      error = String(e);
      starting = false;
      if (failoverCount < 4 && nextUntriedCandidate) {
        failoverCount += 1;
        await startCandidate(nextUntriedCandidate);
        return;
      }
    } finally {
      starting = false;
    }
  }

  async function tryNextCandidate() {
    if (!nextUntriedCandidate) {
      if (activeSessionId) {
        await api.cancelTorrentSession(activeSessionId).catch(() => {});
        activeSessionId = null;
      }
      return;
    }
    await startCandidate(nextUntriedCandidate);
  }

  async function backToCandidateList() {
    if (activeSessionId) {
      await api.cancelTorrentSession(activeSessionId).catch(() => {});
      activeSessionId = null;
    }
  }

  async function startManualMagnet() {
    const val = manualMagnet.trim();
    if (!val) return;
    const isHttp = val.startsWith("http://") || val.startsWith("https://");
    const candidate: TorrentCandidate = {
      title: `${title} - Ep ${episodeNumber}`,
      group: null,
      resolution: null,
      size: 0,
      seeders: 1,
      leechers: 0,
      torrentUrl: isHttp ? val : null,
      magnetUri: !isHttp ? val : null,
      infoHash: null,
      publishedAt: null,
      source: "Manual",
      isBatch: false,
      score: 1000,
      refSubtitleUrl: null,
    };
    await startCandidate(candidate);
  }

  async function cancelActive() {
    if (activeSessionId) {
      await api.cancelTorrentSession(activeSessionId);
      activeSessionId = null;
    }
    onclose();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="backdrop"
  role="dialog"
  aria-modal="true"
  tabindex="-1"
  onclick={(e) => e.target === e.currentTarget && onclose()}
>
  <div class="modal card">
    <header class="modal-head">
      <div>
        <span class="eyebrow"
          >Torrent Stream & Download · Episode {episodeNumber}</span
        >
        <h2>{title}</h2>
      </div>
      <div class="head-actions">
        <button
          class="btn btn-ghost btn-sm"
          onclick={() => (showExtensions = !showExtensions)}
          title="Manage modular torrent extensions & indexers"
        >
          🧩 Extensions ({extensions.filter((e) => e.enabled).length})
        </button>
        <button
          class="btn btn-ghost btn-icon"
          onclick={onclose}
          aria-label="Close"
        >
          <Icon name="x" size={16} />
        </button>
      </div>
    </header>

    {#if showExtensions}
      <div class="ext-box">
        <div class="ext-box-head">
          <strong>Modular Torrent Extensions</strong>
          <small class="faint">
            Kura ships without built-in piracy indexers. Add an extension
            manifest URL (Hayase <code>index.json</code> or Kura JSON) or any RSS
            / API source URL.
          </small>
        </div>
        <form
          class="search-row"
          onsubmit={(e) => {
            e.preventDefault();
            void handleImportExtension();
          }}
        >
          <input
            class="input"
            placeholder="Paste extension manifest URL or source URL (e.g. https://…)"
            bind:value={extInput}
          />
          <button
            class="btn btn-primary btn-sm"
            type="submit"
            disabled={importingExt || !extInput.trim()}
          >
            {importingExt ? "Importing…" : "+ Add Extension"}
          </button>
        </form>
        {#if extensions.length > 0}
          <div class="ext-list">
            {#each extensions as ext (ext.id)}
              <div class="ext-item">
                <label class="ext-toggle">
                  <input
                    type="checkbox"
                    checked={ext.enabled}
                    onchange={() => void handleToggleExtension(ext)}
                  />
                  <strong>{ext.name}</strong>
                  <span class="badge">{ext.kind}</span>
                </label>
                <div class="ext-item-right">
                  <code class="faint ext-url">{ext.url}</code>
                  <button
                    class="btn btn-ghost btn-sm"
                    onclick={() => void handleRemoveExtension(ext.id)}
                    title="Remove extension"
                  >
                    ✕
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {#if liveSession}
      <div class="live-box">
        <div class="live-status-row">
          <div class="live-title">
            <span class="spinner"></span>
            <div>
              <strong>
                {#if liveSession.state === "buffering"}
                  Buffering Matroska headers & initial pieces for player…
                {:else if liveSession.state === "streaming"}
                  Streaming in external player…
                {:else if liveSession.state === "completed"}
                  Download complete!
                {:else if liveSession.state === "error"}
                  Torrent error
                {:else}
                  Downloading episode…
                {/if}
              </strong>
              <small class="faint">{liveSession.fileName}</small>
            </div>
          </div>
          <span class="badge badge-accent"
            >{liveSession.progressPct.toFixed(1)}%</span
          >
        </div>

        <div class="prog-bar">
          <div
            class="prog-fill"
            style:width={`${Math.max(3, liveSession.progressPct)}%`}
          ></div>
        </div>

        <div class="live-metrics">
          <span
            ><strong>{formatBytes(liveSession.downloadedBytes)}</strong> / {formatBytes(
              liveSession.totalBytes,
            )}</span
          >
          <span class="seeders"
            >↓ <strong>{formatBytes(liveSession.speedBps)}/s</strong></span
          >
          <span
            >↑ <strong>{formatBytes(liveSession.uploadSpeedBps ?? 0)}/s</strong
            ></span
          >
          <span><strong>{liveSession.peers}</strong> connected peers</span>
          {#if app.torrentAutoSubs && app.jimakuApiKey}
            <span class="badge badge-ok">Jimaku Auto-Retime Active</span>
          {/if}
        </div>

        {#if liveSession.error}
          <div class="err-box">{liveSession.error}</div>
        {/if}

        <div class="live-actions">
          <button class="btn btn-ghost" onclick={backToCandidateList}>
            ← Pick another torrent
          </button>
          {#if nextUntriedCandidate}
            <button
              class="btn btn-primary"
              disabled={starting}
              onclick={tryNextCandidate}
            >
              Try Next Release ⏭
            </button>
          {/if}
          <button class="btn btn-ghost" onclick={onclose}>
            Keep running in background
          </button>
          <button class="btn btn-danger" onclick={cancelActive}>
            Cancel torrent
          </button>
        </div>
      </div>
    {:else}
      <div class="controls-bar">
        <div
          class="mode-pills"
          role="radiogroup"
          aria-label="Torrent action mode"
        >
          <button
            class="mode-pill"
            class:on={chosenMode === "stream"}
            onclick={() => (chosenMode = "stream")}
            title="Stream into mpv/Memento using temporary cache (cleaned up after playback)"
          >
            Stream Only
          </button>
          <button
            class="mode-pill"
            class:on={chosenMode === "hybrid"}
            onclick={() => (chosenMode = "hybrid")}
            title="Play immediately in mpv/Memento while saving permanently into your Kura library"
          >
            Stream & Save to Library
          </button>
          <button
            class="mode-pill"
            class:on={chosenMode === "download"}
            onclick={() => (chosenMode = "download")}
            title="Download into your Kura library in the background"
          >
            Download Only
          </button>
        </div>

        <form
          class="search-row"
          onsubmit={(e) => {
            e.preventDefault();
            search(customQuery);
          }}
        >
          <input
            class="input"
            type="search"
            placeholder="Custom search query (e.g. SubsPlease 1080p)…"
            bind:value={customQuery}
          />
          <button class="btn btn-sm" type="submit" disabled={loading}>
            <Icon name="search" size={14} /> Search
          </button>
        </form>
      </div>

      {#if error}
        <div class="err-box">{error}</div>
      {/if}

      <div class="cand-list">
        {#if loading}
          <div class="center-box">
            <span class="spinner"></span>
            <span
              >Searching enabled extensions for Episode {episodeNumber}…</span
            >
          </div>
        {:else if extensions.filter((e) => e.enabled).length === 0}
          <div class="center-box">
            <strong>No torrent extensions enabled yet.</strong>
            <p class="faint">
              Paste an extension manifest or RSS/API source URL in the
              Extensions box above to enable searching.
            </p>
          </div>
        {:else if candidates.length === 0}
          <div class="center-box">
            <strong>No torrents found automatically.</strong>
            <p class="faint">
              Try a custom search above or paste a magnet / .torrent URL below.
            </p>
          </div>
        {:else}
          {#each candidates as c, i (c.infoHash ?? c.title + i)}
            <div class="cand-row" class:best={i === 0}>
              <div class="cand-info">
                <div class="cand-badges">
                  {#if i === 0}<span class="badge badge-ok">Best Match</span
                    >{/if}
                  {#if c.resolution}<span class="badge badge-accent"
                      >{c.resolution}</span
                    >{/if}
                  {#if c.group}<span class="badge">{c.group}</span>{/if}
                  {#if c.isBatch}<span class="badge badge-warn">Batch Pack</span
                    >{/if}
                  <span class="faint">{c.source}</span>
                </div>
                <strong class="cand-title" title={c.title}>{c.title}</strong>
                <div class="cand-meta">
                  {#if c.size > 0}<span>{formatBytes(c.size)}</span>{/if}
                  <span class="seeders">▲ {c.seeders} seeders</span>
                  <span>▼ {c.leechers} leechers</span>
                </div>
              </div>
              <button
                class="btn btn-primary btn-sm"
                disabled={starting}
                onclick={() => startCandidate(c)}
              >
                <Icon
                  name={chosenMode === "download" ? "download" : "play"}
                  size={13}
                  fill={chosenMode !== "download"}
                />
                {chosenMode === "download" ? "Download" : "Stream"}
              </button>
            </div>
          {/each}
        {/if}
      </div>

      <footer class="manual-foot">
        <input
          class="input"
          placeholder="Or paste a magnet:?xt=urn:btih:… or .torrent URL directly"
          bind:value={manualMagnet}
        />
        <button
          class="btn btn-sm"
          disabled={!manualMagnet.trim() || starting}
          onclick={startManualMagnet}
        >
          Load Link
        </button>
      </footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 120;
    background: rgba(10, 6, 8, 0.76);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }
  .modal {
    width: min(760px, 100%);
    max-height: 86vh;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 20px;
    background: var(--surface);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-xl);
    box-shadow: var(--sticker-lg);
  }
  .modal-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .eyebrow {
    font-size: 11px;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--coral);
  }
  .modal-head h2 {
    font-size: 20px;
    margin-top: 2px;
  }
  .controls-bar {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .mode-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .mode-pill {
    padding: 6px 12px;
    border-radius: 8px;
    border: 1.5px solid var(--border-strong);
    background: var(--bg-elev);
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 12.5px;
    font-weight: 700;
    cursor: pointer;
  }
  .mode-pill.on {
    background: var(--coral);
    color: var(--on-coral);
    border-color: var(--line);
    box-shadow: 2px 2px 0 var(--line);
  }
  .search-row {
    display: flex;
    gap: 8px;
  }
  .search-row .input {
    flex: 1;
  }
  .cand-list {
    flex: 1;
    min-height: 200px;
    max-height: 48vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-right: 4px;
  }
  .cand-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border-radius: var(--r-md);
    background: var(--bg-elev);
    border: 1.5px solid var(--border-strong);
  }
  .cand-row.best {
    border-color: var(--coral);
  }
  .cand-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .cand-badges {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    font-size: 11px;
  }
  .cand-title {
    font-size: 13px;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .cand-meta {
    display: flex;
    gap: 12px;
    font-size: 11.5px;
    color: var(--text-3);
  }
  .seeders {
    color: var(--mint);
    font-weight: 700;
  }
  .center-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 20px;
    text-align: center;
    color: var(--text-2);
  }
  .manual-foot {
    display: flex;
    gap: 8px;
    padding-top: 8px;
    border-top: 1.5px solid var(--border);
  }
  .manual-foot .input {
    flex: 1;
  }
  .live-box {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px;
    border-radius: var(--r-md);
    background: var(--bg-elev);
    border: 1.5px solid var(--line);
  }
  .live-status-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .live-title {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .live-title strong {
    display: block;
    font-size: 14px;
  }
  .prog-bar {
    height: 10px;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1.5px solid var(--line);
    overflow: hidden;
  }
  .prog-fill {
    height: 100%;
    background: var(--coral);
    transition: width 0.3s ease;
  }
  .live-metrics {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    font-size: 12.5px;
    color: var(--text-2);
  }
  .live-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .err-box {
    padding: 10px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--danger) 15%, transparent);
    border: 1.5px solid var(--danger);
    color: var(--danger);
    font-size: 12.5px;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ext-box {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    background: var(--bg-elev);
    border: 1.5px solid var(--border-strong);
  }
  .ext-box-head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 12.5px;
  }
  .ext-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 150px;
    overflow-y: auto;
  }
  .ext-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    font-size: 12px;
  }
  .ext-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .ext-item-right {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .ext-url {
    font-size: 11px;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
