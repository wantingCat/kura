<script lang="ts">
  import { untrack } from "svelte";
  import { page } from "$app/state";
  import type { MediaCard } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, groupCards, norm, searchHaystack, type LibraryItem } from "$lib/format";
  import { JA, jaWatchStatus } from "$lib/ja";
  import HomePanels from "$lib/components/HomePanels.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Ja from "$lib/components/Ja.svelte";
  import LibraryFilterPanel from "$lib/components/LibraryFilterPanel.svelte";
  import NewLibraryForm from "$lib/components/NewLibraryForm.svelte";
  import PosterCard from "$lib/components/PosterCard.svelte";
  import {
    AIRING,
    filters,
    SORTS,
    WATCH_TABS,
    watchBucket,
    type FormatFilter,
    type SortKey,
    type WatchFilter,
  } from "$lib/libraryFilters.svelte";

  const libId = $derived(page.url.searchParams.get("lib"));
  const library = $derived(libId ? app.libraries.find((l) => String(l.id) === libId) : null);

  /** Navbar sections: `?format=series`, `?format=movie`, `?watch=planning` (My List). */
  const formatParam = $derived(page.url.searchParams.get("format"));
  const watchParam = $derived(page.url.searchParams.get("watch"));
  const SERIES_FORMATS = ["TV", "TV_SHORT", "ONA", "OVA", "SPECIAL"];
  const urlFormatOk = (c: MediaCard) =>
    formatParam === "series"
      ? SERIES_FORMATS.includes(c.format ?? "")
      : formatParam === "movie"
        ? c.format === "MOVIE"
        : true;
  const modeHead = $derived(
    formatParam === "series"
      ? JA.pages.series
      : formatParam === "movie"
        ? JA.pages.movies
        : watchParam === "planning"
          ? JA.pages.myList
          : null,
  );

  // `?watch=planning` selects the Planning tab when the URL changes (user can still switch tabs).
  $effect(() => {
    if (watchParam === "planning") untrack(() => filters.setWatch("planning"));
  });

  const scoped = $derived(
    (library ? app.cards.filter((c) => c.libraryIds.includes(library.id)) : app.cards).filter(urlFormatOk),
  );

  const haystacks = $derived(new Map(scoped.map((c) => [c.anilistId, searchHaystack(c)])));

  const total = (c: MediaCard) => c.episodes ?? (c.format === "MOVIE" ? 1 : null);
  const progressRatio = (c: MediaCard) => {
    const denom = total(c) ?? c.ownedCount;
    return denom > 0 ? Math.min(1, c.watchedCount / denom) : 0;
  };

  const formatOk = (c: MediaCard, f: FormatFilter) => {
    if (f === "tv") return ["TV", "TV_SHORT", "ONA"].includes(c.format ?? "");
    if (f === "movie") return c.format === "MOVIE";
    if (f === "extra") return ["OVA", "SPECIAL", "ONA"].includes(c.format ?? "");
    return true;
  };
  /** Seasons of one show become a single item unless the user turned grouping off. */
  const present = (list: MediaCard[]) => groupCards(list, app.groupSeasons);

  function countGrouped(list: MediaCard[], group: boolean): number {
    if (!group || list.length <= 1) return list.length;
    const seen = new Set<number>();
    for (let i = 0; i < list.length; i++) seen.add(list[i].franchiseId);
    return seen.size;
  }

  const baseEntries = $derived.by(() => {
    const q = norm(app.query);
    const terms = q ? q.split(" ") : [];
    return scoped.filter((c) => {
      if (terms.length) {
        const h = haystacks.get(c.anilistId) ?? "";
        if (!terms.every((t) => h.includes(t))) return false;
      }
      if (!formatOk(c, filters.format)) return false;
      return filters.matches(c);
    });
  });

  const watchCounts = $derived.by(() => {
    const group = app.groupSeasons;
    if (!group) {
      const out = { watching: 0, planning: 0, completed: 0, paused: 0, dropped: 0, all: baseEntries.length };
      for (const c of baseEntries) out[watchBucket(c)]++;
      return out;
    }
    const allIds = new Set<number>();
    const byBucket: Record<Exclude<WatchFilter, "all">, Set<number>> = {
      watching: new Set<number>(),
      planning: new Set<number>(),
      completed: new Set<number>(),
      paused: new Set<number>(),
      dropped: new Set<number>(),
    };
    for (const c of baseEntries) {
      allIds.add(c.franchiseId);
      byBucket[watchBucket(c)].add(c.franchiseId);
    }
    return {
      watching: byBucket.watching.size,
      planning: byBucket.planning.size,
      completed: byBucket.completed.size,
      paused: byBucket.paused.size,
      dropped: byBucket.dropped.size,
      all: allIds.size,
    };
  });

  $effect(() => {
    if (!filters.userPickedWatch && scoped.length > 0 && watchCounts[filters.watch] === 0) {
      if (watchCounts.watching > 0) filters.watch = "watching";
      else if (watchCounts.planning > 0) filters.watch = "planning";
      else if (watchCounts.completed > 0) filters.watch = "completed";
      else filters.watch = "all";
    }
  });

  /** Memoize the single `groupCards` call for the active watch tab so sorting never re-groups. */
  const groupedActive = $derived.by(() =>
    present(
      filters.watch === "all"
        ? baseEntries
        : baseEntries.filter((c) => watchBucket(c) === filters.watch),
    ),
  );

  const filtered = $derived.by(() => {
    const list = [...groupedActive];
    const dir = filters.desc ? -1 : 1;
    const tie = (a: LibraryItem, b: LibraryItem) => displayTitle(a).localeCompare(displayTitle(b));
    const by: Record<SortKey, (a: LibraryItem, b: LibraryItem) => number> = {
      added: (a, b) => dir * (a.addedAt - b.addedAt) || tie(a, b),
      title: (a, b) => dir * tie(a, b),
      year: (a, b) => dir * ((a.yearEnd ?? a.seasonYear ?? 0) - (b.yearEnd ?? b.seasonYear ?? 0)) || tie(a, b),
      watched: (a, b) => dir * ((a.lastWatchedAt ?? 0) - (b.lastWatchedAt ?? 0)) || tie(a, b),
      score: (a, b) => dir * ((a.averageScore ?? 0) - (b.averageScore ?? 0)) || tie(a, b),
      progress: (a, b) => dir * (progressRatio(a) - progressRatio(b)) || tie(a, b),
    };
    return list.sort(by[filters.sort]);
  });

  const GRID_INITIAL_LIMIT = 180;
  const GRID_STEP = 120;
  let gridLimit = $state(GRID_INITIAL_LIMIT);

  $effect(() => {
    void libId;
    void formatParam;
    void app.query;
    void filters.watch;
    void filters.format;
    void filters.airing;
    void filters.sort;
    void filters.desc;
    void filters.advancedCount;
    gridLimit = GRID_INITIAL_LIMIT;
  });

  const visibleGrid = $derived(
    filtered.length > gridLimit ? filtered.slice(0, gridLimit) : filtered,
  );

  function infiniteReveal(node: HTMLElement) {
    if (typeof IntersectionObserver === "undefined") return;
    const obs = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting) && gridLimit < filtered.length) {
          gridLimit = Math.min(filtered.length, gridLimit + GRID_STEP);
        }
      },
      { rootMargin: "420px" },
    );
    obs.observe(node);
    return {
      destroy() {
        obs.disconnect();
      },
    };
  }

  const browsing = $derived(!app.query && !filters.active && (filters.watch === "watching" || filters.watch === "all"));
  /** Hero + rows only on the plain Home view (not Series / Movies / My List). */
  const showHome = $derived(browsing && !modeHead);

  const counts = $derived.by(() => {
    const group = app.groupSeasons;
    const panelScoped = scoped.filter(
      (c) => filters.matches(c) && (filters.watch === "all" || watchBucket(c) === filters.watch),
    );
    return {
      all: countGrouped(panelScoped, group),
      tv: countGrouped(panelScoped.filter((c) => formatOk(c, "tv")), group),
      movie: countGrouped(panelScoped.filter((c) => formatOk(c, "movie")), group),
      extra: countGrouped(panelScoped.filter((c) => formatOk(c, "extra")), group),
    };
  });

  const activeWatchLabel = $derived(
    WATCH_TABS.find((t) => t.key === filters.watch)?.label ?? "Library",
  );
  const airingLabel = $derived(AIRING.find((a) => a.key === filters.airing)?.label ?? "");
</script>

<svelte:head>
  <title>{library ? `${library.name} · Kura` : "Kura"}</title>
  <meta name="description" content="Your local anime library — continue watching, new episodes and everything you own." />
</svelte:head>

{#if !app.loaded}
  <div class="page top-pad">
    <div class="skeleton hero-skel"></div>
    <div class="grid">
      {#each Array(12) as _, i (i)}<div class="skeleton poster-skel"></div>{/each}
    </div>
  </div>
{:else if app.libraries.length === 0 && app.appMode === "local"}
  <!-- First run -->
  <section class="onboarding">
    <div class="onboard-card">
      <img class="onboard-logo" src="/brand/logo.png" alt="Kura logo" width="112" height="112" />
      <h1>Your anime folders,<br /><span class="hl">neatly stored.</span></h1>
      <p class="muted lead">
        <span class="jp">蔵</span> (kura) means storehouse. Point Kura at the folders where you keep anime — it reads your
        release filenames, matches them with AniList, and builds a library with artwork, synopses and full episode info.
        Your files stay where they are.
      </p>
      <div class="form-wrap">
        <img class="mascot form-cat" src="/brand/cat.png" alt="" width="130" />
        <div class="card form-card">
          <NewLibraryForm />
        </div>
        <div class="card stream-mode-promo" style="margin-top: 14px; padding: 14px 16px; display: flex; align-items: center; justify-content: space-between; gap: 12px;">
          <div style="text-align: left;">
            <strong>Want to stream or browse the full AniList catalog?</strong>
            <p class="faint" style="font-size: 12.5px; margin-top: 2px;">Switch to Streaming &amp; Hybrid Mode to browse every show, filter by voice actors, and stream or download episodes in Kura's Built-In Player or mpv.</p>
          </div>
          <button class="btn btn-primary btn-sm" onclick={() => app.setAppMode("streaming")}>
            Enable Streaming Mode
          </button>
        </div>
      </div>
    </div>
  </section>
{:else}
  {#if showHome}
    <HomePanels {scoped} />
  {/if}

  <div class="page" class:after-hero={showHome} class:top-pad={!showHome}>
    {#if modeHead}
      <header class="page-head">
        <Ja text={modeHead.ja} class="kanji-watermark" />
        <h1>{library ? `${modeHead.en} · ${library.name}` : modeHead.en}</h1>
      </header>
    {:else}
      <h1 class="sr-only">{library?.name ?? "Your anime library"}</h1>
    {/if}

    {#if app.libraries.length > 1}
      <nav class="libs" aria-label="Libraries">
        <a class="chip" class:active={!library} href="/" id="lib-all">All libraries</a>
        {#each app.libraries as lib (lib.id)}
          <a class="chip" class:active={library?.id === lib.id} href={`/?lib=${lib.id}`} id={`lib-${lib.id}`}>
            {lib.name}<span class="chip-count">{lib.mediaCount}</span>
          </a>
        {/each}
      </nav>
    {/if}

    {#if app.reviewCount > 0 && browsing}
      <a class="review-banner" href="/review" id="review-banner">
        <Icon name="alert" size={18} />
        <span>
          {#if app.unmatched.length > 0 && app.reviewCount > app.unmatched.length}
            <strong>{app.reviewCount} folders</strong> need review ({app.unmatched.length} unmatched, {app.reviewCount - app.unmatched.length} low-confidence).
          {:else if app.unmatched.length > 0}
            <strong>{app.unmatched.length} folder{app.unmatched.length === 1 ? "" : "s"}</strong>
            couldn't be identified automatically.
          {:else}
            <strong>{app.reviewCount} match{app.reviewCount === 1 ? "" : "es"}</strong> may need verification.
          {/if}
        </span>
        <span class="link">Review <Icon name="chevron-right" size={14} /></span>
      </a>
    {/if}

    <section class="row" aria-labelledby={modeHead ? undefined : "library-heading"}>
      {#if !modeHead}
        <h2 class="lib-heading" id="library-heading">Your Library<Ja text={JA.rows.library} class="ja-sub" /></h2>
      {/if}
      <nav class="status-tabs" aria-label="Watch status">
        {#each WATCH_TABS as tab (tab.key)}
          {@const cnt = watchCounts[tab.key]}
          {#if tab.key === "watching" || tab.key === "planning" || tab.key === "completed" || tab.key === "all" || cnt > 0 || filters.watch === tab.key}
            <button
              class="status-tab"
              class:active={filters.watch === tab.key}
              aria-pressed={filters.watch === tab.key}
              onclick={() => filters.setWatch(tab.key as WatchFilter)}
              id={`tab-watch-${tab.key}`}
            >
              <span>{tab.label}</span>
              <Ja text={jaWatchStatus(tab.key)} class="tab-ja" />
              <span class="tab-badge">{cnt}</span>
            </button>
          {/if}
        {/each}
      </nav>

      <div class="toolbar">
        <h3 class="section-title">
          {#if app.query}
            Results for “{app.query}” · {activeWatchLabel} <span class="count">{filtered.length}</span>
          {:else}
            {library ? `${library.name} · ${activeWatchLabel}` : activeWatchLabel} <span class="count">{filtered.length}</span>
          {/if}
        </h3>
        <div class="right">
          <button
            class="btn btn-sm filter-btn"
            class:active={filters.panelOpen || filters.advancedCount > 0}
            aria-expanded={filters.panelOpen}
            onclick={() => (filters.panelOpen = !filters.panelOpen)}
            id="toggle-filters"
          >
            <Icon name="filter" size={14} />
            Filters
            {#if filters.advancedCount > 0}
              <span class="filter-badge">{filters.advancedCount}</span>
            {/if}
          </button>
          <div class="sort-group">
            <select
              class="input select"
              value={filters.sort}
              onchange={(e) => filters.setSort((e.currentTarget as HTMLSelectElement).value as SortKey)}
              id="sort-select"
              aria-label="Sort"
            >
              {#each SORTS as s (s.key)}
                <option value={s.key}>{s.label}</option>
              {/each}
            </select>
            <button
              class="btn btn-sm btn-icon sort-dir"
              onclick={() => filters.flipDirection()}
              title={filters.desc ? "Sort descending (click to reverse)" : "Sort ascending (click to reverse)"}
              aria-label={filters.desc ? "Sort descending" : "Sort ascending"}
              id="sort-direction"
            >
              <Icon name={filters.desc ? "arrow-down" : "arrow-up"} size={14} />
            </button>
          </div>
        </div>
      </div>

      {#if !formatParam}
        <div class="chips">
          {#each [["all", "All"], ["tv", "Series"], ["movie", "Movies"], ["extra", "OVA & Specials"]] as [k, label] (k)}
            <button
              class="chip"
              class:active={filters.format === k}
              onclick={() => (filters.format = k as FormatFilter)}
              id={`filter-format-${k}`}
            >
              {label}<span class="chip-count">{counts[k as FormatFilter]}</span>
            </button>
          {/each}
        </div>
      {/if}

      {#if filters.panelOpen}
        <LibraryFilterPanel entries={scoped} />
      {/if}

      {#if filters.advancedCount > 0}
        <div class="active-filters" aria-label="Active filters">
          {#if filters.airing !== "all"}
            <button class="active-pill" onclick={() => (filters.airing = "all")}>
              <span class="k">Status:</span> {airingLabel}
              <Icon name="x" size={12} />
            </button>
          {/if}
          {#each filters.genres as g (g)}
            <button class="active-pill" onclick={() => filters.toggle("genres", g)}>
              <span class="k">Genre:</span> {g}
              <Icon name="x" size={12} />
            </button>
          {/each}
          {#each filters.studios as s (s)}
            <button class="active-pill" onclick={() => filters.toggle("studios", s)}>
              <span class="k">Studio:</span> {s}
              <Icon name="x" size={12} />
            </button>
          {/each}
          {#each filters.tags as t (t)}
            <button class="active-pill" onclick={() => filters.toggle("tags", t)}>
              <span class="k">Tag:</span> {t}
              <Icon name="x" size={12} />
            </button>
          {/each}
          <button class="clear-link" onclick={() => filters.clearAll()} id="clear-all-filters">
            Clear all
          </button>
        </div>
      {/if}

      {#if filtered.length > 0}
        <div class="grid">
          {#each visibleGrid as m, i (m.anilistId)}<PosterCard {m} index={i} />{/each}
        </div>
        {#if visibleGrid.length < filtered.length}
          <div class="grid-more" use:infiniteReveal>
            <button
              type="button"
              class="btn btn-glass btn-pill"
              onclick={() => (gridLimit = Math.min(filtered.length, gridLimit + GRID_STEP))}
            >
              Show more ({filtered.length - visibleGrid.length} remaining)
            </button>
          </div>
        {/if}
      {:else if scoped.length === 0 && app.scanning}
        <div class="empty">
          <img class="mascot empty-cat" src="/brand/cat.png" alt="" width="150" />
          <h3>Building your library…</h3>
          <p class="muted">{app.progress?.message ?? "Scanning your folders"}</p>
        </div>
      {:else if scoped.length === 0}
        <div class="empty">
          <img class="mascot empty-cat" src="/brand/cat.png" alt="" width="150" />
          <h3>No anime found yet</h3>
          <p class="muted">Check your folders in Settings, then rescan.</p>
          <div class="empty-actions">
            <a class="btn" href="/settings">Open settings</a>
            <button class="btn btn-primary" onclick={() => app.scan()} id="empty-rescan">
              <Icon name="refresh" size={15} /> Rescan
            </button>
          </div>
        </div>
      {:else}
        <div class="empty">
          <img class="mascot empty-cat" src="/brand/cat.png" alt="" width="150" />
          <h3>Nothing matches</h3>
          <p class="muted">Try a different title, or clear the filters.</p>
          {#if app.query || filters.active}
            <div class="empty-actions">
              {#if app.query}
                <button class="btn" onclick={() => (app.query = "")} id="clear-search">Clear search</button>
              {/if}
              {#if filters.active}
                <button class="btn btn-primary" onclick={() => filters.clearAll()} id="empty-clear-filters">
                  Clear filters
                </button>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </section>
  </div>
{/if}

<style>
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .libs {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 18px;
  }

  /* Library status tabs ---------------------------------------------------- */
  .status-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 18px;
  }
  .status-tab {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 34px;
    padding: 0 14px;
    border-radius: 99px;
    border: none;
    background: rgba(255, 255, 255, 0.04);
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 13.5px;
    font-weight: 600;
    cursor: pointer;
    transition:
      background 0.14s ease,
      color 0.14s ease;
  }
  .status-tab:hover {
    color: var(--text);
    background: var(--glass);
  }
  .status-tab.active {
    background: var(--primary);
    color: var(--on-primary);
  }
  .tab-badge {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .status-tab.active .tab-badge {
    color: color-mix(in srgb, var(--on-primary) 55%, transparent);
  }

  /* Library toolbar -------------------------------------------------------- */
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .toolbar .section-title {
    margin: 0;
  }
  .chips,
  .right {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
  }
  .chips {
    margin-bottom: 18px;
  }
  .chip-count {
    font-family: var(--font);
    font-size: 11px;
    font-weight: 700;
    color: var(--text-3);
  }
  .filter-btn {
    height: 34px;
    padding: 0 12px;
    font-size: 13px;
  }
  .filter-btn.active {
    border-color: var(--coral);
    color: var(--coral);
  }
  .filter-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--coral);
    color: var(--on-coral);
    font-size: 11px;
    font-weight: 800;
  }
  .sort-group {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .sort-dir {
    width: 34px;
    height: 34px;
    padding: 0;
  }
  .select {
    height: 34px;
    padding: 0 30px 0 12px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23cdbab0' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 10px center;
  }
  .select option {
    background: var(--surface);
  }

  .active-filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-bottom: 20px;
  }
  .active-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 999px;
    border: 1.5px solid var(--coral);
    background: var(--coral-soft);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
    overflow-wrap: anywhere;
  }
  .active-pill .k {
    color: var(--text-3);
    font-weight: 600;
  }
  .active-pill:hover {
    background: var(--coral);
    color: var(--on-coral);
  }
  .active-pill:hover .k {
    color: inherit;
    opacity: 0.85;
  }
  .clear-link {
    padding: 4px 8px;
    border: 0;
    background: none;
    color: var(--coral);
    font: inherit;
    font-size: 12.5px;
    font-weight: 700;
    cursor: pointer;
  }
  .clear-link:hover {
    text-decoration: underline;
  }

  .review-banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    margin-bottom: 28px;
    border-radius: var(--r-md);
    background: var(--surface);
    border: 1px solid var(--border);
    border-left: 4px solid var(--amber);
    box-shadow: var(--sticker);
    color: var(--amber);
    transition:
      background var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease);
  }
  .review-banner:hover {
    background: var(--surface-2);
    border-color: var(--border-strong);
  }
  .review-banner span {
    color: var(--text-2);
  }
  .review-banner strong {
    color: var(--text);
  }
  .review-banner .link {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--amber);
    font-weight: 600;
  }

  /* Rows & grid ----------------------------------------------------------- */
  .row {
    margin-bottom: 36px;
  }
  .page {
    padding-left: var(--gutter);
    padding-right: var(--gutter);
    max-width: none;
  }
  .page.after-hero {
    padding-top: 28px;
  }
  /* No hero → clear the floating glass navbar. */
  .page.top-pad {
    padding-top: calc(var(--nav-h) + var(--nav-inset) + 28px);
  }
  .page-head {
    position: relative;
    overflow: hidden;
    isolation: isolate;
    padding: 36px 0 28px;
    margin-bottom: 8px;
  }
  .page-head h1 {
    position: relative;
    z-index: 1;
    font-size: clamp(32px, 3.4vw, 48px);
    font-weight: 800;
    letter-spacing: -0.03em;
  }
  .lib-heading {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    margin-bottom: 14px;
  }
  .status-tab :global(.tab-ja) {
    font-size: 11px;
    font-weight: 500;
    opacity: 0.75;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(158px, 1fr));
    gap: 28px 18px;
  }
  .grid-more {
    display: flex;
    justify-content: center;
    padding: 28px 0 8px;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 50px 20px 70px;
    text-align: center;
    color: var(--text-3);
  }
  .empty-cat {
    margin-bottom: 4px;
    animation: peek 600ms var(--bounce) both;
  }
  .empty h3 {
    color: var(--text);
    font-size: 22px;
    margin-top: 6px;
  }
  .empty-actions {
    display: flex;
    gap: 10px;
    margin-top: 10px;
  }

  /* Skeletons ------------------------------------------------------------- */
  .hero-skel {
    height: 300px;
    border-radius: var(--r-xl);
    margin-bottom: 32px;
  }
  .poster-skel {
    aspect-ratio: 2/3;
    border-radius: var(--r-md);
  }

  /* Onboarding ------------------------------------------------------------ */
  .onboarding {
    position: relative;
    min-height: 100vh;
    display: grid;
    place-items: center;
    padding: 48px 32px;
    overflow: hidden;
  }
  .onboard-card {
    position: relative;
    width: min(620px, 100%);
    animation: fade-up 700ms var(--ease) both;
  }
  .onboard-logo {
    display: block;
    filter: drop-shadow(0 18px 36px rgba(3, 4, 10, 0.75)) drop-shadow(0 0 28px rgba(139, 92, 246, 0.35));
    transition: transform var(--t-med) var(--ease);
  }
  .onboard-logo:hover {
    transform: translateY(-2px) scale(1.03);
  }
  .onboarding h1 {
    margin-top: 28px;
    font-size: 48px;
    font-weight: 600;
  }
  .hl {
    color: var(--coral);
  }
  .lead {
    margin-top: 16px;
    font-size: 15.5px;
    max-width: 560px;
  }
  .lead .jp {
    color: var(--text);
    font-weight: 700;
  }
  .form-wrap {
    position: relative;
    margin-top: 28px;
    padding-top: 62px;
  }
  .form-cat {
    position: absolute;
    top: 0;
    right: 48px;
    animation: peek 600ms var(--bounce) both 400ms;
  }
  .form-card {
    position: relative;
    padding: 26px;
    box-shadow: var(--sticker-lg);
  }
</style>
