<script lang="ts">
  import { api, img, type MediaCard, type UpNextItem } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { clock, displayTitle } from "$lib/format";
  import Icon from "./Icon.svelte";
  import Rail from "./Rail.svelte";

  let { scoped }: { scoped: MediaCard[] } = $props();

  let upNext = $state<UpNextItem[]>([]);
  let fresh = $state<UpNextItem[]>([]);
  let heroIdx = $state(0);
  let paused = $state(false);

  $effect(() => {
    void app.version;
    Promise.all([api.getUpNext(14), api.getNewEpisodes(14)])
      .then(([u, n]) => {
        upNext = u;
        fresh = n;
      })
      .catch((e) => console.warn("[kura] home panels", e));
  });

  const ids = $derived(new Set(scoped.map((c) => c.anilistId)));
  const cardById = $derived(new Map(scoped.map((c) => [c.anilistId, c])));
  const cont = $derived(upNext.filter((i) => ids.has(i.anilistId)));
  const newEps = $derived(fresh.filter((i) => ids.has(i.anilistId)));

  type Hero = { card: MediaCard; item: UpNextItem | null; kicker: string };

  /** Hero rotation: what you're watching first, then new episodes, then recently added shows with artwork. */
  const heroes = $derived.by<Hero[]>(() => {
    const out: Hero[] = [];
    const seen = new Set<number>();
    const push = (card: MediaCard | undefined, item: UpNextItem | null, kicker: string) => {
      if (!card || seen.has(card.anilistId) || out.length >= 6) return;
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
    return out;
  });

  const hero = $derived(heroes.length ? heroes[heroIdx % heroes.length] : null);

  $effect(() => {
    const n = heroes.length;
    if (n < 2 || paused) return;
    const t = setInterval(() => (heroIdx = (heroIdx + 1) % n), 9000);
    return () => clearInterval(t);
  });

  const backdrop = (c: MediaCard) => img(c.bannerPath, c.bannerUrl) ?? img(c.coverPath, c.coverUrl);
  const logo = (c: MediaCard) => img(c.logoPath, c.logoUrl);
  const thumb = (i: UpNextItem) =>
    img(i.thumbPath, i.thumbUrl) ?? img(i.bannerPath, i.bannerUrl) ?? img(i.coverPath, i.coverUrl);
  const cover = (i: UpNextItem) => img(i.coverPath, i.coverUrl);
  const pct = (i: UpNextItem) => (i.duration > 0 ? Math.min(100, (i.position / i.duration) * 100) : 0);
  const epShort = (i: UpNextItem) => (i.epKey.startsWith("S") ? `SP ${i.number}` : `EP ${i.number}`);
  const plain = (s: string | null) =>
    (s ?? "")
      .replace(/<br\s*\/?>/gi, " ")
      .replace(/<[^>]+>/g, "")
      .replace(/\(Source:[^)]*\)/gi, "")
      .replace(/\s+/g, " ")
      .trim();
  const minutesLeft = (i: UpNextItem) => Math.max(1, Math.round((i.duration - i.position) / 60));

  function playHero(h: Hero) {
    if (h.item) app.play(h.item.anilistId, h.item.epKey, h.item.path);
  }
</script>

{#if hero}
  {@const h = hero}
  {@const c = h.card}
  {@const launching = h.item ? app.isLaunching(h.item.anilistId, h.item.epKey) : false}
  {@const playing = h.item ? app.isPlaying(h.item.anilistId, h.item.epKey) : false}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <section
    class="hero"
    aria-label="Featured"
    onmouseenter={() => (paused = true)}
    onmouseleave={() => (paused = false)}
  >
    {#key c.anilistId}
      <div class="hero-art">
        {#if backdrop(c)}<img src={backdrop(c)} alt="" />{/if}
      </div>
    {/key}
    <div class="hero-shade"></div>

    {#key c.anilistId}
      <div class="hero-body">
        <span class="hero-kicker">{h.kicker}</span>
        {#if logo(c)}
          <a href={`/anime/${c.anilistId}`} class="hero-logo-link" title={displayTitle(c)}>
            <img class="hero-logo" src={logo(c)} alt={displayTitle(c)} />
          </a>
        {:else}
          <a class="hero-title" href={`/anime/${c.anilistId}`}>{displayTitle(c)}</a>
        {/if}

        <div class="hero-meta">
          {#if c.averageScore}<span class="score">{c.averageScore}%</span>{/if}
          {#if c.seasonYear}<span>{c.seasonYear}</span>{/if}
          {#if c.format}<span>{c.format === "TV" ? "TV" : c.format.replace("_", " ")}{c.episodes ? ` · ${c.episodes} eps` : ""}</span>{/if}
          {#if c.genres.length}<span>{c.genres.slice(0, 3).join(" · ")}</span>{/if}
        </div>

        {#if h.item}
          <p class="hero-ep">
            <strong>{h.item.epKey.startsWith("S") ? `Special ${h.item.number}` : `Episode ${h.item.number}`}</strong>
            {#if h.item.episodeTitle}<span> · {h.item.episodeTitle}</span>{/if}
          </p>
        {:else if c.description}
          <p class="hero-desc">{plain(c.description)}</p>
        {/if}

        <div class="hero-actions">
          {#if h.item}
            <button
              class="btn btn-primary btn-lg"
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
                {h.item.position > 0 ? `Resume · ${clock(h.item.position)}` : `Play · Ep ${h.item.number}`}
              {/if}
            </button>
          {/if}
          <a class="btn btn-glass btn-lg" href={`/anime/${c.anilistId}`} id="home-details">Details</a>
        </div>
      </div>
    {/key}

    {#if heroes.length > 1}
      <div class="hero-dots" role="tablist" aria-label="Featured shows">
        {#each heroes as item, i (item.card.anilistId)}
          <button
            class="dot"
            class:on={i === heroIdx % heroes.length}
            role="tab"
            aria-selected={i === heroIdx % heroes.length}
            aria-label={displayTitle(item.card)}
            onclick={() => (heroIdx = i)}
          ></button>
        {/each}
      </div>
    {/if}
  </section>
{/if}

<div class="rows">
  {#if cont.length}
    <Rail title="Continue Watching">
      {#each cont as i (i.anilistId)}
        {@const launching = app.isLaunching(i.anilistId, i.epKey)}
        <article class="wide-card">
          <button
            class="wide-art"
            onclick={() => app.play(i.anilistId, i.epKey, i.path)}
            disabled={launching}
            title={`Play ${displayTitle(i)} · Episode ${i.number}`}
          >
            {#if thumb(i)}<img src={thumb(i)} alt="" loading="lazy" />{/if}
            <span class="wide-shade"></span>
            <span class="ep-tag">{epShort(i)}</span>
            <span class="play-glyph">
              {#if launching}<span class="spinner"></span>{:else}<Icon name="play" size={18} fill />{/if}
            </span>
            {#if i.position > 0}
              <span class="prog"><span style:width={`${pct(i)}%`}></span></span>
            {/if}
          </button>
          <a class="card-title" href={`/anime/${i.anilistId}`} title={displayTitle(i)}>{displayTitle(i)}</a>
          <span class="card-sub">
            {#if i.position > 0 && i.duration > 0}{minutesLeft(i)} min left{:else}{i.episodeTitle ?? `Episode ${i.number}`}{/if}
          </span>
        </article>
      {/each}
    </Rail>
  {/if}

  {#if newEps.length}
    <Rail title="New Episodes">
      {#each newEps as i (i.anilistId + i.epKey)}
        <article class="poster-card">
          <button
            class="poster-art"
            onclick={() => app.play(i.anilistId, i.epKey, i.path)}
            title={`Play ${displayTitle(i)} · Episode ${i.number}`}
          >
            {#if cover(i)}<img src={cover(i)} alt="" loading="lazy" />{/if}
            <span class="new-tag">{epShort(i)}{i.newCount > 1 ? ` +${i.newCount - 1}` : ""}</span>
            <span class="play-glyph"><Icon name="play" size={18} fill /></span>
          </button>
          <a class="card-title" href={`/anime/${i.anilistId}`} title={displayTitle(i)}>{displayTitle(i)}</a>
          <span class="card-sub">{i.episodeTitle ?? `Episode ${i.number}`}</span>
        </article>
      {/each}
    </Rail>
  {/if}
</div>

<style>
  /* Hero ------------------------------------------------------------------ */
  .hero {
    position: relative;
    height: clamp(380px, 58vh, 560px);
    overflow: hidden;
    isolation: isolate;
  }
  .hero-art {
    position: absolute;
    inset: 0;
    z-index: -2;
    background: var(--surface);
    animation: fade-in 700ms var(--ease) both;
  }
  .hero-art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 30%;
    animation: hero-zoom 12s ease-out both;
  }
  @keyframes hero-zoom {
    from {
      transform: scale(1.06);
    }
    to {
      transform: scale(1);
    }
  }
  .hero-shade {
    position: absolute;
    inset: 0;
    z-index: -1;
    background:
      linear-gradient(90deg, var(--bg) 0%, rgba(8, 8, 11, 0.86) 30%, rgba(8, 8, 11, 0.2) 65%, rgba(8, 8, 11, 0) 100%),
      linear-gradient(0deg, var(--bg) 0%, rgba(8, 8, 11, 0) 45%),
      linear-gradient(180deg, rgba(8, 8, 11, 0.5) 0%, rgba(8, 8, 11, 0) 22%);
  }
  .hero-body {
    position: absolute;
    left: var(--page-x);
    bottom: 56px;
    width: min(620px, 55%);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
    animation: fade-up 520ms var(--ease) both;
  }
  .hero-kicker {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: rgba(243, 243, 245, 0.6);
  }
  .hero-logo-link {
    display: block;
  }
  .hero-logo {
    max-width: min(420px, 100%);
    max-height: 140px;
    object-fit: contain;
    object-position: left bottom;
    filter: drop-shadow(0 6px 24px rgba(0, 0, 0, 0.6));
  }
  .hero-title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-family: var(--font-display);
    font-size: clamp(32px, 3.6vw, 52px);
    font-weight: 800;
    letter-spacing: -0.03em;
    line-height: 1.04;
    color: #fff;
    text-shadow: 0 6px 28px rgba(0, 0, 0, 0.6);
  }
  .hero-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    font-size: 13.5px;
    color: rgba(243, 243, 245, 0.72);
  }
  .hero-meta .score {
    color: var(--ok);
    font-weight: 700;
  }
  .hero-desc {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-size: 15px;
    line-height: 1.55;
    color: rgba(243, 243, 245, 0.72);
  }
  .hero-ep {
    font-size: 15px;
    color: rgba(243, 243, 245, 0.72);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }
  .hero-ep strong {
    color: #fff;
  }
  .hero-actions {
    display: flex;
    gap: 10px;
    margin-top: 10px;
  }
  .hero-dots {
    position: absolute;
    right: var(--page-x);
    bottom: 30px;
    display: flex;
    gap: 6px;
  }
  .dot {
    width: 7px;
    height: 7px;
    padding: 0;
    border: none;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.35);
    cursor: pointer;
    transition:
      width var(--t-med) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .dot.on {
    width: 24px;
    background: #fff;
  }

  /* Rows ------------------------------------------------------------------ */
  .rows {
    display: flex;
    flex-direction: column;
    gap: 34px;
    padding: 8px 0;
  }

  .wide-card {
    width: 290px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .poster-card {
    width: 158px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .wide-art,
  .poster-art {
    position: relative;
    display: block;
    width: 100%;
    padding: 0;
    border: none;
    border-radius: var(--r-md);
    overflow: hidden;
    background: var(--surface-2);
    cursor: pointer;
    margin-bottom: 6px;
    transition: transform var(--t-med) var(--ease);
  }
  .wide-art {
    aspect-ratio: 16 / 9;
  }
  .poster-art {
    aspect-ratio: 2 / 3;
  }
  .wide-art:hover,
  .poster-art:hover {
    transform: translateY(-3px);
  }
  .wide-art img,
  .poster-art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .wide-shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(0deg, rgba(0, 0, 0, 0.45) 0%, transparent 50%);
  }
  .ep-tag,
  .new-tag {
    position: absolute;
    top: 8px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.02em;
    padding: 2px 7px;
    border-radius: 6px;
  }
  .ep-tag {
    left: 8px;
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
  }
  .new-tag {
    right: 8px;
    background: var(--coral);
    color: #fff;
  }
  .play-glyph {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 44px;
    height: 44px;
    margin: -22px 0 0 -22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: rgba(255, 255, 255, 0.92);
    color: #08080b;
    opacity: 0;
    transform: scale(0.85);
    transition:
      opacity var(--t-fast) var(--ease),
      transform var(--t-fast) var(--ease);
  }
  .play-glyph :global(svg) {
    margin-left: 2px;
  }
  .wide-art:hover .play-glyph,
  .poster-art:hover .play-glyph,
  .wide-art:disabled .play-glyph {
    opacity: 1;
    transform: none;
  }
  .prog {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: rgba(255, 255, 255, 0.2);
  }
  .prog span {
    display: block;
    height: 100%;
    background: var(--coral);
  }
  .card-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-title:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .card-sub {
    font-size: 12px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  @media (max-width: 1000px) {
    .hero-body {
      width: calc(100% - 2 * var(--page-x));
    }
  }
</style>
