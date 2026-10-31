<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { api, img, type SeasonalCalendar, type SeasonalItem } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, formatLabel, untilTime } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";

  const SEASONS = [
    { key: "WINTER", label: "Winter" },
    { key: "SPRING", label: "Spring" },
    { key: "SUMMER", label: "Summer" },
    { key: "FALL", label: "Fall" },
  ] as const;

  const DAYS = [
    { idx: 1, short: "Mon", name: "Monday" },
    { idx: 2, short: "Tue", name: "Tuesday" },
    { idx: 3, short: "Wed", name: "Wednesday" },
    { idx: 4, short: "Thu", name: "Thursday" },
    { idx: 5, short: "Fri", name: "Friday" },
    { idx: 6, short: "Sat", name: "Saturday" },
    { idx: 0, short: "Sun", name: "Sunday" },
  ];

  function currentSeasonYear(): { season: string; year: number } {
    const d = new Date();
    const m = d.getMonth() + 1;
    const season = m <= 3 ? "WINTER" : m <= 6 ? "SPRING" : m <= 9 ? "SUMMER" : "FALL";
    return { season, year: d.getFullYear() };
  }

  const cur = currentSeasonYear();
  let season = $state(cur.season);
  let year = $state(cur.year);
  let cal = $state<SeasonalCalendar | null>(null);
  let loading = $state(true);
  let refreshing = $state(false);
  let error = $state<string | null>(null);
  let scope = $state<"all" | "library" | "tracked">("all");
  let query = $state("");
  let nowSec = $state(Math.floor(Date.now() / 1000));

  async function load(force = false, silent = false) {
    if (force) refreshing = true;
    else if (!silent) loading = true;
    error = null;
    try {
      cal = await api.getSeasonalCalendar({ season, year, refresh: force });
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  function stepSeason(delta: number) {
    const idx = SEASONS.findIndex((s) => s.key === season);
    const next = idx + delta;
    if (next < 0) {
      season = "FALL";
      year -= 1;
    } else if (next >= SEASONS.length) {
      season = "WINTER";
      year += 1;
    } else {
      season = SEASONS[next].key;
    }
    load(false);
  }

  function resetToCurrent() {
    season = cur.season;
    year = cur.year;
    load(false);
  }

  function pickSeason(s: string) {
    season = s;
    load(false);
  }

  let prevAppVersion = app.version;
  onMount(() => {
    load(false);
    const t = setInterval(() => {
      nowSec = Math.floor(Date.now() / 1000);
    }, 30_000);
    return () => clearInterval(t);
  });

  $effect(() => {
    const v = app.version;
    if (v !== prevAppVersion) {
      prevAppVersion = v;
      untrack(() => {
        if (cal) load(false, true);
      });
    }
  });

  const todayJsDay = $derived.by(() => {
    void nowSec;
    return new Date().getDay();
  });

  const filteredItems = $derived.by(() => {
    const list = cal?.items ?? [];
    const q = query.trim().toLowerCase();
    return list.filter((item) => {
      if (scope === "library" && !item.inLibrary) return false;
      if (
        scope === "tracked" &&
        !item.inLibrary &&
        item.anilistStatus !== "CURRENT" &&
        item.anilistStatus !== "PLANNING"
      ) {
        return false;
      }
      if (q) {
        const hay = [
          item.titleEnglish,
          item.titleRomaji,
          item.titleNative,
          ...item.studios,
          ...item.genres,
        ]
          .filter(Boolean)
          .join(" ")
          .toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    });
  });

  function localMinuteOfDay(unixSec: number): number {
    const d = new Date(unixSec * 1000);
    return d.getHours() * 60 + d.getMinutes();
  }

  function formatLocalTime(unixSec: number): string {
    return new Date(unixSec * 1000).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function itemDayOfWeek(item: SeasonalItem): number | null {
    if (item.nextAiringAt) {
      return new Date(item.nextAiringAt * 1000).getDay();
    }
    if (item.startDate && /^\d{4}-\d{2}-\d{2}$/.test(item.startDate) && item.format !== "MOVIE") {
      const d = new Date(`${item.startDate}T12:00:00`);
      if (!isNaN(d.getTime())) return d.getDay();
    }
    return null;
  }

  const byDay = $derived.by(() => {
    const map = new Map<number, SeasonalItem[]>();
    for (const d of DAYS) map.set(d.idx, []);
    for (const item of filteredItems) {
      const day = itemDayOfWeek(item);
      if (day !== null) {
        map.get(day)?.push(item);
      }
    }
    for (const list of map.values()) {
      list.sort((a, b) => {
        if (a.nextAiringAt && b.nextAiringAt) {
          return (
            localMinuteOfDay(a.nextAiringAt) - localMinuteOfDay(b.nextAiringAt) ||
            displayTitle(a).localeCompare(displayTitle(b))
          );
        }
        if (a.nextAiringAt) return -1;
        if (b.nextAiringAt) return 1;
        return (
          Number(b.inLibrary) - Number(a.inLibrary) ||
          (b.averageScore ?? 0) - (a.averageScore ?? 0) ||
          displayTitle(a).localeCompare(displayTitle(b))
        );
      });
    }
    return map;
  });

  const unscheduled = $derived(
    filteredItems
      .filter((i) => itemDayOfWeek(i) === null)
      .sort((a, b) => Number(b.inLibrary) - Number(a.inLibrary) || (b.averageScore ?? 0) - (a.averageScore ?? 0)),
  );

  const inLibraryCount = $derived((cal?.items ?? []).filter((i) => i.inLibrary).length);
  const airingCount = $derived(filteredItems.filter((i) => itemDayOfWeek(i) !== null).length);
  const timezoneName = Intl.DateTimeFormat().resolvedOptions().timeZone || "Local time";
</script>

<svelte:head>
  <title>Seasonal Calendar · Kura</title>
</svelte:head>

<div class="page">
  <header class="cal-head">
    <div class="title-block">
      <div class="kicker-row">
        <span class="badge badge-accent"><Icon name="calendar" size={11} /> Broadcast Schedule</span>
        <span class="tz faint">{timezoneName}</span>
      </div>
      <h1>Seasonal Airing Calendar</h1>
      <p class="muted">
        Weekly broadcast schedule in your local timezone, with live episode countdowns and library highlights.
      </p>
    </div>

    <div class="controls">
      <div class="season-picker card">
        <button class="btn btn-ghost btn-sm btn-icon" onclick={() => stepSeason(-1)} aria-label="Previous season" id="cal-prev">
          <Icon name="arrow-left" size={15} />
        </button>
        <div class="season-tabs">
          {#each SEASONS as s (s.key)}
            <button
              class="s-tab"
              class:active={season === s.key}
              onclick={() => pickSeason(s.key)}
              id={`cal-season-${s.key.toLowerCase()}`}
            >
              {s.label}
            </button>
          {/each}
        </div>
        <span class="year-label">{year}</span>
        <button class="btn btn-ghost btn-sm btn-icon" onclick={() => stepSeason(1)} aria-label="Next season" id="cal-next">
          <Icon name="chevron-right" size={16} />
        </button>
      </div>

      {#if season !== cur.season || year !== cur.year}
        <button class="btn btn-sm" onclick={resetToCurrent} id="cal-current">This season</button>
      {/if}

      <button
        class="btn btn-sm"
        onclick={() => load(true)}
        disabled={refreshing || loading}
        title="Refresh airing schedule from AniList"
        id="cal-refresh"
      >
        {#if refreshing}
          <span class="spinner spinner-xs"></span>
        {:else}
          <Icon name="refresh" size={14} />
        {/if}
        Refresh
      </button>
    </div>
  </header>

  <div class="subbar">
    <div class="chips">
      <button class="chip" class:active={scope === "all"} onclick={() => (scope = "all")} id="cal-scope-all">
        All seasonal <span class="chip-count">{cal?.items.length ?? 0}</span>
      </button>
      <button class="chip" class:active={scope === "library"} onclick={() => (scope = "library")} id="cal-scope-library">
        In my library <span class="chip-count">{inLibraryCount}</span>
      </button>
      <button class="chip" class:active={scope === "tracked"} onclick={() => (scope = "tracked")} id="cal-scope-tracked">
        Library & AniList
      </button>
    </div>

    <label class="cal-search">
      <Icon name="search" size={14} />
      <input
        class="input"
        bind:value={query}
        placeholder="Filter calendar — title, studio, genre…"
        aria-label="Filter calendar"
        id="cal-filter-input"
      />
      {#if query}
        <button class="clear-btn" onclick={() => (query = "")} aria-label="Clear"><Icon name="x" size={13} /></button>
      {/if}
    </label>
  </div>

  {#if error}
    <div class="notice danger">
      <Icon name="alert" size={16} />
      <span>{error}</span>
    </div>
  {/if}

  {#if loading}
    <div class="week-grid">
      {#each DAYS as d (d.idx)}
        <div class="day-col card">
          <div class="day-head"><h3>{d.name}</h3></div>
          <div class="skeleton item-skel"></div>
          <div class="skeleton item-skel"></div>
        </div>
      {/each}
    </div>
  {:else}
    {#if airingCount > 0}
      <section class="week-grid" aria-label="Weekly broadcast schedule">
        {#each DAYS as d (d.idx)}
          {@const items = byDay.get(d.idx) ?? []}
          {@const isToday = d.idx === todayJsDay}
          <div class="day-col card" class:today={isToday} id={`cal-day-${d.short.toLowerCase()}`}>
            <header class="day-head">
              <h3>{d.name}</h3>
              {#if isToday}
                <span class="badge badge-accent">Today</span>
              {:else}
                <span class="day-count">{items.length}</span>
              {/if}
            </header>

            <div class="day-list">
              {#each items as item (item.anilistId)}
                {@const cover = img(item.coverPath, item.coverUrl)}
                {@const countdown = item.nextAiringAt ? untilTime(item.nextAiringAt) : null}
                {#snippet cardInner()}
                  <div class="thumb">
                    {#if cover}<img src={cover} alt="" loading="lazy" />{/if}
                    {#if item.nextAiringAt}
                      <span class="time-pill">{formatLocalTime(item.nextAiringAt)}</span>
                    {/if}
                  </div>
                  <div class="body">
                    <div class="top-badges">
                      {#if item.inLibrary}
                        <span class="badge badge-ok">In Library</span>
                      {:else if item.anilistStatus}
                        <span class="badge">{item.anilistStatus === "CURRENT" ? "Watching" : "Planning"}</span>
                      {/if}
                      {#if item.nextAiringEpisode && countdown}
                        <span class="countdown">
                          Ep {item.nextAiringEpisode}
                          {#if item.nextAiringAt && item.nextAiringAt > nowSec}
                            in {countdown}
                          {:else}
                            aired
                          {/if}
                        </span>
                      {/if}
                    </div>
                    <h4 title={displayTitle(item)}>{displayTitle(item)}</h4>
                    <p class="sub faint">
                      {#if item.inLibrary}
                        {item.ownedCount}{#if item.episodes}/{item.episodes}{/if} eps in library · {item.watchedCount} watched
                      {:else}
                        {[formatLabel(item.format), item.studios[0], item.averageScore ? `★ ${item.averageScore}%` : null]
                          .filter(Boolean)
                          .join(" · ")}
                      {/if}
                    </p>
                  </div>
                {/snippet}

                <a
                  class="cal-item"
                  class:owned={item.inLibrary}
                  class:external={!item.inLibrary}
                  href={`/anime/${item.linkId}`}
                  id={`cal-item-${item.anilistId}`}
                >
                  {@render cardInner()}
                </a>
              {:else}
                <p class="no-airing faint">No broadcasts scheduled</p>
              {/each}
            </div>
          </div>
        {/each}
      </section>
    {/if}

    {#if unscheduled.length > 0}
      <section class="unscheduled-block">
        <h2 class="section-title">
          {airingCount > 0 ? "Finished or TBA this season" : `${season} ${year} Lineup`}
          <span class="count">{unscheduled.length}</span>
        </h2>
        <div class="unscheduled-grid">
          {#each unscheduled as item (item.anilistId)}
            {@const cover = img(item.coverPath, item.coverUrl)}
            {#snippet unschedInner()}
              <div class="u-cover">
                {#if cover}<img src={cover} alt="" loading="lazy" />{/if}
              </div>
              <div class="u-body">
                <div class="top-badges">
                  {#if item.inLibrary}
                    <span class="badge badge-ok">In Library</span>
                  {:else if item.anilistStatus}
                    <span class="badge">{item.anilistStatus === "CURRENT" ? "Watching" : item.anilistStatus}</span>
                  {/if}
                  {#if item.format}<span class="badge">{formatLabel(item.format)}</span>{/if}
                </div>
                <h4 title={displayTitle(item)}>{displayTitle(item)}</h4>
                <p class="faint">
                  {#if item.inLibrary}
                    {item.ownedCount}{#if item.episodes}/{item.episodes}{/if} eps · {item.watchedCount} watched
                  {:else}
                    {[item.studios[0], item.episodes ? `${item.episodes} eps` : null, item.averageScore ? `★ ${item.averageScore}%` : null]
                      .filter(Boolean)
                      .join(" · ")}
                  {/if}
                </p>
              </div>
            {/snippet}

            <a
              class="cal-item card"
              class:owned={item.inLibrary}
              class:external={!item.inLibrary}
              href={`/anime/${item.linkId}`}
            >
              {@render unschedInner()}
            </a>
          {/each}
        </div>
      </section>
    {:else if airingCount === 0}
      <div class="empty">
        <img class="mascot" src="/brand/cat.png" alt="" width="140" />
        <h3>No shows match</h3>
        <p class="muted">Try switching from “In my library” to “All seasonal”, or clear the filter.</p>
      </div>
    {/if}
  {/if}
</div>

<style>
  .cal-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 20px;
    flex-wrap: wrap;
    margin: 8px 0 22px;
  }
  .kicker-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  .tz {
    font-size: 12px;
    font-weight: 600;
  }
  .cal-head h1 {
    font-size: 34px;
    font-weight: 600;
  }
  .cal-head p {
    margin-top: 4px;
    max-width: 600px;
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .season-picker {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border-radius: var(--r-md);
  }
  .season-tabs {
    display: inline-flex;
    gap: 3px;
  }
  .s-tab {
    padding: 5px 10px;
    border-radius: 8px;
    border: 1.5px solid transparent;
    background: transparent;
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 13.5px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .s-tab:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .s-tab.active {
    background: var(--coral);
    color: var(--on-coral);
    border-color: var(--line);
    box-shadow: 2px 2px 0 var(--line);
  }
  .year-label {
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 700;
    padding: 0 6px;
    color: var(--text);
  }

  .subbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    flex-wrap: wrap;
    margin-bottom: 22px;
  }
  .chips {
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
  .cal-search {
    position: relative;
    display: flex;
    align-items: center;
    width: min(320px, 100%);
  }
  .cal-search > :global(svg) {
    position: absolute;
    left: 11px;
    color: var(--text-3);
    pointer-events: none;
  }
  .cal-search .input {
    width: 100%;
    height: 34px;
    padding-left: 32px;
    padding-right: 28px;
    font-size: 13px;
  }
  .clear-btn {
    position: absolute;
    right: 6px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 5px;
    background: var(--surface-3);
    color: var(--text-2);
    cursor: pointer;
  }

  .week-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
    gap: 16px;
    align-items: start;
    margin-bottom: 36px;
  }
  .day-col {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .day-col.today {
    border-color: var(--coral);
    box-shadow: 5px 5px 0 var(--coral);
  }
  .day-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: 8px;
    border-bottom: 1.5px dashed var(--border);
  }
  .day-head h3 {
    font-size: 17px;
    font-weight: 600;
  }
  .day-count {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-3);
  }

  .day-list {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .cal-item {
    display: flex;
    gap: 11px;
    padding: 9px;
    border-radius: var(--r-md);
    border: 1.5px solid var(--border-strong);
    background: var(--bg-elev);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    min-width: 0;
    transition:
      transform var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .cal-item:hover {
    transform: translate(-2px, -2px);
    border-color: var(--line);
    box-shadow: 3px 3px 0 var(--line);
  }
  .cal-item.owned {
    border-color: var(--ok);
    background: color-mix(in srgb, var(--ok) 8%, var(--bg-elev));
  }
  .cal-item.owned:hover {
    box-shadow: 3px 3px 0 var(--ok);
  }

  .thumb,
  .u-cover {
    position: relative;
    width: 56px;
    height: 80px;
    flex: none;
    border-radius: 7px;
    border: 1.5px solid var(--line);
    overflow: hidden;
    background: var(--surface-2);
  }
  .thumb img,
  .u-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .time-pill {
    position: absolute;
    left: 3px;
    right: 3px;
    bottom: 3px;
    padding: 1px 4px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--line) 88%, transparent);
    color: var(--cream);
    font-size: 10px;
    font-weight: 800;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .body,
  .u-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .top-badges {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-wrap: wrap;
  }
  .countdown {
    font-size: 11px;
    font-weight: 800;
    color: var(--coral);
  }
  h4 {
    font-family: var(--font-display);
    font-size: 14px;
    font-weight: 600;
    line-height: 1.25;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .sub {
    font-size: 11.5px;
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .no-airing {
    padding: 16px 6px;
    text-align: center;
    font-size: 12.5px;
  }

  .unscheduled-block {
    margin-top: 24px;
  }
  .unscheduled-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }
  .item-skel {
    height: 92px;
    border-radius: var(--r-md);
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    margin-bottom: 18px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    border-left: 8px solid var(--danger);
    background: var(--surface);
    color: var(--danger);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 50px 20px;
    text-align: center;
  }
</style>

