<!--
  TopBar — Tokyo Night floating glass navbar (蔵 · 東京ナイト).
  Fixed, inset var(--nav-inset), becomes more opaque once #main-scroll scrolls.
  Left: 蔵 mark + KURA · primary links. Right: search pill (Ctrl/⌘K),
  torrent / now-playing / scan chips, alerts bell, avatar menu.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { afterNavigate, goto } from "$app/navigation";
  import { page } from "$app/state";
  import { app } from "$lib/app.svelte";
  import { api, img, type SearchResult } from "$lib/api";
  import { detailsModal } from "$lib/detailsModal.svelte";
  import { JA } from "$lib/ja";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";

  const PHASES: Record<string, string> = {
    discover: "Scanning folders",
    match: "Identifying anime",
    resolve: "Mapping episodes",
    metadata: "Fetching metadata",
    artwork: "Downloading artwork",
    done: "Up to date",
  };

  /** Local accent vocabulary not present in ja.ts. */
  const JA_LOCAL = {
    alerts: "通知",
    library: "ライブラリ",
    anilist: "アニリスト",
    rescan: "再スキャン",
  } as const;

  // ---------------------------------------------------------------------------
  // Route state
  // ---------------------------------------------------------------------------
  const path = $derived(page.url.pathname);
  const formatParam = $derived(page.url.searchParams.get("format"));
  const watchParam = $derived(page.url.searchParams.get("watch"));

  type NavKey = "home" | "series" | "movies" | "myList";
  const LINKS: { key: NavKey; href: string; label: string }[] = [
    { key: "home", href: "/", label: "Home" },
    { key: "series", href: "/?format=series", label: "Series" },
    { key: "movies", href: "/?format=movie", label: "Movies" },
    { key: "myList", href: "/?watch=planning", label: "My List" },
  ];
  const activeKey = $derived.by((): NavKey | null => {
    if (path !== "/") return null;
    if (watchParam === "planning") return "myList";
    if (formatParam === "series") return "series";
    if (formatParam === "movie") return "movies";
    if (!formatParam && !watchParam) return "home";
    return null;
  });

  // ---------------------------------------------------------------------------
  // Status
  // ---------------------------------------------------------------------------
  const pct = $derived(
    app.progress && app.progress.total > 0
      ? Math.min(100, (app.progress.current / app.progress.total) * 100)
      : null,
  );
  const playing = $derived(app.nowPlaying);
  const playingCard = $derived(
    playing ? app.cards.find((c) => c.anilistId === playing.anilistId) : null,
  );
  const activeTorrent = $derived(
    (app.torrentSessions ?? []).find(
      (s) =>
        s.state === "buffering" ||
        s.state === "downloading" ||
        s.state === "resolving",
    ) ?? null,
  );
  const unreadAlertsCount = $derived(app.alerts.filter((a) => !a.read).length);
  const avatarUrl = $derived(
    app.anilist?.connected ? app.anilist.avatarUrl : null,
  );

  async function cancelTorrent(id: string) {
    app.torrentSessions = (app.torrentSessions ?? []).filter(
      (s) => s.sessionId !== id,
    );
    try {
      await api.cancelTorrentSession(id);
    } catch {}
  }

  // ---------------------------------------------------------------------------
  // Scroll-aware glass
  // ---------------------------------------------------------------------------
  let scrolled = $state(false);
  $effect(() => {
    const el = document.getElementById("main-scroll");
    if (!el) return;
    const onScroll = () => {
      const next = el.scrollTop > 8;
      if (next !== scrolled) scrolled = next;
    };
    onScroll();
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  });

  let isMac = $state(false);
  $effect(() => {
    isMac = /Mac|iPhone|iPad/i.test(navigator.platform || navigator.userAgent);
  });

  // ---------------------------------------------------------------------------
  // Search (library first, then all of AniList)
  // ---------------------------------------------------------------------------
  let searchWrap: HTMLDivElement | undefined = $state();
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
        const syn =
          c.synonyms?.some((s) => s.toLowerCase().includes(q)) ?? false;
        return en.includes(q) || ro.includes(q) || na.includes(q) || syn;
      })
      .slice(0, 8);
  });

  const anilistMatches = $derived.by(() => {
    const libIds = new Set(libraryMatches.map((c) => c.anilistId));
    return remoteResults.filter((r) => !libIds.has(r.anilistId));
  });

  const combinedNav = $derived([
    ...libraryMatches.map((c) => ({ id: c.anilistId, inLibrary: true })),
    ...anilistMatches.map((r) => ({ id: r.anilistId, inLibrary: false })),
  ]);

  async function openSearch() {
    alertsOpen = false;
    menuOpen = false;
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

  function onSearchInput() {
    searchOpen = true;
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

  function selectAnime(anilistId: number, inLibrary: boolean) {
    closeSearch();
    searchEl?.blur();
    if (inLibrary) {
      detailsModal.open(anilistId);
    } else {
      goto(`/anime/${anilistId}`);
    }
  }

  function onSearchKeydown(e: KeyboardEvent) {
    const total = combinedNav.length;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeSearch();
      searchEl?.blur();
    } else if (e.key === "ArrowDown" && total > 0) {
      e.preventDefault();
      searchOpen = true;
      selectedIndex = (selectedIndex + 1) % total;
      scrollSelectedIntoView();
    } else if (e.key === "ArrowUp" && total > 0) {
      e.preventDefault();
      searchOpen = true;
      selectedIndex = (selectedIndex - 1 + total) % total;
      scrollSelectedIntoView();
    } else if (e.key === "Enter" && total > 0 && searchOpen) {
      e.preventDefault();
      const target = combinedNav[selectedIndex] ?? combinedNav[0];
      if (target) selectAnime(target.id, target.inLibrary);
    } else if (e.key === "Tab") {
      closeSearch();
    }
  }

  async function scrollSelectedIntoView() {
    await tick();
    document
      .getElementById(`search-opt-${selectedIndex}`)
      ?.scrollIntoView({ block: "nearest" });
  }

  // ---------------------------------------------------------------------------
  // Alerts popover
  // ---------------------------------------------------------------------------
  let alertsWrap: HTMLDivElement | undefined = $state();
  let bellBtn: HTMLButtonElement | undefined = $state();
  let alertsOpen = $state(false);

  function toggleAlerts() {
    alertsOpen = !alertsOpen;
    if (alertsOpen) {
      menuOpen = false;
      closeSearch();
      if (unreadAlertsCount > 0) app.markAllAlertsRead();
    }
  }

  // ---------------------------------------------------------------------------
  // Avatar menu
  // ---------------------------------------------------------------------------
  let menuWrap: HTMLDivElement | undefined = $state();
  let menuBtn: HTMLButtonElement | undefined = $state();
  let menuEl: HTMLDivElement | undefined = $state();
  let menuOpen = $state(false);

  type MenuItem = {
    href: string;
    label: string;
    ja: string;
    icon: string;
    id: string;
  };
  const menuItems = $derived.by((): MenuItem[] => {
    const items: MenuItem[] = [];
    // Discover is only meaningful in streaming mode (unchanged from the old rail).
    if (app.appMode === "streaming") {
      items.push({
        href: "/discover",
        label: "Discover",
        ja: JA.pages.discover.ja,
        icon: "sparkles",
        id: "nav-discover",
      });
    }
    items.push(
      {
        href: "/calendar",
        label: "Calendar",
        ja: JA.pages.calendar.ja,
        icon: "calendar",
        id: "nav-calendar",
      },
      {
        href: "/immersion",
        label: "Immersion",
        ja: JA.pages.immersion.ja,
        icon: "book-open",
        id: "nav-immersion",
      },
      {
        href: "/review",
        label: "Review",
        ja: JA.pages.review.ja,
        icon: "alert",
        id: "nav-review",
      },
      {
        href: "/settings",
        label: "Settings",
        ja: JA.pages.settings.ja,
        icon: "settings",
        id: "nav-settings",
      },
    );
    return items;
  });

  function menuFocusables(): HTMLElement[] {
    return menuEl
      ? Array.from(menuEl.querySelectorAll<HTMLElement>('[role="menuitem"]'))
      : [];
  }

  async function openMenu(focus: "first" | "last" = "first") {
    alertsOpen = false;
    closeSearch();
    menuOpen = true;
    await tick();
    const items = menuFocusables();
    (focus === "first" ? items[0] : items[items.length - 1])?.focus();
  }

  function closeMenu(returnFocus = false) {
    menuOpen = false;
    if (returnFocus) menuBtn?.focus();
  }

  function onMenuButtonKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      openMenu("first");
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      openMenu("last");
    }
  }

  function onMenuKeydown(e: KeyboardEvent) {
    const items = menuFocusables();
    const idx = items.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenu(true);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      items[(idx + 1) % items.length]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      items[(idx - 1 + items.length) % items.length]?.focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      items[0]?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      items[items.length - 1]?.focus();
    } else if (e.key === "Tab") {
      closeMenu();
    }
  }

  // ---------------------------------------------------------------------------
  // Global listeners: Ctrl/⌘K, Esc, click-outside
  // ---------------------------------------------------------------------------
  async function onWindowKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      if (searchOpen && document.activeElement === searchEl) {
        closeSearch();
        searchEl?.blur();
      } else {
        await openSearch();
      }
    } else if (e.key === "Escape") {
      if (searchOpen) {
        closeSearch();
      } else if (alertsOpen) {
        alertsOpen = false;
        bellBtn?.focus();
      } else if (menuOpen) {
        closeMenu(true);
      }
    }
  }

  function onWindowPointerDown(e: PointerEvent) {
    const t = e.target as Node | null;
    if (!t) return;
    const evPath = e.composedPath();
    const inside = (el: HTMLElement | undefined) => !!el && (evPath.includes(el) || el.contains(t));
    if (searchOpen && !inside(searchWrap)) closeSearch();
    if (alertsOpen && !inside(alertsWrap)) alertsOpen = false;
    if (menuOpen && !inside(menuWrap)) menuOpen = false;
  }

  function closeMenuAfterClick() {
    setTimeout(() => closeMenu(), 0);
  }

  afterNavigate(() => {
    menuOpen = false;
    alertsOpen = false;
  });
</script>

<svelte:window onkeydown={onWindowKey} onpointerdown={onWindowPointerDown} />

<div class="nav-backdrop" class:visible={scrolled} aria-hidden="true"></div>

<nav class="navbar glass" class:scrolled aria-label="Main">
  <!-- Brand (pure kanji mark) -->
  <a
    class="brand"
    href="/"
    id="nav-home"
    title="Kura"
    aria-label="Kura home"
    onclick={() => (app.query = "")}
  >
    <span class="brand-mark" lang="ja" aria-hidden="true">{JA.brandMark}</span>
  </a>

  <!-- Primary links -->
  <ul class="links no-scrollbar">
    {#each LINKS as link (link.key)}
      <li>
        <a
          class="link"
          class:active={activeKey === link.key}
          href={link.href}
          id={`nav-${link.key}`}
          aria-current={activeKey === link.key ? "page" : undefined}
          onclick={() => {
            if (link.key === "home") app.query = "";
          }}
        >
          {link.label}
        </a>
      </li>
    {/each}
  </ul>

  <div class="spacer"></div>

  <!-- Right cluster -->
  <div class="cluster">
    <!-- Search pill + results -->
    <div class="search-wrap" bind:this={searchWrap}>
      <div class="search-pill" class:open={searchOpen}>
        <span class="search-icon"><Icon name="search" size={15} /></span>
        <input
          bind:this={searchEl}
          bind:value={globalQuery}
          oninput={onSearchInput}
          onkeydown={onSearchKeydown}
          onfocus={() => {
            alertsOpen = false;
            menuOpen = false;
            searchOpen = true;
          }}
          class="search-input"
          type="search"
          placeholder={JA.placeholders.search}
          id="library-search"
          role="combobox"
          autocomplete="off"
          spellcheck="false"
          aria-label="Search your library or AniList"
          aria-expanded={searchOpen}
          aria-controls="search-results"
          aria-autocomplete="list"
          aria-activedescendant={searchOpen && combinedNav.length > 0
            ? `search-opt-${selectedIndex}`
            : undefined}
        />
        {#if remoteLoading}
          <span class="spinner spinner-xs" aria-hidden="true"></span>
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
            <Icon name="x" size={12} />
          </button>
        {:else}
          <kbd class="kbd" aria-hidden="true">{isMac ? "⌘K" : "Ctrl K"}</kbd>
        {/if}
      </div>

      {#if searchOpen}
        <div
          class="search-panel glass"
          id="search-results"
          role="listbox"
          aria-label="Search results"
        >
          {#if libraryMatches.length > 0}
            <div
              class="search-section"
              role="group"
              aria-labelledby="search-lib-heading"
            >
              <div class="section-heading" id="search-lib-heading">
                <span>
                  {globalQuery.trim() ? "In Your Library" : "Recent in Library"}
                  <Ja text={JA_LOCAL.library} class="ja-sub" />
                </span>
                <span class="section-count">{libraryMatches.length}</span>
              </div>
              {#each libraryMatches as item, idx (item.anilistId)}
                {@const cover = img(item.coverPath, item.coverUrl)}
                {@const title =
                  item.titleEnglish ??
                  item.titleRomaji ??
                  item.titleNative ??
                  "Untitled"}
                {@const sub = item.titleNative ?? item.titleRomaji}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <div
                  class="result-row"
                  class:selected={selectedIndex === idx}
                  role="option"
                  tabindex="-1"
                  id={`search-opt-${idx}`}
                  aria-selected={selectedIndex === idx}
                  onmouseenter={() => (selectedIndex = idx)}
                  onclick={() => selectAnime(item.anilistId, true)}
                >
                  <div class="res-poster">
                    {#if cover}
                      <img src={cover} alt="" loading="lazy" decoding="async" />
                    {/if}
                  </div>
                  <div class="res-meta">
                    <div class="res-title">{title}</div>
                    <div class="res-sub">
                      {#if sub && sub !== title}
                        <span lang={sub === item.titleNative ? "ja" : undefined}
                          >{sub}</span
                        >
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
                    <span class="tag lib">Library</span>
                    {#if item.watchedCount > 0 || item.episodes}
                      <span class="tag"
                        >{item.watchedCount}/{item.episodes ?? "?"} ep</span
                      >
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          {/if}

          {#if globalQuery.trim().length >= 2}
            <div
              class="search-section"
              role="group"
              aria-labelledby="search-al-heading"
            >
              <div class="section-heading" id="search-al-heading">
                <span>All of AniList</span>
                {#if remoteLoading}
                  <span class="section-count">Searching…</span>
                {:else}
                  <span class="section-count">{anilistMatches.length}</span>
                {/if}
              </div>

              {#each anilistMatches as r, rIdx (r.anilistId)}
                {@const flatIdx = libraryMatches.length + rIdx}
                {@const title =
                  r.titleEnglish ??
                  r.titleRomaji ??
                  r.titleNative ??
                  "Untitled"}
                {@const sub = r.titleNative ?? r.titleRomaji}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <div
                  class="result-row"
                  class:selected={selectedIndex === flatIdx}
                  role="option"
                  tabindex="-1"
                  id={`search-opt-${flatIdx}`}
                  aria-selected={selectedIndex === flatIdx}
                  onmouseenter={() => (selectedIndex = flatIdx)}
                  onclick={() => selectAnime(r.anilistId, false)}
                >
                  <div class="res-poster">
                    {#if r.coverUrl}
                      <img
                        src={r.coverUrl}
                        alt=""
                        loading="lazy"
                        decoding="async"
                      />
                    {/if}
                  </div>
                  <div class="res-meta">
                    <div class="res-title">{title}</div>
                    <div class="res-sub">
                      {#if sub && sub !== title}
                        <span lang={sub === r.titleNative ? "ja" : undefined}
                          >{sub}</span
                        >
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
                    <span class="tag al">AniList</span>
                  </div>
                </div>
              {/each}
              {#if anilistMatches.length === 0 && !remoteLoading}
                <div class="search-empty">
                  No additional AniList matches for "{globalQuery.trim()}".
                </div>
              {/if}
            </div>
          {:else if libraryMatches.length === 0}
            <div class="search-empty">
              Type at least 2 characters to search all of AniList.
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- Torrent chip -->
    {#if activeTorrent && !playing}
      <div class="chip-pill torrent" id="now-torrent">
        <a
          class="chip-main"
          href={`/anime/${activeTorrent.anilistId}`}
          title={`${activeTorrent.title} · Ep ${activeTorrent.epKey} (${Math.round(activeTorrent.progressPct)}%)`}
          aria-label={`Streaming ${activeTorrent.title}, episode ${activeTorrent.epKey}, ${Math.round(activeTorrent.progressPct)}% buffered`}
        >
          <span class="spinner spinner-xs" aria-hidden="true"></span>
          <span class="chip-num">{Math.round(activeTorrent.progressPct)}%</span>
        </a>
        <button
          type="button"
          class="chip-x"
          title="Cancel torrent stream"
          aria-label="Cancel torrent stream"
          onclick={() => cancelTorrent(activeTorrent.sessionId)}
        >
          <Icon name="x" size={11} stroke={2.5} />
        </button>
      </div>
    {/if}

    <!-- Now-playing chip -->
    {#if playing}
      <a
        class="chip-pill playing"
        href={`/anime/${playing.anilistId}`}
        id="now-playing"
        title={playingCard
          ? `Now Playing: ${playingCard.titleEnglish ?? playingCard.titleRomaji} · Ep ${playing.epKey}`
          : `Now Playing · Ep ${playing.epKey}`}
      >
        <span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>
        <span class="sr-only">Now playing</span>
        <Ja text={JA.states.nowPlaying} class="chip-ja" />
        <span class="chip-num">Ep {playing.epKey}</span>
      </a>
    {/if}

    <!-- Scan progress chip -->
    {#if app.progress}
      <div
        class="chip-pill scan"
        class:done={!app.scanning}
        role="status"
        title={`${PHASES[app.progress.phase] ?? app.progress.phase}: ${app.progress.message}`}
      >
        {#if app.scanning}
          <span class="spinner spinner-xs" aria-hidden="true"></span>
          <span class="sr-only"
            >{PHASES[app.progress.phase] ?? app.progress.phase}</span
          >
          {#if pct !== null}
            <span class="chip-num">{Math.round(pct)}%</span>
          {/if}
        {:else}
          <span class="ok-dot" aria-hidden="true"
            ><Icon name="check" size={11} stroke={3} /></span
          >
          <span class="sr-only">Library up to date</span>
        {/if}
      </div>
    {/if}

    <!-- Alerts bell -->
    <div class="pop-wrap" bind:this={alertsWrap}>
      <button
        type="button"
        class="icon-btn"
        class:active={alertsOpen}
        bind:this={bellBtn}
        onclick={toggleAlerts}
        id="nav-notifications"
        title="Episode & Jimaku JP Subtitle Notifications"
        aria-label={unreadAlertsCount > 0
          ? `Notifications, ${unreadAlertsCount} unread`
          : "Notifications"}
        aria-haspopup="dialog"
        aria-expanded={alertsOpen}
      >
        <Icon name="bell" size={18} />
        {#if unreadAlertsCount > 0}
          <span class="unread-dot" aria-hidden="true"></span>
        {/if}
      </button>

      {#if alertsOpen}
        <div
          class="alerts-popover glass"
          role="dialog"
          aria-label="Episode and Jimaku subtitle notifications"
        >
          <div class="alerts-head">
            <div class="alerts-title">
              <span>Episode &amp; JP Sub Alerts</span>
              <Ja text={JA_LOCAL.alerts} class="ja-sub" />
            </div>
            {#if app.alerts.length > 0}
              <button
                type="button"
                class="alerts-clear"
                onclick={() => app.clearAlerts()}>Clear</button
              >
            {/if}
          </div>

          <div class="alerts-list">
            {#if app.alerts.length === 0}
              <div class="alerts-empty">
                No notifications yet. Kura automatically checks your <strong
                  >Watching</strong
                > list for newly aired episodes and Jimaku JP subtitles.
              </div>
            {:else}
              {#each app.alerts as alert (alert.id)}
                <button
                  type="button"
                  class="alert-row"
                  class:unread={!alert.read}
                  onclick={() => {
                    alertsOpen = false;
                    goto(`/anime/${alert.anilistId}`);
                  }}
                >
                  <span
                    class="alert-kind"
                    class:sub={alert.kind === "jimaku_sub" ||
                      alert.kind === "episode_and_sub"}
                  >
                    {#if alert.kind === "jimaku_sub"}
                      <span lang="ja">日本語</span> SUB
                    {:else if alert.kind === "episode_and_sub"}
                      EP + SUB
                    {:else}
                      NEW EP
                    {/if}
                  </span>
                  <span class="alert-body">
                    <span class="alert-show"
                      >{alert.title} · Ep {alert.episode}</span
                    >
                    <span class="alert-msg">{alert.message}</span>
                  </span>
                </button>
              {/each}
            {/if}
          </div>
        </div>
      {/if}
    </div>

    <!-- Avatar menu -->
    <div class="pop-wrap" bind:this={menuWrap}>
      <button
        type="button"
        class="avatar-btn"
        class:active={menuOpen}
        bind:this={menuBtn}
        id="nav-account"
        aria-label={app.reviewCount > 0
          ? `Open menu, ${app.reviewCount} folders need review`
          : "Open menu"}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-controls="nav-account-menu"
        onclick={() => (menuOpen ? closeMenu() : openMenu())}
        onkeydown={onMenuButtonKeydown}
      >
        {#if avatarUrl}
          <img src={avatarUrl} alt="" />
        {:else}
          <span class="avatar-fallback" aria-hidden="true">
            {#if app.anilist?.userName}
              {app.anilist.userName.charAt(0).toUpperCase()}
            {:else}
              <Icon name="user" size={17} stroke={2.2} />
            {/if}
          </span>
        {/if}
        {#if app.reviewCount > 0}
          <span class="avatar-badge" aria-hidden="true"
            >{app.reviewCount > 9 ? "9+" : app.reviewCount}</span
          >
        {/if}
      </button>

      {#if menuOpen}
        <!-- svelte-ignore a11y_interactive_supports_focus -->
        <div
          class="menu glass"
          id="nav-account-menu"
          role="menu"
          aria-label="Kura menu"
          bind:this={menuEl}
          onkeydown={onMenuKeydown}
        >
          {#if app.anilist?.connected}
            <a
              class="menu-account"
              role="menuitem"
              href="/settings"
              id="nav-anilist"
              title={`AniList connected as ${app.anilist.userName ?? "user"} (${app.anilist.trackedCount} synced)`}
              onclick={() => {
                try {
                  localStorage.setItem(
                    "kura.settings.category",
                    "integrations",
                  );
                } catch {}
                closeMenuAfterClick();
              }}
            >
              <span class="acct-avatar">
                {#if app.anilist.avatarUrl}
                  <img src={app.anilist.avatarUrl} alt="" />
                {:else}
                  <Icon name="user" size={15} />
                {/if}
                <span class="online-dot" aria-hidden="true"></span>
              </span>
              <span class="acct-meta">
                <span class="acct-name"
                  >{app.anilist.userName ?? "AniList user"}</span
                >
                <span class="acct-sub"
                  >AniList · {app.anilist.trackedCount} synced</span
                >
              </span>
            </a>
            <div class="menu-sep" role="separator"></div>
          {/if}

          {#each menuItems as item (item.href)}
            <a
              class="menu-item"
              class:current={path.startsWith(item.href)}
              role="menuitem"
              href={item.href}
              id={item.id}
              aria-current={path.startsWith(item.href) ? "page" : undefined}
              onclick={closeMenuAfterClick}
            >
              <span class="mi-icon"><Icon name={item.icon} size={16} /></span>
              <span class="mi-label">{item.label}</span>
              <Ja text={item.ja} class="mi-ja" />
              {#if item.href === "/review" && app.reviewCount > 0}
                <span
                  class="mi-badge"
                  title={`${app.reviewCount} unmatched folders need review`}
                >
                  {app.reviewCount}<span class="sr-only"> need review</span>
                </span>
              {/if}
            </a>
          {/each}

          {#if app.libraries.length > 0 && !app.scanning}
            <div class="menu-sep" role="separator"></div>
            <button
              type="button"
              class="menu-item"
              role="menuitem"
              id="nav-rescan"
              title="Rescan local library folders"
              onclick={() => {
                closeMenu(true);
                app.scan();
              }}
            >
              <span class="mi-icon"><Icon name="refresh" size={16} /></span>
              <span class="mi-label">Rescan libraries</span>
              <Ja text={JA_LOCAL.rescan} class="mi-ja" />
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</nav>

<style>
  /* ── Bar ─────────────────────────────────────────────────────────────── */
  .navbar {
    position: fixed;
    top: var(--nav-inset);
    left: var(--nav-inset);
    right: var(--nav-inset);
    z-index: 800;
    height: var(--nav-h);
    display: flex;
    align-items: center;
    gap: 22px;
    padding: 0 10px 0 12px;
    border-radius: var(--r-lg);
    box-shadow: 0 10px 30px -14px rgba(3, 4, 10, 0.7);
    user-select: none;
    transition:
      background var(--t-med) var(--ease),
      box-shadow var(--t-med) var(--ease);
  }
  .navbar.scrolled {
    background: color-mix(in srgb, var(--bg) 93%, transparent);
    backdrop-filter: blur(12px) saturate(135%);
    -webkit-backdrop-filter: blur(12px) saturate(135%);
    border-color: rgba(255, 255, 255, 0.08);
    box-shadow: var(--shadow-lg);
  }
  :global(html.libmpv-active) .navbar,
  :global(html.libmpv-active) .nav-backdrop {
    display: none;
  }

  .nav-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 799;
    height: calc(var(--nav-h) + var(--nav-inset) * 2 + 12px);
    background: linear-gradient(
      180deg,
      var(--bg) 0%,
      var(--bg) calc(var(--nav-inset) + var(--nav-h) * 0.75),
      color-mix(in srgb, var(--bg) 0%, transparent) 100%
    );
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--t-med) var(--ease);
  }
  .nav-backdrop.visible {
    opacity: 1;
  }

  /* ── Brand ───────────────────────────────────────────────────────────── */
  .brand {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 11px;
  }
  .brand-mark {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background:
      radial-gradient(circle at 22% 18%, rgba(244, 114, 182, 0.35), transparent 58%),
      linear-gradient(135deg, #7c3aed 0%, #4c1d95 58%, #181b29 100%);
    border: 1px solid rgba(167, 139, 250, 0.35);
    color: #ffffff;
    font-family: var(--font-jp);
    font-size: 19px;
    font-weight: 800;
    line-height: 1;
    box-shadow: 0 6px 18px -4px rgba(139, 92, 246, 0.55);
    transition:
      transform var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease);
  }
  .brand:hover .brand-mark {
    transform: translateY(-1px);
    border-color: rgba(244, 114, 182, 0.55);
    box-shadow: var(--neon-glow-strong);
  }

  /* ── Primary links ───────────────────────────────────────────────────── */
  .links {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-x: auto;
  }
  .link {
    position: relative;
    display: inline-flex;
    align-items: center;
    height: 36px;
    padding: 0 12px;
    border-radius: 8px;
    color: var(--text-2);
    font-size: 13.5px;
    font-weight: 600;
    white-space: nowrap;
    transition:
      color var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .link:hover {
    color: var(--text);
    background: rgba(255, 255, 255, 0.04);
  }
  .link.active {
    color: var(--text);
  }
  .link.active::after {
    content: "";
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 3px;
    height: 2px;
    border-radius: 2px;
    background: var(--neon);
    box-shadow: 0 0 10px rgba(244, 114, 182, 0.55);
  }

  .spacer {
    flex: 1;
  }

  /* ── Right cluster ───────────────────────────────────────────────────── */
  .cluster {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  /* Search pill */
  .search-wrap {
    position: relative;
  }
  .search-pill {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 260px;
    height: 38px;
    padding: 0 6px 0 12px;
    border-radius: var(--r-pill);
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid var(--glass-hairline);
    transition:
      width var(--t-med) var(--ease),
      background var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .search-pill:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .search-pill.open,
  .search-pill:focus-within {
    width: 340px;
    background: rgba(255, 255, 255, 0.08);
    border-color: color-mix(in srgb, var(--neon-violet) 55%, transparent);
    box-shadow: var(--neon-glow);
  }
  .search-icon {
    display: grid;
    place-items: center;
    color: var(--text-2);
  }
  .search-input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    outline: none;
  }
  .search-input:focus-visible {
    outline: none;
    box-shadow: none;
  }
  .search-input::placeholder {
    color: var(--text-2);
    font-family: var(--font-jp);
  }
  .search-input::-webkit-search-cancel-button {
    display: none;
  }
  .kbd {
    flex: none;
    height: 22px;
    padding: 0 7px;
    display: inline-flex;
    align-items: center;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid var(--glass-hairline);
    color: var(--text-2);
    font-family: var(--font-label);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .search-clear {
    flex: none;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-2);
    cursor: pointer;
  }
  .search-clear:hover {
    color: var(--text);
    background: rgba(255, 255, 255, 0.14);
  }

  .search-panel {
    position: absolute;
    top: calc(100% + 12px);
    right: 0;
    width: min(520px, calc(100vw - 2 * var(--nav-inset) - 20px));
    max-height: min(70vh, 620px);
    overflow-y: auto;
    padding: 10px;
    border-radius: var(--r-xl);
    background: rgba(12, 14, 24, 0.96);
    backdrop-filter: blur(28px) saturate(160%);
    -webkit-backdrop-filter: blur(28px) saturate(160%);
    box-shadow: var(--shadow-lg);
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: drop-in var(--t-fast) var(--ease) both;
  }
  .search-section {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .section-heading {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    padding: 4px 8px 6px;
    font-family: var(--font-label);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-2);
  }
  .section-heading :global(.ja-sub) {
    text-transform: none;
    letter-spacing: 0.04em;
  }
  .section-count {
    font-variant-numeric: tabular-nums;
  }
  .result-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 8px;
    border-radius: 10px;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .result-row.selected {
    background: rgba(139, 92, 246, 0.16);
    box-shadow: inset 2px 0 0 var(--neon-violet);
  }
  .res-poster {
    flex: none;
    width: 34px;
    height: 48px;
    border-radius: 6px;
    overflow: hidden;
    background: var(--surface-3);
  }
  .res-poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .res-meta {
    flex: 1;
    min-width: 0;
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
    gap: 5px;
    font-size: 12px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    opacity: 0.6;
  }
  .res-badges {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
  }
  .tag {
    padding: 1px 7px;
    border-radius: var(--r-pill);
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-2);
    font-size: 10.5px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .tag.lib {
    background: color-mix(in srgb, var(--neon-violet) 22%, transparent);
    color: var(--text);
  }
  .tag.al {
    background: color-mix(in srgb, var(--sat-blue) 20%, transparent);
    color: var(--text);
  }
  .search-empty {
    padding: 14px 10px;
    font-size: 12.5px;
    color: var(--text-2);
    text-align: center;
  }

  /* Status chips */
  .chip-pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 32px;
    padding: 0 11px;
    border-radius: var(--r-pill);
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid var(--glass-hairline);
    color: var(--text-2);
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .chip-num {
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .chip-pill.playing {
    border-color: color-mix(in srgb, var(--neon-pink) 40%, transparent);
    color: var(--neon-pink);
  }
  .chip-pill.playing:hover {
    box-shadow: var(--neon-glow);
  }
  .chip-pill :global(.chip-ja) {
    font-size: 11px;
  }
  .chip-pill.torrent {
    padding: 0 4px 0 0;
    gap: 2px;
  }
  .chip-main {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 100%;
    padding: 0 6px 0 11px;
    border-radius: var(--r-pill);
  }
  .chip-x {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }
  .chip-x:hover {
    background: color-mix(in srgb, var(--danger) 22%, transparent);
    color: var(--text);
  }
  .chip-pill.scan.done {
    color: var(--ok);
  }
  .ok-dot {
    display: grid;
    place-items: center;
    color: var(--ok);
  }
  .eq {
    display: inline-flex;
    align-items: flex-end;
    gap: 2px;
    height: 12px;
  }
  .eq i {
    width: 2.5px;
    height: 100%;
    border-radius: 1px;
    background: currentColor;
    animation: eq-pulse 0.7s ease-in-out infinite alternate;
  }
  .eq i:nth-child(2) {
    animation-delay: 0.25s;
  }
  .eq i:nth-child(3) {
    animation-delay: 0.12s;
  }

  /* Icon buttons */
  .pop-wrap {
    position: relative;
  }
  .icon-btn {
    position: relative;
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }
  .icon-btn:hover,
  .icon-btn.active {
    background: rgba(255, 255, 255, 0.07);
    color: var(--text);
  }
  .unread-dot {
    position: absolute;
    top: 8px;
    right: 9px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--neon-pink);
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 8px var(--neon-pink);
  }

  /* Avatar */
  .avatar-btn {
    position: relative;
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    margin-left: 2px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: transparent;
    cursor: pointer;
    box-shadow: 0 0 0 2px transparent;
    transition: box-shadow var(--t-fast) var(--ease);
  }
  .avatar-btn:hover,
  .avatar-btn.active {
    box-shadow:
      0 0 0 2px var(--neon-violet),
      var(--neon-glow);
  }
  .avatar-btn img {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    object-fit: cover;
  }
  .avatar-fallback {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--neon-violet), var(--neon-pink));
    color: #fff;
    font-family: var(--font-jp);
    font-size: 15px;
    font-weight: 700;
  }
  .avatar-badge {
    position: absolute;
    top: -3px;
    right: -5px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    background: var(--amber);
    color: #141210;
    font-size: 9.5px;
    font-weight: 800;
    box-shadow: 0 0 0 2px var(--surface);
  }

  /* Dropdowns (menu + alerts) */
  .menu,
  .alerts-popover {
    position: absolute;
    top: calc(100% + 14px);
    right: 0;
    border-radius: var(--r-xl);
    background: rgba(12, 14, 24, 0.96);
    backdrop-filter: blur(28px) saturate(160%);
    -webkit-backdrop-filter: blur(28px) saturate(160%);
    box-shadow: var(--shadow-lg);
    animation: drop-in var(--t-fast) var(--ease) both;
  }
  .menu {
    width: 244px;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .menu-item,
  .menu-account {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 40px;
    padding: 0 10px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: var(--text);
    font-size: 13.5px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .menu-item:hover,
  .menu-item:focus-visible,
  .menu-account:hover,
  .menu-account:focus-visible {
    background: rgba(139, 92, 246, 0.16);
  }
  .menu-item:focus-visible,
  .menu-account:focus-visible {
    outline: 2px solid var(--neon-violet);
    outline-offset: -2px;
    box-shadow: none;
  }
  .menu-item.current {
    box-shadow: inset 2px 0 0 var(--neon-violet);
  }
  .mi-icon {
    display: grid;
    place-items: center;
    color: var(--text-2);
  }
  .mi-label {
    flex: 1;
  }
  .menu-item :global(.mi-ja) {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-2);
    letter-spacing: 0.04em;
  }
  .mi-badge {
    min-width: 20px;
    height: 18px;
    padding: 0 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-pill);
    background: var(--amber);
    color: #141210;
    font-size: 10.5px;
    font-weight: 800;
  }
  .menu-sep {
    height: 1px;
    margin: 4px 8px;
    background: var(--glass-hairline);
  }
  .menu-account {
    min-height: 52px;
  }
  .acct-avatar {
    position: relative;
    flex: none;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: var(--surface-3);
    color: var(--text-2);
  }
  .acct-avatar img {
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
    box-shadow: 0 0 0 2px var(--surface);
  }
  .acct-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .acct-name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .acct-sub {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--text-2);
  }

  /* Alerts */
  .alerts-popover {
    width: min(380px, calc(100vw - 2 * var(--nav-inset) - 20px));
    max-height: min(70vh, 560px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .alerts-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px 10px;
    border-bottom: 1px solid var(--glass-hairline);
  }
  .alerts-title {
    font-size: 13.5px;
    font-weight: 700;
  }
  .alerts-clear {
    height: 28px;
    padding: 0 10px;
    border: none;
    border-radius: var(--r-pill);
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-2);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }
  .alerts-clear:hover {
    background: rgba(255, 255, 255, 0.12);
    color: var(--text);
  }
  .alerts-list {
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .alerts-empty {
    padding: 18px 12px;
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--text-2);
  }
  .alerts-empty strong {
    color: var(--text);
  }
  .alert-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    padding: 9px 10px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .alert-row:hover {
    background: rgba(139, 92, 246, 0.14);
  }
  .alert-kind {
    flex: none;
    margin-top: 1px;
    padding: 2px 7px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--neon-violet) 24%, transparent);
    color: var(--text);
    font-family: var(--font-label);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    white-space: nowrap;
  }
  .alert-kind.sub {
    background: color-mix(in srgb, var(--neon-pink) 22%, transparent);
  }
  .alert-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .alert-show {
    font-size: 13px;
    font-weight: 600;
  }
  .alert-msg {
    font-size: 12px;
    color: var(--text-2);
  }

  @keyframes drop-in {
    from {
      opacity: 0;
      transform: translateY(-6px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }

  /* ── Responsive ──────────────────────────────────────────────────────── */
  @media (max-width: 1180px) {
    .search-pill {
      width: 210px;
    }
    .search-pill.open,
    .search-pill:focus-within {
      width: 280px;
    }
  }
  @media (max-width: 1020px) {
    .navbar {
      gap: 12px;
    }
    .kbd {
      display: none;
    }
    .chip-pill :global(.chip-ja) {
      display: none;
    }
  }
  @media (max-width: 820px) {
    .search-pill {
      width: 150px;
    }
    .search-pill.open,
    .search-pill:focus-within {
      width: 220px;
    }
    .link {
      padding: 0 8px;
    }
    .link.active::after {
      left: 8px;
      right: 8px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .navbar,
    .search-pill {
      transition: none;
    }
    .search-panel,
    .menu,
    .alerts-popover {
      animation: none;
    }
    .eq i {
      animation: none;
    }
  }
</style>
