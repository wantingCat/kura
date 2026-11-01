<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, img, type EpisodeRow, type ExtraFile, type LinkedSubtitle, type MediaDetail } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import {
    cleanDescription,
    clock,
    displayTitle,
    formatBytes,
    formatDate,
    formatLabel,
    franchiseLabel,
    relationLabel,
    seasonLabel,
    statusLabel,
    untilTime,
  } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import FixMatchDialog from "$lib/components/FixMatchDialog.svelte";
  import TrackPrefsDialog from "$lib/components/TrackPrefsDialog.svelte";
  import JimakuDialog from "$lib/components/JimakuDialog.svelte";
  import SubtitleMatcherDialog from "$lib/components/SubtitleMatcherDialog.svelte";
  import PersonModal from "$lib/components/PersonModal.svelte";
  import TorrentPickerDialog from "$lib/components/TorrentPickerDialog.svelte";

  const id = $derived(Number(page.params.id));
  let d = $state<MediaDetail | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let showFix = $state(false);
  let showTracks = $state(false);
  let showMatcher = $state(false);
  let jimakuTarget = $state<{ epKey: string | null; epNumber: number | null } | null>(null);
  let torrentTarget = $state<{ epKey: string; epNumber: number; autoStart: boolean } | null>(null);
  let togglingStreamLib = $state(false);
  let selectedPerson = $state<{ id: number; name: string } | null>(null);
  let retimingSubId = $state<number | null>(null);
  let showAllChars = $state(false);
  let synopsisOpen = $state(false);
  let refreshing = $state(false);
  let expanded = $state<Set<string>>(new Set());

  async function load(target: number) {
    if (!d || d.anilistId !== target) {
      loading = true;
      error = null;
    }
    try {
      const res = await api.getMediaDetail(target);
      if (target === id) {
        d = res;
        error = null;
      }
    } catch (e) {
      if (target === id) {
        error = String(e);
      }
    } finally {
      if (target === id) {
        loading = false;
      }
    }
  }

  // Reload on navigation and whenever the library changes (scan progress, watch toggles…).
  $effect(() => {
    void app.version;
    load(id);
  });
  let logoFailed = $state(false);

  $effect(() => {
    void id;
    synopsisOpen = false;
    showAllChars = false;
    expanded = new Set();
    logoFailed = false;
  });

  const regular = $derived(d?.episodeList.filter((e) => !e.isSpecial) ?? []);
  const specials = $derived(d?.episodeList.filter((e) => e.isSpecial) ?? []);
  const owned = $derived(regular.filter((e) => e.files.length > 0).length);
  const watched = $derived(regular.filter((e) => e.watchedAt).length);
  const totalEps = $derived(Math.max(d?.episodes ?? 0, regular.length));
  /** An episode left part-way through wins over the first unwatched one. */
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
  const playingKey = $derived(
    app.nowPlaying && d && app.nowPlaying.anilistId === d.anilistId ? app.nowPlaying.epKey : null,
  );
  const banner = $derived(d ? (img(d.bannerPath, d.bannerUrl) ?? img(d.coverPath, d.coverUrl)) : null);
  const cover = $derived(d ? img(d.coverPath, d.coverUrl) : null);
  const logo = $derived(d && !logoFailed ? img(d.logoPath, d.logoUrl) : null);
  const description = $derived(cleanDescription(d?.description));
  const lowConfidence = $derived(d?.groups.some((g) => !g.manual && (g.confidence ?? 1) < 0.8) ?? false);
  const effectiveWatched = $derived(Math.max(watched, d?.anilistProgress ?? 0));
  const pct = $derived(totalEps ? Math.round((effectiveWatched / totalEps) * 100) : 0);
  /** Other owned seasons / movies of this franchise, shown as tabs when grouping is on. */
  const seasons = $derived(app.groupSeasons && d && d.franchise.length > 1 ? d.franchise : []);
  const inTabs = $derived(new Set(seasons.map((s) => s.anilistId)));
  // Owned entries already reachable via the tabs don't need to be repeated under "Related".
  const relations = $derived(d?.relations.filter((r) => !inTabs.has(r.relatedId)) ?? []);

  const ANILIST_STATUSES = [
    { key: "CURRENT", label: "Watching" },
    { key: "PLANNING", label: "Planning" },
    { key: "COMPLETED", label: "Completed" },
    { key: "PAUSED", label: "Paused" },
    { key: "DROPPED", label: "Dropped" },
    { key: "REPEATING", label: "Rewatching" },
  ] as const;

  let anilistSaving = $state(false);

  async function updateAnilist(opts: { status?: string | null; progress?: number | null; score?: number | null }) {
    if (!d || anilistSaving) return;
    anilistSaving = true;
    try {
      const updated = await api.anilistUpdateEntry({
        anilistId: d.anilistId,
        status: opts.status,
        progress: opts.progress,
        score: opts.score,
      });
      d.anilistStatus = updated.status;
      d.anilistProgress = updated.progress;
      d.anilistScore = updated.score;
      await load(d.anilistId);
    } catch (e) {
      app.toast(String(e), "error", 6000);
    } finally {
      anilistSaving = false;
    }
  }

  async function toggleWatched(e: EpisodeRow) {
    if (!d) return;
    await api.setWatched(d.anilistId, [e.epKey], !e.watchedAt);
  }

  async function watchedUpTo(e: EpisodeRow) {
    if (!d) return;
    const keys = regular.filter((x) => x.number <= e.number && !x.watchedAt).map((x) => x.epKey);
    if (keys.length) await api.setWatched(d.anilistId, keys, true);
  }

  async function play(e: EpisodeRow) {
    if (!d) return;
    const f = e.files[0];
    if (f) {
      await app.play(d.anilistId, e.epKey, f.path);
    } else if (app.appMode === "streaming") {
      torrentTarget = {
        epKey: e.epKey,
        epNumber: e.number,
        autoStart: app.torrentQuality !== "ask",
      };
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

  const progressPct = (e: EpisodeRow) =>
    e.progressPos && e.progressDur ? Math.min(100, (e.progressPos / e.progressDur) * 100) : 0;

  async function refresh() {
    if (!d) return;
    refreshing = true;
    try {
      await api.refreshMedia(d.anilistId);
      await load(d.anilistId);
    } catch (e) {
      error = `Refresh failed (offline?): ${e}`;
    } finally {
      refreshing = false;
    }
  }

  function toggleExpand(key: string) {
    const s = new Set(expanded);
    s.has(key) ? s.delete(key) : s.add(key);
    expanded = s;
  }

  function epTitle(e: EpisodeRow) {
    if (e.titleEn) return e.titleEn;
    if (e.titleRomaji) return e.titleRomaji;
    if (e.titleJa) return e.titleJa;
    return e.isSpecial ? `Special ${e.number}` : `Episode ${e.number}`;
  }

  async function playExtra(ex: ExtraFile) {
    if (!d) return;
    await app.play(d.anilistId, `extra:${ex.id}`, ex.path);
  }

  async function retimeSub(sub: LinkedSubtitle) {
    if (!d || retimingSubId !== null) return;
    retimingSubId = sub.id;
    try {
      const report = await api.retimeEpisodeSubtitle({
        anilistId: d.anilistId,
        epKey: sub.epKey,
        subtitlePath: sub.path,
      });
      const sec = (report.offsetMs >= 0 ? "+" : "") + (report.offsetMs / 1000).toFixed(2) + "s";
      app.toast(`Retimed subtitle (${sec} across ${report.matchedCues} cues)`, "info", 5000);
      await load(d.anilistId);
    } catch (e) {
      app.toast(`Retime failed: ${e}`, "error", 6000);
    } finally {
      retimingSubId = null;
    }
  }

  async function unlinkSub(subId: number) {
    if (!d) return;
    try {
      await api.removeLinkedSubtitle(subId);
      await load(d.anilistId);
    } catch (e) {
      app.toast(String(e), "error", 5000);
    }
  }

  function formatOffsetMs(ms: number | null) {
    if (ms === null || ms === undefined) return "";
    const s = (ms / 1000).toFixed(1);
    return ms >= 0 ? `+${s}s` : `${s}s`;
  }

  function extraBadge(kind: ExtraFile["kind"]) {
    switch (kind) {
      case "opening": return "OP";
      case "ending": return "ED";
      case "trailer": return "Trailer";
      case "pv": return "PV";
      case "bonus": return "Bonus";
      default: return "Extra";
    }
  }
</script>

<svelte:head>
  <title>{d ? `${displayTitle(d)} · Kura` : "Kura"}</title>
</svelte:head>

{#if error && (!d || d.anilistId !== id)}
  <div class="page center">
    <Icon name="alert" size={32} />
    <h2>Couldn't load this entry</h2>
    <p class="muted">{error}</p>
    <a class="btn" href="/">Back to library</a>
  </div>
{:else if !d || d.anilistId !== id || loading}
  <div class="detail loading-detail">
    <section class="banner skeleton">
      <button class="btn btn-ghost back" onclick={() => history.back()} id="detail-back-loading">
        <Icon name="arrow-left" size={16} /> Back
      </button>
      <div class="loading-banner-pill">
        <span class="spinner"></span>
        <span>Loading anime details…</span>
      </div>
    </section>
    <div class="page body">
      <header class="head">
        <div class="poster skeleton-box"></div>
        <div class="head-info skeleton-info">
          <div class="skeleton-line w-30"></div>
          <div class="skeleton-line w-70 h-lg"></div>
          <div class="skeleton-line w-45"></div>
          <div class="skeleton-line w-90 h-md"></div>
        </div>
      </header>
    </div>
  </div>
{:else}
  {#key d.anilistId}
    <div class="detail">
      <section class="banner">
        {#if banner}<img src={banner} alt="" class:blur={!d.bannerUrl && !d.bannerPath} />{/if}
        <div class="banner-shade"></div>
        <button class="btn btn-ghost back" onclick={() => history.back()} id="detail-back">
          <Icon name="arrow-left" size={16} /> Back
        </button>
        {#if logo}
          <div class="banner-clearart">
            <img src={logo} alt="" onerror={() => (logoFailed = true)} />
          </div>
        {/if}
      </section>

      <div class="page body">
        <header class="head">
          <div class="poster">
            {#if cover}<img src={cover} alt={`${displayTitle(d)} poster`} />{/if}
          </div>
          <div class="head-info">
            <div class="badges">
              {#if d.format}<span class="badge badge-accent">{formatLabel(d.format)}</span>{/if}
              {#if d.status}
                <span class="badge" class:badge-ok={d.status === "RELEASING"}>{statusLabel(d.status)}</span>
              {/if}
              {#if d.seasonYear}<span class="badge">{seasonLabel(d.season, d.seasonYear)}</span>{/if}
              {#if d.averageScore}
                <span class="badge"><Icon name="star" size={11} fill /> {d.averageScore}%</span>
              {/if}
              {#if d.anilistStatus}
                <span class="badge badge-ok" title="Synced with your AniList account">
                  AniList · {d.anilistStatus === "CURRENT" ? "Watching" : d.anilistStatus.charAt(0) + d.anilistStatus.slice(1).toLowerCase()}{#if d.anilistProgress} ({d.anilistProgress}){/if}
                </span>
              {/if}
            </div>
            <h1>{displayTitle(d)}</h1>
            {#if d.titleRomaji && d.titleRomaji !== displayTitle(d)}<p class="alt">{d.titleRomaji}</p>{/if}
            {#if d.titleNative}<p class="alt jp">{d.titleNative}</p>{/if}

            <div class="stats">
              {#if d.format !== "MOVIE"}
                <div class="stat">
                  <strong>{effectiveWatched}<span>/{totalEps || "?"}</span></strong>
                  <small>watched</small>
                </div>
                <div class="stat">
                  <strong class:warn={totalEps > 0 && owned < totalEps && d.status === "FINISHED"}>
                    {owned}<span>/{totalEps || "?"}</span>
                  </strong>
                  <small>in library</small>
                </div>
              {/if}
              {#if d.studios.length}
                <div class="stat"><strong class="sm">{d.studios[0]}</strong><small>studio</small></div>
              {/if}
              {#if d.duration}
                <div class="stat"><strong class="sm">{d.duration} min</strong><small>per episode</small></div>
              {/if}
              {#if d.nextAiringEpisode && d.nextAiringAt}
                <div class="stat">
                  <strong class="sm airing">Ep {d.nextAiringEpisode} in {untilTime(d.nextAiringAt)}</strong>
                  <small>next episode</small>
                </div>
              {/if}
            </div>
            {#if totalEps > 0 && d.format !== "MOVIE"}
              <div class="progress" title={`${pct}% watched`}><div style:width={`${pct}%`}></div></div>
            {/if}

            <div class="actions">
              {#if nextUp}
                {@const isLaunchingNext = app.isLaunching(d.anilistId, nextUp.epKey)}
                {@const isPlayingNext = app.isPlaying(d.anilistId, nextUp.epKey)}
                <button
                  class="btn btn-primary"
                  class:launching={isLaunchingNext}
                  disabled={isLaunchingNext}
                  onclick={() => play(nextUp!)}
                  id="detail-play-next"
                >
                  {#if isLaunchingNext}
                    <span class="spinner"></span> Launching player…
                  {:else if isPlayingNext}
                    <span class="eq-bars"><span class="eq-bar"></span><span class="eq-bar"></span><span class="eq-bar"></span></span>
                    Now Playing · {d.format === "MOVIE" ? "Movie" : `Ep ${nextUp.number}`}
                  {:else}
                    <Icon name="play" size={15} fill />
                    {#if nextUp.progressPos}
                      Resume · {d.format === "MOVIE" ? "" : `Ep ${nextUp.number} · `}{clock(nextUp.progressPos)}
                    {:else}
                      {nextUp.files.length === 0 ? "Stream" : d.format === "MOVIE" ? "Play" : `${effectiveWatched > 0 ? "Continue" : "Start"}`} · {d.format === "MOVIE" ? "Movie" : `Ep ${nextUp.number}`}
                    {/if}
                  {/if}
                </button>
              {/if}
              {#if app.appMode === "streaming" && owned === 0}
                <button
                  class="btn"
                  onclick={toggleStreamLibrary}
                  disabled={togglingStreamLib}
                  id="detail-toggle-stream-lib"
                  title={d.inStreamLibrary ? "Remove from your Kura library" : "Bookmark this show in your Kura library"}
                >
                  <Icon name={d.inStreamLibrary ? "check" : "plus"} size={15} />
                  {d.inStreamLibrary ? "In Library" : "Add to Library"}
                </button>
              {/if}
              {#if app.appMode === "streaming" && (nextUp || regular[0])}
                <button
                  class="btn"
                  onclick={() => {
                    const target = nextUp ?? regular[0];
                    if (target) torrentTarget = { epKey: target.epKey, epNumber: target.number, autoStart: false };
                  }}
                  id="detail-pick-torrent"
                  title="Choose a torrent release to stream or download"
                >
                  <Icon name="download" size={15} /> Torrents
                </button>
              {/if}
              {#if owned > 0 || d.groups.length > 0 || app.appMode === "streaming"}
                <button class="btn" onclick={() => (showTracks = true)} id="detail-track-prefs" title="Audio & Subtitles">
                  <Icon name="subtitles" size={15} /> Audio & Subs
                  {#if d.trackPref}
                    <span class="custom-track-badge">Custom</span>
                  {/if}
                </button>
                <button class="btn" onclick={() => (showMatcher = true)} id="detail-match-subs" title="Match external subtitle files & auto-retime">
                  <Icon name="languages" size={15} /> Match Subtitles
                </button>
                <button
                  class="btn"
                  onclick={() => (jimakuTarget = { epKey: nextUp?.epKey ?? regular[0]?.epKey ?? null, epNumber: nextUp?.number ?? regular[0]?.number ?? null })}
                  id="detail-jimaku-subs"
                  title="Download Japanese subtitles from Jimaku.cc"
                >
                  <Icon name="download" size={15} /> Jimaku Subs
                </button>
                <button class="btn" onclick={() => (showFix = true)} id="detail-fix-match">
                  <Icon name="link" size={15} /> Fix match
                </button>
              {/if}
              <button class="btn btn-ghost btn-icon" onclick={refresh} disabled={refreshing} title="Refresh metadata" id="detail-refresh">
                {#if refreshing}<div class="spinner"></div>{:else}<Icon name="refresh" size={16} />{/if}
              </button>
              <button
                class="btn btn-ghost btn-icon"
                onclick={() => openUrl(`https://anilist.co/anime/${d!.anilistId}`)}
                title="Open on AniList"
                id="detail-anilist"
              >
                <Icon name="external" size={16} />
              </button>
            </div>

            <div class="anilist-tracker card" id="detail-anilist-tracker">
              <div class="tracker-head">
                <span class="tracker-brand">
                  <Icon name="user" size={13} />
                  AniList Tracker
                  {#if anilistSaving}<span class="spinner spinner-xs"></span>{/if}
                </span>
                {#if !app.anilist?.connected}
                  <button
                    class="tracker-signin-link"
                    onclick={() => {
                      try { localStorage.setItem("kura.settings.category", "integrations"); } catch {}
                      goto("/settings");
                    }}
                  >
                    Connect AniList in Settings to sync →
                  </button>
                {/if}
              </div>

              <div class="tracker-controls">
                <div class="status-pills" role="group" aria-label="AniList status">
                  {#each ANILIST_STATUSES as st (st.key)}
                    <button
                      class="status-pill"
                      class:active={d.anilistStatus === st.key}
                      disabled={anilistSaving}
                      onclick={() => updateAnilist({ status: st.key })}
                      id={`al-status-${st.key.toLowerCase()}`}
                    >
                      {st.label}
                    </button>
                  {/each}
                </div>

                <div class="tracker-right">
                  <div class="prog-stepper" aria-label="Episode progress">
                    <button
                      class="step-btn"
                      disabled={anilistSaving || effectiveWatched <= 0}
                      onclick={() => updateAnilist({ progress: Math.max(0, effectiveWatched - 1) })}
                      title="Decrease watched episodes by 1"
                      id="al-prog-dec"
                    >
                      −
                    </button>
                    <span class="prog-readout">
                      <strong>{effectiveWatched}</strong><span>/{totalEps || "?"}</span> eps
                    </span>
                    <button
                      class="step-btn plus"
                      disabled={anilistSaving || (totalEps > 0 && effectiveWatched >= totalEps)}
                      onclick={() => updateAnilist({ progress: effectiveWatched + 1 })}
                      title={`Mark episode ${effectiveWatched + 1} as watched`}
                      id="al-prog-inc"
                    >
                      +1 Ep
                    </button>
                  </div>

                  <label class="score-picker" title="Your AniList score">
                    <Icon name="star" size={13} fill />
                    <select
                      value={d.anilistScore ? String(Math.round(d.anilistScore)) : ""}
                      disabled={anilistSaving}
                      onchange={(ev) => {
                        const val = (ev.currentTarget as HTMLSelectElement).value;
                        if (val) updateAnilist({ score: Number(val) });
                      }}
                      id="al-score-select"
                    >
                      <option value="">Score —</option>
                      <option value="10">10 · Masterpiece</option>
                      <option value="9">9 · Great</option>
                      <option value="8">8 · Very Good</option>
                      <option value="7">7 · Good</option>
                      <option value="6">6 · Fine</option>
                      <option value="5">5 · Average</option>
                      <option value="4">4 · Bad</option>
                      <option value="3">3 · Very Bad</option>
                      <option value="2">2 · Horrible</option>
                      <option value="1">1 · Appalling</option>
                    </select>
                  </label>
                </div>
              </div>
            </div>
          </div>
        </header>

        {#if seasons.length > 1}
          <nav class="seasons" aria-label="Seasons">
            {#each seasons as s, i (s.anilistId)}
              {@const t = s.episodes ?? (s.format === "MOVIE" ? 1 : null)}
              {@const done = t !== null && t > 0 && s.watchedCount >= t}
              {@const current = s.anilistId === d.anilistId}
              <a
                class="season"
                class:active={current}
                class:done
                href={`/anime/${s.anilistId}`}
                data-sveltekit-replacestate
                data-sveltekit-noscroll
                aria-current={current ? "page" : undefined}
                title={displayTitle(s)}
                style:--i={i}
                id={`season-tab-${s.anilistId}`}
              >
                <span class="s-label">{franchiseLabel(s, seasons[0])}</span>
                <span class="s-meta">{[formatLabel(s.format), s.seasonYear].filter(Boolean).join(" · ")}</span>
                <span class="s-progress">
                  {#if done}
                    <Icon name="check" size={11} stroke={3} /> Watched
                  {:else}
                    {s.watchedCount}/{t ?? "?"} watched
                  {/if}
                </span>
                <span class="s-bar"><span style:width={`${t ? Math.min(100, (s.watchedCount / t) * 100) : 0}%`}></span></span>
              </a>
            {/each}
          </nav>
        {/if}

        {#if lowConfidence}
          <div class="notice">
            <Icon name="alert" size={16} />
            <span>Kura isn't fully sure this is the right anime. If it's wrong, use <strong>Fix match</strong>.</span>
          </div>
        {/if}
        {#if error}
          <div class="notice danger"><Icon name="alert" size={16} /><span>{error}</span></div>
        {/if}

        <div class="columns">
          <div class="main-col">
            {#if description}
              <section class="block">
                <h2 class="section-title">Synopsis</h2>
                <p class="synopsis" class:open={synopsisOpen}>{description}</p>
                {#if description.length > 420}
                  <button class="more" onclick={() => (synopsisOpen = !synopsisOpen)}>
                    {synopsisOpen ? "Show less" : "Read more"}
                  </button>
                {/if}
              </section>
            {/if}

            {#snippet episodeList(list: EpisodeRow[])}
              <ol class="episodes">
                {#each list as e, i (e.epKey)}
                  {@const has = e.files.length > 0}
                  {@const isNext = nextUp?.epKey === e.epKey}
                  {@const thumb = img(e.thumbPath, e.thumbUrl) ?? (d ? img(d.bannerPath, d.bannerUrl) : null)}
                  {@const isLaunchingThis = app.isLaunching(d?.anilistId, e.epKey)}
                  {@const isPlayingThis = playingKey === e.epKey}
                  <li
                    class="ep"
                    class:missing={!has && app.appMode !== "streaming"}
                    class:watched={!!e.watchedAt}
                    class:next={isNext}
                    style:--i={Math.min(i, 30)}
                    id={`ep-${e.epKey}`}
                  >
                    <button
                      class="ep-thumb"
                      onclick={() => play(e)}
                      disabled={(!has && app.appMode !== "streaming") || isLaunchingThis}
                      aria-label={`Play ${epTitle(e)}`}
                    >
                      {#if thumb}<img src={thumb} alt="" loading="lazy" class:fallback={!e.thumbUrl} />{/if}
                      <span class="ep-num">{e.isSpecial ? "SP" : ""}{e.number}</span>
                      {#if isLaunchingThis}
                        <span class="play-ov active"><span class="spinner spinner-sm"></span></span>
                      {:else if isPlayingThis}
                        <span class="play-ov active"><span class="eq-bars"><span class="eq-bar"></span><span class="eq-bar"></span><span class="eq-bar"></span></span></span>
                      {:else if has || app.appMode === "streaming"}
                        <span class="play-ov"><Icon name="play" size={20} fill /></span>
                      {/if}
                      {#if e.watchedAt}<span class="seen"><Icon name="check" size={12} stroke={3} /></span>{/if}
                      {#if !e.watchedAt && progressPct(e) > 0}
                        <span class="ep-prog"><span style:width={`${progressPct(e)}%`}></span></span>
                      {/if}
                    </button>

                    <div class="ep-body">
                      <div class="ep-title-row">
                        <h3 title={epTitle(e)}>{epTitle(e)}</h3>
                        {#if isLaunchingThis}
                          <span class="badge badge-accent launching-badge">
                            <span class="spinner spinner-xs"></span> Launching…
                          </span>
                        {:else if isPlayingThis}
                          <span class="badge badge-ok playing-badge">
                            <span class="eq-bars"><span class="eq-bar"></span><span class="eq-bar"></span><span class="eq-bar"></span></span>
                            Playing
                          </span>
                        {:else if isNext}
                          <span class="badge badge-accent">Up next</span>
                        {/if}
                        {#if e.filler}<span class="badge">Filler</span>{/if}
                        {#if e.recap}<span class="badge">Recap</span>{/if}
                      </div>
                      {#if e.titleJa && e.titleJa !== epTitle(e)}<p class="ep-jp jp">{e.titleJa}</p>{/if}
                      {#if e.overview}
                        <button class="ep-overview" class:open={expanded.has(e.epKey)} onclick={() => toggleExpand(e.epKey)}>
                          {e.overview}
                        </button>
                      {/if}
                      <div class="ep-meta">
                        {#if e.airDate}<span>{formatDate(e.airDate)}</span>{/if}
                        {#if e.runtime}<span><Icon name="clock" size={12} /> {e.runtime}m</span>{/if}
                        {#if !e.watchedAt && e.progressPos && has}
                          <span class="resume-at">Resume from {clock(e.progressPos)}</span>
                        {/if}
                        {#if has}
                          <button class="file-chip" onclick={() => api.revealFile(e.files[0].path)} title={e.files[0].path}>
                            <Icon name="file" size={12} />
                            {formatBytes(e.files[0].size)}
                            {#if e.files.length > 1}· {e.files.length} files{/if}
                          </button>
                        {:else if app.appMode === "streaming"}
                          <button
                            class="file-chip"
                            onclick={() => (torrentTarget = { epKey: e.epKey, epNumber: e.number, autoStart: false })}
                            title="Stream or download via BitTorrent"
                          >
                            <Icon name="sparkles" size={12} /> Stream ready
                          </button>
                        {:else}
                          <span class="no-file"><Icon name="file-missing" size={12} /> Not in library</span>
                        {/if}
                      </div>
                      {#if e.subtitles && e.subtitles.length > 0}
                        <div class="ep-subs">
                          {#each e.subtitles as sub (sub.id)}
                            <span class="sub-pill" title={sub.fileName}>
                              <Icon name="subtitles" size={11} />
                              <strong>{sub.lang.toUpperCase()}</strong>
                              {#if sub.cc}<span class="sub-tag">CC</span>{/if}
                              <span class="sub-ext">.{sub.fileName.split(".").pop()?.toLowerCase() ?? "srt"}</span>
                              {#if sub.offsetMs !== null}
                                <span class="sub-offset" title="Auto-retimed offset">{formatOffsetMs(sub.offsetMs)}</span>
                              {/if}
                              {#if has}
                                <button
                                  class="sub-act"
                                  disabled={retimingSubId === sub.id}
                                  onclick={() => retimeSub(sub)}
                                  title="Auto-retime this subtitle against the episode video"
                                >
                                  {retimingSubId === sub.id ? "…" : "Retime"}
                                </button>
                              {/if}
                              <button
                                class="sub-act danger"
                                onclick={() => unlinkSub(sub.id)}
                                title="Unlink subtitle"
                                aria-label="Unlink subtitle"
                              >
                                ×
                              </button>
                            </span>
                          {/each}
                        </div>
                      {/if}
                    </div>

                    <div class="ep-actions">
                      {#if app.appMode === "streaming"}
                        <button
                          class="btn btn-ghost btn-sm upto"
                          onclick={() => (torrentTarget = { epKey: e.epKey, epNumber: e.number, autoStart: false })}
                          title="Choose torrent release to stream or download"
                        >
                          <Icon name="download" size={13} /> Torrent
                        </button>
                      {/if}
                      {#if has || app.appMode === "streaming"}
                        <button
                          class="btn btn-ghost btn-sm upto"
                          onclick={() => (jimakuTarget = { epKey: e.epKey, epNumber: e.number })}
                          title="Fetch & auto-retime Japanese subtitles from Jimaku"
                        >
                          <Icon name="subtitles" size={13} /> Subs
                        </button>
                      {/if}
                      {#if !e.isSpecial && !e.watchedAt && e.number > 1}
                        <button class="btn btn-ghost btn-sm upto" onclick={() => watchedUpTo(e)} title="Mark all episodes up to here as watched">
                          Up to here
                        </button>
                      {/if}
                      <button
                        class="watch-toggle"
                        class:on={!!e.watchedAt}
                        onclick={() => toggleWatched(e)}
                        aria-label={e.watchedAt ? "Mark as unwatched" : "Mark as watched"}
                        title={e.watchedAt ? "Watched — click to unmark" : "Mark as watched"}
                        id={`ep-toggle-${e.epKey}`}
                      >
                        <Icon name="check" size={15} stroke={3} />
                      </button>
                    </div>
                  </li>
                {/each}
              </ol>
            {/snippet}

            {#if regular.length > 0}
              <section class="block">
                <h2 class="section-title">
                  {d.format === "MOVIE" ? "Movie" : "Episodes"}
                  <span class="count">{owned} of {totalEps || regular.length} in library · {watched} watched</span>
                </h2>
                {@render episodeList(regular)}
              </section>
            {/if}

            {#if specials.length > 0}
              <section class="block">
                <h2 class="section-title">
                  Specials <span class="count">{specials.filter((s) => s.files.length).length} of {specials.length} in library</span>
                </h2>
                <p class="hint faint">Extra episodes listed by the metadata provider. Not all of these are required viewing.</p>
                {@render episodeList(specials)}
              </section>
            {/if}

            {#if d.extras && d.extras.length > 0}
              <section class="block">
                <h2 class="section-title">
                  Extras & Bonus <span class="count">{d.extras.length}</span>
                </h2>
                <p class="hint faint">Openings, endings, trailers, and promotional material.</p>
                <div class="extras-grid">
                  {#each d.extras as ex (ex.id)}
                    {@const isLaunchingEx = app.isLaunching(d.anilistId, `extra:${ex.id}`)}
                    {@const isPlayingEx = app.isPlaying(d.anilistId, `extra:${ex.id}`)}
                    <div class="extra-card" id={`extra-${ex.id}`}>
                      <div class="extra-thumb">
                        <span class="extra-kind-badge" class:badge-accent={ex.kind === 'opening' || ex.kind === 'ending'} class:badge-ok={ex.kind === 'trailer' || ex.kind === 'pv'}>
                          {extraBadge(ex.kind)}
                        </span>
                        <button class="extra-play-btn" onclick={() => playExtra(ex)} disabled={isLaunchingEx} title={`Play ${ex.title}`}>
                          {#if isLaunchingEx}
                            <span class="spinner spinner-xs"></span>
                          {:else if isPlayingEx}
                            <span class="eq-bars"><span class="eq-bar"></span><span class="eq-bar"></span><span class="eq-bar"></span></span>
                          {:else}
                            <Icon name="play" size={16} fill />
                          {/if}
                        </button>
                      </div>
                      <div class="extra-body">
                        <div class="extra-title-row">
                          <h4 class="extra-title" title={ex.title}>{ex.title}</h4>
                        </div>
                        <p class="extra-filename faint" title={ex.fileName}>{ex.fileName}</p>
                        <div class="extra-meta">
                          <span class="file-chip" title={ex.path}>
                            <Icon name="file" size={11} /> {formatBytes(ex.size)}
                          </span>
                          <button class="btn btn-ghost btn-sm extra-reveal" onclick={() => api.revealFile(ex.path)} title="Show in folder">
                            <Icon name="folder" size={12} /> Show
                          </button>
                        </div>
                      </div>
                    </div>
                  {/each}
                </div>
              </section>
            {/if}

            {#if d.otherFiles.length > 0}
              <section class="block">
                <h2 class="section-title">Other files <span class="count">{d.otherFiles.length}</span></h2>
                <p class="hint faint">These files were found but couldn't be matched to a specific episode.</p>
                <ul class="other-files">
                  {#each d.otherFiles as f (f.id)}
                    <li>
                      <Icon name="file" size={14} />
                      <span title={f.path}>{f.fileName}</span>
                      <button class="btn btn-ghost btn-sm" onclick={() => api.openFile(f.path)}>Open</button>
                      <button class="btn btn-ghost btn-sm" onclick={() => api.revealFile(f.path)}>Show in folder</button>
                    </li>
                  {/each}
                </ul>
              </section>
            {/if}

            {#if d.characters && d.characters.length > 0}
              {@const shownChars = showAllChars ? d.characters : d.characters.slice(0, 8)}
              <section class="block" id="detail-characters">
                <h2 class="section-title">
                  Characters & Voice Cast <span class="count">{d.characters.length}</span>
                </h2>
                <div class="chars-grid">
                  {#each shownChars as ch (ch.characterId + "-" + (ch.vaId ?? 0))}
                    <div class="char-card">
                      <div class="char-side">
                        <div class="char-avatar">
                          {#if ch.imageUrl}<img src={ch.imageUrl} alt={ch.nameFull} loading="lazy" />{/if}
                        </div>
                        <div class="char-text">
                          <strong title={ch.nameFull}>{ch.nameFull}</strong>
                          {#if ch.role}<span class="char-role">{ch.role.charAt(0) + ch.role.slice(1).toLowerCase()}</span>{/if}
                        </div>
                      </div>

                      {#if ch.vaId && ch.vaNameFull}
                        <button
                          class="va-side"
                          onclick={() => (selectedPerson = { id: ch.vaId!, name: ch.vaNameFull! })}
                          title={`View other anime with ${ch.vaNameFull}`}
                        >
                          <div class="va-text">
                            <strong title={ch.vaNameFull}>{ch.vaNameFull}</strong>
                            {#if ch.vaNameNative}<span class="char-role jp">{ch.vaNameNative}</span>{:else}<span class="char-role">Japanese VA</span>{/if}
                          </div>
                          <div class="char-avatar va-avatar">
                            {#if ch.vaImageUrl}<img src={ch.vaImageUrl} alt={ch.vaNameFull} loading="lazy" />{/if}
                          </div>
                        </button>
                      {/if}
                    </div>
                  {/each}
                </div>
                {#if d.characters.length > 8}
                  <button class="more" onclick={() => (showAllChars = !showAllChars)}>
                    {showAllChars ? "Show fewer characters" : `Show all ${d.characters.length} characters`}
                  </button>
                {/if}
              </section>
            {/if}

            {#if d.staff && d.staff.length > 0}
              <section class="block" id="detail-staff">
                <h2 class="section-title">
                  Staff <span class="count">{d.staff.length}</span>
                </h2>
                <div class="staff-grid">
                  {#each d.staff as st, idx (st.staffId + "-" + idx)}
                    <button
                      class="staff-card"
                      onclick={() => (selectedPerson = { id: st.staffId, name: st.nameFull })}
                      title={`View other anime by ${st.nameFull}`}
                    >
                      <div class="char-avatar">
                        {#if st.imageUrl}<img src={st.imageUrl} alt={st.nameFull} loading="lazy" />{/if}
                      </div>
                      <div class="char-text">
                        <strong title={st.nameFull}>{st.nameFull}</strong>
                        <span class="char-role" title={st.role ?? ""}>{st.role ?? "Staff"}</span>
                      </div>
                    </button>
                  {/each}
                </div>
              </section>
            {/if}
          </div>

          <aside class="side-col">
            {#if relations.length > 0}
              <section class="card side-card">
                <h2 class="side-title">Related</h2>
                <div class="relations">
                  {#each relations as r (r.relatedId)}
                    {@const rc = img(r.coverPath, r.coverUrl)}
                    <a class="relation" class:not-owned={r.ownedCount === 0} href={`/anime/${r.relatedId}`}>
                      <div class="r-cover">{#if rc}<img src={rc} alt="" loading="lazy" />{/if}</div>
                      <div class="r-info">
                        <span class="r-type">{relationLabel(r.relationType)}</span>
                        <strong>{r.title}</strong>
                        <span class="faint">{[formatLabel(r.format), r.seasonYear].filter(Boolean).join(" · ")}</span>
                      </div>
                      {#if r.ownedCount > 0}
                        <span class="badge badge-ok">Owned</span>
                      {:else}
                        <span class="badge" title="View details in Kura">View</span>
                      {/if}
                    </a>
                  {/each}
                </div>
              </section>
            {/if}

            <section class="card side-card">
              <h2 class="side-title">Details</h2>
              <dl>
                {#if d.titleEnglish}<dt>English</dt><dd>{d.titleEnglish}</dd>{/if}
                {#if d.titleRomaji}<dt>Romaji</dt><dd>{d.titleRomaji}</dd>{/if}
                {#if d.titleNative}<dt>Native</dt><dd class="jp">{d.titleNative}</dd>{/if}
                {#if d.synonyms.length}<dt>Also known as</dt><dd>{d.synonyms.slice(0, 5).join(" · ")}</dd>{/if}
                {#if d.startDate}
                  <dt>Aired</dt>
                  <dd>{formatDate(d.startDate)}{#if d.endDate && d.endDate !== d.startDate} → {formatDate(d.endDate)}{/if}</dd>
                {/if}
                {#if d.studios.length}<dt>Studio</dt><dd>{d.studios.join(", ")}</dd>{/if}
                {#if d.episodes}<dt>Episodes</dt><dd>{d.episodes}</dd>{/if}
              </dl>
              {#if d.genres.length}
                <div class="tags">{#each d.genres as g (g)}<span class="chip static">{g}</span>{/each}</div>
              {/if}
              {#if d.tags.length}
                <div class="tags small">{#each d.tags.slice(0, 12) as t (t)}<span class="badge">{t}</span>{/each}</div>
              {/if}
            </section>

            <section class="card side-card">
              <h2 class="side-title">Files</h2>
              {#each d.groups as g (g.id)}
                <div class="group">
                  <Icon name="folder" size={14} />
                  <span title={g.folderPath ?? g.displayName}>{g.displayName}</span>
                  {#if g.manual}<span class="badge badge-ok">Manual</span>
                  {:else if g.confidence !== null}<span class="badge" class:badge-warn={g.confidence < 0.8}>{Math.round(g.confidence * 100)}%</span>{/if}
                </div>
              {/each}
              <p class="src faint">
                AniList #{d.anilistId}{#if d.idMal} · MAL #{d.idMal}{/if}
                {#if d.episodesSource} · episodes via {d.episodesSource.replace("anizip", "ani.zip").replace("+", " + ")}{/if}
              </p>
            </section>
          </aside>
        </div>
      </div>
    </div>

    {#if showFix}
      <FixMatchDialog
        initialQuery={d.groups[0]?.displayName ?? d.titleRomaji ?? ""}
        fromAnilistId={d.anilistId}
        subtitle={`Currently matched to “${displayTitle(d)}”`}
        onclose={() => (showFix = false)}
        ondone={(newId) => goto(`/anime/${newId}`)}
      />
    {/if}

    {#if showTracks}
      <TrackPrefsDialog
        anilistId={d.anilistId}
        title={displayTitle(d)}
        initialPref={d.trackPref}
        onclose={() => (showTracks = false)}
        onsave={(saved) => {
          if (d) d.trackPref = saved;
        }}
      />
    {/if}

    {#if showMatcher}
      <SubtitleMatcherDialog
        anilistId={d.anilistId}
        title={displayTitle(d)}
        onclose={() => (showMatcher = false)}
        onapplied={() => load(d!.anilistId)}
      />
    {/if}

    {#if jimakuTarget}
      <JimakuDialog
        anilistId={d.anilistId}
        title={displayTitle(d)}
        epKey={jimakuTarget.epKey ?? "1"}
        episodeNumber={jimakuTarget.epNumber}
        hasLocalVideo={owned > 0}
        onclose={() => (jimakuTarget = null)}
        ondownloaded={() => load(d!.anilistId)}
      />
    {/if}

    {#if selectedPerson !== null}
      <PersonModal
        staffId={selectedPerson.id}
        initialName={selectedPerson.name}
        onclose={() => (selectedPerson = null)}
      />
    {/if}

    {#if torrentTarget !== null}
      <TorrentPickerDialog
        anilistId={d.anilistId}
        title={displayTitle(d)}
        epKey={torrentTarget.epKey}
        episodeNumber={torrentTarget.epNumber}
        autoStart={torrentTarget.autoStart}
        onclose={() => (torrentTarget = null)}
      />
    {/if}
  {/key}
{/if}

<style>
  .center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding-top: 120px;
    text-align: center;
    color: var(--text-3);
  }
  .center h2 {
    color: var(--text);
  }

  /* Banner ----------------------------------------------------------------
   * A grand cinematic backdrop panel; the poster overlaps its bottom edge. */
  .banner {
    position: relative;
    height: 380px;
    margin: 20px 28px 0;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-xl);
    background: var(--coral);
    box-shadow: var(--sticker-lg);
    overflow: hidden;
  }
  .banner.skeleton {
    background: linear-gradient(
      110deg,
      var(--surface) 25%,
      var(--surface-2) 37%,
      var(--surface) 63%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s ease infinite;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .loading-banner-pill {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 10px 18px;
    border-radius: 999px;
    background: var(--bg-elev);
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 14px;
    color: var(--text);
  }
  .skeleton-box {
    background: linear-gradient(
      110deg,
      var(--surface) 25%,
      var(--surface-2) 37%,
      var(--surface) 63%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s ease infinite;
  }
  .skeleton-info {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding-top: 8px;
  }
  .skeleton-line {
    height: 16px;
    border-radius: 8px;
    background: linear-gradient(
      110deg,
      var(--surface) 25%,
      var(--surface-2) 37%,
      var(--surface) 63%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s ease infinite;
    border: 1.5px solid var(--border);
  }
  .skeleton-line.h-lg {
    height: 38px;
  }
  .skeleton-line.h-md {
    height: 64px;
  }
  .skeleton-line.w-30 { width: 30%; }
  .skeleton-line.w-45 { width: 45%; }
  .skeleton-line.w-70 { width: 70%; }
  .skeleton-line.w-90 { width: 90%; }
  @keyframes shimmer {
    0% { background-position: 100% 50%; }
    100% { background-position: 0% 50%; }
  }
  .banner img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 25%;
    animation: hero-in 1s var(--ease) both;
  }
  .banner img.blur {
    filter: blur(24px) saturate(1.2) brightness(0.8);
    transform: scale(1.2);
  }
  @keyframes hero-in {
    from {
      opacity: 0;
      transform: scale(1.05);
    }
  }
  .banner-shade {
    display: block;
    position: absolute;
    inset: 0;
    background: linear-gradient(
      to bottom,
      color-mix(in srgb, var(--line) 20%, transparent) 0%,
      transparent 35%,
      color-mix(in srgb, var(--bg) 40%, transparent) 70%,
      var(--bg) 100%
    );
    pointer-events: none;
  }
  .back {
    position: absolute;
    z-index: 2;
    top: 16px;
    left: 16px;
    height: 36px;
    background: var(--surface);
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
    color: var(--text);
  }
  .back:hover {
    background: var(--surface-3);
    transform: translate(-1px, -1px);
    box-shadow: 4px 4px 0 var(--line);
  }

  /* Header ---------------------------------------------------------------- */
  .body {
    position: relative;
  }
  .head {
    display: flex;
    gap: 32px;
    align-items: flex-start;
    padding: 0 24px;
    animation: fade-up 500ms var(--ease) both;
  }
  .poster {
    width: 210px;
    flex: none;
    margin-top: -150px;
    aspect-ratio: 2/3;
    border-radius: var(--r-lg);
    border: var(--bw) solid var(--line);
    overflow: hidden;
    background: var(--surface-2);
    box-shadow: var(--sticker-lg);
    transform: rotate(-1.5deg);
    transition: transform var(--t-med) var(--bounce);
  }
  .poster:hover {
    transform: rotate(0deg) scale(1.02);
  }
  .poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .head-info {
    flex: 1;
    min-width: 0;
    padding-top: 22px;
  }
  .badges {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .badge-accent {
    background: var(--coral);
    color: var(--on-coral);
  }
  .banner-clearart {
    position: absolute;
    right: 32px;
    bottom: 24px;
    z-index: 2;
    max-width: min(380px, 42vw);
    max-height: 120px;
    display: flex;
    align-items: flex-end;
    justify-content: flex-end;
    pointer-events: none;
    user-select: none;
  }
  .banner-clearart img {
    max-width: 100%;
    max-height: 120px;
    width: auto;
    height: auto;
    object-fit: contain;
    filter: drop-shadow(0 4px 16px rgba(0, 0, 0, 0.8)) drop-shadow(0 2px 4px rgba(0, 0, 0, 0.6));
    animation: hero-in 600ms var(--ease) both;
  }
  :global([data-theme="manga"]) .banner-clearart img {
    filter: drop-shadow(3px 3px 0 #000) drop-shadow(-1px -1px 0 #fff);
  }
  @media (max-width: 768px) {
    .banner-clearart {
      right: 16px;
      bottom: 16px;
      max-width: 180px;
      max-height: 60px;
    }
    .banner-clearart img {
      max-height: 60px;
    }
  }
  h1 {
    margin-top: 12px;
    font-size: 38px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .alt {
    margin-top: 4px;
    color: var(--text-2);
    font-size: 15px;
    overflow-wrap: anywhere;
  }
  .alt.jp {
    color: var(--text-3);
  }
  .stats {
    display: flex;
    gap: 32px;
    margin-top: 20px;
    flex-wrap: wrap;
  }
  .stat strong {
    display: block;
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 600;
    line-height: 1.1;
  }
  .stat strong span {
    color: var(--text-3);
    font-weight: 500;
    font-size: 17px;
  }
  .stat strong.sm {
    font-size: 17px;
    padding-top: 6px;
  }
  .stat strong.warn span {
    color: var(--amber);
  }
  .stat strong.airing {
    color: var(--ok);
  }
  .stat small {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-3);
  }
  .progress {
    margin-top: 16px;
    height: 10px;
    max-width: 520px;
    border-radius: 99px;
    background: var(--bg-elev);
    border: var(--bw) solid var(--line);
    overflow: hidden;
  }
  .progress div {
    height: 100%;
    background: var(--coral);
    border-radius: inherit;
    transition: width 600ms var(--ease);
  }
  .actions {
    display: flex;
    gap: 10px;
    margin-top: 22px;
    flex-wrap: wrap;
  }
  .actions .btn-primary {
    height: 44px;
    padding: 0 22px;
  }
  .actions .btn:not(.btn-primary):not(.btn-icon) {
    height: 44px;
  }
  .actions .btn-icon {
    height: 44px;
    width: 44px;
  }

  /* Season tabs -----------------------------------------------------------
   * One sticker per owned season / movie of the franchise. */
  .seasons {
    display: flex;
    gap: 12px;
    margin-top: 30px;
    padding: 6px 6px 12px 4px;
    overflow-x: auto;
    scroll-snap-type: x proximity;
  }
  .season {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 150px;
    max-width: 250px;
    padding: 10px 14px 14px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--surface);
    box-shadow: var(--sticker);
    color: var(--text);
    scroll-snap-align: start;
    overflow: hidden;
    transition:
      transform var(--t-fast) var(--bounce),
      box-shadow var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .season:hover {
    transform: translate(-2px, -2px) rotate(-0.6deg);
    box-shadow: 5px 5px 0 var(--line);
  }
  .season.active {
    background: var(--coral);
    color: var(--on-coral);
    transform: translate(-2px, -2px);
    box-shadow: 5px 5px 0 var(--line);
  }
  .s-label {
    font-family: var(--font-display);
    font-size: 15.5px;
    font-weight: 600;
    line-height: 1.25;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .s-meta,
  .s-progress {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-3);
    white-space: nowrap;
  }
  .s-progress {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 2px;
  }
  .season.done .s-progress {
    color: var(--ok);
  }
  .season.active .s-meta,
  .season.active .s-progress {
    color: color-mix(in srgb, var(--on-coral) 75%, transparent);
  }
  .s-bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 5px;
    background: var(--bg-elev);
    border-top: 1.5px solid var(--line);
  }
  .s-bar span {
    display: block;
    height: 100%;
    background: var(--coral);
    transition: width 600ms var(--ease);
  }
  .season.active .s-bar span {
    background: var(--on-coral);
  }
  .season.done .s-bar span {
    background: var(--ok);
  }

  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 28px;
    padding: 11px 16px;
    border-radius: var(--r-md);
    background: var(--surface);
    border: var(--bw) solid var(--line);
    border-left: 8px solid var(--amber);
    box-shadow: var(--sticker);
    color: var(--amber);
    font-size: 13.5px;
  }
  .notice span {
    color: var(--text-2);
  }
  .notice.danger {
    border-left-color: var(--danger);
    color: var(--danger);
  }

  /* Columns --------------------------------------------------------------- */
  .columns {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 340px;
    gap: 36px;
    margin-top: 36px;
  }
  @media (max-width: 1180px) {
    .columns {
      grid-template-columns: 1fr;
    }
  }
  .block {
    margin-bottom: 38px;
  }
  .synopsis {
    color: var(--text-2);
    white-space: pre-line;
    line-height: 1.7;
    max-width: 820px;
    display: -webkit-box;
    -webkit-line-clamp: 5;
    line-clamp: 5;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .synopsis.open {
    display: block;
  }
  .more {
    margin-top: 8px;
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    font-weight: 600;
    cursor: pointer;
  }
  .hint {
    margin: -6px 0 14px;
    font-size: 13px;
  }

  /* Episodes -------------------------------------------------------------- */
  .episodes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ep {
    display: flex;
    gap: 18px;
    align-items: flex-start;
    padding: 10px;
    border-radius: var(--r-lg);
    border: var(--bw) solid transparent;
    transition:
      background var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
    animation: fade-up 380ms var(--ease) both;
    animation-delay: calc(var(--i) * 14ms);
  }
  .ep:hover {
    background: var(--surface);
    border-color: var(--line);
  }
  .ep.next {
    background: var(--surface);
    border-color: var(--line);
    box-shadow: 4px 4px 0 var(--coral);
  }
  .ep-thumb {
    position: relative;
    width: 200px;
    flex: none;
    aspect-ratio: 16/9;
    border-radius: var(--r-md);
    overflow: hidden;
    border: var(--bw) solid var(--line);
    padding: 0;
    background: var(--surface-2);
    cursor: pointer;
  }
  .ep-thumb:disabled {
    cursor: default;
  }
  .ep-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition: transform 500ms var(--ease);
  }
  .ep-thumb img.fallback {
    filter: blur(6px) brightness(0.6);
    transform: scale(1.15);
  }
  .ep:hover .ep-thumb:not(:disabled) img {
    transform: scale(1.05);
  }
  .ep.missing .ep-thumb img {
    filter: grayscale(1) brightness(0.45);
  }
  .ep-num {
    position: absolute;
    left: 6px;
    bottom: 6px;
    min-width: 28px;
    height: 26px;
    padding: 0 7px;
    display: grid;
    place-items: center;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    background: var(--bg);
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 15px;
    line-height: 1;
  }
  .ep.next .ep-num {
    background: var(--coral);
    color: var(--on-coral);
  }
  .play-ov {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg) 45%, transparent);
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }
  .play-ov.active {
    opacity: 1;
    background: color-mix(in srgb, var(--bg) 60%, transparent);
  }
  .play-ov .spinner {
    width: 26px;
    height: 26px;
    border-width: 2.5px;
    border-color: rgba(255, 255, 255, 0.3);
    border-top-color: var(--coral);
  }
  .play-ov .eq-bars {
    background: var(--coral);
    color: var(--on-coral);
    padding: 7px 9px;
    border-radius: 12px;
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
    height: 12px;
    gap: 2.5px;
  }
  .launching-badge,
  .playing-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .play-ov :global(svg) {
    width: 44px;
    height: 44px;
    padding: 11px 9px 11px 13px;
    border-radius: 50%;
    border: var(--bw) solid var(--line);
    background: var(--coral);
    color: var(--on-coral);
    box-shadow: var(--sticker);
    transform: scale(0.85);
    transition: transform var(--t-med) var(--bounce);
  }
  .ep-thumb:hover .play-ov,
  .ep-thumb:focus-visible .play-ov {
    opacity: 1;
  }
  .ep-thumb:hover .play-ov :global(svg) {
    transform: scale(1);
  }
  .seen {
    position: absolute;
    top: 6px;
    right: 6px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 1.5px solid var(--line);
    background: var(--ok);
    color: var(--on-coral);
  }
  .ep-prog {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 5px;
    background: color-mix(in srgb, var(--bg) 70%, transparent);
  }
  .ep-prog span {
    display: block;
    height: 100%;
    background: var(--coral);
  }
  .resume-at {
    color: var(--coral);
    font-weight: 700;
  }
  .ep-body {
    flex: 1;
    min-width: 0;
    padding-top: 2px;
  }
  .ep-title-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    min-width: 0;
  }
  /* Badges keep their size; the title is what gives way on long names. */
  .ep-title-row > :global(.badge) {
    flex-shrink: 0;
    white-space: nowrap;
    margin-top: 2px;
  }
  .ep h3 {
    font-family: var(--font-display);
    font-size: 16px;
    font-weight: 500;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
    /* Flex items default to min-width: auto, which stops text from shrinking. */
    min-width: 0;
    flex: 0 1 auto;
  }
  .ep.watched h3 {
    color: var(--text-2);
  }
  .ep.missing h3 {
    color: var(--text-3);
  }
  .ep-jp {
    margin-top: 1px;
    font-size: 12.5px;
    color: var(--text-3);
    overflow-wrap: anywhere;
  }
  .ep-overview {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    margin-top: 6px;
    padding: 0;
    border: none;
    background: none;
    text-align: left;
    font-size: 13px;
    line-height: 1.55;
    color: var(--text-2);
    cursor: pointer;
  }
  .ep-overview.open {
    display: block;
  }
  .ep.missing .ep-overview {
    color: var(--text-3);
  }
  .ep-meta {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 8px;
    font-size: 12px;
    color: var(--text-3);
  }
  .ep-meta span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .file-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border-radius: 7px;
    border: none;
    background: color-mix(in srgb, var(--ok) 14%, transparent);
    color: var(--ok);
    font-size: 11.5px;
    font-weight: 700;
    cursor: pointer;
  }
  .file-chip:hover {
    background: color-mix(in srgb, var(--ok) 24%, transparent);
  }
  .no-file {
    color: var(--text-3);
  }
  .ep-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    align-self: center;
  }
  .upto {
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }
  .ep:hover .upto {
    opacity: 1;
  }
  .watch-toggle {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    border: var(--bw) solid var(--border-strong);
    background: transparent;
    color: transparent;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .watch-toggle:hover {
    border-color: var(--ok);
    color: var(--ok);
  }
  .watch-toggle.on {
    background: var(--ok);
    border-color: var(--line);
    color: var(--on-coral);
    box-shadow: 2px 2px 0 var(--line);
    animation: pop-in 300ms var(--bounce);
  }
  @keyframes pop-in {
    50% {
      transform: scale(1.2) rotate(-8deg);
    }
  }

  .other-files {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .other-files li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: var(--r-md);
    background: var(--surface);
    color: var(--text-3);
  }
  .other-files li span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
    font-size: 13px;
  }

  /* Extras & Bonus -------------------------------------------------------- */
  .extras-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
    margin-top: 12px;
  }
  .extra-card {
    display: flex;
    gap: 12px;
    padding: 12px;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-lg);
    background: var(--surface);
    box-shadow: var(--sticker-sm);
    transition: transform var(--t-fast) var(--ease), box-shadow var(--t-fast) var(--ease);
  }
  .extra-card:hover {
    transform: translateY(-2px);
    box-shadow: var(--sticker-md);
  }
  .extra-thumb {
    position: relative;
    width: 64px;
    height: 64px;
    flex-shrink: 0;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-md);
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .extra-kind-badge {
    position: absolute;
    top: 4px;
    left: 4px;
    padding: 1px 5px;
    font-size: 10px;
    font-family: var(--font-display);
    font-weight: 700;
    border-radius: var(--r-full);
    border: 1.5px solid var(--line);
    background: var(--surface-3);
    color: var(--text);
  }
  .extra-kind-badge.badge-accent {
    background: var(--coral);
    color: var(--cream);
  }
  .extra-kind-badge.badge-ok {
    background: var(--mint);
    color: var(--ink);
  }
  .extra-play-btn {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: var(--bw) solid var(--line);
    background: var(--surface);
    color: var(--coral);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    box-shadow: var(--sticker-sm);
    transition: transform var(--t-fast) var(--ease), background var(--t-fast) var(--ease);
    margin-top: 12px;
  }
  .extra-play-btn:hover {
    transform: scale(1.1);
    background: var(--coral);
    color: var(--cream);
  }
  .extra-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }
  .extra-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .extra-title {
    font-family: var(--font-display);
    font-size: 14px;
    font-weight: 600;
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }
  .extra-filename {
    font-size: 11.5px;
    margin: 2px 0 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-3);
  }
  .extra-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .extra-reveal {
    padding: 2px 6px;
    font-size: 11px;
    height: 22px;
  }

  /* Side column ----------------------------------------------------------- */
  .side-col {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .side-card {
    padding: 18px;
  }
  .side-title {
    font-size: 17px;
    font-weight: 600;
    margin-bottom: 14px;
  }
  .relations {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .relation {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px;
    border: none;
    border-radius: var(--r-md);
    background: transparent;
    text-align: left;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .relation:hover {
    background: var(--surface-2);
  }
  .relation.not-owned .r-cover {
    filter: grayscale(0.8) brightness(0.6);
  }
  .relation.not-owned strong {
    color: var(--text-2);
  }
  .r-cover {
    width: 42px;
    height: 60px;
    flex: none;
    border-radius: 7px;
    border: 1.5px solid var(--line);
    overflow: hidden;
    background: var(--surface-3);
  }
  .r-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .r-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .r-type {
    font-family: var(--font-display);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--coral);
  }
  .r-info strong {
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .r-info .faint {
    font-size: 12px;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 14px;
    margin: 0;
    font-size: 13px;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    color: var(--text-2);
    word-break: break-word;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 16px;
  }
  .tags.small {
    margin-top: 10px;
  }
  .tags .badge {
    text-transform: none;
    letter-spacing: 0;
    font-weight: 500;
  }
  .chip.static {
    cursor: default;
    height: 26px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    color: var(--text-3);
    font-size: 13px;
  }
  .group span:not(.badge) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }
  .src {
    margin-top: 10px;
    font-size: 11.5px;
  }
  .custom-track-badge {
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--coral);
    color: var(--on-coral);
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-left: 2px;
  }

  .anilist-tracker {
    margin-top: 14px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .tracker-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }
  .tracker-brand {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: var(--font-display);
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-2);
  }
  .tracker-signin-link {
    background: transparent;
    border: 0;
    padding: 0;
    color: var(--coral);
    font-size: 12px;
    font-weight: 700;
    cursor: pointer;
  }
  .tracker-signin-link:hover {
    text-decoration: underline;
  }
  .tracker-controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .status-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .status-pill {
    height: 30px;
    padding: 0 11px;
    border-radius: 8px;
    border: 1.5px solid var(--border-strong);
    background: var(--bg-elev);
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .status-pill:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--line);
    transform: translate(-1px, -1px);
  }
  .status-pill.active {
    background: var(--coral);
    color: var(--on-coral);
    border-color: var(--line);
    box-shadow: 2px 2px 0 var(--line);
  }
  .tracker-right {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .prog-stepper {
    display: inline-flex;
    align-items: center;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    background: var(--bg-elev);
    overflow: hidden;
    height: 32px;
  }
  .step-btn {
    height: 100%;
    padding: 0 10px;
    border: 0;
    background: var(--surface-2);
    color: var(--text);
    font-family: var(--font-display);
    font-size: 12.5px;
    font-weight: 700;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .step-btn:hover:not(:disabled) {
    background: var(--coral);
    color: var(--on-coral);
  }
  .step-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .step-btn.plus {
    border-left: 1.5px solid var(--line);
  }
  .prog-readout {
    padding: 0 10px;
    font-size: 12.5px;
    font-variant-numeric: tabular-nums;
    border-left: 1.5px solid var(--line);
  }
  .prog-readout strong {
    font-weight: 800;
    color: var(--text);
  }
  .prog-readout span {
    color: var(--text-3);
  }
  .score-picker {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 32px;
    padding: 0 8px 0 9px;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    background: var(--bg-elev);
    color: var(--amber);
  }
  .score-picker select {
    border: 0;
    background: transparent;
    color: var(--text);
    font-family: var(--font-display);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    outline: none;
  }
  .score-picker select option {
    background: var(--surface);
    color: var(--text);
  }

  /* Linked Episode Subtitles -------------------------------------------- */
  .ep-subs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }
  .sub-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 7px;
    border-radius: 6px;
    background: var(--surface-2);
    border: 1.5px solid var(--border-strong);
    font-size: 11px;
    color: var(--text-2);
  }
  .sub-pill strong {
    color: var(--text);
    font-weight: 800;
  }
  .sub-tag {
    padding: 0 4px;
    border-radius: 4px;
    background: var(--bg-elev);
    font-size: 10px;
    font-weight: 700;
    color: var(--amber);
  }
  .sub-ext {
    color: var(--text-3);
    font-family: var(--font-mono);
  }
  .sub-offset {
    padding: 0 4px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--mint) 18%, transparent);
    color: var(--mint);
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 700;
  }
  .sub-act {
    background: var(--bg-elev);
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    padding: 1px 5px;
    font-size: 10.5px;
    font-weight: 700;
    color: var(--text-2);
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .sub-act:hover:not(:disabled) {
    background: var(--coral);
    color: var(--on-coral);
    border-color: var(--line);
  }
  .sub-act.danger:hover {
    background: var(--danger);
    color: #fff;
  }

  /* Characters & Staff -------------------------------------------------- */
  .chars-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(310px, 1fr));
    gap: 10px;
  }
  .char-card {
    display: flex;
    align-items: stretch;
    justify-content: space-between;
    gap: 8px;
    background: var(--surface);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-md);
    box-shadow: var(--sticker-sm);
    overflow: hidden;
  }
  .char-side {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px;
    min-width: 0;
    flex: 1;
  }
  .va-side {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    padding: 8px;
    min-width: 0;
    flex: 1;
    background: var(--bg-elev);
    border: 0;
    border-left: 1.5px solid var(--border);
    text-align: right;
    color: inherit;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .va-side:hover {
    background: var(--surface-2);
  }
  .va-side:hover strong {
    color: var(--coral);
  }
  .char-avatar {
    width: 46px;
    height: 60px;
    border-radius: 6px;
    overflow: hidden;
    background: var(--surface-2);
    border: 1.5px solid var(--line);
    flex-shrink: 0;
  }
  .char-avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .char-text,
  .va-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .char-text strong,
  .va-text strong {
    font-size: 13px;
    font-weight: 700;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .char-role {
    font-size: 11px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .staff-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
    gap: 10px;
  }
  .staff-card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px;
    background: var(--surface);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-md);
    box-shadow: var(--sticker-sm);
    text-align: left;
    color: inherit;
    cursor: pointer;
    transition: transform var(--t-fast) var(--ease), background var(--t-fast) var(--ease);
  }
  .staff-card:hover {
    background: var(--surface-2);
    transform: translate(-1px, -1px);
  }
  .staff-card:hover strong {
    color: var(--coral);
  }
  @media (max-width: 900px) {
    .head {
      gap: 20px;
      padding: 0 12px;
    }
    .poster {
      width: 155px;
      margin-top: -100px;
    }
    h1 {
      font-size: 28px;
    }
    .ep-thumb {
      width: 150px;
    }
  }
</style>
