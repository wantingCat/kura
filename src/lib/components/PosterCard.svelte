<script lang="ts">
  import type { MediaCard } from "$lib/api";
  import { img } from "$lib/api";
  import { detailsModal } from "$lib/detailsModal.svelte";
  import { displayTitle, formatLabel, franchiseBadge, totalEpisodes, type LibraryItem } from "$lib/format";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";

  let { m, index = 0 }: { m: MediaCard | LibraryItem; index?: number } = $props();

  const item = $derived("members" in m ? m : null);
  const badge = $derived(item ? franchiseBadge(item) : null);
  const targetId = $derived(item?.linkId ?? m.anilistId);
  const href = $derived(`/anime/${targetId}`);
  const title = $derived(displayTitle(m));
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

<article
  class="poster-card"
  class:stack={!!badge}
  style:--i={Math.min(index, 24)}
  style:--glow={m.coverColor || "var(--neon-violet)"}
  id={`card-${m.anilistId}`}
>
  <div class="art">
    <button class="hit" onclick={() => detailsModal.open(targetId)} aria-label={`${title} — more info`}>
      {#if src}
        <img {src} alt="" loading="lazy" decoding="async" class:loaded onload={() => (loaded = true)} use:imgAction />
      {:else}
        <img
          class="kanji-fallback"
          src="/brand/kanji-white.png"
          alt=""
        />
      {/if}
    </button>

    <div class="top">
      {#if badge}
        <span class="badge seasons"><Icon name="library" size={10} stroke={2.5} /> {badge}</span>
      {:else if m.format && m.format !== "TV"}
        <span class="badge tag">{formatLabel(m.format)}</span>
      {/if}
      {#if m.needsReview}
        <span class="badge badge-warn" title="Match may be wrong — check it">
          <Icon name="alert" size={10} stroke={2.5} />
          <span class="sr-only">Match may be wrong</span>
        </span>
      {/if}
      {#if m.status === "RELEASING"}
        <span class="badge airing-pill" title="Currently airing"
          ><span class="airing-dot"></span><span class="sr-only">Currently airing</span></span
        >
      {/if}
    </div>

    {#if complete}
      <span class="done-badge" title="Completed"><Icon name="check" size={12} stroke={3} /></span>
    {/if}

    {#if m.watchedCount > 0 && !complete}
      <div class="neon-progress progress" aria-hidden="true"><span style:width={`${watchedPct}%`}></span></div>
    {/if}
  </div>
  <div class="meta">
    <h3 {title}><a {href}>{title}</a></h3>
    {#if m.titleNative && m.titleNative !== title}
      <Ja text={m.titleNative} block class="native" />
    {/if}
    <p class:warn={missing} title={missing ? `${m.ownedCount} of ${total} episodes in local library` : undefined}>
      {#if complete}
        Completed
      {:else if m.watchedCount > 0}
        {m.watchedCount} / {total ?? m.ownedCount} {m.format === "MOVIE" ? "" : "eps"}
      {:else if missing}
        {m.ownedCount} / {total} {m.format === "MOVIE" ? "film" : "eps"}
      {:else}
        {total ?? m.ownedCount} {m.format === "MOVIE" ? "film" : "eps"}
      {/if}
      {#if years} · {years}{/if}
    </p>
  </div>
</article>

<style>
  .poster-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    content-visibility: auto;
    contain-intrinsic-size: 160px 300px;
  }

  .art {
    position: relative;
    z-index: 1;
    isolation: isolate;
    aspect-ratio: 2 / 3;
    border-radius: var(--r-md);
    overflow: hidden;
    background: var(--surface-2);
    box-shadow: var(--shadow-sm);
    transition:
      transform var(--t-med) var(--ease),
      box-shadow var(--t-med) var(--ease);
  }

  .poster-card:hover .art,
  .poster-card:focus-within .art {
    transform: translateY(-6px) scale(1.03);
    box-shadow:
      0 18px 40px -14px color-mix(in srgb, var(--glow) 70%, transparent),
      0 0 26px -4px color-mix(in srgb, var(--glow) 45%, transparent);
  }

  .hit {
    position: absolute;
    inset: 0;
    width: 100%;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    border-radius: inherit;
  }
  .hit:focus-visible {
    outline-offset: -2px;
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
    transition: transform var(--t-med) var(--ease);
  }

  .poster-card.stack:hover::before {
    transform: translateY(-9px);
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
    pointer-events: none;
  }

  .top .badge {
    height: 20px;
    padding: 0 7px;
    font-size: 10px;
    border-radius: var(--r-pill);
  }

  .seasons,
  .tag {
    background: rgba(8, 9, 16, 0.78);
    color: #f4f2ff;
  }

  .airing-pill {
    width: 20px;
    padding: 0 !important;
    justify-content: center;
    background: rgba(8, 9, 16, 0.78);
    margin-left: auto;
  }

  .airing-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
    box-shadow: 0 0 8px var(--ok);
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
    background: rgba(8, 9, 16, 0.7);
    color: var(--ok);
    pointer-events: none;
  }

  .progress {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 3;
    border-radius: 0;
    pointer-events: none;
  }

  .meta {
    padding: 0 2px;
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
  h3 a:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  .meta :global(.native) {
    font-size: 11.5px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .meta p {
    margin-top: 2px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }

  .meta p.warn {
    color: color-mix(in srgb, var(--amber) 80%, var(--text-2));
  }

  @media (prefers-reduced-motion: reduce) {
    .poster-card:hover .art,
    .poster-card:focus-within .art,
    .poster-card.stack:hover::before {
      transform: none;
    }
  }
</style>
