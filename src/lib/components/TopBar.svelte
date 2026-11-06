<script lang="ts">
  import { tick } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { app } from "$lib/app.svelte";
  import { api, img, type SearchResult } from "$lib/api";
  import Icon from "./Icon.svelte";

  const PHASES: Record<string, string> = {
    discover: "Scanning folders",
    match: "Identifying anime",
    resolve: "Mapping episodes",
    metadata: "Fetching metadata",
    artwork: "Downloading artwork",
    done: "Up to date",
  };

  const path = $derived(page.url.pathname);
  const pct = $derived(
    app.progress && app.progress.total > 0 ? Math.min(100, (app.progress.current / app.progress.total) * 100) : null,
  );
  const playing = $derived(app.nowPlaying);
  const playingCard = $derived(playing ? app.cards.find((c) => c.anilistId === playing.anilistId) : null);
  const activeTorrent = $derived(
    app.torrentSessions.find((s) => s.state === "buffering" || s.state === "downloading" || s.state === "resolving") ?? null,
  );
  const unreadAlertsCount = $derived(app.alerts.filter((a) => !a.read).length);

  let alertsOpen = $state(false);
  let searchOpen = $state(false);
  let searchEl: HTMLInputElement | undefined = $state();
  let globalQuery = $state("");
  let remoteResults = $state<SearchResult[]>([]);
  let remoteLoading = $state(false);
  let selectedIndex = $state(0);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let searchSeq = 0;

  const libraryMatches = $derived.by(() => {
    const q = globalQuery.trim().toLowerCase();
    if (!q) {
      return app.cards.slice(0, 6);
    }
    return app.cards
      .filter((c) => {
        const en = c.titleEnglish?.toLowerCase() ?? "";
        const ro = c.titleRomaji?.toLowerCase() ?? "";
        const na = c.titleNative?.toLowerCase() ?? "";
        const syn = c.synonyms?.some((s) => s.toLowerCase().includes(q)) ?? false;
        return en.includes(q) || ro.includes(q) || na.includes(q) || syn;
      })
      .slice(0, 8);
  });

  const anilistMatches = $derived.by(() => {
    const libIds = new Set(libraryMatches.map((c) => c.anilistId));
    return remoteResults.filter((r) => !libIds.has(r.anilistId));
  });

  const combinedNavIds = $derived([
    ...libraryMatches.map((c) => c.anilistId),
    ...anilistMatches.map((r) => r.anilistId),
  ]);

  async function openSearch() {
    searchOpen = true;
    selectedIndex = 0;
    await tick();
    searchEl?.focus();
    searchEl?.select();
  }

  function closeSearch() {
    searchOpen = false;
    remoteLoading = false;
    if (debounceTimer) clearTimeout(debounceTimer);
  }

  async function toggleSearch() {
    if (searchOpen) {
      closeSearch();
    } else {
      await openSearch();
    }
  }

  function onSearchInput() {
    selectedIndex = 0;
    const q = globalQuery.trim();
    if (debounceTimer) clearTimeout(debounceTimer);
    if (q.length < 2) {
      remoteResults = [];
      remoteLoading = false;
      return;
    }
    remoteLoading = true;
    const mySeq = ++searchSeq;
    debounceTimer = setTimeout(async () => {
      try {
        const res = await api.searchAnilist(q);
        if (mySeq === searchSeq) {
          remoteResults = res;
        }
      } catch {
        if (mySeq === searchSeq) {
          remoteResults = [];
        }
      } finally {
        if (mySeq === searchSeq) {
          remoteLoading = false;
        }
      }
    }, 180);
  }

  function selectAnime(anilistId: number) {
    closeSearch();
    goto(`/anime/${anilistId}`);
  }

  async function onKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      if (searchOpen) {
        closeSearch();
      } else {
        await openSearch();
      }
    } else if (e.key === "Escape" && searchOpen) {
      e.preventDefault();
      closeSearch();
    }
  }

  function onModalKeydown(e: KeyboardEvent) {
    const total = combinedNavIds.length;
    if (e.key === "Escape") {
      e.preventDefault();
      closeSearch();
    } else if (e.key === "ArrowDown" && total > 0) {
      e.preventDefault();
      selectedIndex = (selectedIndex + 1) % total;
    } else if (e.key === "ArrowUp" && total > 0) {
      e.preventDefault();
      selectedIndex = (selectedIndex - 1 + total) % total;
    } else if (e.key === "Enter" && total > 0) {
      e.preventDefault();
      const targetId = combinedNavIds[selectedIndex] ?? combinedNavIds[0];
      if (targetId) selectAnime(targetId);
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<aside class="nav-rail" aria-label="Main navigation">
  <!-- Hanko Brand Seal -->
  <div class="rail-top">
    <a
      class="brand-seal"
      href="/"
      aria-label="Kura Library"
      id="nav-home"
      title="Kura · Library"
      onclick={() => (app.query = "")}
    >
      <img class="brand-mark" src="/brand/logo.png" alt="Kura" width="38" height="38" />
    </a>
  </div>

  <!-- Primary Vertical Navigation -->
  <nav class="rail-nav" aria-label="Primary">
    <a
      class="rail-item"
      class:active={path === "/" && !searchOpen}
      href="/"
      title="Library"
      aria-label="Library"
    >
      <Icon name="library" size={19} />
      <span class="rail-label">Library</span>
    </a>

    <button
      type="button"
      class="rail-item"
      class:active={searchOpen}
      onclick={toggleSearch}
      title="Global Search (Ctrl+K)"
      aria-label="Global Search"
      id="nav-search-btn"
    >
      <Icon name="search" size={19} />
      <span class="rail-label">Search</span>
    </button>

    {#if app.appMode === "streaming"}
      <a
        class="rail-item"
        class:active={path.startsWith("/discover")}
        href="/discover"
        id="nav-discover"
        title="Discover Catalog & Voice Actors"
        aria-label="Discover"
      >
        <Icon name="sparkles" size={19} />
        <span class="rail-label">Discover</span>
      </a>
    {/if}

    <a
      class="rail-item"
      class:active={path.startsWith("/calendar")}
      href="/calendar"
      id="nav-calendar"
      title="Seasonal Airing Calendar"
      aria-label="Airing Calendar"
    >
      <Icon name="calendar" size={19} />
      <span class="rail-label">Schedule</span>
    </a>

    {#if app.reviewCount > 0}
      <a
        class="rail-item warn"
        class:active={path.startsWith("/review")}
        href="/review"
        id="nav-review"
        title={`${app.reviewCount} unmatched folders need review`}
        aria-label="Review unmatched folders"
      >
        <Icon name="alert" size={18} />
        <span class="rail-label">Review</span>
        <span class="rail-badge">{app.reviewCount}</span>
      </a>
    {/if}
  </nav>

  <!-- Bottom Utilities & Status -->
  <div class="rail-bottom">
    {#if activeTorrent && !playing}
      <div class="status-pop" id="now-torrent">
        <a class="rail-status torrent" href={`/anime/${activeTorrent.anilistId}`} title={`${activeTorrent.title} · Ep ${activeTorrent.epKey} (${Math.round(activeTorrent.progressPct)}%)`}>
          <span class="spinner spinner-sm"></span>
          <span class="pct-mini">{Math.round(activeTorrent.progressPct)}%</span>
        </a>
        <button
          type="button"
          class="cancel-mini"
          title="Cancel torrent stream"
          aria-label="Cancel torrent stream"
          onclick={async () => {
            const id = activeTorrent.sessionId;
            app.torrentSessions = app.torrentSessions.filter((s) => s.sessionId !== id);
            try {
              const { api } = await import("$lib/api");
              await api.cancelTorrentSession(id);
            } catch {}
          }}
        >
          ×
        </button>
      </div>
    {/if}

    {#if playing}
      <a
        class="rail-status playing"
        href={`/anime/${playing.anilistId}`}
        id="now-playing"
        title={playingCard ? `Now Playing: ${playingCard.titleEnglish ?? playingCard.titleRomaji} · Ep ${playing.epKey}` : `Now Playing · Ep ${playing.epKey}`}
      >
        <span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>
        <span class="pct-mini">Ep {playing.epKey}</span>
      </a>
    {/if}

    {#if app.progress}
      <div
        class="rail-status scan"
        class:done={!app.scanning}
        title={`${PHASES[app.progress.phase] ?? app.progress.phase}: ${app.progress.message}`}
      >
        {#if app.scanning}
          <span class="spinner spinner-sm"></span>
          {#if pct !== null}
            <span class="pct-mini">{Math.round(pct)}%</span>
          {/if}
        {:else}
          <span class="ok-dot"><Icon name="check" size={12} stroke={3} /></span>
        {/if}
      </div>
    {:else if app.libraries.length > 0}
      <button
        type="button"
        class="rail-icon-btn"
        onclick={() => app.scan()}
        id="nav-rescan"
        title="Rescan local library folders"
        aria-label="Rescan libraries"
      >
        <Icon name="refresh" size={17} />
      </button>
    {/if}

    {#if app.anilist?.connected}
      <a
        class="rail-avatar"
        href="/settings"
        onclick={() => {
          try {
            localStorage.setItem("kura.settings.category", "integrations");
          } catch {}
        }}
        id="nav-anilist"
        title={`AniList connected as ${app.anilist.userName ?? "user"} (${app.anilist.trackedCount} synced)`}
      >
        {#if app.anilist.avatarUrl}
          <img src={app.anilist.avatarUrl} alt="" />
        {:else}
          <Icon name="user" size={15} />
        {/if}
        <span class="online-dot"></span>
      </a>
    {/if}

    <button
      type="button"
      class="rail-icon-btn bell-btn"
      class:active={alertsOpen}
      onclick={() => {
        alertsOpen = !alertsOpen;
        if (alertsOpen && unreadAlertsCount > 0) {
          app.markAllAlertsRead();
        }
      }}
      id="nav-notifications"
      title="Episode & Jimaku JP Subtitle Notifications"
      aria-label="Notifications"
    >
      <Icon name="bell" size={18} />
      {#if unreadAlertsCount > 0}
        <span class="bell-badge">{unreadAlertsCount > 9 ? "9+" : unreadAlertsCount}</span>
      {/if}
    </button>

    <a
      class="rail-icon-btn"
      class:active={path.startsWith("/settings")}
      href="/settings"
      id="nav-settings"
      title="Settings"
      aria-label="Settings"
    >
      <Icon name="settings" size={18} />
    </a>
  </div>
</aside>

{#if alertsOpen}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="alerts-backdrop" onclick={() => (alertsOpen = false)}>
    <div
      class="alerts-popover"
      role="dialog"
      tabindex="-1"
      aria-label="Episode and Jimaku subtitle notifications"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="alerts-head">
        <div class="alerts-title">
          <Icon name="bell" size={15} />
          <span>Episode &amp; JP Sub Alerts</span>
        </div>
        {#if app.alerts.length > 0}
          <button
            type="button"
            class="alerts-clear"
            onclick={() => app.clearAlerts()}
          >
            Clear
          </button>
        {/if}
      </div>

      <div class="alerts-list">
        {#if app.alerts.length === 0}
          <div class="alerts-empty">
            No notifications yet. Kura automatically checks your <strong>Watching</strong> list for newly aired episodes and Jimaku JP subtitles.
          </div>
        {:else}
          {#each app.alerts as alert (alert.id)}
            <button
              type="button"
              class="alert-row"
              onclick={() => {
                alertsOpen = false;
                goto(`/anime/${alert.anilistId}`);
              }}
            >
              <div class="alert-kind-badge" class:sub={alert.kind === "jimaku_sub" || alert.kind === "episode_and_sub"}>
                {#if alert.kind === "jimaku_sub"}
                  日本語 SUB
                {:else if alert.kind === "episode_and_sub"}
                  EP + SUB
                {:else}
                  NEW EP
                {/if}
              </div>
              <div class="alert-body">
                <div class="alert-show">{alert.title} · Ep {alert.episode}</div>
                <div class="alert-msg">{alert.message}</div>
              </div>
            </button>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<!-- Global Search Modal (Library First, then All of AniList) -->
{#if searchOpen}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="search-backdrop" onclick={closeSearch}>
    <div
      class="search-palette"
      role="dialog"
      tabindex="-1"
      aria-label="Global anime search"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="search-header">
        <span class="search-icon"><Icon name="search" size={18} /></span>
        <input
          bind:this={searchEl}
          bind:value={globalQuery}
          oninput={onSearchInput}
          onkeydown={onModalKeydown}
          class="search-input"
          placeholder="Search your library or all of AniList (English, Romaji, 日本語)…"
          id="library-search"
          aria-label="Global anime search"
        />
        {#if remoteLoading}
          <span class="spinner spinner-sm"></span>
        {/if}
        {#if globalQuery}
          <button
            type="button"
            class="search-clear"
            onclick={() => {
              globalQuery = "";
              remoteResults = [];
              selectedIndex = 0;
              searchEl?.focus();
            }}
            aria-label="Clear search"
          >
            <Icon name="x" size={13} />
          </button>
        {/if}
        <button
          type="button"
          class="search-close"
          onclick={closeSearch}
          title="Close search (Esc)"
        >
          Esc
        </button>
      </div>

      <div class="search-body">
        {#if libraryMatches.length > 0}
          <div class="search-section">
            <div class="section-heading">
              <span>{globalQuery.trim() ? "In Your Library" : "Recent in Library"}</span>
              <span class="section-count">{libraryMatches.length}</span>
            </div>
            <div class="result-list">
              {#each libraryMatches as item, idx (item.anilistId)}
                {@const cover = img(item.coverPath, item.coverUrl)}
                {@const title = item.titleEnglish ?? item.titleRomaji ?? item.titleNative ?? "Untitled"}
                {@const sub = item.titleNative ?? item.titleRomaji}
                <button
                  type="button"
                  class="result-row"
                  class:selected={selectedIndex === idx}
                  onmouseenter={() => (selectedIndex = idx)}
                  onclick={() => selectAnime(item.anilistId)}
                >
                  <div class="res-poster">
                    {#if cover}
                      <img src={cover} alt="" loading="lazy" />
                    {/if}
                  </div>
                  <div class="res-meta">
                    <div class="res-title">{title}</div>
                    <div class="res-sub">
                      {#if sub && sub !== title}
                        <span>{sub}</span>
                        <span class="dot">·</span>
                      {/if}
                      <span>{item.format ?? "TV"}</span>
                      {#if item.seasonYear}
                        <span class="dot">·</span>
                        <span>{item.seasonYear}</span>
                      {/if}
                    </div>
                  </div>
                  <div class="res-badges">
                    <span class="lib-tag">Library</span>
                    {#if item.watchedCount > 0 || item.episodes}
                      <span class="prog-tag">
                        {item.watchedCount}/{item.episodes ?? "?"} ep
                      </span>
                    {/if}
                  </div>
                </button>
              {/each}
            </div>
          </div>
        {/if}

        {#if globalQuery.trim().length >= 2}
          <div class="search-section">
            <div class="section-heading">
              <span>All of AniList</span>
              {#if remoteLoading}
                <span class="section-count">Searching…</span>
              {:else}
                <span class="section-count">{anilistMatches.length}</span>
              {/if}
            </div>

            {#if anilistMatches.length > 0}
              <div class="result-list">
                {#each anilistMatches as r, rIdx (r.anilistId)}
                  {@const flatIdx = libraryMatches.length + rIdx}
                  {@const title = r.titleEnglish ?? r.titleRomaji ?? r.titleNative ?? "Untitled"}
                  {@const sub = r.titleNative ?? r.titleRomaji}
                  <button
                    type="button"
                    class="result-row"
                    class:selected={selectedIndex === flatIdx}
                    onmouseenter={() => (selectedIndex = flatIdx)}
                    onclick={() => selectAnime(r.anilistId)}
                  >
                    <div class="res-poster">
                      {#if r.coverUrl}
                        <img src={r.coverUrl} alt="" loading="lazy" />
                      {/if}
                    </div>
                    <div class="res-meta">
                      <div class="res-title">{title}</div>
                      <div class="res-sub">
                        {#if sub && sub !== title}
                          <span>{sub}</span>
                          <span class="dot">·</span>
                        {/if}
                        <span>{r.format ?? "ANIME"}</span>
                        {#if r.seasonYear}
                          <span class="dot">·</span>
                          <span>{r.seasonYear}</span>
                        {/if}
                        {#if r.episodes}
                          <span class="dot">·</span>
                          <span>{r.episodes} eps</span>
                        {/if}
                      </div>
                    </div>
                    <div class="res-badges">
                      <span class="anilist-tag">AniList</span>
                    </div>
                  </button>
                {/each}
              </div>
            {:else if !remoteLoading}
              <div class="search-empty">No additional AniList matches for "{globalQuery.trim()}".</div>
            {/if}
          </div>
        {:else if libraryMatches.length === 0}
          <div class="search-empty">Type at least 2 characters to search all of AniList.</div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .nav-rail {
    position: relative;
    z-index: 40;
    flex: 0 0 var(--sidebar-w);
    width: var(--sidebar-w);
    height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: space-between;
    padding: 18px 0 16px;
    background: color-mix(in srgb, var(--bg) 70%, black);
    border-right: 1px solid var(--line);
    user-select: none;
  }

  .rail-top {
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  .brand-seal {
    position: relative;
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: 11px;
    overflow: hidden;
    transition: transform var(--t-fast) var(--ease);
  }

  .brand-seal:hover {
    transform: scale(1.06);
  }

  .brand-mark {
    width: 38px;
    height: 38px;
    border-radius: 11px;
    object-fit: cover;
  }

  .rail-nav {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: 100%;
  }

  .rail-item {
    position: relative;
    width: 44px;
    height: 44px;
    padding: 0;
    border-radius: 12px;
    border: none;
    background: transparent;
    color: var(--text-3);
    display: grid;
    place-items: center;
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }

  .rail-item:hover {
    background: var(--glass);
    color: var(--text);
  }

  .rail-item.active {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text);
  }

  .rail-item.active::before {
    content: "";
    position: absolute;
    left: -10px;
    top: 12px;
    bottom: 12px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--text);
  }

  .rail-label {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .rail-item.warn {
    color: var(--amber);
  }

  .rail-badge {
    position: absolute;
    top: 4px;
    right: 6px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 99px;
    background: var(--amber);
    color: #141210;
    font-size: 9.5px;
    font-weight: 800;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .rail-bottom {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 0 8px;
  }

  .rail-icon-btn {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-3);
    display: grid;
    place-items: center;
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease);
  }

  .rail-icon-btn:hover {
    background: var(--surface);
    color: var(--text);
    border-color: var(--border);
  }

  .rail-icon-btn.active {
    background: color-mix(in srgb, var(--coral) 15%, var(--surface));
    color: var(--coral);
    border-color: color-mix(in srgb, var(--coral) 35%, transparent);
  }

  #nav-rescan:hover :global(svg) {
    transform: rotate(90deg);
    transition: transform var(--t-med) var(--ease);
  }

  .status-pop {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .rail-status {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    width: 52px;
    padding: 7px 4px;
    border-radius: 10px;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    font-size: 9.5px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }

  .rail-status.playing {
    border-color: color-mix(in srgb, var(--coral) 45%, transparent);
    color: var(--coral);
  }

  .cancel-mini {
    position: absolute;
    top: -5px;
    right: -5px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 1px solid var(--border-strong);
    background: var(--surface-3);
    color: var(--text);
    font-size: 10px;
    line-height: 1;
    display: grid;
    place-items: center;
    cursor: pointer;
  }

  .cancel-mini:hover {
    background: var(--danger);
    color: #fff;
  }

  .pct-mini {
    font-size: 9.5px;
    line-height: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 46px;
  }

  .ok-dot {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--ok) 22%, transparent);
    color: var(--ok);
    display: grid;
    place-items: center;
  }

  .eq {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 12px;
  }
  .eq i {
    width: 2.5px;
    background: var(--coral);
    border-radius: 2px;
    animation: eq 900ms ease-in-out infinite;
  }
  .eq i:nth-child(2) {
    animation-delay: -300ms;
  }
  .eq i:nth-child(3) {
    animation-delay: -600ms;
  }
  @keyframes eq {
    0%,
    100% {
      height: 3px;
    }
    50% {
      height: 12px;
    }
  }

  .rail-avatar {
    position: relative;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    display: grid;
    place-items: center;
    overflow: visible;
  }

  .rail-avatar img {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    object-fit: cover;
  }

  .online-dot {
    position: absolute;
    right: -1px;
    bottom: -1px;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--ok);
    border: 2px solid var(--bg-elev);
  }

  /* Global Search Palette Modal */
  .search-backdrop {
    position: fixed;
    inset: 0;
    z-index: 120;
    background: rgba(6, 6, 9, 0.72);
    backdrop-filter: blur(8px);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 10vh 24px 24px;
    animation: fade-in 120ms ease both;
  }

  .search-palette {
    width: min(660px, 100%);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
    border-radius: 16px;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    box-shadow: 0 28px 70px -12px rgba(0, 0, 0, 0.85);
    overflow: hidden;
    animation: fade-up 150ms var(--ease) both;
  }

  .search-header {
    height: 54px;
    padding: 0 14px 0 18px;
    display: flex;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elev);
  }

  .search-icon {
    color: var(--coral);
    display: grid;
    place-items: center;
  }

  .search-input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 14.5px;
    font-weight: 500;
    outline: none;
  }

  .search-input::placeholder {
    color: var(--text-3);
  }

  .search-clear {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    border: none;
    background: var(--surface-3);
    color: var(--text-2);
    display: grid;
    place-items: center;
    cursor: pointer;
  }

  .search-close {
    height: 26px;
    padding: 0 8px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-3);
    font-size: 10.5px;
    font-weight: 700;
    cursor: pointer;
  }

  .search-body {
    overflow-y: auto;
    padding: 10px 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .search-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .section-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-3);
  }

  .section-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-3);
  }

  .result-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .result-row {
    width: 100%;
    padding: 8px 10px;
    border-radius: 10px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text);
    display: flex;
    align-items: center;
    gap: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 100ms ease, border-color 100ms ease;
  }

  .result-row:hover,
  .result-row.selected {
    background: color-mix(in srgb, var(--coral) 12%, var(--surface-2));
    border-color: color-mix(in srgb, var(--coral) 32%, transparent);
  }

  .res-poster {
    width: 36px;
    height: 50px;
    border-radius: 6px;
    background: var(--surface-3);
    overflow: hidden;
    flex-shrink: 0;
  }

  .res-poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .res-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .res-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .res-sub {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .res-sub .dot {
    opacity: 0.5;
  }

  .res-badges {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .lib-tag,
  .anilist-tag,
  .prog-tag {
    padding: 3px 8px;
    border-radius: 6px;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.02em;
  }

  .lib-tag {
    background: color-mix(in srgb, var(--coral) 18%, transparent);
    color: var(--coral);
    border: 1px solid color-mix(in srgb, var(--coral) 35%, transparent);
  }

  .anilist-tag {
    background: var(--surface-3);
    color: var(--text-2);
    border: 1px solid var(--border);
  }

  .prog-tag {
    background: var(--surface-2);
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }

  .search-empty {
    padding: 20px 12px;
    text-align: center;
    font-size: 12.5px;
    color: var(--text-3);
  }

  .bell-btn {
    position: relative;
  }

  .bell-badge {
    position: absolute;
    top: 3px;
    right: 3px;
    min-width: 15px;
    height: 15px;
    padding: 0 4px;
    border-radius: 99px;
    background: var(--coral);
    color: #fff;
    font-size: 9px;
    font-weight: 800;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    line-height: 1;
  }

  .alerts-backdrop {
    position: fixed;
    inset: 0;
    z-index: 115;
    background: transparent;
  }

  .alerts-popover {
    position: fixed;
    left: 82px;
    bottom: 20px;
    width: min(380px, calc(100vw - 100px));
    max-height: 68vh;
    display: flex;
    flex-direction: column;
    border-radius: 14px;
    background: var(--surface);
    border: 1px solid var(--border-strong);
    box-shadow: 0 24px 60px -10px rgba(0, 0, 0, 0.85);
    overflow: hidden;
    animation: fade-up 140ms var(--ease) both;
  }

  .alerts-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    background: var(--bg-elev);
    border-bottom: 1px solid var(--border);
  }

  .alerts-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    font-weight: 700;
    color: var(--text);
  }

  .alerts-title :global(svg) {
    color: var(--coral);
  }

  .alerts-clear {
    border: none;
    background: transparent;
    color: var(--text-3);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 6px;
  }

  .alerts-clear:hover {
    background: var(--surface-3);
    color: var(--text);
  }

  .alerts-list {
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .alerts-empty {
    padding: 22px 14px;
    text-align: center;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-3);
  }

  .alert-row {
    width: 100%;
    padding: 10px 11px;
    border-radius: 10px;
    border: 1px solid transparent;
    background: var(--surface-2);
    color: var(--text);
    display: flex;
    align-items: flex-start;
    gap: 10px;
    text-align: left;
    cursor: pointer;
    transition: background 100ms ease, border-color 100ms ease;
  }

  .alert-row:hover {
    background: color-mix(in srgb, var(--coral) 12%, var(--surface-2));
    border-color: color-mix(in srgb, var(--coral) 32%, transparent);
  }

  .alert-kind-badge {
    flex-shrink: 0;
    margin-top: 2px;
    padding: 2px 6px;
    border-radius: 5px;
    font-size: 9.5px;
    font-weight: 800;
    letter-spacing: 0.04em;
    background: var(--surface-3);
    color: var(--text-2);
    border: 1px solid var(--border);
  }

  .alert-kind-badge.sub {
    background: color-mix(in srgb, var(--coral) 18%, transparent);
    color: var(--coral);
    border-color: color-mix(in srgb, var(--coral) 38%, transparent);
  }

  .alert-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .alert-show {
    font-size: 12.5px;
    font-weight: 700;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .alert-msg {
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--text-2);
  }
</style>
