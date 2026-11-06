<script lang="ts">
  import type { MediaCard } from "$lib/api";
  import { img } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, formatLabel, franchiseBadge, totalEpisodes, type LibraryItem } from "$lib/format";
  import Icon from "./Icon.svelte";

  let { m, index = 0 }: { m: MediaCard | LibraryItem; index?: number } = $props();

  const item = $derived("members" in m ? m : null);
  const badge = $derived(item ? franchiseBadge(item) : null);
  const href = $derived(`/anime/${item?.linkId ?? m.anilistId}`);
  const years = $derived(
    item?.yearEnd && m.seasonYear && item.yearEnd !== m.seasonYear ? `${m.seasonYear}–${item.yearEnd}` : m.seasonYear,
  );
  const src = $derived(img(m.coverPath, m.coverUrl));
  const total = $derived(totalEpisodes(m));
  const watchedPct = $derived(total ? Math.min(100, (m.watchedCount / total) * 100) : 0);
  const complete = $derived(total !== null && m.watchedCount >= total && total > 0);
  const missing = $derived(total !== null && m.ownedCount < total && m.status === "FINISHED");
  let loaded = $state(false);

  function imgAction(node: HTMLImageElement) {
    if (node.complete && node.naturalWidth > 0) {
      loaded = true;
    }
  }
</script>

<a
  class="poster-card"
  class:stack={!!badge}
  {href}
  style:--i={Math.min(index, 24)}
  style:--cover-accent={m.coverColor || "var(--coral)"}
  id={`card-${m.anilistId}`}
>
  <div class="art">
    {#if src}
      <img {src} alt="" loading="lazy" decoding="async" class:loaded onload={() => (loaded = true)} use:imgAction />
    {:else}
      <img
        class="kanji-fallback"
        src={app.theme === "manga" ? "/brand/kanji.png" : "/brand/kanji-white.png"}
        alt=""
      />
    {/if}

    <div class="top">
      {#if badge}
        <span class="badge seasons"><Icon name="library" size={10} stroke={2.5} /> {badge}</span>
      {:else if m.format && m.format !== "TV"}
        <span class="badge tag">{formatLabel(m.format)}</span>
      {/if}
      {#if m.needsReview}
        <span class="badge badge-warn" title="Match may be wrong — check it">
          <Icon name="alert" size={10} stroke={2.5} />
        </span>
      {/if}
      {#if m.status === "RELEASING"}
        <span class="badge airing-pill" title="Currently airing"><span class="airing-dot"></span></span>
      {/if}
    </div>

    {#if complete}
      <span class="done-badge" title="Completed"><Icon name="check" size={12} stroke={3} /></span>
    {/if}

    {#if m.watchedCount > 0 && !complete}
      <div class="progress"><div style:width={`${watchedPct}%`}></div></div>
    {/if}
  </div>
  <div class="meta">
    <h3 title={displayTitle(m)}>{displayTitle(m)}</h3>
    <p class:warn={missing}>
      {#if complete}
        Completed
      {:else if m.watchedCount > 0}
        {m.watchedCount} / {total ?? m.ownedCount} {m.format === "MOVIE" ? "" : "eps"}
      {:else}
        {total ?? m.ownedCount} {m.format === "MOVIE" ? "film" : "eps"}
      {/if}
      {#if years} · {years}{/if}
    </p>
  </div>
</a>

<style>
  .poster-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    content-visibility: auto;
    contain-intrinsic-size: 160px 280px;
    outline: none;
  }

  .art {
    position: relative;
    z-index: 1;
    isolation: isolate;
    aspect-ratio: 2 / 3;
    border-radius: var(--r-md);
    overflow: hidden;
    background: var(--surface-2);
    transition:
      transform 240ms var(--ease),
      box-shadow 240ms var(--ease);
  }

  .poster-card:hover .art,
  .poster-card:focus-visible .art {
    transform: translateY(-4px) scale(1.02);
    box-shadow: 0 18px 36px -12px rgba(0, 0, 0, 0.8);
  }

  .poster-card:focus-visible .art {
    outline: 2px solid var(--text);
    outline-offset: 2px;
  }

  .poster-card.stack {
    position: relative;
  }

  .poster-card.stack::before {
    content: "";
    position: absolute;
    z-index: 0;
    top: -4px;
    left: 7px;
    right: 7px;
    aspect-ratio: 2 / 3;
    border-radius: var(--r-md);
    background: var(--surface-3);
    opacity: 0.6;
    transition: transform 240ms var(--ease);
  }

  .poster-card.stack:hover::before {
    transform: translateY(-7px);
  }

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    transition: opacity 360ms var(--ease);
  }

  img.loaded {
    opacity: 1;
  }

  .kanji-fallback {
    position: absolute;
    inset: 25%;
    width: 50%;
    height: auto;
    opacity: 0.1;
  }

  .top {
    position: absolute;
    top: 8px;
    left: 8px;
    right: 8px;
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
    align-items: center;
    z-index: 2;
  }

  .top .badge {
    height: 20px;
    padding: 0 7px;
    font-size: 10px;
    border-radius: 6px;
  }

  .seasons {
    background: rgba(10, 10, 14, 0.84);
    color: #fff;
  }

  .tag {
    background: rgba(10, 10, 14, 0.84);
    color: #f3f3f5;
  }

  .airing-pill {
    width: 20px;
    padding: 0 !important;
    justify-content: center;
    background: rgba(10, 10, 14, 0.84);
    margin-left: auto;
  }

  .airing-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #34d399;
    box-shadow: 0 0 8px #34d399;
  }

  .done-badge {
    position: absolute;
    right: 8px;
    bottom: 8px;
    z-index: 2;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.65);
    color: #34d399;
  }

  .progress {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 3;
    height: 3px;
    background: rgba(0, 0, 0, 0.45);
  }

  .progress div {
    height: 100%;
    background: var(--coral);
  }

  .meta {
    padding: 0 1px;
    min-width: 0;
  }

  h3 {
    font-family: var(--font-display);
    font-size: 13.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    line-height: 1.32;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta p {
    margin-top: 2px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }

  .meta p.warn {
    color: color-mix(in srgb, var(--amber) 75%, var(--text-3));
  }
</style>
