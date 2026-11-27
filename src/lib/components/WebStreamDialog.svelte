<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    api,
    type InstalledExtension,
    type MarketplaceExtension,
    type MediaDetail,
  } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import type { OnlinestreamSearchResult } from "$lib/extensions/host";
  import { cleanExtensionError, installUnifiedExtension } from "$lib/extensions/install";
  import {
    invalidateOnlinestreamCache,
    launchResolvedWebOption,
    listEnabledOnlinestreamProviders,
    resolveEpisodeOptionsForProvider,
    searchProviderAnime,
    type ResolvedWebStreamOption,
  } from "$lib/onlinestream.svelte";
  import Icon from "./Icon.svelte";

  let {
    detail,
    epKey,
    episodeNumber,
    onclose,
    onswitchtotorrent,
  }: {
    detail: MediaDetail;
    epKey: string;
    episodeNumber: number;
    onclose: () => void;
    onswitchtotorrent?: () => void;
  } = $props();

  let providers = $state<InstalledExtension[]>([]);
  let selectedProviderId = $state("");
  let loading = $state(true);
  let launchingKey = $state<string | null>(null);
  let error = $state<string | null>(null);

  let matchedId = $state("");
  let matchedTitle = $state("");
  let hasManualMapping = $state(false);
  let options = $state<ResolvedWebStreamOption[]>([]);
  let hardsubSkippedCount = $state(0);

  // Rematch inline state
  let showRematch = $state(false);
  let rematchQuery = $state("");
  let rematchLoading = $state(false);
  let rematchResults = $state<OnlinestreamSearchResult[]>([]);

  // Inline quick extension install state (when 0 providers installed)
  let showInstallBox = $state(false);
  let extSourceInput = $state("");
  let installingExt = $state(false);
  let catalogChoices = $state<MarketplaceExtension[]>([]);
  let reqSeq = 0;
  let rematchSeq = 0;

  const activeProvider = $derived<InstalledExtension | null>(
    providers.find((p) => p.id === selectedProviderId) ?? providers[0] ?? null,
  );

  const displayTitle = $derived(
    detail.titleEnglish ?? detail.titleRomaji ?? `Anime #${detail.anilistId}`,
  );

  async function initProvidersAndLoad(forceRefresh = false) {
    const seq = ++reqSeq;
    loading = true;
    error = null;
    try {
      const nextProviders = await listEnabledOnlinestreamProviders();
      await app.refreshExtensions();
      if (seq !== reqSeq) return;
      providers = nextProviders;
      if (providers.length === 0) {
        showInstallBox = true;
        options = [];
        loading = false;
        return;
      }
      if (!selectedProviderId || !providers.some((p) => p.id === selectedProviderId)) {
        selectedProviderId = providers[0].id;
      }
      await loadForSelectedProvider(forceRefresh, seq);
    } catch (e) {
      if (seq === reqSeq) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === reqSeq) loading = false;
    }
  }

  async function loadForSelectedProvider(forceRefresh = false, parentSeq?: number) {
    const ext = providers.find((p) => p.id === selectedProviderId) ?? providers[0];
    if (!ext) return;
    const seq = parentSeq ?? ++reqSeq;
    loading = true;
    error = null;
    try {
      const manual = await api.getOnlinestreamMapping(ext.id, detail.anilistId);
      if (seq !== reqSeq) return;
      hasManualMapping = Boolean(manual?.providerAnimeId);
      const res = await resolveEpisodeOptionsForProvider(
        ext,
        detail,
        episodeNumber,
        app.streamSoftSubOnly,
        forceRefresh,
      );
      if (seq !== reqSeq) return;
      matchedId = res.matchedId;
      matchedTitle = res.matchedTitle;
      options = res.options;
      hardsubSkippedCount = res.hardsubSkippedCount;
      if (!rematchQuery) {
        rematchQuery = detail.titleRomaji ?? detail.titleEnglish ?? "";
      }
    } catch (e) {
      if (seq === reqSeq) {
        options = [];
        error = e instanceof Error ? e.message : String(e);
      }
    } finally {
      if (seq === reqSeq) loading = false;
    }
  }

  async function handleToggleSoftSubOnly(next: boolean) {
    await app.setStreamSoftSubOnly(next);
    await loadForSelectedProvider(false);
  }

  async function handleRematchSearch() {
    if (!activeProvider) return;
    const seq = ++rematchSeq;
    rematchLoading = true;
    error = null;
    try {
      const res = await searchProviderAnime(
        activeProvider,
        detail,
        rematchQuery.trim() || undefined,
      );
      if (seq === rematchSeq) rematchResults = res;
    } catch (e) {
      if (seq === rematchSeq) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === rematchSeq) rematchLoading = false;
    }
  }

  async function applyManualMatch(item: OnlinestreamSearchResult) {
    if (!activeProvider) return;
    try {
      await api.setOnlinestreamMapping(
        activeProvider.id,
        detail.anilistId,
        item.id,
        item.title,
      );
      invalidateOnlinestreamCache(detail.anilistId);
      showRematch = false;
      app.toast(`Mapped to "${item.title}" on ${activeProvider.name}`, "info", 4000);
      await loadForSelectedProvider(true);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function clearManualMatch() {
    if (!activeProvider) return;
    try {
      await api.removeOnlinestreamMapping(activeProvider.id, detail.anilistId);
      invalidateOnlinestreamCache(detail.anilistId);
      hasManualMapping = false;
      showRematch = false;
      app.toast("Reset to automatic title matching", "info", 3500);
      await loadForSelectedProvider(true);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleQuickInstallExtension(manifestOverride?: string) {
    const val = (manifestOverride ?? extSourceInput).trim();
    if (!val || installingExt) return;
    installingExt = true;
    error = null;
    try {
      const result = await installUnifiedExtension(val, {
        preferredExtType: "onlinestream-provider",
        allowNativeTorrentFallback: false,
      });
      if (result.kind === "marketplace_catalog") {
        catalogChoices = result.items;
        extSourceInput = "";
        app.toast(`Found ${result.items.length} web stream extension(s) — select one to install`, "info", 4000);
        return;
      }
      if (result.kind === "seanime_installed") {
        const installed = result.extension;
        extSourceInput = "";
        catalogChoices = [];
        showInstallBox = false;
        app.toast(`Installed extension "${installed.name}"`, "info", 4500);
        selectedProviderId = installed.id;
        await initProvidersAndLoad(true);
      }
    } catch (e) {
      error = cleanExtensionError(e);
    } finally {
      installingExt = false;
    }
  }

  async function playOption(opt: ResolvedWebStreamOption, idx: number) {
    if (launchingKey) return;
    const key = `${opt.server}:${opt.quality}:${idx}`;
    launchingKey = key;
    error = null;
    try {
      await launchResolvedWebOption(detail.anilistId, epKey, opt);
      app.toast(
        `Streaming Ep ${episodeNumber} via ${opt.providerName} · ${opt.server}`,
        "info",
        4000,
        detail.anilistId,
      );
      onclose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      launchingKey = null;
    }
  }

  function handleWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopImmediatePropagation();
      onclose();
    }
  }

  onMount(() => {
    void initProvidersAndLoad();
  });
</script>

<svelte:window onkeydown={handleWindowKeydown} />

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
        <span class="eyebrow">Direct Web Stream · Episode {episodeNumber}</span>
        <h2>{displayTitle}</h2>
      </div>
      <div class="head-actions">
        {#if onswitchtotorrent}
          <button
            type="button"
            class="btn btn-ghost btn-sm"
            onclick={() => {
              onswitchtotorrent();
              onclose();
            }}
            title="Switch to BitTorrent Swarm picker for this episode"
          >
            Magnet / Torrents
          </button>
        {/if}
        <button
          type="button"
          class="btn btn-ghost btn-sm"
          onclick={() => (showInstallBox = !showInstallBox)}
          title="Install or manage Seanime web streaming extensions"
        >
          Extensions ({providers.length})
        </button>
        <button
          type="button"
          class="btn btn-ghost btn-icon"
          onclick={onclose}
          aria-label="Close"
        >
          <Icon name="x" size={16} />
        </button>
      </div>
    </header>

    {#if showInstallBox}
      <div class="ext-box">
        <div class="ext-box-head">
          <strong>Seanime Web Stream Extensions (<code>onlinestream-provider</code>)</strong>
          <small class="faint">
            Kura ships clean with zero hardcoded streaming scrapers. Paste any Seanime-compatible
            <code>onlinestream-provider</code> manifest URL (<code>.json</code>) or manage marketplaces in
            <button
              type="button"
              class="inline-link"
              onclick={() => {
                onclose();
                void goto("/settings");
              }}
            >Settings → Extensions</button>.
          </small>
        </div>
        <form
          class="search-row"
          onsubmit={(e) => {
            e.preventDefault();
            void handleQuickInstallExtension();
          }}
        >
          <input
            class="input"
            placeholder="Paste Seanime manifest URL or GitHub repo URL (https://…)"
            bind:value={extSourceInput}
          />
          <button
            class="btn btn-primary btn-sm"
            type="submit"
            disabled={installingExt || !extSourceInput.trim()}
          >
            {installingExt ? "Installing…" : "+ Install"}
          </button>
        </form>
        {#if catalogChoices.length > 0}
          <div class="rematch-list">
            {#each catalogChoices as item (item.id + item.manifestUri)}
              <div class="rematch-item">
                <span class="rematch-item-title">{item.name}</span>
                <button
                  type="button"
                  class="btn btn-primary btn-sm"
                  disabled={installingExt || !item.manifestUri}
                  onclick={() => void handleQuickInstallExtension(item.manifestUri)}
                >
                  Install
                </button>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {#if providers.length > 0}
      <div class="toolbar-bar">
        <div class="provider-row">
          <label class="provider-select-wrap">
            <span class="faint label-sm">Provider</span>
            <select
              class="input select-sm"
              bind:value={selectedProviderId}
              onchange={() => {
                showRematch = false;
                void loadForSelectedProvider(false);
              }}
            >
              {#each providers as p (p.id)}
                <option value={p.id}>{p.name} (v{p.version})</option>
              {/each}
            </select>
          </label>

          {#if matchedTitle}
            <div class="match-pill" title={`Provider ID: ${matchedId}`}>
              <span class="faint">Matched:</span>
              <strong class="match-title">{matchedTitle}</strong>
              {#if hasManualMapping}
                <span class="badge badge-accent">Pinned</span>
              {/if}
            </div>
          {/if}

          <div class="match-actions">
            <button
              type="button"
              class="btn btn-ghost btn-sm"
              onclick={() => {
                showRematch = !showRematch;
                if (showRematch && rematchResults.length === 0) {
                  void handleRematchSearch();
                }
              }}
            >
              {showRematch ? "Hide Rematch" : "Rematch"}
            </button>
            <button
              type="button"
              class="btn btn-ghost btn-sm"
              disabled={loading}
              onclick={() => void loadForSelectedProvider(true)}
              title="Refresh servers & stream links"
            >
              ↻
            </button>
          </div>
        </div>

        <div class="filter-row">
          <label class="softsub-toggle">
            <input
              type="checkbox"
              checked={app.streamSoftSubOnly}
              onchange={(e) =>
                void handleToggleSoftSubOnly((e.currentTarget as HTMLInputElement).checked)}
            />
            <span>Soft-Sub Only (clean video + external <code>.vtt</code> for Jimaku sync)</span>
          </label>
          {#if app.jimakuApiKey}
            <span class="badge badge-ok">Jimaku VTT Auto-Retime Ready</span>
          {/if}
        </div>
      </div>

      {#if !app.streamSoftSubOnly}
        <div class="warn-box">
          Hardsub fallback is enabled: streams marked <strong>Hardsub</strong> have burned-in subtitles
          baked into the video frames, which may sit underneath Kura's interactive Japanese subtitles.
        </div>
      {/if}

      {#if showRematch}
        <div class="rematch-box">
          <div class="rematch-head">
            <strong>Manual Provider Match ({activeProvider?.name})</strong>
            {#if hasManualMapping}
              <button type="button" class="btn btn-ghost btn-sm" onclick={clearManualMatch}>
                Reset to Auto-Match
              </button>
            {/if}
          </div>
          <form
            class="search-row"
            onsubmit={(e) => {
              e.preventDefault();
              void handleRematchSearch();
            }}
          >
            <input
              class="input"
              type="search"
              placeholder="Search anime title on provider…"
              bind:value={rematchQuery}
            />
            <button class="btn btn-sm" type="submit" disabled={rematchLoading}>
              <Icon name="search" size={14} /> {rematchLoading ? "Searching…" : "Search"}
            </button>
          </form>
          {#if rematchResults.length > 0}
            <div class="rematch-list">
              {#each rematchResults as item (item.id)}
                <button
                  type="button"
                  class="rematch-item"
                  class:selected={item.id === matchedId}
                  onclick={() => void applyManualMatch(item)}
                >
                  <span class="rematch-item-title">{item.title}</span>
                  <span class="badge">{item.subOrDub || "sub"}</span>
                </button>
              {/each}
            </div>
          {:else if !rematchLoading}
            <small class="faint">No search results yet.</small>
          {/if}
        </div>
      {/if}
    {/if}

    {#if error}
      <div class="err-box">{error}</div>
    {/if}

    <div class="cand-list">
      {#if loading}
        <div class="center-box">
          <span class="spinner"></span>
          <span>Resolving clean soft-sub servers for Episode {episodeNumber}…</span>
        </div>
      {:else if providers.length === 0}
        <div class="center-box">
          <strong>No Web Streaming extensions installed yet.</strong>
          <p class="faint">
            Paste a Seanime <code>onlinestream-provider</code> manifest URL above, or open
            Settings → Extensions to browse an extension marketplace.
          </p>
        </div>
      {:else if options.length === 0}
        <div class="center-box">
          <strong>No matching web streams available.</strong>
          {#if app.streamSoftSubOnly && hardsubSkippedCount > 0}
            <p class="faint">
              Skipped <strong>{hardsubSkippedCount}</strong> stream(s) because they only had burned-in
              hardsubs. Uncheck <em>Soft-Sub Only</em> above to allow hardsub fallback, or try
              Rematch / Torrents.
            </p>
          {:else}
            <p class="faint">
              Try clicking <strong>Rematch</strong> if the provider matched the wrong season, or switch
              to Magnet / Torrents.
            </p>
          {/if}
        </div>
      {:else}
        {#each options as opt, i (`${opt.server}:${opt.quality}:${i}`)}
          {@const rowKey = `${opt.server}:${opt.quality}:${i}`}
          <div class="cand-row" class:best={i === 0}>
            <div class="cand-info">
              <div class="cand-badges">
                {#if i === 0}
                  <span class="badge badge-ok">★ Recommended</span>
                {/if}
                {#if opt.isSoftSub}
                  <span class="badge badge-ok">Soft-Sub (.vtt)</span>
                {:else}
                  <span class="badge badge-warn">Hardsub</span>
                {/if}
                <span class="badge badge-accent">{opt.quality.toUpperCase()}</span>
                <span class="badge">{opt.streamType.toUpperCase()}</span>
                <span class="faint">{opt.server}</span>
              </div>
              <strong class="cand-title">{opt.label}</strong>
              <div class="cand-meta">
                <span>Provider: <strong>{opt.providerName}</strong></span>
                {#if opt.subtitles.length > 0}
                  <span class="subs-list">
                    Tracks: {opt.subtitles.map((s) => s.label || s.language).slice(0, 5).join(", ")}
                    {opt.subtitles.length > 5 ? ` +${opt.subtitles.length - 5}` : ""}
                  </span>
                {:else}
                  <span>No external subtitle tracks</span>
                {/if}
              </div>
            </div>
            <button
              type="button"
              class="btn btn-primary btn-sm"
              disabled={launchingKey !== null}
              onclick={() => void playOption(opt, i)}
            >
              <Icon name="play" size={13} fill />
              {launchingKey === rowKey ? "Starting…" : "Play"}
            </button>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 950;
    background: rgba(10, 6, 8, 0.78);
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
    gap: 12px;
    padding: 20px;
    background: var(--glass-tn-strong);
    backdrop-filter: var(--blur-tn);
    -webkit-backdrop-filter: var(--blur-tn);
    border: 1px solid var(--glass-hairline);
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
  .head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .toolbar-bar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border-radius: var(--r-md);
    background: var(--bg-elev);
    border: 1.5px solid var(--border-strong);
  }
  .provider-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
  }
  .provider-select-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .label-sm {
    font-size: 12px;
    font-weight: 600;
  }
  .select-sm {
    padding: 5px 10px;
    font-size: 12.5px;
    min-width: 170px;
  }
  .match-pill {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    min-width: 0;
    flex: 1;
  }
  .match-title {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }
  .match-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }
  .filter-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
    padding-top: 6px;
    border-top: 1px solid var(--line);
  }
  .softsub-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text-2);
    cursor: pointer;
  }
  .ext-box,
  .rematch-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    background: var(--bg-elev);
    border: 1.5px solid var(--border-strong);
  }
  .ext-box-head {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 12.5px;
  }
  .inline-link {
    background: none;
    border: none;
    padding: 0;
    color: var(--coral);
    font-weight: 700;
    cursor: pointer;
    text-decoration: underline;
  }
  .rematch-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12.5px;
  }
  .search-row {
    display: flex;
    gap: 8px;
  }
  .search-row .input {
    flex: 1;
  }
  .rematch-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 150px;
    overflow-y: auto;
  }
  .rematch-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 7px 10px;
    border-radius: 8px;
    background: var(--surface);
    border: 1px solid var(--line);
    color: var(--text);
    font-size: 12.5px;
    text-align: left;
    cursor: pointer;
  }
  .rematch-item:hover,
  .rematch-item.selected {
    border-color: var(--coral);
  }
  .rematch-item-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cand-list {
    flex: 1;
    min-height: 190px;
    max-height: 46vh;
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
    flex-wrap: wrap;
    gap: 12px;
    font-size: 11.5px;
    color: var(--text-3);
  }
  .subs-list {
    color: var(--mint);
  }
  .center-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 36px 20px;
    text-align: center;
    color: var(--text-2);
  }
  .err-box {
    padding: 10px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--danger) 15%, transparent);
    border: 1.5px solid var(--danger);
    color: var(--danger);
    font-size: 12.5px;
  }
  .warn-box {
    padding: 8px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--coral) 12%, transparent);
    border: 1px solid var(--coral);
    color: var(--text-2);
    font-size: 12px;
  }

  /* Tokyo Night pass: neon focus */
  button:focus-visible,
  input:focus-visible,
  select:focus-visible {
    outline: 2px solid var(--neon-violet);
    outline-offset: 2px;
  }
</style>
