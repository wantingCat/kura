<script lang="ts">
  import { page } from "$app/state";
  import type { MediaCard } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, groupCards, norm, searchHaystack, type LibraryItem } from "$lib/format";
  import HomePanels from "$lib/components/HomePanels.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import NewLibraryForm from "$lib/components/NewLibraryForm.svelte";
  import PosterCard from "$lib/components/PosterCard.svelte";

  type FormatFilter = "all" | "tv" | "movie" | "extra";
  type WatchFilter = "all" | "unwatched" | "watching" | "completed";
  type SortKey = "added" | "title" | "year" | "watched" | "score";

  let formatFilter = $state<FormatFilter>("all");
  let watchFilter = $state<WatchFilter>("all");
  let sort = $state<SortKey>("added");

  const libId = $derived(page.url.searchParams.get("lib"));
  const library = $derived(libId ? app.libraries.find((l) => String(l.id) === libId) : null);

  const scoped = $derived(
    library ? app.cards.filter((c) => c.libraryIds.includes(library.id)) : app.cards,
  );

  const haystacks = $derived(new Map(scoped.map((c) => [c.anilistId, searchHaystack(c)])));

  const total = (c: MediaCard) => c.episodes ?? (c.format === "MOVIE" ? 1 : null);
  const isCompleted = (c: MediaCard) => {
    const t = total(c);
    return t !== null && t > 0 && c.watchedCount >= t;
  };
  const isWatching = (c: MediaCard) => c.watchedCount > 0 && !isCompleted(c);

  const formatOk = (c: MediaCard, f: FormatFilter) => {
    if (f === "tv") return ["TV", "TV_SHORT", "ONA"].includes(c.format ?? "");
    if (f === "movie") return c.format === "MOVIE";
    if (f === "extra") return ["OVA", "SPECIAL", "ONA"].includes(c.format ?? "");
    return true;
  };
  /** Seasons of one show become a single item unless the user turned grouping off. */
  const present = (list: MediaCard[]) => groupCards(list, app.groupSeasons);
  const all = $derived(present(scoped));

  const filtered = $derived.by(() => {
    const q = norm(app.query);
    const terms = q ? q.split(" ") : [];
    // Title + format filters look at individual entries (so "Movies" pulls the movie out of its franchise)…
    const entries = scoped.filter((c) => {
      if (terms.length) {
        const h = haystacks.get(c.anilistId) ?? "";
        if (!terms.every((t) => h.includes(t))) return false;
      }
      return formatOk(c, formatFilter);
    });
    // …while watch status is judged for the franchise as a whole.
    const list = present(entries).filter((c) => {
      if (watchFilter === "unwatched" && c.watchedCount > 0) return false;
      if (watchFilter === "watching" && !isWatching(c)) return false;
      if (watchFilter === "completed" && !isCompleted(c)) return false;
      return true;
    });
    const by: Record<SortKey, (a: LibraryItem, b: LibraryItem) => number> = {
      added: (a, b) => b.addedAt - a.addedAt,
      title: (a, b) => displayTitle(a).localeCompare(displayTitle(b)),
      year: (a, b) => (b.yearEnd ?? b.seasonYear ?? 0) - (a.yearEnd ?? a.seasonYear ?? 0),
      watched: (a, b) => (b.lastWatchedAt ?? 0) - (a.lastWatchedAt ?? 0),
      score: (a, b) => (b.averageScore ?? 0) - (a.averageScore ?? 0),
    };
    return list.sort(by[sort]);
  });

  const browsing = $derived(!app.query && formatFilter === "all" && watchFilter === "all");
  const recentlyAdded = $derived([...all].sort((a, b) => b.addedAt - a.addedAt).slice(0, 12));

  const counts = $derived({
    all: all.length,
    tv: present(scoped.filter((c) => formatOk(c, "tv"))).length,
    movie: present(scoped.filter((c) => formatOk(c, "movie"))).length,
    extra: present(scoped.filter((c) => formatOk(c, "extra"))).length,
  });
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
{:else if app.libraries.length === 0}
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
      <div class="toolbar">
        <h2 class="section-title">
          {#if app.query}
            Results for “{app.query}” <span class="count">{filtered.length}</span>
          {:else}
            {library?.name ?? "Library"} <span class="count">{filtered.length}</span>
          {/if}
        </h2>
        <div class="right">
          <select class="input select" bind:value={watchFilter} id="filter-watch" aria-label="Watch status">
            <option value="all">Any status</option>
            <option value="unwatched">Unwatched</option>
            <option value="watching">Watching</option>
            <option value="completed">Completed</option>
          </select>
          <select class="input select" bind:value={sort} id="sort-select" aria-label="Sort">
            <option value="added">Recently added</option>
            <option value="watched">Recently watched</option>
            <option value="title">Title</option>
            <option value="year">Year</option>
            <option value="score">Score</option>
          </select>
        </div>
      </div>

      <div class="chips">
        {#each [["all", "All"], ["tv", "Series"], ["movie", "Movies"], ["extra", "OVA & Specials"]] as [k, label] (k)}
          <button
            class="chip"
            class:active={formatFilter === k}
            onclick={() => (formatFilter = k as FormatFilter)}
            id={`filter-format-${k}`}
          >
            {label}<span class="chip-count">{counts[k as FormatFilter]}</span>
          </button>
        {/each}
      </div>

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
          {#if app.query}
            <div class="empty-actions">
              <button class="btn" onclick={() => (app.query = "")} id="clear-search">Clear search</button>
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

  /* Library toolbar -------------------------------------------------------- */
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 12px;
  }
  .toolbar .section-title {
    margin: 0;
  }
  .chips,
  .right {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .chips {
    margin-bottom: 24px;
  }
  .chip-count {
    font-family: var(--font);
    font-size: 11px;
    font-weight: 700;
    color: var(--text-3);
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
