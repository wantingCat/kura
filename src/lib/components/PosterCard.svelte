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
        <span class="badge seasons"><Icon name="library" size={11} stroke={2.5} /> {badge}</span>
      {:else if m.format && m.format !== "TV"}
        <span class="badge tag">{formatLabel(m.format)}</span>
      {/if}
      {#if m.needsReview}
        <span class="badge badge-warn" title="Match may be wrong — check it">
          <Icon name="alert" size={11} stroke={2.5} />
        </span>
      {/if}
      {#if m.status === "RELEASING"}
        <span class="badge badge-ok">Airing</span>
      {/if}
    </div>

    <div class="bottom">
      {#if complete}
        <span class="done"><Icon name="check" size={12} stroke={3} /> Watched</span>
      {:else}
        <span class="eps" class:warn={missing}>
          {m.ownedCount}{#if total}<span class="of">/{total}</span>{/if}
          <span class="lbl">{m.format === "MOVIE" ? "file" : "eps"}</span>
        </span>
      {/if}
    </div>

    {#if m.watchedCount > 0 && !complete}
      <div class="progress"><div style:width={`${watchedPct}%`}></div></div>
    {/if}
  </div>
  <div class="meta">
    <h3 title={displayTitle(m)}>{displayTitle(m)}</h3>
    <p>
      {#if years}{years}{/if}
      {#if years && m.genres.length} · {/if}
      {m.genres.slice(0, 2).join(", ")}
    </p>
  </div>
</a>

<style>
  .poster-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    animation: fade-up 420ms var(--ease) both;
    animation-delay: calc(var(--i) * 18ms);
    outline: none;
  }
  .art {
    position: relative;
    z-index: 1;
    isolation: isolate;
    -webkit-backface-visibility: hidden;
    backface-visibility: hidden;
    transform: translateZ(0);
    aspect-ratio: 2 / 3;
    border-radius: var(--r-md);
    overflow: hidden;
    border: var(--bw) solid var(--line);
    background: var(--surface-2);
    box-shadow: var(--sticker);
    transition:
      transform var(--t-med) var(--bounce),
      box-shadow var(--t-med) var(--ease);
  }
  .poster-card:hover .art,
  .poster-card:focus-visible .art {
    transform: translate(-3px, -3px) rotate(-0.6deg) translateZ(0);
    box-shadow: 6px 6px 0 var(--coral);
  }
  /* Grouped franchise: a second "sheet" peeking out behind the cover, like a stack of volumes. */
  .poster-card.stack {
    position: relative;
  }
  .poster-card.stack::before {
    content: "";
    position: absolute;
    z-index: 0;
    top: -6px;
    left: 7px;
    right: -7px;
    aspect-ratio: 2 / 3;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--surface-3);
    transform: rotate(2.2deg);
    transition: transform var(--t-med) var(--bounce);
  }
  .poster-card.stack:hover::before {
    transform: rotate(4deg) translate(3px, -1px);
  }
  .seasons {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--coral);
    color: var(--on-coral);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    transition: opacity 400ms var(--ease);
  }
  img.loaded {
    opacity: 1;
  }
  .kanji-fallback {
    position: absolute;
    inset: 25%;
    width: 50%;
    height: auto;
    opacity: 0.12;
  }
  .top {
    position: absolute;
    top: 8px;
    left: 8px;
    right: 8px;
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
  }
  .top .badge {
    border: 1.5px solid var(--line);
  }
  .tag {
    background: var(--bg);
    color: var(--text);
  }
  .bottom {
    position: absolute;
    left: 8px;
    bottom: 10px;
    right: 8px;
    display: flex;
    justify-content: flex-start;
    align-items: flex-end;
  }
  .eps,
  .done {
    display: inline-flex;
    align-items: baseline;
    gap: 1px;
    height: 24px;
    padding: 2px 8px 0;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    background: var(--bg);
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 14px;
    line-height: 20px;
  }
  .eps .of {
    color: var(--text-3);
    font-weight: 500;
  }
  .eps .lbl {
    font-family: var(--font);
    font-size: 11px;
    font-weight: 700;
    color: var(--text-3);
    margin-left: 3px;
  }
  .eps.warn .of {
    color: var(--amber);
  }
  .done {
    align-items: center;
    gap: 4px;
    padding-top: 0;
    background: var(--ok);
    color: var(--on-coral);
    font-size: 12px;
  }
  .progress {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 5px;
    background: var(--line);
  }
  .progress div {
    height: 100%;
    background: var(--coral);
  }
  .meta {
    padding: 0 2px;
    min-width: 0;
  }
  h3 {
    font-family: var(--font-display);
    font-size: 14.5px;
    font-weight: 500;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    transition: color var(--t-fast) var(--ease);
  }
  .poster-card:hover h3 {
    color: var(--coral);
  }
  .meta p {
    margin-top: 3px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
