<script lang="ts">
  import { page } from "$app/state";
  import type { MediaCard } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, groupCards, norm, searchHaystack, type LibraryItem } from "$lib/format";
  import HomePanels from "$lib/components/HomePanels.svelte";
  import Icon from "$lib/components/Icon.svelte";
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

  const scoped = $derived(
    library ? app.cards.filter((c) => c.libraryIds.includes(library.id)) : app.cards,
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
  const all = $derived(present(scoped));

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

  const watchCounts = $derived({
    watching: present(baseEntries.filter((c) => watchBucket(c) === "watching")).length,
    planning: present(baseEntries.filter((c) => watchBucket(c) === "planning")).length,
    completed: present(baseEntries.filter((c) => watchBucket(c) === "completed")).length,
    paused: present(baseEntries.filter((c) => watchBucket(c) === "paused")).length,
    dropped: present(baseEntries.filter((c) => watchBucket(c) === "dropped")).length,
    all: present(baseEntries).length,
  });

  $effect(() => {
    if (!filters.userPickedWatch && scoped.length > 0 && watchCounts[filters.watch] === 0) {
      if (watchCounts.watching > 0) filters.watch = "watching";
      else if (watchCounts.planning > 0) filters.watch = "planning";
      else if (watchCounts.completed > 0) filters.watch = "completed";
      else filters.watch = "all";
    }
  });

  const filtered = $derived.by(() => {
    const entries =
      filters.watch === "all"
        ? baseEntries
        : baseEntries.filter((c) => watchBucket(c) === filters.watch);
    const list = present(entries);
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

  const browsing = $derived(!app.query && !filters.active && (filters.watch === "watching" || filters.watch === "all"));
  const recentlyAdded = $derived([...all].sort((a, b) => b.addedAt - a.addedAt).slice(0, 12));

  const panelScoped = $derived(
    scoped.filter(
      (c) => filters.matches(c) && (filters.watch === "all" || watchBucket(c) === filters.watch),
    ),
  );
  const counts = $derived({
    all: present(panelScoped).length,
    tv: present(panelScoped.filter((c) => formatOk(c, "tv"))).length,
    movie: present(panelScoped.filter((c) => formatOk(c, "movie"))).length,
    extra: present(panelScoped.filter((c) => formatOk(c, "extra"))).length,
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
  <div class="page">
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
            <strong>Want to stream or browse all of AniList (Hayase-style)?</strong>
            <p class="faint" style="font-size: 12.5px; margin-top: 2px;">Switch to Streaming & Hybrid Mode to browse every show, filter by voice actors, and stream or download episodes in mpv/Memento.</p>
          </div>
          <button class="btn btn-primary btn-sm" onclick={() => app.setAppMode("streaming")}>
            Enable Streaming Mode
          </button>
        </div>
      </div>
    </div>
  </section>
{:else}
  <div class="page">
    <h1 class="sr-only">{library?.name ?? "Your anime library"}</h1>

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

    {#if browsing}
      <HomePanels {scoped} />
    {/if}

    {#if app.unmatched.length > 0 && browsing}
      <a class="review-banner" href="/review" id="review-banner">
        <Icon name="alert" size={18} />
        <span>
          <strong>{app.unmatched.length} folder{app.unmatched.length === 1 ? "" : "s"}</strong>
          couldn't be identified automatically.
        </span>
        <span class="link">Review <Icon name="chevron-right" size={14} /></span>
      </a>
    {/if}

    {#if browsing && recentlyAdded.length > 0 && all.length > 8}
      <section class="row">
        <h2 class="section-title">Recently added</h2>
        <div class="rail">
          {#each recentlyAdded as m, i (m.anilistId)}<PosterCard {m} index={i} />{/each}
        </div>
      </section>
    {/if}

    <section class="row">
      <nav class="status-tabs" aria-label="Watch status">
        {#each WATCH_TABS as tab (tab.key)}
          {@const cnt = watchCounts[tab.key]}
          {#if tab.key === "watching" || tab.key === "planning" || tab.key === "completed" || tab.key === "all" || cnt > 0 || filters.watch === tab.key}
            <button
              class="status-tab"
              class:active={filters.watch === tab.key}
              onclick={() => filters.setWatch(tab.key as WatchFilter)}
              id={`tab-watch-${tab.key}`}
            >
              <span>{tab.label}</span>
              <span class="tab-badge">{cnt}</span>
            </button>
          {/if}
        {/each}
      </nav>

      <div class="toolbar">
        <h2 class="section-title">
          {#if app.query}
            Results for “{app.query}” · {activeWatchLabel} <span class="count">{filtered.length}</span>
          {:else}
            {library ? `${library.name} · ${activeWatchLabel}` : activeWatchLabel} <span class="count">{filtered.length}</span>
          {/if}
        </h2>
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
          {#each filtered as m, i (m.anilistId)}<PosterCard {m} index={i} />{/each}
        </div>
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
    gap: 8px;
    margin-bottom: 18px;
    padding-bottom: 14px;
    border-bottom: var(--bw) solid var(--border);
  }
  .status-tab {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 38px;
    padding: 0 16px;
    border-radius: var(--r-sm);
    border: var(--bw) solid var(--border);
    background: var(--surface);
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 13.5px;
    font-weight: 800;
    letter-spacing: 0.01em;
    cursor: pointer;
    box-shadow: 2px 2px 0 var(--shadow-color);
    transition:
      transform 0.14s var(--ease-spring),
      box-shadow 0.14s var(--ease-spring),
      background 0.14s ease,
      color 0.14s ease,
      border-color 0.14s ease;
  }
  .status-tab:hover {
    transform: translate(-1px, -1px);
    box-shadow: 3px 3px 0 var(--shadow-color);
    color: var(--text);
    background: var(--surface-2);
  }
  .status-tab.active {
    background: var(--coral);
    color: var(--on-coral);
    border-color: var(--ink);
    box-shadow: 3px 3px 0 var(--ink);
  }
  .tab-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 20px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--bg-2);
    color: var(--text-3);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 700;
  }
  .status-tab.active .tab-badge {
    background: rgba(20, 12, 13, 0.24);
    color: var(--on-coral);
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
    margin-bottom: 30px;
    border-radius: var(--r-md);
    background: var(--surface);
    border: var(--bw) solid var(--line);
    border-left: 8px solid var(--amber);
    box-shadow: var(--sticker);
    color: var(--amber);
    transition:
      transform var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .review-banner:hover {
    transform: translate(-1px, -1px);
    box-shadow: 4px 4px 0 var(--line);
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
  .rail {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 140px;
    gap: 16px;
    overflow-x: auto;
    padding: 8px 4px 14px;
    margin: -8px -4px 0;
    scroll-snap-type: x proximity;
  }
  .rail > :global(*) {
    scroll-snap-align: start;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(158px, 1fr));
    gap: 26px 20px;
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
    border-radius: 26px;
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker-lg);
    transform: rotate(-4deg);
    transition: transform var(--t-med) var(--bounce);
  }
  .onboard-logo:hover {
    transform: rotate(3deg) scale(1.04);
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
