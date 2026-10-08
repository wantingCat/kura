<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, img, type EpisodeRow, type ExtraFile, type MediaDetail } from "$lib/api";
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

  const id = $derived(Number(page.params.id));
  let d = $state<MediaDetail | null>(null);
  let error = $state<string | null>(null);
  let showFix = $state(false);
  let showTracks = $state(false);
  let synopsisOpen = $state(false);
  let refreshing = $state(false);
  let expanded = $state<Set<string>>(new Set());

  async function load(target: number) {
    try {
      const res = await api.getMediaDetail(target);
      if (target === id) {
        d = res;
        error = null;
      }
    } catch (e) {
      error = String(e);
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
      null,
  );
  const playingKey = $derived(
    app.nowPlaying && d && app.nowPlaying.anilistId === d.anilistId ? app.nowPlaying.epKey : null,
  );
  const banner = $derived(d ? (img(d.bannerPath, d.bannerUrl) ?? img(d.coverPath, d.coverUrl)) : null);
  const cover = $derived(d ? img(d.coverPath, d.coverUrl) : null);
  const logo = $derived(d && !logoFailed ? img(d.logoPath, d.logoUrl) : null);
  const description = $derived(cleanDescription(d?.description));
  const lowConfidence = $derived(d?.groups.some((g) => !g.manual && (g.confidence ?? 1) < 0.8) ?? false);
  const pct = $derived(totalEps ? Math.round((watched / totalEps) * 100) : 0);
  /** Other owned seasons / movies of this franchise, shown as tabs when grouping is on. */
  const seasons = $derived(app.groupSeasons && d && d.franchise.length > 1 ? d.franchise : []);
  const inTabs = $derived(new Set(seasons.map((s) => s.anilistId)));
  // Owned entries already reachable via the tabs don't need to be repeated under "Related".
  const relations = $derived(d?.relations.filter((r) => !inTabs.has(r.relatedId)) ?? []);

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
    const f = e.files[0];
    if (!f || !d) return;
    await app.play(d.anilistId, e.epKey, f.path);
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

{#if error && !d}
  <div class="page center">
    <Icon name="alert" size={32} />
    <h2>Couldn't load this entry</h2>
    <p class="muted">{error}</p>
    <a class="btn" href="/">Back to library</a>
  </div>
{:else if !d}
  <div class="banner skeleton"></div>
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
            </div>
            <h1>{displayTitle(d)}</h1>
            {#if d.titleRomaji && d.titleRomaji !== displayTitle(d)}<p class="alt">{d.titleRomaji}</p>{/if}
            {#if d.titleNative}<p class="alt jp">{d.titleNative}</p>{/if}

            <div class="stats">
              {#if d.format !== "MOVIE"}
                <div class="stat">
                  <strong>{watched}<span>/{totalEps || "?"}</span></strong>
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
                <button class="btn btn-primary" onclick={() => play(nextUp!)} id="detail-play-next">
                  <Icon name="play" size={15} fill />
                  {#if nextUp.progressPos}
                    Resume · {d.format === "MOVIE" ? "" : `Ep ${nextUp.number} · `}{clock(nextUp.progressPos)}
                  {:else}
                    {d.format === "MOVIE" ? "Play" : `${watched > 0 ? "Continue" : "Start"} · Ep ${nextUp.number}`}
                  {/if}
                </button>
              {/if}
              <button class="btn" onclick={() => (showTracks = true)} id="detail-track-prefs" title="Audio & Subtitles">
                <Icon name="subtitles" size={15} /> Audio & Subs
                {#if d.trackPref}
                  <span class="custom-track-badge">Custom</span>
                {/if}
              </button>
              <button class="btn" onclick={() => (showFix = true)} id="detail-fix-match">
                <Icon name="link" size={15} /> Fix match
              </button>
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
                  <li
                    class="ep"
                    class:missing={!has}
                    class:watched={!!e.watchedAt}
                    class:next={isNext}
                    style:--i={Math.min(i, 30)}
                    id={`ep-${e.epKey}`}
                  >
                    <button class="ep-thumb" onclick={() => play(e)} disabled={!has} aria-label={`Play ${epTitle(e)}`}>
                      {#if thumb}<img src={thumb} alt="" loading="lazy" class:fallback={!e.thumbUrl} />{/if}
                      <span class="ep-num">{e.isSpecial ? "SP" : ""}{e.number}</span>
                      {#if has}
                        <span class="play-ov"><Icon name="play" size={20} fill /></span>
                      {/if}
                      {#if e.watchedAt}<span class="seen"><Icon name="check" size={12} stroke={3} /></span>{/if}
                      {#if !e.watchedAt && progressPct(e) > 0}
                        <span class="ep-prog"><span style:width={`${progressPct(e)}%`}></span></span>
                      {/if}
                    </button>

                    <div class="ep-body">
                      <div class="ep-title-row">
                        <h3>{epTitle(e)}</h3>
                        {#if playingKey === e.epKey}
                          <span class="badge badge-ok">Playing</span>
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
                        {:else}
                          <span class="no-file"><Icon name="file-missing" size={12} /> Not in library</span>
                        {/if}
                      </div>
                    </div>

                    <div class="ep-actions">
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
                    <div class="extra-card" id={`extra-${ex.id}`}>
                      <div class="extra-thumb">
                        <span class="extra-kind-badge" class:badge-accent={ex.kind === 'opening' || ex.kind === 'ending'} class:badge-ok={ex.kind === 'trailer' || ex.kind === 'pv'}>
                          {extraBadge(ex.kind)}
                        </span>
                        <button class="extra-play-btn" onclick={() => playExtra(ex)} title={`Play ${ex.title}`}>
                          <Icon name="play" size={16} fill />
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
          </div>

          <aside class="side-col">
            {#if relations.length > 0}
              <section class="card side-card">
                <h2 class="side-title">Related</h2>
                <div class="relations">
                  {#each relations as r (r.relatedId)}
                    {@const rc = img(r.coverPath, r.coverUrl)}
                    {#if r.ownedCount > 0}
                      <a class="relation" href={`/anime/${r.relatedId}`}>
                        <div class="r-cover">{#if rc}<img src={rc} alt="" loading="lazy" />{/if}</div>
                        <div class="r-info">
                          <span class="r-type">{relationLabel(r.relationType)}</span>
                          <strong>{r.title}</strong>
                          <span class="faint">{[formatLabel(r.format), r.seasonYear].filter(Boolean).join(" · ")}</span>
                        </div>
                        <span class="badge badge-ok">Owned</span>
                      </a>
                    {:else}
                      <button class="relation not-owned" onclick={() => openUrl(`https://anilist.co/anime/${r.relatedId}`)}>
                        <div class="r-cover">{#if rc}<img src={rc} alt="" loading="lazy" />{/if}</div>
                        <div class="r-info">
                          <span class="r-type">{relationLabel(r.relationType)}</span>
                          <strong>{r.title}</strong>
                          <span class="faint">{[formatLabel(r.format), r.seasonYear].filter(Boolean).join(" · ")}</span>
                        </div>
                        <span class="badge" title="Not in your library">—</span>
                      </button>
                    {/if}
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
    background: var(--surface);
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
  }
  .alt {
    margin-top: 4px;
    color: var(--text-2);
    font-size: 15px;
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
    align-items: center;
    gap: 8px;
  }
  .ep h3 {
    font-family: var(--font-display);
    font-size: 16px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
</style>
