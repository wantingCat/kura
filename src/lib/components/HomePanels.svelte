<script lang="ts">
  import { api, img, type MediaCard, type UpNextItem } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { detailsModal } from "$lib/detailsModal.svelte";
  import { clock, displayTitle, formatLabel, groupCards, seasonLabel, type LibraryItem } from "$lib/format";
  import { JA, jaEpisodeCount, jaFormat, jaGenre, jaSeason } from "$lib/ja";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";
  import PosterTile, { score10 } from "./PosterTile.svelte";
  import Rail from "./Rail.svelte";
  import StillTile from "./StillTile.svelte";

  let { scoped }: { scoped: MediaCard[] } = $props();

  let upNext = $state<UpNextItem[]>([]);
  let fresh = $state<UpNextItem[]>([]);
  let heroIdx = $state(0);
  let hovered = $state(false);
  let focused = $state(false);
  let reducedMotion = $state(false);

  $effect(() => {
    void app.version;
    Promise.all([api.getUpNext(14), api.getNewEpisodes(14)])
      .then(([u, n]) => {
        upNext = u;
        fresh = n;
      })
      .catch((e) => console.warn("[kura] home panels", e));
  });

  $effect(() => {
    if (typeof matchMedia === "undefined") return;
    const mq = matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotion = mq.matches;
    const on = (e: MediaQueryListEvent) => (reducedMotion = e.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  });

  const ids = $derived(new Set(scoped.map((c) => c.anilistId)));
  const cardById = $derived(new Map(scoped.map((c) => [c.anilistId, c])));
  const cont = $derived(upNext.filter((i) => ids.has(i.anilistId)));
  const newEps = $derived(fresh.filter((i) => ids.has(i.anilistId)));

  type Hero = { card: MediaCard; item: UpNextItem | null; kicker: string };

  /** Spotlight: what you're watching first, then new episodes, then recently added shows with artwork. */
  const heroes = $derived.by<Hero[]>(() => {
    const out: Hero[] = [];
    const seen = new Set<number>();
    const push = (card: MediaCard | undefined, item: UpNextItem | null, kicker: string) => {
      if (!card || seen.has(card.anilistId) || out.length >= 5) return;
      seen.add(card.anilistId);
      out.push({ card, item, kicker });
    };
    for (const i of cont) push(cardById.get(i.anilistId), i, "Continue watching");
    for (const i of newEps) push(cardById.get(i.anilistId), i, "New episode");
    if (out.length < 3) {
      [...scoped]
        .filter((c) => c.bannerUrl || c.bannerPath)
        .sort((a, b) => b.addedAt - a.addedAt)
        .slice(0, 6)
        .forEach((c) => push(c, null, "In your library"));
    }
    if (out.length < 3) {
      [...scoped].sort((a, b) => b.addedAt - a.addedAt).forEach((c) => push(c, null, "In your library"));
    }
    return out;
  });

  const current = $derived(heroes.length ? heroIdx % heroes.length : 0);
  const hero = $derived(heroes.length ? heroes[current] : null);
  const paused = $derived(
    hovered || focused || reducedMotion || Boolean(app.builtinPlayer?.active),
  );

  $effect(() => {
    const n = heroes.length;
    if (n < 2 || paused) return;
    const t = setInterval(() => {
      if (typeof document !== "undefined" && document.hidden) return;
      heroIdx = (heroIdx + 1) % n;
    }, 9000);
    return () => clearInterval(t);
  });

  const banner = (c: MediaCard) => img(c.bannerPath, c.bannerUrl);
  const logo = (c: MediaCard) => img(c.logoPath, c.logoUrl);
  const cover = (c: MediaCard) => img(c.coverPath, c.coverUrl);
  const epShort = (i: UpNextItem) => (i.epKey.startsWith("S") ? `SP ${i.number}` : `EP ${i.number}`);
  const ep2 = (i: UpNextItem) =>
    `${i.epKey.startsWith("S") ? "SP" : "EP"} ${String(i.number).padStart(2, "0")}`;
  const plain = (s: string | null) =>
    (s ?? "")
      .replace(/<br\s*\/?>/gi, " ")
      .replace(/<[^>]+>/g, "")
      .replace(/\(Source:[^)]*\)/gi, "")
      .replace(/\s+/g, " ")
      .trim();

  function playHero(h: Hero) {
    if (h.item) app.play(h.item.anilistId, h.item.epKey, h.item.path);
  }

  function onFocusOut(e: FocusEvent) {
    const next = e.relatedTarget as Node | null;
    if (!next || !(e.currentTarget as HTMLElement).contains(next)) focused = false;
  }

  // Rows ------------------------------------------------------------------
  const all = $derived(groupCards(scoped, app.groupSeasons));

  /** Top 10: highest AniList average score among your shows (ties → more episodes watched). */
  const top10 = $derived.by<LibraryItem[]>(() => {
    if (all.length < 6) return [];
    return all
      .filter((c) => c.averageScore)
      .sort((a, b) => (b.averageScore ?? 0) - (a.averageScore ?? 0) || b.watchedCount - a.watchedCount)
      .slice(0, 10);
  });

  const recentlyAdded = $derived([...all].sort((a, b) => b.addedAt - a.addedAt).slice(0, 16));

  /** Up to 4 rows for the library's most common genres that have a Japanese label and ≥ 4 shows. */
  const SKIP_GENRES = new Set(["Hentai", "Ecchi"]);
  const genreRows = $derived.by(() => {
    const count = new Map<string, number>();
    for (const c of all) for (const g of c.genres) count.set(g, (count.get(g) ?? 0) + 1);
    return [...count.entries()]
      .filter(([g, n]) => n >= 4 && jaGenre(g) && !SKIP_GENRES.has(g))
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .slice(0, 4)
      .map(([genre]) => ({
        genre,
        items: all
          .filter((c) => c.genres.includes(genre))
          .sort((a, b) => (b.averageScore ?? 0) - (a.averageScore ?? 0))
          .slice(0, 20),
      }));
  });

  const modalId = (m: LibraryItem) => m.linkId ?? m.anilistId;
  const asCard = (m: LibraryItem): MediaCard => ({ ...m, anilistId: modalId(m) });
</script>

{#if hero}
  {@const h = hero}
  {@const c = h.card}
  {@const launching = h.item ? app.isLaunching(h.item.anilistId, h.item.epKey) : false}
  {@const playing = h.item ? app.isPlaying(h.item.anilistId, h.item.epKey) : false}
  {@const synopsis = plain(c.description)}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <section
    class="hero"
    aria-roledescription="carousel"
    aria-label="Featured"
    style:--glow={c.coverColor || "var(--neon-violet)"}
    onmouseenter={() => (hovered = true)}
    onmouseleave={() => (hovered = false)}
    onfocusin={() => (focused = true)}
    onfocusout={onFocusOut}
  >
    {#key c.anilistId}
      <div class="hero-art" class:from-cover={!banner(c)}>
        {#if banner(c)}
          <img src={banner(c)} alt="" />
        {:else if cover(c)}
          <img class="blurred" src={cover(c)} alt="" />
        {/if}
      </div>
      {#if !banner(c) && cover(c)}
        <img class="hero-poster" src={cover(c)} alt="" />
      {/if}
    {/key}
    <div class="hero-glow" aria-hidden="true"></div>
    <div class="hero-shade" aria-hidden="true"></div>

    {#key c.anilistId}
      <div class="hero-body">
        {#if c.titleNative}
          <Ja text={c.titleNative} class="tategaki hero-tategaki" />
        {/if}
        <div class="hero-text">
          <p class="hero-kicker">
            {#if c.titleNative}<Ja text={c.titleNative} class="neon-text native" />{/if}
            <span class="tn-label">{h.kicker}</span>
          </p>
          {#if logo(c)}
            <a href={`/anime/${c.anilistId}`} class="hero-logo-link" title={displayTitle(c)}>
              <img class="hero-logo" src={logo(c)} alt={displayTitle(c)} />
            </a>
          {:else}
            <h2 class="hero-h">
              <a class="hero-title" href={`/anime/${c.anilistId}`}>{displayTitle(c)}</a>
            </h2>
          {/if}

          <div class="hero-meta">
            {#if c.averageScore}
              <span class="score-pill" aria-label={`Score ${score10(c.averageScore)} out of 10`}>
                <Icon name="star" size={12} fill />
                {score10(c.averageScore)}
              </span>
            {/if}
            {#if c.season || c.seasonYear}
              <span><Ja text={jaSeason(c.season, c.seasonYear)} label={seasonLabel(c.season, c.seasonYear)} essential /></span>
            {/if}
            {#if c.format}
              <span class="fmt"><Ja text={jaFormat(c.format)} label={formatLabel(c.format)} essential /></span>
            {/if}
            {#if c.episodes && c.format !== "MOVIE"}
              <span><Ja text={jaEpisodeCount(c.episodes)} label={`${c.episodes} episodes`} essential /></span>
            {/if}
            {#each c.genres.slice(0, 2) as g (g)}
              <span class="genre-tag">{g}</span>
            {/each}
          </div>

          {#if h.item}
            <p class="hero-ep">
              <strong>{h.item.epKey.startsWith("S") ? `Special ${h.item.number}` : `Episode ${h.item.number}`}</strong>
              {#if h.item.episodeTitle}<span> · {h.item.episodeTitle}</span>{/if}
            </p>
          {/if}
          {#if synopsis}
            <p class="hero-desc">{synopsis}</p>
          {/if}

          <div class="hero-actions">
            {#if h.item}
              <button
                class="btn btn-cta btn-lg"
                class:launching
                disabled={launching}
                onclick={() => playHero(h)}
                id="home-resume"
              >
                {#if launching}
                  <span class="spinner"></span> Launching…
                {:else if playing}
                  <span class="eq-bars"><span class="eq-bar"></span><span class="eq-bar"></span><span class="eq-bar"></span></span>
                  Now playing
                {:else}
                  <Icon name="play" size={16} fill />
                  {h.item.position > 0 ? `Resume ${ep2(h.item)} · ${clock(h.item.position)}` : `Play ${ep2(h.item)}`}
                {/if}
              </button>
            {/if}
            <button
              class="btn btn-lg btn-pill glass more-info"
              onclick={() => detailsModal.open(c.anilistId)}
              id="home-details"
            >
              <Icon name="info" size={16} /> More Info
            </button>
          </div>
        </div>
      </div>
    {/key}

    {#if heroes.length > 1}
      <aside class="spotlight" aria-label="Spotlight">
        <p class="tn-label spot-label">Spotlight<Ja text={` · ${JA.rows.spotlight}`} /></p>
        <ul>
          {#each heroes as item, i (item.card.anilistId)}
            <li>
              <button
                class="spot"
                class:on={i === current}
                aria-current={i === current ? "true" : undefined}
                aria-label={`Feature ${displayTitle(item.card)}`}
                onclick={() => (heroIdx = i)}
              >
                {#if cover(item.card)}<img src={cover(item.card)} alt="" loading="lazy" decoding="async" />{/if}
              </button>
            </li>
          {/each}
        </ul>
      </aside>
    {/if}
  </section>
{/if}

<div class="rows" class:no-hero={!hero}>
  {#if cont.length}
    <Rail title="Continue Watching" ja={JA.rows.continueWatching}>
      {#each cont as i (i.anilistId)}
        <StillTile item={i} card={cardById.get(i.anilistId) ?? null} />
      {/each}
    </Rail>
  {/if}

  {#if newEps.length}
    <Rail title="New Episodes" ja={JA.rows.newEpisodes}>
      {#each newEps as i (i.anilistId + i.epKey)}
        {@const card = cardById.get(i.anilistId)}
        {#if card}
          <PosterTile
            m={card}
            item={i}
            badge={`${epShort(i)}${i.newCount > 1 ? ` +${i.newCount - 1}` : ""}`}
            sub={i.episodeTitle ?? `Episode ${i.number}`}
          />
        {/if}
      {/each}
    </Rail>
  {/if}

  {#if top10.length}
    <Rail title="Top 10 in Your Library" ja={JA.rows.top10}>
      {#each top10 as m, i (m.anilistId)}
        <PosterTile m={asCard(m)} rank={i + 1} />
      {/each}
    </Rail>
  {/if}

  {#if recentlyAdded.length && all.length >= 6}
    <Rail title="Recently Added" ja={JA.rows.recentlyAdded}>
      {#each recentlyAdded as m (m.anilistId)}
        <PosterTile m={asCard(m)} />
      {/each}
    </Rail>
  {/if}

  {#each genreRows as row (row.genre)}
    <Rail title={row.genre} ja={jaGenre(row.genre)}>
      {#each row.items as m (m.anilistId)}
        <PosterTile m={asCard(m)} />
      {/each}
    </Rail>
  {/each}
</div>

<style>
  /* Hero (billboard) ------------------------------------------------------- */
  .hero {
    position: relative;
    height: 80vh;
    min-height: 520px;
    max-height: 980px;
    overflow: hidden;
    isolation: isolate;
  }
  .hero-art {
    position: absolute;
    inset: 0;
    z-index: -3;
    background: var(--surface);
    animation: fade-in 500ms var(--ease) both;
  }
  .hero-art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 25%;
    /* Slightly darken the key art so white text always sits on a dark field. */
    filter: brightness(0.78) saturate(1.05);
  }
  /* Cover-only fallback: blurred, darkened wash + a sharp poster on top. */
  .hero-art img.blurred {
    filter: blur(20px) brightness(0.45) saturate(1.18);
    transform: scale(1.12);
  }
  .hero-poster {
    position: absolute;
    right: calc(var(--gutter) + 110px);
    bottom: clamp(48px, 9vh, 96px);
    z-index: 0;
    width: clamp(150px, 15vw, 240px);
    aspect-ratio: 2 / 3;
    object-fit: cover;
    border-radius: var(--r-md);
    box-shadow:
      0 30px 60px -20px rgba(0, 0, 0, 0.8),
      0 0 40px -8px color-mix(in srgb, var(--glow) 55%, transparent);
    animation: fade-up 560ms var(--ease) both;
  }
  .hero-glow {
    position: absolute;
    inset: 0;
    z-index: -2;
    background: radial-gradient(55% 60% at 14% 82%, color-mix(in srgb, var(--glow) 18%, transparent), transparent 70%);
    pointer-events: none;
  }
  .hero-shade {
    position: absolute;
    inset: 0;
    z-index: -1;
    background:
      linear-gradient(90deg, var(--bg) 0%, color-mix(in srgb, var(--bg) 85%, transparent) 28%, color-mix(in srgb, var(--bg) 40%, transparent) 60%, color-mix(in srgb, var(--bg) 10%, transparent) 85%),
      linear-gradient(0deg, var(--bg) 0%, color-mix(in srgb, var(--bg) 75%, transparent) 20%, transparent 55%),
      linear-gradient(180deg, color-mix(in srgb, var(--bg) 65%, transparent) 0%, transparent 24%);
  }
  .hero-body {
    position: absolute;
    left: var(--gutter);
    bottom: clamp(48px, 9vh, 96px);
    width: min(680px, 58%);
    display: flex;
    align-items: flex-end;
    gap: 22px;
    animation: fade-up 560ms var(--ease) both;
  }
  .hero-body :global(.hero-tategaki) {
    align-self: stretch;
    max-height: 300px;
    overflow: hidden;
    padding-right: 14px;
    border-right: 1px solid color-mix(in srgb, var(--neon-violet) 35%, transparent);
    color: color-mix(in srgb, var(--neon-violet) 50%, transparent);
    font-size: 13px;
    font-weight: 600;
  }
  .hero-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
  }
  .hero-kicker {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 12px;
  }
  .hero-kicker :global(.native) {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .hero-kicker .tn-label {
    color: var(--text-2);
  }
  .hero-logo-link {
    display: block;
  }
  .hero-logo {
    max-width: min(440px, 100%);
    max-height: 150px;
    object-fit: contain;
    object-position: left bottom;
    filter: drop-shadow(0 6px 24px rgba(0, 0, 0, 0.6));
  }
  .hero-h {
    margin: 0;
  }
  .hero-title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-family: var(--font-display);
    font-size: clamp(36px, 4.4vw, 64px);
    font-weight: 800;
    letter-spacing: -0.035em;
    line-height: 1.02;
    color: var(--text);
    text-shadow: 0 6px 32px color-mix(in srgb, var(--bg) 70%, transparent);
  }
  .hero-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text-2);
  }
  .score-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--neon-pink) 16%, transparent);
    color: var(--neon-pink);
    font-family: var(--font-label);
    font-weight: 700;
    box-shadow: 0 0 18px -2px color-mix(in srgb, var(--neon-pink) 55%, transparent);
  }
  .fmt {
    padding: 1px 7px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--text) 10%, transparent);
    color: var(--text);
    font-size: 12px;
  }
  .genre-tag {
    padding: 1px 8px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--text) 7%, transparent);
    font-family: var(--font-label);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .hero-desc {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    max-width: 100%;
    font-size: 15px;
    line-height: 1.55;
    color: var(--text-2);
  }
  .hero-ep {
    font-size: 15px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }
  .hero-ep strong {
    color: var(--text);
  }
  .hero-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 8px;
  }
  .hero-actions .btn-lg {
    border-radius: var(--r-pill);
    padding: 0 24px;
  }
  .more-info {
    color: var(--text);
  }
  .more-info:hover {
    background: color-mix(in srgb, var(--text) 14%, transparent);
    box-shadow: var(--neon-glow);
  }

  /* Spotlight rail -------------------------------------------------------- */
  .spotlight {
    position: absolute;
    right: var(--gutter);
    bottom: clamp(48px, 9vh, 96px);
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 10px;
  }
  .spot-label {
    margin: 0;
  }
  .spotlight ul {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .spot {
    display: block;
    width: 52px;
    aspect-ratio: 2 / 3;
    padding: 0;
    border: 0;
    border-radius: 8px;
    overflow: hidden;
    background: var(--surface-2);
    cursor: pointer;
    opacity: 0.55;
    box-shadow: var(--shadow-sm);
    transition:
      opacity var(--t-fast) var(--ease),
      transform var(--t-med) var(--ease),
      box-shadow var(--t-med) var(--ease);
  }
  .spot img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .spot:hover,
  .spot:focus-visible {
    opacity: 0.9;
  }
  .spot.on {
    opacity: 1;
    transform: scale(1.12);
    box-shadow:
      0 0 0 2px var(--neon-violet),
      var(--neon-glow);
  }

  /* Rows ------------------------------------------------------------------ */
  .rows {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-top: -40px;
    padding-bottom: 8px;
  }
  .rows.no-hero {
    margin-top: 0;
    padding-top: calc(var(--nav-h) + var(--nav-inset) + 16px);
  }

  @media (max-width: 1100px) {
    .hero-body :global(.hero-tategaki) {
      display: none;
    }
  }
  @media (max-width: 1000px) {
    .hero-body {
      width: calc(100% - 2 * var(--gutter) - 80px);
    }
  }
  @media (max-width: 720px) {
    .spotlight {
      display: none;
    }
    .hero-body {
      width: calc(100% - 2 * var(--gutter));
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spot.on {
      transform: none;
    }
  }
</style>
