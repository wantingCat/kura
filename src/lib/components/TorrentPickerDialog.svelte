<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    type MediaDetail,
    type RememberedBatch,
    type TorrentCandidate,
    type TorrentExtension,
    type TorrentFileEntry,
    type TorrentSessionStatus,
  } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { searchInstalledTorrentExtensions } from "$lib/extensions/host";
  import { formatBytes } from "$lib/format";
  import { getCachedOrLoadMediaDetail } from "$lib/onlinestream.svelte";
  import Icon from "./Icon.svelte";

  let {
    anilistId,
    title,
    epKey,
    episodeNumber,
    detail = null,
    autoStart = false,
    onclose,
    onswitchtoweb,
  }: {
    anilistId: number;
    title: string;
    epKey: string;
    episodeNumber: number;
    detail?: MediaDetail | null;
    autoStart?: boolean;
    onclose: () => void;
    onswitchtoweb?: () => void;
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

  // Batch memory & explicit batch file picker state
  let rememberedBatch = $state<RememberedBatch | null>(null);
  let expandedBatchKey = $state<string | null>(null);
  let loadingBatchFiles = $state(false);
  let batchFilesByKey = $state<Record<string, TorrentFileEntry[]>>({});

  const enabledJsTorrentCount = $derived(
    app.installedExtensions.filter((e) => e.enabled && e.extType === "anime-torrent-provider")
      .length,
  );
  const totalEnabledTorrentSources = $derived(
    extensions.filter((e) => e.enabled).length + enabledJsTorrentCount,
  );

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
      const [nativeRes, jsRes, remRes] = await Promise.allSettled([
        api.searchEpisodeTorrents(anilistId, episodeNumber, q || null),
        (async () => {
          const d = detail ?? (await getCachedOrLoadMediaDetail(anilistId));
          return searchInstalledTorrentExtensions(
            d,
            episodeNumber,
            app.torrentQuality,
            q || undefined,
          );
        })(),
        api.getRememberedBatch(anilistId),
      ]);

      if (remRes.status === "fulfilled") {
        rememberedBatch = remRes.value;
      }

      const mergedMap = new Map<string, TorrentCandidate>();
      const pushList = (items: TorrentCandidate[]) => {
        for (const item of items) {
          const k = candidateKey(item);
          const prev = mergedMap.get(k);
          if (!prev || item.score > prev.score) {
            mergedMap.set(k, item);
          }
        }
      };

      let nativeErr: string | null = null;
      if (nativeRes.status === "fulfilled") {
        pushList(nativeRes.value);
      } else {
        nativeErr = String(nativeRes.reason);
      }
      if (jsRes.status === "fulfilled") {
        pushList(jsRes.value);
      }

      const merged = Array.from(mergedMap.values()).sort((a, b) => b.score - a.score);
      // Mark best release if none marked yet
      if (merged.length > 0 && !merged.some((c) => c.isBestRelease)) {
        const top = merged.find((c) => c.seeders > 0) ?? merged[0];
        top.isBestRelease = true;
      }
      candidates = merged;

      if (merged.length === 0 && nativeErr) {
        if (nativeErr.includes("NO_TORRENT_EXTENSIONS") && enabledJsTorrentCount === 0) {
          showExtensions = true;
          error = null;
        } else if (!nativeErr.includes("NO_TORRENT_EXTENSIONS")) {
          error = nativeErr;
        }
      }

      if (!q && !customQuery) {
        customQuery = `${title} ${String(episodeNumber).padStart(2, "0")}`;
      }
      if (autoStart && !q && app.torrentQuality !== "ask" && candidates.length > 0) {
        const best =
          candidates.find((c) => c.isBestRelease && c.seeders > 0) ??
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
      if (msg.includes("NO_TORRENT_EXTENSIONS") && enabledJsTorrentCount === 0) {
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
      const [nativeList] = await Promise.all([
        api.listTorrentExtensions(),
        app.refreshExtensions(),
      ]);
      extensions = nativeList;
      if (nativeList.filter((e) => e.enabled).length === 0 && enabledJsTorrentCount === 0) {
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
      if (val.startsWith("{") || val.endsWith(".json") || val.includes("manifest") || val.includes("/blob/")) {
        try {
          const installed = await api.installExtension(val);
          await app.refreshExtensions();
          extInput = "";
          showExtensions = false;
          app.toast(`Installed "${installed.name}" (${installed.extType})`, "info", 4500);
          await search(customQuery || undefined);
          return;
        } catch (seErr) {
          const s = String(seErr);
          if (s.includes("UNSUPPORTED_SEANIME_TYPE") || s.includes("SEANIME_MANIFEST_ERR")) {
            throw new Error(s.replace(/^[A-Z_]+:\s*/, ""));
          }
        }
      }
      extensions = await api.importTorrentExtension(val);
      extInput = "";
      showExtensions = false;
      app.toast(
        `Enabled ${extensions.filter((e) => e.enabled).length} torrent extension(s)`,
        "info",
      );
      await search(customQuery || undefined);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
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

  async function startCandidate(c: TorrentCandidate, explicitFileIndex?: number | null) {
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
        fileIndex: explicitFileIndex ?? null,
      });
      activeSessionId = status.sessionId;
      if (chosenMode === "download") {
        app.toast(`Downloading ${c.title} in background…`, "info", 5000);
      }
    } catch (e) {
      error = String(e);
      starting = false;
      if (explicitFileIndex == null && failoverCount < 4 && nextUntriedCandidate) {
        failoverCount += 1;
        await startCandidate(nextUntriedCandidate);
        return;
      }
    } finally {
      starting = false;
    }
  }

  async function toggleBatchFiles(c: TorrentCandidate) {
    const key = candidateKey(c);
    if (expandedBatchKey === key) {
      expandedBatchKey = null;
      return;
    }
    expandedBatchKey = key;
    if (batchFilesByKey[key]) return;
    loadingBatchFiles = true;
    error = null;
    try {
      const files = await api.listTorrentFiles(c);
      batchFilesByKey = { ...batchFilesByKey, [key]: files };
    } catch (e) {
      error = String(e);
    } finally {
      loadingBatchFiles = false;
    }
  }

  async function forgetRememberedBatch() {
    try {
      await api.clearRememberedBatch(anilistId);
      rememberedBatch = null;
      app.toast("Cleared remembered batch torrent for this series", "info", 3500);
      await search(customQuery || undefined);
    } catch (e) {
      error = String(e);
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
        {#if onswitchtoweb}
          <button
            type="button"
            class="btn btn-ghost btn-sm"
            onclick={() => {
              onswitchtoweb();
              onclose();
            }}
            title="Switch to Direct Soft-Sub Web Stream for this episode"
          >
            Web Stream
          </button>
        {/if}
        <button
          class="btn btn-ghost btn-sm"
          onclick={() => (showExtensions = !showExtensions)}
          title="Manage modular torrent extensions & indexers"
        >
          🧩 Extensions ({totalEnabledTorrentSources})
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

      {#if rememberedBatch}
        <div class="remembered-batch-bar">
          <span class="faint">Remembered Batch:</span>
          <strong class="rem-batch-title" title={rememberedBatch.title}>{rememberedBatch.title}</strong>
          <button type="button" class="btn btn-ghost btn-sm" onclick={forgetRememberedBatch}>
            Forget Batch
          </button>
        </div>
      {/if}

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
        {:else if totalEnabledTorrentSources === 0}
          <div class="center-box">
            <strong>No torrent extensions enabled yet.</strong>
            <p class="faint">
              Paste an extension manifest or RSS/API source URL in the
              Extensions box above (or install a Seanime torrent extension in Settings) to enable searching.
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
            {@const cKey = candidateKey(c)}
            <div class="cand-card" class:best={Boolean(c.isBestRelease || i === 0)}>
              <div class="cand-row-inner">
                <div class="cand-info">
                  <div class="cand-badges">
                    {#if c.isBestRelease || i === 0}<span class="badge badge-ok">★ Best</span
                      >{/if}
                    {#if c.source === "Remembered Batch"}<span class="badge badge-ok"
                        >Remembered Batch</span
                      >{/if}
                    {#if c.resolution}<span class="badge badge-accent"
                        >{c.resolution}</span
                      >{/if}
                    {#if c.codec}<span class="badge">{c.codec}</span>{/if}
                    {#if c.bitDepth}<span class="badge">{c.bitDepth}</span>{/if}
                    {#if c.isDualAudio}<span class="badge badge-accent">Dual Audio</span>{/if}
                    {#if c.refSubtitleUrl}<span class="badge badge-ok" title="Direct subtitle reference track for instant Jimaku alignment">Fast Sub Sync</span>{/if}
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
                <div class="cand-actions">
                  {#if c.isBatch && (c.torrentUrl || c.magnetUri)}
                    <button
                      type="button"
                      class="btn btn-ghost btn-sm"
                      disabled={starting}
                      onclick={() => void toggleBatchFiles(c)}
                      title="Inspect files inside this batch torrent and choose a specific file"
                    >
                      {expandedBatchKey === cKey ? "Hide Files" : "Files…"}
                    </button>
                  {/if}
                  <button
                    class="btn btn-primary btn-sm"
                    disabled={starting}
                    onclick={() => void startCandidate(c)}
                  >
                    <Icon
                      name={chosenMode === "download" ? "download" : "play"}
                      size={13}
                      fill={chosenMode !== "download"}
                    />
                    {chosenMode === "download" ? "Download" : "Stream"}
                  </button>
                </div>
              </div>

              {#if expandedBatchKey === cKey}
                <div class="batch-files-panel">
                  {#if loadingBatchFiles && !batchFilesByKey[cKey]}
                    <div class="batch-files-loading">
                      <span class="spinner"></span>
                      <span class="faint">Fetching torrent file index…</span>
                    </div>
                  {:else if (batchFilesByKey[cKey] ?? []).filter((f) => f.isVideo).length === 0}
                    <small class="faint">Could not inspect individual video files (magnet-only metadata will resolve automatically on start).</small>
                  {:else}
                    {#each (batchFilesByKey[cKey] ?? []).filter((f) => f.isVideo) as f (f.index)}
                      <button
                        type="button"
                        class="batch-file-row"
                        class:matched={f.detectedEpisode === episodeNumber}
                        disabled={starting}
                        onclick={() => void startCandidate(c, f.index)}
                      >
                        <div class="batch-file-meta">
                          {#if f.detectedEpisode !== null}
                            <span class="badge" class:badge-ok={f.detectedEpisode === episodeNumber}>
                              Ep {f.detectedEpisode}
                            </span>
                          {/if}
                          <span class="batch-file-name" title={f.name}>{f.name.split(/[\\/]/).pop() ?? f.name}</span>
                        </div>
                        <span class="faint">{formatBytes(f.length)}</span>
                      </button>
                    {/each}
                  {/if}
                </div>
              {/if}
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
    z-index: 950;
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
    box-shadow: var(--shadow-sm);
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
  .cand-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border-radius: var(--r-md);
    background: var(--bg-elev);
    border: 1.5px solid var(--border-strong);
  }
  .cand-card.best {
    border-color: var(--coral);
  }
  .cand-row-inner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .cand-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .remembered-batch-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--bg-elev);
    border: 1px solid var(--border-strong);
    font-size: 12px;
  }
  .rem-batch-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .batch-files-panel {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-top: 8px;
    border-top: 1px solid var(--line);
    max-height: 170px;
    overflow-y: auto;
  }
  .batch-files-loading {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 4px;
    font-size: 12px;
  }
  .batch-file-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 6px 10px;
    border-radius: 6px;
    background: var(--surface);
    border: 1px solid var(--line);
    color: var(--text);
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .batch-file-row:hover,
  .batch-file-row.matched {
    border-color: var(--coral);
  }
  .batch-file-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1;
  }
  .batch-file-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    border-top: 1px solid var(--line);
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
    border: 1px solid transparent;
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
    border: 1px solid transparent;
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

  /* Tokyo Night pass: glass surface + neon focus ------------------------ */
  .modal {
    background: var(--glass-tn-strong);
    backdrop-filter: var(--blur-tn);
    -webkit-backdrop-filter: var(--blur-tn);
    border: 1px solid var(--glass-hairline);
  }
  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--neon-violet);
    outline-offset: 2px;
  }
</style>
