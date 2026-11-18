<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { api, img, type SeasonalCalendar, type SeasonalItem } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, formatLabel, untilTime } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import Ja from "$lib/components/Ja.svelte";
  import { JA, jaWeekday, jaWeekdayLong, enWeekdayShort, jaDate, jaTime, jaStatus, jaSeason } from "$lib/ja";

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

  // Local time, 24-hour (Japanese TV-listing style), e.g. 23:30.
  function formatLocalTime(unixSec: number): string {
    return jaTime(new Date(unixSec * 1000));
  }

  function startOfLocalDay(d: Date): Date {
    const x = new Date(d);
    x.setHours(0, 0, 0, 0);
    return x;
  }

  /** Week columns start at TODAY and run for 7 days, each with its real date. */
  const columns = $derived.by(() => {
    void nowSec;
    const t0 = startOfLocalDay(new Date());
    return Array.from({ length: 7 }, (_, k) => {
      const date = new Date(t0);
      date.setDate(t0.getDate() + k);
      const day = DAYS.find((d) => d.idx === date.getDay()) ?? DAYS[0];
      return { ...day, date, offset: k };
    });
  });

  /** Whole local days between today and the item's next airing (0 = today). */
  function airingDayOffset(unixSec: number): number {
    void nowSec;
    const t0 = startOfLocalDay(new Date()).getTime();
    const a = startOfLocalDay(new Date(unixSec * 1000)).getTime();
    return Math.round((a - t0) / 86_400_000);
  }

  function enShortDate(d: Date): string {
    return d.toLocaleDateString([], { month: "short", day: "numeric" });
  }

  const STATUS_EN: Record<string, string> = {
    RELEASING: "Airing",
    FINISHED: "Finished",
    NOT_YET_RELEASED: "Not yet aired",
    CANCELLED: "Cancelled",
    HIATUS: "On hiatus",
  };

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
  const trackedCount = $derived(
    (cal?.items ?? []).filter(
      (i) => i.inLibrary || i.anilistStatus === "CURRENT" || i.anilistStatus === "PLANNING",
    ).length,
  );
  const airingCount = $derived(filteredItems.filter((i) => itemDayOfWeek(i) !== null).length);
  const timezoneName = Intl.DateTimeFormat().resolvedOptions().timeZone || "Local time";
</script>

<svelte:head>
  <title>Seasonal Calendar · Kura</title>
</svelte:head>

<div class="page cal-page">
  <header class="cal-head">
    <Ja text={JA.pages.calendar.ja} class="kanji-watermark" />
    <div class="title-block">
      <div class="kicker-row">
        <span class="badge badge-accent"><Icon name="calendar" size={11} /> Broadcast Schedule</span>
        <span class="tz tn-label">{timezoneName}</span>
      </div>
      <h1>Seasonal Airing Calendar <Ja text={JA.pages.calendar.ja} class="ja-sub" /></h1>
      <p class="muted">
        Weekly broadcast schedule in your local timezone, with live episode countdowns and library highlights.
      </p>
    </div>

    <div class="controls">
      <div class="season-picker">
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
              {s.label}<Ja text={jaSeason(s.key)} class="s-tab-ja" />
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
        Library & AniList <span class="chip-count">{trackedCount}</span>
      </button>
    </div>

    <label class="cal-search">
      <Icon name="search" size={14} />
      <input
        class="input"
        bind:value={query}
        placeholder={app.japaneseAccents ? JA.placeholders.calendarFilter : "Filter calendar — title, studio, genre…"}
        aria-label="Filter calendar by title, studio or genre"
        id="cal-filter-input"
      />
      {#if query}
        <button class="clear-btn" onclick={() => (query = "")} aria-label="Clear filter"><Icon name="x" size={13} /></button>
      {/if}
    </label>
  </div>

  {#if error}
    <div class="notice danger">
      <Icon name="alert" size={16} />
      <span>{error}</span>
    </div>
  {/if}

  {#snippet dayHead(d: (typeof columns)[number], isToday: boolean, count: number | null)}
    {@const date = d.date}
    <header class="day-head">
      <h3 class="day-mark" title={`${jaWeekdayLong(d.idx)} · ${d.name}`}>
        <Ja text={jaWeekday(d.idx)} label={d.name} essential class="day-kanji" />
        <span class="tn-label day-en ja-deco" aria-hidden="true">{enWeekdayShort(d.idx)}</span>
      </h3>
      <div class="day-meta">
        {#if app.japaneseAccents}
          <span class="day-long" lang="ja" aria-hidden="true">{jaWeekdayLong(d.idx)} <span lang="en">· {d.name}</span></span>
        {/if}
        <Ja text={jaDate(date)} label={enShortDate(date)} essential class="day-date" />
      </div>
      {#if isToday}
        <span class="today-chip" title="Today">
          <Ja text={JA.states.today} label="Today" essential />
        </span>
      {:else if count !== null}
        <span class="day-count" title={`${count} shows`}>{count}</span>
      {/if}
    </header>
  {/snippet}

  {#if loading}
    <div class="week-grid no-scrollbar">
      {#each columns as d (d.idx)}
        <div class="day-col" class:today={d.offset === 0}>
          {@render dayHead(d, d.offset === 0, null)}
          <div class="skeleton item-skel"></div>
          <div class="skeleton item-skel"></div>
        </div>
      {/each}
    </div>
  {:else}
    {#if airingCount > 0}
      <section class="week-grid no-scrollbar" aria-label="Weekly broadcast schedule, starting today">
        {#each columns as d (d.idx)}
          {@const items = byDay.get(d.idx) ?? []}
          {@const isToday = d.offset === 0}
          <div
            class="day-col"
            class:today={isToday}
            id={`cal-day-${d.short.toLowerCase()}`}
            aria-current={isToday ? "date" : undefined}
          >
            {@render dayHead(d, isToday, items.length)}

            <div class="day-list">
              {#each items as item (item.anilistId)}
                {@const cover = img(item.coverPath, item.coverUrl)}
                {@const countdown = item.nextAiringAt ? untilTime(item.nextAiringAt) : null}
                {@const nextWeek = item.nextAiringAt ? airingDayOffset(item.nextAiringAt) >= 7 : false}
                {#snippet cardInner()}
                  <div class="thumb">
                    {#if cover}<img src={cover} alt="" loading="lazy" />{/if}
                    {#if item.nextAiringAt}
                      <span class="time-pill" title="Local time">{formatLocalTime(item.nextAiringAt)}</span>
                    {/if}
                  </div>
                  <div class="body">
                    <div class="top-badges">
                      {#if item.status === "RELEASING"}
                        <span class="badge badge-air"><Ja text={jaStatus(item.status)} label="Airing" essential /></span>
                      {/if}
                      {#if item.inLibrary}
                        <span class="badge badge-ok">In Library</span>
                      {:else if item.anilistStatus}
                        <span class="badge">{item.anilistStatus === "CURRENT" ? "Watching" : "Planning"}</span>
                      {/if}
                      {#if nextWeek && item.nextAiringAt}
                        <span class="badge badge-later" title={`Next airs ${enShortDate(new Date(item.nextAiringAt * 1000))}`}>
                          <Ja text={`来週 ${jaDate(new Date(item.nextAiringAt * 1000))}`} label={`Next week, ${enShortDate(new Date(item.nextAiringAt * 1000))}`} essential />
                        </span>
                      {/if}
                      {#if item.nextAiringEpisode && countdown}
                        <span class="countdown">
                          {#if item.nextAiringAt && item.nextAiringAt > nowSec}<Ja text={JA.states.next} class="next-chip" />{/if}
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
                    {#if item.titleNative && item.titleNative !== displayTitle(item)}
                      <Ja text={item.titleNative} block class="native" />
                    {/if}
                    <p class="sub faint">
                      {#if item.inLibrary}
                        {item.ownedCount ?? 0}{#if item.episodes}/{item.episodes}{/if} eps in library · {item.watchedCount} watched
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
          <Ja text={jaSeason(season, year)} class="ja-sub" />
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
                  {#if item.status && jaStatus(item.status)}
                    <span class="badge" class:badge-air={item.status === "RELEASING"}>
                      <Ja text={jaStatus(item.status)} label={STATUS_EN[item.status] ?? item.status} essential />
                    </span>
                  {/if}
                </div>
                <h4 title={displayTitle(item)}>{displayTitle(item)}</h4>
                {#if item.titleNative && item.titleNative !== displayTitle(item)}
                  <Ja text={item.titleNative} block class="native" />
                {/if}
                <p class="faint">
                  {#if item.inLibrary}
                    {item.ownedCount ?? 0}{#if item.episodes}/{item.episodes}{/if} eps · {item.watchedCount} watched
                  {:else}
                    {[item.studios[0], item.episodes ? `${item.episodes} eps` : null, item.averageScore ? `★ ${item.averageScore}%` : null]
                      .filter(Boolean)
                      .join(" · ")}
                  {/if}
                </p>
              </div>
            {/snippet}

            <a
              class="cal-item u-item"
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
  /* Tokyo Night calendar — see design-system/kura-tokyo-night/pages/calendar.md */
  .cal-page {
    padding-top: calc(var(--nav-h) + var(--nav-inset) + 24px);
  }
  .cal-head {
    position: relative;
    overflow: hidden;
    isolation: isolate;
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 20px;
    flex-wrap: wrap;
    margin: 0 0 26px;
    padding-top: 12px;
  }
  .title-block,
  .controls {
    position: relative;
    z-index: 1;
  }
  .kicker-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  .tz {
    color: var(--text-2);
  }
  .cal-head h1 {
    font-family: var(--font);
    font-size: 34px;
    font-weight: 700;
    letter-spacing: -0.02em;
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
    border-radius: var(--r-pill);
    background: var(--surface);
    box-shadow: var(--shadow-sm);
  }
  .season-tabs {
    display: inline-flex;
    gap: 3px;
  }
  .s-tab {
    display: inline-flex;
    align-items: baseline;
    gap: 5px;
    padding: 5px 12px;
    border-radius: var(--r-pill);
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-family: var(--font);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition:
      color var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .s-tab :global(.s-tab-ja) {
    font-size: 11px;
    opacity: 0.75;
  }
  .s-tab:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .s-tab.active {
    background: var(--primary);
    color: var(--on-primary);
  }
  .s-tab:focus-visible {
    outline: 2px solid var(--neon-violet);
    outline-offset: 2px;
  }
  .year-label {
    font-family: var(--font-label);
    font-size: 14px;
    font-weight: 700;
    padding: 0 6px;
    color: var(--text);
    font-variant-numeric: tabular-nums;
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
    font-family: var(--font-label);
    font-size: 11px;
    font-weight: 700;
    color: var(--text-2);
  }
  .cal-search {
    position: relative;
    display: flex;
    align-items: center;
    width: min(320px, 100%);
  }
  .cal-search > :global(svg) {
    position: absolute;
    left: 12px;
    color: var(--text-2);
    pointer-events: none;
  }
  .cal-search .input {
    width: 100%;
    height: 36px;
    padding-left: 34px;
    padding-right: 30px;
    font-size: 13px;
    border: 0;
    border-radius: var(--r-pill);
    background: var(--surface);
  }
  .cal-search .input::placeholder {
    font-family: var(--font-jp), var(--font);
  }
  .cal-search .input:focus {
    background: var(--surface-2);
    box-shadow:
      0 0 0 2px var(--neon-violet),
      var(--neon-glow);
  }
  .clear-btn {
    position: absolute;
    right: 7px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: var(--r-pill);
    background: var(--surface-3);
    color: var(--text-2);
    cursor: pointer;
  }

  /* Week grid ---------------------------------------------------------------- */
  /* Below 1200px: one horizontally scrollable row of 7 days (snap). */
  .week-grid {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(250px, 1fr);
    gap: 14px;
    align-items: start;
    margin-bottom: 36px;
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    padding-bottom: 6px;
  }
  .week-grid > .day-col {
    scroll-snap-align: start;
  }
  /* Desktop: a single 7-column week. */
  @media (min-width: 1200px) {
    .week-grid {
      grid-auto-flow: row;
      grid-template-columns: repeat(7, minmax(0, 1fr));
      grid-auto-columns: auto;
      gap: 10px;
      overflow-x: visible;
      scroll-snap-type: none;
    }
    .day-col {
      padding: 10px;
    }
    .cal-item {
      gap: 8px;
      padding: 7px;
    }
    .thumb {
      width: 44px;
      height: 64px;
    }
    .day-mark :global(.day-kanji) {
      font-size: 26px;
    }
    .day-head {
      gap: 8px;
      flex-wrap: wrap;
    }
    h4 {
      font-size: 13px;
    }
  }
  .day-col {
    position: relative;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    border-radius: var(--r-md);
    background: var(--surface);
    box-shadow: var(--shadow-sm);
    transition: box-shadow var(--t-med) var(--ease);
  }
  .day-col.today {
    background: color-mix(in srgb, var(--neon-violet) 7%, var(--surface));
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--neon-violet) 45%, transparent),
      var(--neon-glow);
  }
  .day-head {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    padding-bottom: 12px;
  }
  /* Thin divider (not a card border) — becomes a neon underline on today. */
  .day-head::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 1px;
    background: var(--line);
    border-radius: 99px;
  }
  .day-col.today .day-head::after {
    height: 2px;
    background: var(--neon);
    box-shadow: 0 0 10px rgba(244, 114, 182, 0.55);
  }
  .day-mark {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 44px;
    margin: 0;
    line-height: 1;
  }
  .day-mark :global(.day-kanji) {
    font-size: 30px;
    font-weight: 700;
    line-height: 1.05;
    color: var(--text);
  }
  /* English fallback when accents are off keeps a sane size. */
  .day-mark :global(.day-kanji:not(.ja)) {
    font-family: var(--font);
    font-size: 17px;
    font-weight: 700;
  }
  .day-col.today .day-mark :global(.day-kanji) {
    text-shadow: 0 0 18px rgba(139, 92, 246, 0.6);
  }
  .day-en {
    margin-top: 4px;
    font-size: 10px;
  }
  .day-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .day-long {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .day-long span {
    font-family: var(--font);
    color: var(--text-2);
    font-weight: 500;
  }
  .day-meta :global(.day-date) {
    font-size: 12px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .today-chip {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--r-pill);
    background: var(--cta);
    color: var(--on-cta);
    font-size: 11.5px;
    font-weight: 700;
    box-shadow: 0 0 14px rgba(190, 24, 93, 0.35);
  }
  .day-count {
    font-family: var(--font-label);
    font-size: 12px;
    font-weight: 700;
    color: var(--text-2);
    min-width: 22px;
    text-align: center;
  }

  .day-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cal-item {
    display: flex;
    gap: 11px;
    padding: 9px;
    border-radius: var(--r-md);
    border: 0;
    background: var(--surface-2);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    min-width: 0;
    text-decoration: none;
    transition:
      transform var(--t-med) var(--ease),
      background var(--t-med) var(--ease),
      box-shadow var(--t-med) var(--ease);
  }
  .cal-item:hover {
    transform: translateY(-2px);
    background: var(--surface-3);
    box-shadow: var(--neon-glow);
  }
  .cal-item:focus-visible {
    outline: 2px solid var(--neon-violet);
    outline-offset: 2px;
    box-shadow: var(--neon-glow);
  }
  .cal-item.owned {
    background: color-mix(in srgb, var(--ok) 7%, var(--surface-2));
    box-shadow: inset 3px 0 0 var(--ok);
  }
  .cal-item.owned:hover {
    box-shadow:
      inset 3px 0 0 var(--ok),
      var(--neon-glow);
  }
  .u-item {
    background: var(--surface);
    box-shadow: var(--shadow-sm);
  }

  .thumb,
  .u-cover {
    position: relative;
    width: 56px;
    height: 80px;
    flex: none;
    border-radius: 8px;
    overflow: hidden;
    background: var(--surface-3);
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
    border-radius: 6px;
    background: rgba(8, 9, 16, 0.82);
    color: #f4f2ff;
    font-family: var(--font-label);
    font-size: 11px;
    font-weight: 700;
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
  .badge-later {
    background: var(--surface-3);
    color: var(--text-2);
    letter-spacing: 0.02em;
    text-transform: none;
  }
  .badge-air {
    background: color-mix(in srgb, var(--neon-pink) 16%, var(--surface-3));
    color: var(--neon-pink);
    letter-spacing: 0.02em;
  }
  .countdown {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 700;
    color: var(--coral-hi);
  }
  .countdown :global(.next-chip) {
    padding: 0 5px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--neon-violet) 20%, transparent);
    font-size: 10.5px;
  }
  h4 {
    font-family: var(--font);
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
  .body :global(.native),
  .u-body :global(.native) {
    font-size: 11.5px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 11.5px;
    line-height: 1.35;
    color: var(--text-2);
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
    color: var(--text-2);
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
    box-shadow: inset 4px 0 0 var(--danger);
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

  @media (prefers-reduced-motion: reduce) {
    .cal-item,
    .cal-item:hover {
      transform: none;
      transition: none;
    }
  }
</style>
