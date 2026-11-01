<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    api,
    img,
    type PersonSearchResult,
    type SeasonalItem,
  } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import {
    displayTitle,
    formatLabel,
    seasonLabel,
    statusLabel,
    untilTime,
  } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import PersonModal from "$lib/components/PersonModal.svelte";
  import TorrentPickerDialog from "$lib/components/TorrentPickerDialog.svelte";

  const GENRES = [
    "Action",
    "Adventure",
    "Comedy",
    "Drama",
    "Ecchi",
    "Fantasy",
    "Horror",
    "Mahou Shoujo",
    "Mecha",
    "Music",
    "Mystery",
    "Psychological",
    "Romance",
    "Sci-Fi",
    "Slice of Life",
    "Sports",
    "Supernatural",
    "Thriller",
  ];

  const POPULAR_SEIYUU = [
    "Atsumi Tanezaki",
    "Kana Hanazawa",
    "Aoi Yuuki",
    "Saori Hayami",
    "Rie Takahashi",
    "Kenjiro Tsuda",
    "Yuichi Nakamura",
    "Mamoru Miyano",
    "Takahiro Sakurai",
    "Hiroshi Kamiya",
    "Megumi Ogata",
    "Maaya Sakamoto",
  ];

  let tab = $state<"anime" | "people">("anime");
  let search = $state("");
  let genre = $state("");
  let format = $state("");
  let status = $state("");
  let season = $state("");
  let year = $state<string>("");
  let sort = $state("TRENDING_DESC");
  let pageNum = $state(1);

  let items = $state<SeasonalItem[]>([]);
  let loading = $state(true);
  let loadingMore = $state(false);
  let hasMore = $state(true);
  let error = $state<string | null>(null);
  let heroIdx = $state(0);
  let sentinelEl = $state<HTMLDivElement | undefined>(undefined);

  // Voice Actor / Staff search state
  let peopleQuery = $state("");
  let peopleResults = $state<PersonSearchResult[]>([]);
  let peopleLoading = $state(false);
  let selectedPerson = $state<{ id: number; name: string } | null>(null);
  let streamTarget = $state<{ anilistId: number; title: string } | null>(null);

  const heroItems = $derived(items.slice(0, 6));
  const hero = $derived(heroItems[heroIdx] ?? null);

  async function fetchCatalog(reset = true) {
    if (reset) {
      loading = true;
      pageNum = 1;
      hasMore = true;
      heroIdx = 0;
    } else {
      if (loadingMore || !hasMore) return;
      loadingMore = true;
    }
    error = null;
    try {
      const res = await api.browseCatalog({
        search: search.trim() || null,
        genre: genre || null,
        format: format || null,
        status: status || null,
        season: season || null,
        year: year ? Number(year) : null,
        sort: search.trim() && sort === "TRENDING_DESC" ? "SEARCH_MATCH" : sort,
        page: pageNum,
      });
      if (res.length < 18) {
        hasMore = false;
      }
      if (reset) {
        items = res;
      } else {
        const seen = new Set(items.map((x) => x.anilistId));
        const added = res.filter((x) => !seen.has(x.anilistId));
        if (added.length === 0) {
          hasMore = false;
        } else {
          items = [...items, ...added];
        }
      }
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  $effect(() => {
    const el = sentinelEl;
    if (!el || tab !== "anime") return;
    const obs = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting && !loading && !loadingMore && hasMore && !error) {
          pageNum += 1;
          fetchCatalog(false);
        }
      },
      { rootMargin: "480px 0px" },
    );
    obs.observe(el);
    return () => obs.disconnect();
  });

  async function runPeopleSearch(q: string) {
    peopleQuery = q;
    if (!q.trim()) {
      peopleResults = [];
      return;
    }
    peopleLoading = true;
    try {
      peopleResults = await api.searchPeople(q);
    } catch (e) {
      app.toast(String(e), "error", 5000);
    } finally {
      peopleLoading = false;
    }
  }

  async function toggleInLibrary(item: SeasonalItem, ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    try {
      const nowIn = await api.toggleStreamLibrary(item.anilistId);
      item.inLibrary = nowIn || item.ownedCount > 0;
      app.toast(
        nowIn
          ? `Added “${displayTitle(item)}” to your library`
          : `Removed “${displayTitle(item)}” from your library`,
        "info",
        3500,
      );
    } catch (e) {
      app.toast(String(e), "error", 5000);
    }
  }

  function resetFilters() {
    search = "";
    genre = "";
    format = "";
    status = "";
    season = "";
    year = "";
    sort = "TRENDING_DESC";
    fetchCatalog(true);
  }

  onMount(() => {
    fetchCatalog(true);
  });
</script>

<svelte:head>
  <title>Discover · Kura</title>
</svelte:head>

<div class="page discover-page">
  <header class="disc-head">
    <div>
      <h1>Discover Anime</h1>
      <p class="muted">
        Browse the entire AniList catalog, filter by genre or voice actor (seiyuu), add shows to your unified library, and stream or download any episode in <strong>mpv</strong> or <strong>Memento</strong>.
      </p>
    </div>
    <div class="tab-switch" role="tablist">
      <button
        class="tab-btn"
        class:active={tab === "anime"}
        onclick={() => (tab = "anime")}
        id="discover-tab-anime"
      >
        <Icon name="library" size={15} /> Anime Catalog
      </button>
      <button
        class="tab-btn"
        class:active={tab === "people"}
        onclick={() => {
          tab = "people";
          if (!peopleQuery && peopleResults.length === 0) {
            runPeopleSearch("Atsumi Tanezaki");
          }
        }}
        id="discover-tab-people"
      >
        <Icon name="user" size={15} /> Voice Actors & Staff
      </button>
    </div>
  </header>

  {#if tab === "anime"}
    {#if hero && !search.trim() && !genre && !format && !status && !season && !year}
      <section class="hero-card">
        {#if hero.bannerUrl || hero.coverUrl}
          <img
            class="hero-bg"
            src={hero.bannerUrl ?? hero.coverUrl}
            alt=""
            class:blur={!hero.bannerUrl}
          />
        {/if}
        <div class="hero-shade"></div>
        <div class="hero-content">
          <div class="hero-badges">
            <span class="badge badge-accent">#{heroIdx + 1} Trending</span>
            {#if hero.format}<span class="badge">{formatLabel(hero.format)}</span>{/if}
            {#if hero.seasonYear}<span class="badge">{seasonLabel(hero.season, hero.seasonYear)}</span>{/if}
            {#if hero.averageScore}<span class="badge"><Icon name="star" size={11} fill /> {hero.averageScore}%</span>{/if}
            {#if hero.nextAiringEpisode && hero.nextAiringAt}
              <span class="badge badge-ok">Ep {hero.nextAiringEpisode} in {untilTime(hero.nextAiringAt)}</span>
            {/if}
          </div>
          <h2>{displayTitle(hero)}</h2>
          {#if hero.genres.length > 0}
            <p class="hero-genres">{hero.genres.slice(0, 5).join(" · ")}</p>
          {/if}
          <div class="hero-actions">
            <button
              class="btn btn-primary"
              onclick={() => (streamTarget = { anilistId: hero.anilistId, title: displayTitle(hero) })}
            >
              <Icon name="play" size={15} fill /> Watch Ep 1
            </button>
            <button class="btn" onclick={() => goto(`/anime/${hero.anilistId}`)}>
              View Details
            </button>
            <button
              class="btn"
              class:btn-accent={hero.inLibrary}
              onclick={(e) => toggleInLibrary(hero, e)}
            >
              <Icon name={hero.inLibrary ? "check" : "plus"} size={14} />
              {hero.inLibrary ? "In Library" : "Add to Library"}
            </button>
          </div>
        </div>
        {#if heroItems.length > 1}
          <div class="hero-dots">
            {#each heroItems as _, i (i)}
              <button
                class="dot"
                class:active={heroIdx === i}
                onclick={() => (heroIdx = i)}
                aria-label={`Slide ${i + 1}`}
              ></button>
            {/each}
          </div>
        {/if}
      </section>
    {/if}

    <section class="filters-bar card">
      <form
        class="search-form"
        onsubmit={(e) => {
          e.preventDefault();
          fetchCatalog(true);
        }}
      >
        <div class="search-box">
          <Icon name="search" size={15} />
          <input
            class="input"
            type="search"
            placeholder="Search any anime on AniList (English, Romaji, 日本語)…"
            bind:value={search}
            id="discover-search-input"
          />
        </div>
        <button class="btn btn-primary btn-sm" type="submit">Search</button>
      </form>

      <div class="filter-row">
        <select class="input select" bind:value={sort} onchange={() => fetchCatalog(true)} aria-label="Sort by">
          <option value="TRENDING_DESC">Trending Now</option>
          <option value="POPULARITY_DESC">Most Popular</option>
          <option value="SCORE_DESC">Highest Rated</option>
          <option value="FAVOURITES_DESC">Most Favorited</option>
          <option value="START_DATE_DESC">Newest Releases</option>
        </select>

        <select class="input select" bind:value={genre} onchange={() => fetchCatalog(true)} aria-label="Genre">
          <option value="">All Genres</option>
          {#each GENRES as g (g)}
            <option value={g}>{g}</option>
          {/each}
        </select>

        <select class="input select" bind:value={format} onchange={() => fetchCatalog(true)} aria-label="Format">
          <option value="">All Formats</option>
          <option value="TV">TV Series</option>
          <option value="MOVIE">Movie</option>
          <option value="ONA">ONA</option>
          <option value="OVA">OVA</option>
          <option value="SPECIAL">Special</option>
        </select>

        <select class="input select" bind:value={status} onchange={() => fetchCatalog(true)} aria-label="Airing status">
          <option value="">Any Status</option>
          <option value="RELEASING">Airing Now</option>
          <option value="FINISHED">Finished</option>
          <option value="NOT_YET_RELEASED">Upcoming</option>
        </select>

        <select class="input select" bind:value={season} onchange={() => fetchCatalog(true)} aria-label="Season">
          <option value="">Any Season</option>
          <option value="WINTER">Winter</option>
          <option value="SPRING">Spring</option>
          <option value="SUMMER">Summer</option>
          <option value="FALL">Fall</option>
        </select>

        <input
          class="input year-input"
          type="number"
          min="1960"
          max="2030"
          placeholder="Year"
          bind:value={year}
          onchange={() => fetchCatalog(true)}
        />

        {#if search || genre || format || status || season || year || sort !== "TRENDING_DESC"}
          <button class="btn btn-ghost btn-sm" onclick={resetFilters}>
            <Icon name="x" size={13} /> Reset
          </button>
        {/if}
      </div>
    </section>

    {#if error}
      <div class="notice danger">
        <Icon name="alert" size={16} />
        <span>{error}</span>
      </div>
    {/if}

    {#if loading}
      <div class="catalog-grid">
        {#each Array(12) as _, i (i)}
          <div class="skeleton card-skel"></div>
        {/each}
      </div>
    {:else if items.length === 0}
      <div class="card empty-box">
        <strong>No anime matched those filters.</strong>
        <button class="btn btn-sm" onclick={resetFilters}>Clear filters</button>
      </div>
    {:else}
      <div class="catalog-grid">
        {#each items as item (item.anilistId)}
          {@const cover = img(item.coverPath, item.coverUrl)}
          <a class="disc-card" href={`/anime/${item.anilistId}`} id={`disc-card-${item.anilistId}`}>
            <div class="disc-poster">
              {#if cover}<img src={cover} alt={displayTitle(item)} loading="lazy" />{/if}
              <div class="top-badges">
                {#if item.averageScore}
                  <span class="badge"><Icon name="star" size={10} fill /> {item.averageScore}%</span>
                {/if}
                {#if item.ownedCount > 0}
                  <span class="badge badge-ok">Local · {item.ownedCount}</span>
                {:else if item.inLibrary}
                  <span class="badge badge-accent">In Library</span>
                {/if}
              </div>
              <button
                class="quick-lib-btn"
                class:active={item.inLibrary}
                onclick={(e) => toggleInLibrary(item, e)}
                title={item.inLibrary ? "Remove from Library" : "Add to Library"}
              >
                <Icon name={item.inLibrary ? "check" : "plus"} size={14} stroke={2.5} />
              </button>
            </div>
            <div class="disc-body">
              <strong class="disc-title" title={displayTitle(item)}>{displayTitle(item)}</strong>
              <span class="disc-sub faint">
                {[formatLabel(item.format), item.seasonYear, statusLabel(item.status)].filter(Boolean).join(" · ")}
              </span>
            </div>
          </a>
        {/each}
      </div>

      <div class="load-more-row" bind:this={sentinelEl}>
        {#if loadingMore}
          <span class="loading-more-pill">
            <span class="spinner"></span> Loading more anime…
          </span>
        {:else if !hasMore && items.length > 0}
          <span class="faint">You've reached the end of the results.</span>
        {/if}
      </div>
    {/if}
  {:else}
    <section class="filters-bar card">
      <form
        class="search-form"
        onsubmit={(e) => {
          e.preventDefault();
          runPeopleSearch(peopleQuery);
        }}
      >
        <div class="search-box">
          <Icon name="search" size={15} />
          <input
            class="input"
            type="search"
            placeholder="Search any Voice Actor (Seiyuu), Director, Composer, or Creator…"
            bind:value={peopleQuery}
            id="discover-people-input"
          />
        </div>
        <button class="btn btn-primary btn-sm" type="submit" disabled={peopleLoading}>
          Search People
        </button>
      </form>

      <div class="seiyuu-chips">
        <span class="faint">Popular Seiyuu:</span>
        {#each POPULAR_SEIYUU as name (name)}
          <button
            class="chip"
            class:active={peopleQuery.toLowerCase() === name.toLowerCase()}
            onclick={() => runPeopleSearch(name)}
          >
            {name}
          </button>
        {/each}
      </div>
    </section>

    {#if peopleLoading}
      <div class="people-grid">
        {#each Array(8) as _, i (i)}
          <div class="skeleton person-skel"></div>
        {/each}
      </div>
    {:else if peopleResults.length === 0}
      <div class="card empty-box">
        <strong>Search for a voice actor or staff member above.</strong>
        <p class="faint">Click any voice actor or director to explore every anime they worked on and play or add them to your library.</p>
      </div>
    {:else}
      <div class="people-grid">
        {#each peopleResults as p (p.staffId)}
          <button
            class="person-card card"
            onclick={() => (selectedPerson = { id: p.staffId, name: p.nameFull })}
          >
            <div class="person-avatar">
              {#if p.imageUrl}<img src={p.imageUrl} alt={p.nameFull} loading="lazy" />{/if}
            </div>
            <div class="person-info">
              <strong>{p.nameFull}</strong>
              {#if p.nameNative}<span class="jp faint">{p.nameNative}</span>{/if}
              {#if p.occupations.length > 0}
                <div class="occ-badges">
                  {#each p.occupations.slice(0, 2) as occ (occ)}
                    <span class="badge">{occ}</span>
                  {/each}
                </div>
              {/if}
            </div>
          </button>
        {/each}
      </div>
    {/if}
  {/if}
</div>

{#if selectedPerson}
  <PersonModal
    staffId={selectedPerson.id}
    initialName={selectedPerson.name}
    onclose={() => (selectedPerson = null)}
  />
{/if}

{#if streamTarget}
  <TorrentPickerDialog
    anilistId={streamTarget.anilistId}
    title={streamTarget.title}
    epKey="1"
    episodeNumber={1}
    autoStart={app.torrentQuality !== "ask"}
    onclose={() => (streamTarget = null)}
  />
{/if}

<style>
  .discover-page {
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding-bottom: 48px;
  }
  .disc-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .tab-switch {
    display: inline-flex;
    gap: 6px;
    padding: 4px;
    background: var(--surface);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-md);
    box-shadow: var(--sticker-sm);
  }
  .tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 7px 14px;
    border-radius: 8px;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 13.5px;
    font-weight: 700;
    cursor: pointer;
  }
  .tab-btn.active {
    background: var(--coral);
    color: var(--on-coral);
  }

  /* Hero */
  .hero-card {
    position: relative;
    min-height: 290px;
    border-radius: var(--r-xl);
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker-lg);
    overflow: hidden;
    display: flex;
    align-items: flex-end;
    padding: 26px;
    background: var(--surface);
  }
  .hero-bg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 25%;
  }
  .hero-bg.blur {
    filter: blur(22px) brightness(0.7);
    transform: scale(1.15);
  }
  .hero-shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--bg) 92%, transparent) 0%,
      color-mix(in srgb, var(--bg) 72%, transparent) 55%,
      color-mix(in srgb, var(--bg) 30%, transparent) 100%
    );
  }
  .hero-content {
    position: relative;
    z-index: 2;
    max-width: 620px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .hero-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .hero-content h2 {
    font-size: 30px;
    line-height: 1.1;
  }
  .hero-genres {
    font-size: 13px;
    color: var(--text-2);
  }
  .hero-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 4px;
  }
  .hero-dots {
    position: absolute;
    z-index: 2;
    right: 22px;
    bottom: 22px;
    display: flex;
    gap: 6px;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 999px;
    border: 1.5px solid var(--line);
    background: var(--surface-2);
    cursor: pointer;
  }
  .dot.active {
    width: 24px;
    background: var(--coral);
  }

  /* Filters */
  .filters-bar {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
  }
  .search-form {
    display: flex;
    gap: 10px;
  }
  .search-box {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--bg-elev);
  }
  .search-box .input {
    border: 0;
    background: transparent;
    padding: 0;
    width: 100%;
  }
  .filter-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .year-input {
    width: 96px;
  }
  .seiyuu-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  /* Grid */
  .catalog-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(175px, 1fr));
    gap: 16px;
  }
  .disc-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    text-decoration: none;
    color: inherit;
  }
  .disc-poster {
    position: relative;
    aspect-ratio: 2 / 3;
    border-radius: var(--r-lg);
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
    overflow: hidden;
    background: var(--surface-2);
    transition: transform var(--t-fast) var(--ease);
  }
  .disc-card:hover .disc-poster {
    transform: translate(-2px, -2px);
  }
  .disc-poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .top-badges {
    position: absolute;
    top: 8px;
    left: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-start;
  }
  .quick-lib-btn {
    position: absolute;
    bottom: 8px;
    right: 8px;
    width: 32px;
    height: 32px;
    border-radius: 999px;
    border: var(--bw) solid var(--line);
    background: var(--surface);
    color: var(--text);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    box-shadow: var(--sticker-sm);
    cursor: pointer;
  }
  .quick-lib-btn.active {
    background: var(--coral);
    color: var(--on-coral);
  }
  .disc-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 2px;
  }
  .disc-title {
    font-size: 13.5px;
    line-height: 1.25;
    display: -webkit-box;
    line-clamp: 2;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .disc-sub {
    font-size: 11.5px;
  }
  .load-more-row {
    display: flex;
    justify-content: center;
    align-items: center;
    min-height: 52px;
    padding: 16px 0 8px;
  }
  .loading-more-pill {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 8px 18px;
    border-radius: 99px;
    border: var(--bw) solid var(--line);
    background: var(--surface-2);
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 700;
    box-shadow: 2px 2px 0 var(--line);
  }

  /* People grid */
  .people-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 12px;
  }
  .person-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px;
    text-align: left;
    color: inherit;
    cursor: pointer;
    transition: transform var(--t-fast) var(--ease);
  }
  .person-card:hover {
    transform: translate(-2px, -2px);
  }
  .person-avatar {
    width: 58px;
    height: 76px;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    overflow: hidden;
    background: var(--surface-2);
    flex-shrink: 0;
  }
  .person-avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .person-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .occ-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .card-skel {
    aspect-ratio: 2 / 3;
    border-radius: var(--r-lg);
  }
  .person-skel {
    height: 96px;
    border-radius: var(--r-md);
  }
  .empty-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 40px 20px;
    text-align: center;
  }
</style>

