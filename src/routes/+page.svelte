<script lang="ts">
  import { page } from "$app/state";
  import { img, type MediaCard } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { cleanDescription, displayTitle, formatLabel, norm, searchHaystack, seasonLabel, subTitle } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import NewLibraryForm from "$lib/components/NewLibraryForm.svelte";
  import PosterCard from "$lib/components/PosterCard.svelte";

  type FormatFilter = "all" | "tv" | "movie" | "extra";
  type WatchFilter = "all" | "unwatched" | "watching" | "completed";
  type SortKey = "added" | "title" | "year" | "watched" | "score";

  let query = $state("");
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

  const filtered = $derived.by(() => {
    const q = norm(query);
    const terms = q ? q.split(" ") : [];
    let list = scoped.filter((c) => {
      if (terms.length) {
        const h = haystacks.get(c.anilistId) ?? "";
        if (!terms.every((t) => h.includes(t))) return false;
      }
      if (formatFilter === "tv" && !["TV", "TV_SHORT", "ONA"].includes(c.format ?? "")) return false;
      if (formatFilter === "movie" && c.format !== "MOVIE") return false;
      if (formatFilter === "extra" && !["OVA", "SPECIAL", "ONA"].includes(c.format ?? "")) return false;
      if (watchFilter === "unwatched" && c.watchedCount > 0) return false;
      if (watchFilter === "watching" && !isWatching(c)) return false;
      if (watchFilter === "completed" && !isCompleted(c)) return false;
      return true;
    });
    const by: Record<SortKey, (a: MediaCard, b: MediaCard) => number> = {
      added: (a, b) => b.addedAt - a.addedAt,
      title: (a, b) => displayTitle(a).localeCompare(displayTitle(b)),
      year: (a, b) => (b.seasonYear ?? 0) - (a.seasonYear ?? 0),
      watched: (a, b) => (b.lastWatchedAt ?? 0) - (a.lastWatchedAt ?? 0),
      score: (a, b) => (b.averageScore ?? 0) - (a.averageScore ?? 0),
    };
    return [...list].sort(by[sort]);
  });

  const browsing = $derived(!query && formatFilter === "all" && watchFilter === "all");
  const continueWatching = $derived(
    scoped.filter(isWatching).sort((a, b) => (b.lastWatchedAt ?? 0) - (a.lastWatchedAt ?? 0)).slice(0, 12),
  );
  const recentlyAdded = $derived([...scoped].sort((a, b) => b.addedAt - a.addedAt).slice(0, 12));

  const featured = $derived.by(() => {
    const pool = continueWatching.length ? continueWatching : recentlyAdded;
    return pool.find((c) => c.bannerUrl || c.bannerPath) ?? pool[0] ?? null;
  });
  const featuredBanner = $derived(featured ? img(featured.bannerPath, featured.bannerUrl) ?? img(featured.coverPath, featured.coverUrl) : null);

  const counts = $derived({
    all: scoped.length,
    tv: scoped.filter((c) => ["TV", "TV_SHORT", "ONA"].includes(c.format ?? "")).length,
    movie: scoped.filter((c) => c.format === "MOVIE").length,
    extra: scoped.filter((c) => ["OVA", "SPECIAL", "ONA"].includes(c.format ?? "")).length,
  });

  let searchEl: HTMLInputElement | undefined = $state();
  function onKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      searchEl?.focus();
    }
  }
</script>

<svelte:window onkeydown={onKey} />
<svelte:head>
  <title>{library ? `${library.name} · Kura` : "Kura"}</title>
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
  {#if featured && browsing}
    <div class="hero-wrap">
      <img class="mascot hero-cat" src="/brand/cat.png" alt="" width="120" />
      <section class="hero">
        <div class="hero-content">
          <span class="eyebrow">
            {continueWatching.length ? "Continue watching" : "Recently added"}
          </span>
          <h1>{displayTitle(featured)}</h1>
          {#if subTitle(featured)}<p class="hero-sub">{subTitle(featured)}</p>{/if}
          <div class="hero-meta">
            {#if featured.format}<span class="badge badge-accent">{formatLabel(featured.format)}</span>{/if}
            {#if featured.seasonYear}<span>{seasonLabel(featured.season, featured.seasonYear)}</span>{/if}
            {#if featured.genres.length}<span>· {featured.genres.slice(0, 3).join(" · ")}</span>{/if}
          </div>
          <p class="hero-desc">{cleanDescription(featured.description)}</p>
          <div class="hero-actions">
            <a class="btn btn-primary" href={`/anime/${featured.anilistId}`} id="hero-open">
              <Icon name="play" size={15} fill />
              {featured.watchedCount > 0 ? "Continue" : "View episodes"}
            </a>
            <span class="faint">
              {featured.ownedCount} episode{featured.ownedCount === 1 ? "" : "s"} in library
            </span>
          </div>
        </div>
        <div class="hero-art">
          {#if featuredBanner}
            <img class="hero-bg" src={featuredBanner} alt="" />
          {:else}
            <img class="hero-kanji" src="/brand/kanji.png" alt="" />
          {/if}
        </div>
      </section>
    </div>
  {/if}

  <div class="page" class:no-hero={!(featured && browsing)}>
    <div class="toolbar">
      <div class="title-block">
        <h2>{library?.name ?? "All anime"}</h2>
        <span class="faint">{scoped.length} titles</span>
      </div>
      <div class="search">
        <Icon name="search" size={16} />
        <input
          bind:this={searchEl}
          class="input"
          bind:value={query}
          placeholder="Search titles — English, romaji, 日本語…"
          id="library-search"
        />
        {#if query}
          <button class="clear" onclick={() => (query = "")} aria-label="Clear search"><Icon name="x" size={14} /></button>
        {:else}
          <kbd>Ctrl K</kbd>
        {/if}
      </div>
    </div>

    <div class="filters">
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

    {#if browsing && continueWatching.length > 0}
      <section class="row">
        <h3 class="section-title">Continue watching <span class="count">{continueWatching.length}</span></h3>
        <div class="rail">
          {#each continueWatching as m, i (m.anilistId)}<PosterCard {m} index={i} />{/each}
        </div>
      </section>
    {/if}

    {#if browsing && recentlyAdded.length > 0 && scoped.length > 12}
      <section class="row">
        <h3 class="section-title">Recently added</h3>
        <div class="rail">
          {#each recentlyAdded as m, i (m.anilistId)}<PosterCard {m} index={i} />{/each}
        </div>
      </section>
    {/if}

    <section class="row">
      {#if browsing}
        <h3 class="section-title">Library <span class="count">{filtered.length}</span></h3>
      {:else}
        <h3 class="section-title">Results <span class="count">{filtered.length}</span></h3>
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
        </div>
      {/if}
    </section>
  </div>
{/if}

<style>
  /* Hero ------------------------------------------------------------------
   * A sticker panel: solid plum info side + banner art, with the logo cat
   * peeking over the top edge. */
  .hero-wrap {
    position: relative;
    margin: 0 auto;
    max-width: 1680px;
    padding: 62px 36px 0;
  }
  .hero-cat {
    position: absolute;
    top: 0;
    right: 96px;
    z-index: 0;
    animation: peek 600ms var(--bounce) both 200ms;
  }
  .hero {
    position: relative;
    z-index: 1;
    display: grid;
    grid-template-columns: minmax(380px, 5fr) 6fr;
    min-height: 340px;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-xl);
    background: var(--surface-3);
    box-shadow: var(--sticker-lg);
    overflow: hidden;
    animation: fade-up 500ms var(--ease) both;
  }
  .hero-art {
    position: relative;
    border-left: var(--bw) solid var(--line);
    background: var(--coral);
    overflow: hidden;
  }
  .hero-bg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 30%;
    animation: hero-in 900ms var(--ease) both;
  }
  .hero-kanji {
    position: absolute;
    inset: 0;
    margin: auto;
    width: 48%;
    height: auto;
  }
  @keyframes hero-in {
    from {
      opacity: 0;
      transform: scale(1.04);
    }
  }
  .hero-content {
    padding: 34px 36px 32px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    min-width: 0;
  }
  .eyebrow {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--coral);
  }
  .hero h1 {
    margin-top: 8px;
    font-size: 38px;
    font-weight: 600;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .hero-sub {
    margin-top: 4px;
    color: var(--text-2);
    font-size: 15px;
  }
  .hero-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    color: var(--text-2);
    font-size: 13px;
    font-weight: 600;
  }
  .hero-desc {
    margin-top: 12px;
    color: var(--text-2);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    max-width: 620px;
  }
  .hero-actions {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-top: 22px;
  }
  .hero-actions .btn {
    height: 44px;
    padding: 0 22px;
  }
  .hero-actions .faint {
    font-weight: 600;
  }
  @media (max-width: 1100px) {
    .hero {
      grid-template-columns: 1fr;
    }
    .hero-art {
      order: -1;
      height: 180px;
      border-left: none;
      border-bottom: var(--bw) solid var(--line);
    }
  }

  /* Toolbar --------------------------------------------------------------- */
  .page.no-hero {
    padding-top: 36px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    margin-bottom: 14px;
  }
  .title-block {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .title-block h2 {
    font-size: 28px;
    font-weight: 600;
  }
  .search {
    position: relative;
    display: flex;
    align-items: center;
    width: min(420px, 45%);
  }
  .search > :global(svg) {
    position: absolute;
    left: 13px;
    color: var(--text-3);
    pointer-events: none;
  }
  .search .input {
    width: 100%;
    padding-left: 38px;
    padding-right: 64px;
  }
  kbd {
    position: absolute;
    right: 10px;
    font-family: var(--font);
    font-size: 11px;
    font-weight: 700;
    color: var(--text-3);
    border: 1.5px solid var(--border-strong);
    border-bottom-width: 3px;
    border-radius: 6px;
    padding: 1px 6px;
    background: var(--surface);
  }
  .clear {
    position: absolute;
    right: 8px;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 6px;
    background: var(--surface-3);
    cursor: pointer;
  }
  .filters {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 26px;
  }
  .chips,
  .right {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
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
    margin-bottom: 26px;
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
    grid-auto-columns: 158px;
    gap: 18px;
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
