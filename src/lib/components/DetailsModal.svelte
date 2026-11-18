<!--
  DetailsModal — Tokyo Night anime quick-view (glass modal, 東京ナイト).

  Opened from anywhere via `detailsModal.open(anilistId)`; mirrors `?anime=<id>`.
  Data comes from the same `api.getMediaDetail` call the full page uses, and
  episodes play through `app.play` exactly like /anime/[id]. Anything heavier
  (torrents, Jimaku, subtitle matcher, fix match, extras, AniList tracker) lives
  on the full page — the modal links there instead of duplicating it.
  Spec: design-system/kura-tokyo-night/pages/details.md
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { api, img, type EpisodeRow, type FranchiseEntry, type MediaDetail } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { detailsModal } from "$lib/detailsModal.svelte";
  import {
    cleanDescription,
    displayTitle,
    formatLabel,
    franchiseLabel,
    seasonLabel,
    statusLabel,
  } from "$lib/format";
  import {
    JA,
    jaCour,
    jaEpisode,
    jaEpisodeCount,
    jaFormat,
    jaGenre,
    jaRemaining,
    jaSeason,
    jaStatus,
  } from "$lib/ja";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";
  import TorrentPickerDialog from "./TorrentPickerDialog.svelte";
  import WebStreamDialog from "./WebStreamDialog.svelte";

  /** Extra Japanese vocabulary used only by the modal (decorative). */
  const JA_LOCAL = {
    play: "再生",
    resume: "続きから",
    synopsis: "あらすじ",
    details: "詳細",
    fullPage: "詳しく見る",
    retry: "再試行",
  } as const;

  /** The franchise entry currently shown (season pills switch this without touching the URL). */
  let viewId = $state<number | null>(null);
  let d = $state<MediaDetail | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let anilistSaving = $state(false);
  let togglingStreamLib = $state(false);
  let webTarget = $state<{ epKey: string; epNumber: number } | null>(null);
  let torrentTarget = $state<{ epKey: string; epNumber: number; autoStart: boolean } | null>(null);
  let dialogEl = $state<HTMLDivElement | undefined>();
  const cache = new Map<number, MediaDetail>();
  let reqSeq = 0;

  // Follow the store: a new id resets the in-modal season selection.
  $effect(() => {
    const id = detailsModal.anilistId;
    viewId = id;
    webTarget = null;
    torrentTarget = null;
    if (id === null) {
      d = null;
      error = null;
    }
  });

  async function load(target: number) {
    const seq = ++reqSeq;
    const cached = cache.get(target);
    if (cached) d = cached;
    else if (d?.anilistId !== target) d = null;
    loading = !d;
    error = null;
    try {
      const res = await api.getMediaDetail(target);
      cache.set(target, res);
      if (seq === reqSeq) d = res;
    } catch (e) {
      if (seq === reqSeq && !d) error = String(e);
    } finally {
      if (seq === reqSeq) loading = false;
    }
  }

  // (Re)load when the shown entry changes or the library changes (watch toggles, scans…).
  $effect(() => {
    void app.version;
    const id = viewId;
    if (id !== null) untrack(() => load(id));
  });

  const open = $derived(detailsModal.anilistId !== null);
  /** Library card for instant header (banner/title) while the detail loads. */
  const card = $derived(viewId !== null ? (app.cards.find((c) => c.anilistId === viewId) ?? null) : null);
  const head = $derived(d ?? card);

  const regular = $derived(d?.episodeList.filter((e) => !e.isSpecial) ?? []);
  const specials = $derived(d?.episodeList.filter((e) => e.isSpecial) ?? []);
  const owned = $derived(regular.filter((e) => e.files.length > 0).length);
  const totalEps = $derived(Math.max(d?.episodes ?? 0, regular.length));
  // Same rule as /anime/[id]: an episode left part-way wins over the first unwatched one.
  const nextUp = $derived(
    regular.find((e) => !e.watchedAt && e.files.length > 0 && (e.progressPos ?? 0) > 0) ??
      regular.find((e) => !e.watchedAt && e.files.length > 0) ??
      (app.appMode === "streaming"
        ? (regular.find((e) => !e.watchedAt && (e.progressPos ?? 0) > 0) ??
            regular.find((e) => !e.watchedAt && e.number > (d?.anilistProgress ?? 0)) ??
            regular[0] ??
            null)
        : null),
  );
  /** Fully-watched shows still get a primary action: rewatch the first owned episode. */
  const ctaEp = $derived(nextUp ?? regular.find((e) => e.files.length > 0) ?? null);
  const rewatch = $derived(!nextUp && !!ctaEp);
  const playingKey = $derived(
    app.nowPlaying && d && app.nowPlaying.anilistId === d.anilistId ? app.nowPlaying.epKey : null,
  );
  const banner = $derived(head ? (img(head.bannerPath, head.bannerUrl) ?? img(head.coverPath, head.coverUrl)) : null);
  const bannerIsCover = $derived(!!head && !head.bannerPath && !head.bannerUrl);
  const description = $derived(cleanDescription(d?.description ?? card?.description));
  const seasons = $derived(app.groupSeasons && d && d.franchise.length > 1 ? d.franchise : []);
  const subLangs = $derived(
    [...new Set(regular.flatMap((e) => (e.subtitles ?? []).map((s) => s.lang.toUpperCase())))].sort(),
  );
  const accent = $derived(head?.coverColor || "#8b5cf6");
  const titleId = "dm-title";

  const progressPct = (e: EpisodeRow) =>
    e.progressPos && e.progressDur ? Math.min(100, (e.progressPos / e.progressDur) * 100) : 0;
  const remaining = (e: EpisodeRow) =>
    e.progressPos && e.progressDur ? Math.max(0, e.progressDur - e.progressPos) : 0;
  const pad = (n: number) => String(n).padStart(2, "0");

  function epTitle(e: EpisodeRow) {
    return e.titleEn || e.titleRomaji || e.titleJa || (e.isSpecial ? `Special ${e.number}` : `Episode ${e.number}`);
  }

  /** 第1期 for TV cours, otherwise the Japanese format (劇場版, OVA…). */
  function seasonJa(s: FranchiseEntry, list: FranchiseEntry[]) {
    const tvish = (x: FranchiseEntry) => !x.format || x.format === "TV" || x.format === "TV_SHORT" || x.format === "ONA";
    if (!tvish(s)) return jaFormat(s.format);
    return jaCour(list.filter(tvish).findIndex((x) => x.anilistId === s.anilistId) + 1);
  }

  async function play(e: EpisodeRow) {
    if (!d) return;
    if (app.isLaunching(d.anilistId, e.epKey)) return;
    const f = e.files[0];
    if (f) {
      await app.play(d.anilistId, e.epKey, f.path);
      return;
    }
    if (app.appMode === "streaming") {
      const action = await app.playStreamEpisode(d, e.number);
      if (action === "open_web_picker") {
        webTarget = { epKey: e.epKey, epNumber: e.number };
      } else if (action === "open_torrent_picker") {
        torrentTarget = {
          epKey: e.epKey,
          epNumber: e.number,
          autoStart: app.torrentQuality !== "ask",
        };
      }
    }
  }

  async function addToPlanning() {
    if (!d || anilistSaving) return;
    anilistSaving = true;
    try {
      const updated = await api.anilistUpdateEntry({ anilistId: d.anilistId, status: "PLANNING" });
      d.anilistStatus = updated.status;
      app.toast(`Added ${displayTitle(d)} to your AniList Planning`, "info", 3500);
    } catch (e) {
      app.toast(String(e), "error", 6000);
    } finally {
      anilistSaving = false;
    }
  }

  async function toggleStreamLibrary() {
    if (!d || togglingStreamLib) return;
    togglingStreamLib = true;
    try {
      const next = await api.toggleStreamLibrary(d.anilistId);
      d.inStreamLibrary = next;
      await app.refresh();
      app.toast(next ? `Added ${displayTitle(d)} to your library` : `Removed from stream library`, "info", 3500);
    } catch (e) {
      app.toast(String(e), "error", 5000);
    } finally {
      togglingStreamLib = false;
    }
  }

  // --- Dialog behaviour ------------------------------------------------------
  const reduceMotion =
    typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  const dur = (ms: number) => (reduceMotion ? 0 : ms);

  // Lock the app's scroll container while open.
  $effect(() => {
    if (!open) return;
    const main = document.getElementById("main-scroll");
    if (!main) return;
    const prev = main.style.overflowY;
    main.style.overflowY = "hidden";
    return () => {
      main.style.overflowY = prev;
    };
  });

  // Move focus into the dialog when it opens.
  $effect(() => {
    if (open && dialogEl) {
      const el = dialogEl;
      queueMicrotask(() => {
        if (!el.contains(document.activeElement)) el.focus({ preventScroll: true });
      });
    }
  });

  function focusables(): HTMLElement[] {
    if (!dialogEl) return [];
    return [
      ...dialogEl.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"]), input, select, textarea',
      ),
    ].filter((el) => el.offsetParent !== null || el === document.activeElement);
  }

  function onKeydown(ev: KeyboardEvent) {
    if (!open) return;
    if (webTarget || torrentTarget) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        webTarget = null;
        torrentTarget = null;
      }
      return;
    }
    if (ev.key === "Escape") {
      ev.preventDefault();
      ev.stopPropagation();
      detailsModal.close();
      return;
    }
    if (ev.key !== "Tab") return;
    const list = focusables();
    if (!list.length) {
      ev.preventDefault();
      dialogEl?.focus();
      return;
    }
    const first = list[0];
    const last = list[list.length - 1];
    const active = document.activeElement as HTMLElement | null;
    if (ev.shiftKey && (active === first || active === dialogEl || !dialogEl?.contains(active))) {
      ev.preventDefault();
      last.focus();
    } else if (!ev.shiftKey && (active === last || !dialogEl?.contains(active))) {
      ev.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div class="dm-root" style:--dm-accent={accent}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dm-scrim" onclick={() => detailsModal.close()} transition:fade|local={{ duration: dur(200) }}>
      <div class="dm-ambient" aria-hidden="true"></div>
    </div>

    <div
      class="dm-modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-busy={loading}
      tabindex="-1"
      bind:this={dialogEl}
      transition:scale|local={{ duration: dur(240), start: 0.96, opacity: 0, easing: cubicOut }}
    >
      <button class="dm-close" onclick={() => detailsModal.close()} aria-label="Close" title="Close">
        <Icon name="x" size={18} stroke={2.4} />
      </button>

      <div class="dm-scroll">
      <!-- Banner ------------------------------------------------------------ -->
      <header class="dm-banner">
        {#if banner}
          <img src={banner} alt="" class:cover-fallback={bannerIsCover} decoding="async" />
        {:else}
          <div class="dm-banner-skel skeleton"></div>
        {/if}
        <div class="dm-banner-shade"></div>
        {#if head?.titleNative}
          <Ja text={head.titleNative} class="tategaki dm-tategaki" />
        {/if}

        <div class="dm-hero">
          {#if head}
            {#if head.titleNative}<Ja text={head.titleNative} block class="neon-text dm-native" />{/if}
            <h2 id={titleId} class="dm-title">{displayTitle(head)}</h2>
          {:else}
            <h2 id={titleId} class="dm-title sr-only">Loading</h2>
            <div class="skel-line w-60 h-lg"></div>
          {/if}

          <div class="dm-actions">
            {#if d && ctaEp}
              {@const launching = app.isLaunching(d.anilistId, ctaEp.epKey)}
              {@const playingNow = app.isPlaying(d.anilistId, ctaEp.epKey)}
              {@const isMovie = d.format === "MOVIE"}
              {@const verb = ctaEp.progressPos ? "Resume" : rewatch ? "Watch again" : "Play"}
              {#if ctaEp.files.length > 0 || app.appMode === "streaming"}
                <button
                  class="btn btn-cta dm-cta"
                  class:launching
                  disabled={launching}
                  onclick={() => play(ctaEp!)}
                  aria-label={launching ? "Launching player" : `${ctaEp.files.length > 0 ? verb : "Stream"} ${isMovie ? "movie" : `episode ${ctaEp.number}`}`}
                >
                  {#if launching}
                    <span class="spinner dm-spin" aria-hidden="true"></span> Launching…
                  {:else if playingNow}
                    <span class="eq-bars" aria-hidden="true"><span class="eq-bar"></span><span class="eq-bar"></span><span class="eq-bar"></span></span>
                    Now Playing
                  {:else}
                    <Icon name="play" size={16} fill />
                    {ctaEp.files.length > 0 ? verb : "Stream"}{isMovie ? "" : ` EP ${pad(ctaEp.number)}`}
                    {#if ctaEp.progressPos && remaining(ctaEp) > 0}
                      <Ja text={`· ${jaRemaining(remaining(ctaEp))}`} class="dm-cta-ja" />
                    {/if}
                  {/if}
                </button>
              {:else}
                <a class="btn btn-cta dm-cta" href={`/anime/${d.anilistId}`}>
                  <Icon name="play" size={16} fill /> Open EP {pad(ctaEp.number)}
                </a>
              {/if}
            {:else if d}
              <!-- Nothing playable locally: the primary action is the full page. -->
              <a
                class="btn btn-cta dm-cta"
                href={`/anime/${d.anilistId}`}
                title="No episodes of this show are in your library yet — manage files and sources on the full page"
              >
                <Icon name="play" size={16} fill /> Open full page
              </a>
              <span class="dm-cta-note">Not in library</span>
            {:else}
              <div class="skel-pill" aria-hidden="true"></div>
            {/if}

            {#if d && app.anilist?.connected && !d.anilistStatus}
              <button
                class="dm-round"
                onclick={addToPlanning}
                disabled={anilistSaving}
                aria-label="Add to My List (AniList Planning)"
                title="Add to My List (AniList Planning)"
              >
                {#if anilistSaving}<span class="spinner spinner-sm"></span>{:else}<Icon name="plus" size={18} />{/if}
              </button>
            {/if}
            {#if d && app.appMode === "streaming" && owned === 0}
              <button
                class="dm-round"
                onclick={toggleStreamLibrary}
                disabled={togglingStreamLib}
                aria-label={d.inStreamLibrary ? "Remove from library" : "Add to library"}
                title={d.inStreamLibrary ? "Remove from your Kura library" : "Bookmark this show in your Kura library"}
                aria-pressed={!!d.inStreamLibrary}
              >
                <Icon name={d.inStreamLibrary ? "check" : "plus"} size={18} />
              </button>
            {/if}
            {#if viewId !== null && !(d && !ctaEp)}
              <a
                class="dm-round"
                href={`/anime/${viewId}`}
                aria-label="Open full page"
                title="Open full page — torrents, subtitles, extras, AniList tracker"
              >
                <Icon name="external" size={17} />
              </a>
            {/if}
          </div>
        </div>
      </header>

      <div class="dm-body">
        {#if error && !d}
          <div class="dm-state" role="alert">
            <Icon name="alert" size={28} />
            <p><strong>Couldn't load this anime</strong> <Ja text={JA.states.error} class="ja-sub" /></p>
            <p class="muted small">{error}</p>
            <button class="btn btn-glass btn-pill" onclick={() => viewId !== null && load(viewId)}>
              <Icon name="refresh" size={15} /> Retry <Ja text={JA_LOCAL.retry} class="ja-sub" />
            </button>
          </div>
        {:else if !d}
          <div class="dm-loading" role="status">
            <span class="sr-only">Loading</span>
            <span class="tn-label loading-label" aria-hidden="true">
              <span class="spinner spinner-sm"></span>
              <Ja text={JA.states.loading} />
              <span>Loading</span>
            </span>
            <div class="skel-line w-40"></div>
            <div class="skel-line w-90"></div>
            <div class="skel-line w-80"></div>
            {#each [0, 1, 2] as i (i)}
              <div class="skel-ep"><div class="skel-thumb"></div><div class="skel-lines"><div class="skel-line w-50"></div><div class="skel-line w-80"></div></div></div>
            {/each}
          </div>
        {:else}
          <!-- Meta row -------------------------------------------------------- -->
          <div class="dm-meta">
            {#if d.averageScore}
              <span class="dm-score" title={`Average score ${d.averageScore}%`}>
                <Icon name="star" size={12} fill /> {(d.averageScore / 10).toFixed(1)}
              </span>
            {/if}
            {#if d.season || d.seasonYear}
              <Ja text={jaSeason(d.season, d.seasonYear)} label={seasonLabel(d.season, d.seasonYear)} essential class="dm-meta-item" />
            {/if}
            {#if d.format}
              <Ja text={jaFormat(d.format)} label={formatLabel(d.format)} essential class="dm-meta-item" />
            {/if}
            {#if d.episodes && d.format !== "MOVIE"}
              <Ja text={jaEpisodeCount(d.episodes)} label={`${d.episodes} episodes`} essential class="dm-meta-item" />
            {/if}
            {#if d.studios.length}<span class="tn-label">{d.studios[0]}</span>{/if}
            {#if d.status}
              {#if jaStatus(d.status)}
                <Ja
                  text={jaStatus(d.status)}
                  label={statusLabel(d.status)}
                  essential
                  class={`dm-status ${d.status === "RELEASING" ? "airing" : ""}`}
                />
              {:else}
                <span class="dm-status">{statusLabel(d.status)}</span>
              {/if}
            {/if}
          </div>

          <!-- Synopsis | facts ------------------------------------------------ -->
          <div class="dm-cols">
            <div class="dm-synopsis">
              {#if description}
                <p>{description}</p>
              {:else}
                <p class="muted">No synopsis available.</p>
              {/if}
            </div>
            <dl class="dm-facts">
              {#if d.genres.length}
                <dt class="tn-label">Genres</dt>
                <dd>
                  <ul class="dm-genres">
                    {#each d.genres as g (g)}
                      <li class="genre">{g}{#if jaGenre(g)}<Ja text={` · ${jaGenre(g)}`} class="genre-ja" />{/if}</li>
                    {/each}
                  </ul>
                </dd>
              {/if}
              {#if d.status}
                <dt class="tn-label">Status</dt>
                <dd>{statusLabel(d.status)}</dd>
              {/if}
              {#if d.format !== "MOVIE" || owned === 0}
                <dt class="tn-label">Local files</dt>
                <dd>{owned} / {totalEps || "?"} episodes</dd>
              {/if}
              {#if d.studios.length}
                <dt class="tn-label">Studio</dt>
                <dd>{d.studios.join(", ")}</dd>
              {/if}
              {#if subLangs.length}
                <dt class="tn-label">Subtitles</dt>
                <dd>{subLangs.join(" · ")} linked</dd>
              {/if}
              {#if d.trackPref}
                <dt class="tn-label">Tracks</dt>
                <dd>Custom audio / subtitle preference</dd>
              {/if}
            </dl>
          </div>

          <!-- Episodes -------------------------------------------------------- -->
          {#if regular.length > 0}
            <section class="dm-section" aria-labelledby="dm-eps-h">
              <div class="dm-section-head">
                <h3 id="dm-eps-h">
                  {d.format === "MOVIE" ? "Movie" : "Episodes"}
                  <Ja text={JA.rows.episodes} class="ja-sub" />
                </h3>
                {#if seasons.length > 1}
                  <div class="dm-seasons no-scrollbar" role="group" aria-label="Seasons">
                    {#each seasons as s (s.anilistId)}
                      {@const current = s.anilistId === d.anilistId}
                      <button
                        class="dm-season"
                        class:active={current}
                        aria-pressed={current}
                        title={displayTitle(s)}
                        onclick={() => (viewId = s.anilistId)}
                      >
                        <Ja text={seasonJa(s, seasons)} class="dm-season-ja" />
                        <span>{franchiseLabel(s, seasons[0])}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>

              <ol class="dm-eps">
                {#each regular as e (e.epKey)}
                  {@const has = e.files.length > 0}
                  {@const isNext = nextUp?.epKey === e.epKey}
                  {@const thumb = img(e.thumbPath, e.thumbUrl) ?? banner}
                  {@const launchingThis = app.isLaunching(d.anilistId, e.epKey)}
                  {@const playingThis = playingKey === e.epKey}
                  {@const pct = e.watchedAt ? 100 : progressPct(e)}
                  <li class="dm-ep" class:next={isNext} class:missing={!has} class:watched={!!e.watchedAt}>
                    {#if has}
                      <button
                        class="dm-ep-btn"
                        onclick={() => play(e)}
                        aria-disabled={launchingThis}
                        aria-label={`Play episode ${e.number}: ${epTitle(e)}${e.watchedAt ? " (watched)" : ""}`}
                      >
                        {@render epInner(e, thumb, pct, launchingThis, playingThis, isNext, true)}
                      </button>
                    {:else if app.appMode === "streaming"}
                      <button
                        class="dm-ep-btn"
                        onclick={() => play(e)}
                        aria-disabled={launchingThis}
                        title="Stream episode directly"
                        aria-label={`Stream episode ${e.number}: ${epTitle(e)}`}
                      >
                        {@render epInner(e, thumb, pct, launchingThis, playingThis, isNext, true)}
                      </button>
                    {:else}
                      <div
                        class="dm-ep-btn"
                        role="button"
                        tabindex="0"
                        aria-disabled="true"
                        title="Not in your library"
                        aria-label={`Episode ${e.number}: ${epTitle(e)} — not in your library`}
                      >
                        {@render epInner(e, thumb, pct, false, false, isNext, false)}
                      </div>
                    {/if}
                  </li>
                {/each}
              </ol>
              {#if specials.length || d.extras.length}
                <a class="dm-more-link" href={`/anime/${d.anilistId}`}>
                  Specials, extras &amp; files on the full page <Icon name="chevron-right" size={14} />
                </a>
              {/if}
            </section>
          {/if}

          <!-- Characters ------------------------------------------------------ -->
          {#if d.characters && d.characters.length > 0}
            <section class="dm-section" aria-labelledby="dm-chars-h">
              <div class="dm-section-head">
                <h3 id="dm-chars-h">Characters <Ja text={JA.rows.characters} class="ja-sub" /></h3>
              </div>
              <ul class="dm-chars no-scrollbar">
                {#each d.characters.slice(0, 14) as ch (ch.characterId + "-" + (ch.vaId ?? 0))}
                  <li class="dm-char">
                    <div class="dm-avatar">
                      {#if ch.imageUrl}<img src={ch.imageUrl} alt="" loading="lazy" decoding="async" />{/if}
                    </div>
                    <span class="dm-char-name" title={ch.nameFull}>{ch.nameFull}</span>
                    {#if ch.nameNative}<span class="dm-char-native" lang="ja">{ch.nameNative}</span>{/if}
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          <div class="dm-foot">
            <a class="btn btn-glass btn-pill" href={`/anime/${d.anilistId}`}>
              Open full page <Ja text={JA_LOCAL.fullPage} class="ja-sub" />
              <Icon name="chevron-right" size={14} />
            </a>
          </div>
        {/if}
      </div>
      </div>
    </div>
  </div>

  {#if d && webTarget !== null}
    <WebStreamDialog
      detail={d}
      epKey={webTarget.epKey}
      episodeNumber={webTarget.epNumber}
      onclose={() => (webTarget = null)}
      onswitchtotorrent={() => {
        if (!webTarget) return;
        torrentTarget = { epKey: webTarget.epKey, epNumber: webTarget.epNumber, autoStart: false };
      }}
    />
  {/if}

  {#if d && torrentTarget !== null}
    <TorrentPickerDialog
      anilistId={d.anilistId}
      title={displayTitle(d)}
      epKey={torrentTarget.epKey}
      episodeNumber={torrentTarget.epNumber}
      detail={d}
      autoStart={torrentTarget.autoStart}
      onclose={() => (torrentTarget = null)}
      onswitchtoweb={() => {
        if (!torrentTarget) return;
        webTarget = { epKey: torrentTarget.epKey, epNumber: torrentTarget.epNumber };
      }}
    />
  {/if}
{/if}

{#snippet epInner(
  e: EpisodeRow,
  thumb: string | null,
  pct: number,
  launchingThis: boolean,
  playingThis: boolean,
  isNext: boolean,
  playable: boolean,
)}
  <span class="dm-ep-num" aria-hidden="true">
    <span class="tn-label">{e.isSpecial ? "SP" : "EP"} {pad(e.number)}</span>
    <Ja text={jaEpisode(e.number)} class="dm-ep-ja" />
  </span>
  <span class="dm-ep-thumb">
    {#if thumb}<img src={thumb} alt="" loading="lazy" decoding="async" class:fallback={!e.thumbUrl && !e.thumbPath} />{/if}
    {#if launchingThis}
      <span class="dm-play active"><span class="spinner spinner-sm"></span></span>
    {:else if playingThis}
      <span class="dm-play active"><span class="eq-bars"><span class="eq-bar"></span><span class="eq-bar"></span><span class="eq-bar"></span></span></span>
    {:else if playable}
      <span class="dm-play"><span class="dm-play-dot"><Icon name="play" size={16} fill /></span></span>
    {/if}
    {#if e.watchedAt}<span class="dm-seen"><Icon name="check" size={11} stroke={3} /></span>{/if}
    {#if pct > 0}
      <span class="neon-progress dm-ep-prog"><span style:width={`${pct}%`}></span></span>
    {/if}
  </span>
  <span class="dm-ep-body">
    <span class="dm-ep-title-row">
      <span class="dm-ep-title">{epTitle(e)}</span>
      {#if e.runtime}<span class="dm-ep-runtime">{e.runtime}m</span>{/if}
    </span>
    {#if isNext}<span class="tn-label dm-upnext">{e.progressPos ? "Resume" : "Up next"}</span>{/if}
    {#if !playable}<span class="dm-ep-missing">Not in library</span>{/if}
    {#if e.overview}<span class="dm-ep-ov">{e.overview}</span>{/if}
  </span>
{/snippet}

<style>
  .dm-root {
    position: fixed;
    inset: 0;
    z-index: 900;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .dm-scrim {
    position: absolute;
    inset: 0;
    background: rgba(5, 6, 12, 0.82);
  }
  .dm-ambient {
    position: absolute;
    left: 50%;
    top: 45%;
    width: min(1150px, 120vw);
    height: 84vh;
    transform: translate(-50%, -50%);
    background: radial-gradient(
      closest-side,
      color-mix(in srgb, var(--dm-accent) 28%, transparent) 0%,
      color-mix(in srgb, var(--dm-accent) 9%, transparent) 52%,
      transparent 100%
    );
    pointer-events: none;
  }

  .dm-modal {
    position: relative;
    width: min(880px, 94vw);
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    container-type: inline-size;
    container-name: dm;
    border-radius: var(--r-xl);
    background: rgba(14, 16, 26, 0.97);
    border: 1px solid var(--glass-hairline);
    box-shadow:
      0 40px 120px -20px rgba(0, 0, 0, 0.85),
      0 0 80px -30px color-mix(in srgb, var(--dm-accent) 50%, transparent);
    outline: none;
  }
  /* Inner scroller: the outer shell clips to the radius, so corners stay round. */
  .dm-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    overscroll-behavior: contain;
    scrollbar-gutter: stable;
    scrollbar-width: thin;
    scrollbar-color: var(--surface-3) transparent;
  }
  .dm-scroll::-webkit-scrollbar {
    width: 8px;
  }
  .dm-scroll::-webkit-scrollbar-thumb {
    background: var(--surface-3);
    border-radius: 99px;
    border: 2px solid transparent;
    background-clip: padding-box;
  }
  .dm-scroll::-webkit-scrollbar-track {
    background: transparent;
    margin: 12px 0;
  }
  .dm-modal:focus-visible {
    box-shadow:
      0 40px 120px -20px rgba(0, 0, 0, 0.85),
      0 0 0 2px var(--neon-violet);
  }

  .dm-close {
    position: absolute;
    top: 14px;
    right: 22px;
    z-index: 5;
    width: 40px;
    height: 40px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    border: 1px solid var(--glass-hairline);
    background: var(--glass-tn);
    backdrop-filter: var(--blur-tn);
    -webkit-backdrop-filter: var(--blur-tn);
    color: var(--text);
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .dm-close:hover {
    background: var(--surface-3);
    box-shadow: var(--neon-glow);
  }

  /* Banner ----------------------------------------------------------------- */
  .dm-banner {
    position: relative;
    height: clamp(300px, 46vh, 420px);
    overflow: hidden;
  }
  /* The art itself fades to transparent, so it melts into whatever the modal
     background is (glass over the scrim) — no hard edge where the banner ends. */
  .dm-banner img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 25%;
    -webkit-mask-image: linear-gradient(180deg, #000 0%, #000 45%, rgba(0, 0, 0, 0.75) 62%, rgba(0, 0, 0, 0.35) 80%, transparent 100%);
    mask-image: linear-gradient(180deg, #000 0%, #000 45%, rgba(0, 0, 0, 0.75) 62%, rgba(0, 0, 0, 0.35) 80%, transparent 100%);
  }
  .dm-banner img.cover-fallback {
    filter: blur(10px) brightness(0.75);
    transform: scale(1.1);
  }
  .dm-banner-skel {
    position: absolute;
    inset: 0;
  }
  /* Text-contrast shade; ends fully transparent so it can't draw an edge either. */
  .dm-banner-shade {
    position: absolute;
    inset: 0;
    background:
      linear-gradient(
        0deg,
        rgba(14, 16, 26, 0.55) 0%,
        rgba(14, 16, 26, 0.45) 25%,
        rgba(14, 16, 26, 0.18) 55%,
        rgba(14, 16, 26, 0) 80%
      ),
      linear-gradient(90deg, rgba(8, 9, 16, 0.55) 0%, rgba(8, 9, 16, 0.2) 45%, transparent 70%);
    pointer-events: none;
  }
  .dm-banner :global(.dm-tategaki) {
    position: absolute;
    right: 24px;
    top: 76px;
    max-height: calc(100% - 200px);
    overflow: hidden;
    padding: 12px 7px;
    border-radius: var(--r-pill);
    background: rgba(8, 9, 16, 0.5);
    font-size: 14px;
    font-weight: 500;
    color: rgba(244, 242, 255, 0.82);
    text-shadow: 0 1px 8px rgba(0, 0, 0, 0.8);
    pointer-events: none;
  }
  @container dm (max-width: 760px) {
    .dm-banner :global(.dm-tategaki) {
      display: none;
    }
  }
  .dm-hero {
    position: absolute;
    left: 32px;
    right: 72px;
    bottom: 24px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }
  .dm-hero :global(.dm-native) {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: 0.04em;
    filter: drop-shadow(0 2px 10px rgba(0, 0, 0, 0.5));
  }
  .dm-title {
    margin: 0;
    font-size: clamp(26px, 3.4vw, 38px);
    font-weight: 800;
    letter-spacing: -0.025em;
    line-height: 1.08;
    color: #fff;
    text-shadow: 0 4px 24px rgba(0, 0, 0, 0.6);
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .dm-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 14px;
    flex-wrap: wrap;
  }
  .dm-cta {
    height: 46px;
    padding: 0 24px;
    font-size: 15px;
  }
  .dm-cta.launching {
    opacity: 0.85;
    cursor: wait;
  }
  .dm-cta :global(.dm-cta-ja) {
    font-weight: 500;
    opacity: 0.9;
  }
  .dm-spin {
    border-color: rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
  }
  .dm-round {
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    border: 1px solid rgba(255, 255, 255, 0.18);
    background: var(--glass-tn);
    backdrop-filter: var(--blur-tn);
    -webkit-backdrop-filter: var(--blur-tn);
    color: var(--text);
    cursor: pointer;
    transition:
      border-color var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .dm-round:hover:not(:disabled) {
    border-color: rgba(255, 255, 255, 0.4);
    background: var(--surface-3);
    box-shadow: var(--neon-glow);
  }
  .dm-round:disabled {
    opacity: 0.55;
    cursor: wait;
  }

  /* Body ------------------------------------------------------------------- */
  .dm-body {
    padding: 8px 32px 32px;
  }
  .dm-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
    color: var(--text-2);
    font-size: 13.5px;
  }
  .dm-score {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--amber) 14%, transparent);
    color: var(--amber);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .dm-meta :global(.dm-meta-item) {
    color: var(--text);
    font-weight: 500;
  }
  .dm-meta :global(.dm-status) {
    display: inline-flex;
    align-items: center;
    height: 24px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    background: var(--surface-3);
    color: var(--text);
    font-size: 12.5px;
    font-weight: 600;
  }
  .dm-meta :global(.dm-status.airing) {
    background: color-mix(in srgb, var(--ok) 16%, transparent);
    color: var(--ok);
  }

  .dm-cols {
    display: grid;
    grid-template-columns: minmax(0, 1.7fr) minmax(0, 1fr);
    gap: 32px;
    margin-top: 20px;
  }
  @media (max-width: 720px) {
    .dm-cols {
      grid-template-columns: 1fr;
      gap: 18px;
    }
  }
  .dm-synopsis p {
    margin: 0;
    color: var(--text);
    opacity: 0.88;
    line-height: 1.65;
    font-size: 14.5px;
    white-space: pre-line;
    display: -webkit-box;
    -webkit-line-clamp: 7;
    line-clamp: 7;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .dm-facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 10px 14px;
    margin: 0;
    font-size: 13.5px;
    align-content: start;
  }
  .dm-facts dt {
    padding-top: 2px;
  }
  .dm-facts dd {
    margin: 0;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .dm-facts :global(.genre-ja) {
    color: var(--text-2);
  }
  .dm-genres {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px 8px;
  }
  .dm-genres .genre {
    white-space: nowrap;
    padding: 2px 10px;
    border-radius: var(--r-pill);
    background: var(--surface-2);
    font-size: 12.5px;
  }
  .dm-cta-note {
    font-size: 12.5px;
    color: var(--text-2);
  }

  .dm-section {
    margin-top: 34px;
  }
  .dm-section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
    margin-bottom: 14px;
  }
  .dm-section-head h3 {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .dm-seasons {
    display: flex;
    gap: 4px;
    padding: 4px;
    border-radius: var(--r-pill);
    background: var(--glass-tn);
    border: 1px solid var(--glass-hairline);
    overflow-x: auto;
    max-width: 100%;
  }
  .dm-season {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 34px;
    padding: 0 14px;
    border: none;
    border-radius: var(--r-pill);
    background: transparent;
    color: var(--text-2);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }
  .dm-season:hover {
    color: var(--text);
    background: var(--surface-3);
  }
  .dm-season.active {
    background: color-mix(in srgb, var(--neon-violet) 26%, var(--surface-2));
    color: var(--text);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--neon-violet) 55%, transparent);
  }
  .dm-season :global(.dm-season-ja) {
    font-size: 12px;
  }
  .dm-season :global(.dm-season-ja)::after {
    content: " ·";
  }

  /* Episodes --------------------------------------------------------------- */
  .dm-eps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dm-ep {
    content-visibility: auto;
    contain-intrinsic-size: auto 120px;
  }
  .dm-ep-btn {
    width: 100%;
    display: grid;
    grid-template-columns: 64px 176px minmax(0, 1fr);
    align-items: center;
    gap: 18px;
    padding: 14px 14px;
    border: none;
    border-radius: var(--r-md);
    background: transparent;
    color: inherit;
    text-align: left;
    font: inherit;
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .dm-ep-btn:hover,
  .dm-ep-btn:focus-visible {
    background: var(--surface-2);
  }
  .dm-ep.next .dm-ep-btn {
    background: color-mix(in srgb, var(--neon-violet) 13%, var(--glass-tn));
    box-shadow:
      inset 0 0 0 1px color-mix(in srgb, var(--neon-violet) 30%, transparent),
      0 0 28px -8px rgba(139, 92, 246, 0.45);
  }
  .dm-ep.missing .dm-ep-btn[aria-disabled="true"] {
    cursor: not-allowed;
  }
  .dm-ep-num {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .dm-ep-num .tn-label {
    font-size: 12px;
    color: var(--text);
  }
  .dm-ep-num :global(.dm-ep-ja) {
    font-size: 11px;
    color: var(--text-2);
  }
  .dm-ep-thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-sm);
    overflow: hidden;
    background: var(--surface-2);
  }
  .dm-ep-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition: transform 400ms var(--ease);
  }
  .dm-ep-thumb img.fallback {
    opacity: 0.5;
  }
  .dm-ep.missing .dm-ep-thumb img {
    filter: grayscale(1) brightness(0.5);
  }
  .dm-ep-btn:hover .dm-ep-thumb img {
    transform: scale(1.05);
  }
  .dm-play {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(8, 9, 16, 0.45);
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }
  .dm-play.active,
  .dm-ep-btn:hover .dm-play,
  .dm-ep-btn:focus-visible .dm-play {
    opacity: 1;
  }
  .dm-play-dot {
    width: 38px;
    height: 38px;
    display: grid;
    place-items: center;
    padding-left: 2px;
    border-radius: 50%;
    background: var(--primary);
    color: var(--on-primary);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.5);
  }
  .dm-play .eq-bars {
    color: #fff;
    height: 14px;
  }
  .dm-seen {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.6);
    color: var(--ok);
  }
  .dm-ep-prog {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    border-radius: 0;
  }
  .dm-ep-body {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .dm-ep-title-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .dm-ep-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dm-ep.watched .dm-ep-title,
  .dm-ep.missing .dm-ep-title {
    color: var(--text-2);
  }
  .dm-ep-runtime {
    flex: none;
    font-size: 12.5px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .dm-upnext {
    color: var(--neon-pink);
  }
  .dm-ep-missing {
    font-size: 12px;
    color: var(--text-2);
  }
  .dm-ep-ov {
    font-size: 13px;
    line-height: 1.5;
    color: var(--text-2);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  @media (max-width: 640px) {
    .dm-ep-btn {
      grid-template-columns: 120px minmax(0, 1fr);
    }
    .dm-ep-num {
      display: none;
    }
    .dm-body {
      padding: 8px 18px 24px;
    }
    .dm-hero {
      left: 18px;
    }
  }
  .dm-more-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 12px;
    padding: 8px 4px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-2);
  }
  .dm-more-link:hover {
    color: var(--text);
  }

  /* Characters ------------------------------------------------------------- */
  .dm-chars {
    list-style: none;
    margin: 0;
    padding: 0 0 4px;
    display: flex;
    gap: 18px;
    overflow-x: auto;
  }
  .dm-char {
    flex: none;
    width: 84px;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 3px;
  }
  .dm-avatar {
    width: 64px;
    height: 64px;
    border-radius: 50%;
    overflow: hidden;
    background: var(--surface-2);
    margin-bottom: 4px;
  }
  .dm-avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .dm-char-name {
    max-width: 100%;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dm-char-native {
    max-width: 100%;
    font-size: 11px;
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dm-foot {
    display: flex;
    justify-content: center;
    margin-top: 32px;
  }

  /* Loading / error -------------------------------------------------------- */
  .dm-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 36px 0 12px;
    text-align: center;
    color: var(--text-2);
  }
  .dm-state strong {
    color: var(--text);
  }
  .small {
    font-size: 12.5px;
    max-width: 520px;
    overflow-wrap: anywhere;
  }
  .dm-loading {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 6px;
  }
  .loading-label {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .skel-line,
  .skel-pill,
  .skel-thumb {
    background: linear-gradient(110deg, var(--surface) 25%, var(--surface-2) 37%, var(--surface) 63%);
    background-size: 200% 100%;
    animation: dm-shimmer 1.4s ease infinite;
  }
  .skel-line {
    height: 14px;
    border-radius: 7px;
  }
  .skel-line.h-lg {
    height: 34px;
  }
  .skel-pill {
    width: 170px;
    height: 46px;
    border-radius: var(--r-pill);
  }
  .w-40 { width: 40%; }
  .w-50 { width: 50%; }
  .w-60 { width: 60%; }
  .w-80 { width: 80%; }
  .w-90 { width: 90%; }
  .skel-ep {
    display: flex;
    gap: 18px;
    align-items: center;
    margin-top: 8px;
  }
  .skel-thumb {
    width: 176px;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-sm);
  }
  .skel-lines {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  @keyframes dm-shimmer {
    0% { background-position: 100% 50%; }
    100% { background-position: 0% 50%; }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel-line,
    .skel-pill,
    .skel-thumb {
      animation: none;
    }
    .dm-ep-btn:hover .dm-ep-thumb img {
      transform: none;
    }
  }
</style>
