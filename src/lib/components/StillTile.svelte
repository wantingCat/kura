<script lang="ts">
  import { img, type MediaCard, type UpNextItem } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { detailsModal } from "$lib/detailsModal.svelte";
  import { displayTitle } from "$lib/format";
  import { jaGenre, jaRemaining } from "$lib/ja";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";
  import { hoverTile, score10 } from "./PosterTile.svelte";

  let {
    item,
    card = null,
  }: {
    /** Episode to resume / play (Continue Watching). */
    item: UpNextItem;
    /** Library card of the show (score, genres). Optional. */
    card?: MediaCard | null;
  } = $props();

  const title = $derived(displayTitle(item));
  const src = $derived(
    img(item.thumbPath, item.thumbUrl) ?? img(item.bannerPath, item.bannerUrl) ?? img(item.coverPath, item.coverUrl),
  );
  const glow = $derived(item.coverColor || "var(--neon-violet)");
  const pct = $derived(item.duration > 0 ? Math.min(100, (item.position / item.duration) * 100) : 0);
  const left = $derived(item.position > 0 && item.duration > 0 ? Math.max(0, item.duration - item.position) : 0);
  const minsLeft = $derived(Math.max(1, Math.round(left / 60)));
  const epLabel = $derived(
    `${item.epKey.startsWith("S") ? "SP" : "EP"} ${String(item.number).padStart(2, "0")}`,
  );
  const launching = $derived(app.isLaunching(item.anilistId, item.epKey));
  const genres = $derived((card?.genres ?? []).slice(0, 3));

  function open() {
    detailsModal.open(item.anilistId);
  }
  function play() {
    app.play(item.anilistId, item.epKey, item.path);
  }
</script>

<article class="tile-wrap">
  <div class="tile" use:hoverTile style:--glow={glow}>
    <div class="art">
      <button class="hit" onclick={open} aria-label={`${title}, ${epLabel} — more info`}>
        {#if src}<img {src} alt="" loading="lazy" decoding="async" />{/if}
        <span class="shade" aria-hidden="true"></span>
      </button>

      <div class="panel">
        <div class="actions">
          <button
            class="round play"
            onclick={play}
            disabled={launching}
            aria-label={`${item.position > 0 ? "Resume" : "Play"} ${title} episode ${item.number}`}
          >
            {#if launching}<span class="spinner spinner-sm"></span>{:else}<Icon name="play" size={15} fill />{/if}
          </button>
          <div class="facts">
            {#if card?.averageScore}
              <span class="score"><Icon name="star" size={11} fill /> {score10(card.averageScore)}</span>
            {/if}
            {#if item.episodes}<span>{item.episodes} EP</span>{/if}
            {#each genres as g (g)}
              {#if jaGenre(g)}
                <Ja text={jaGenre(g)} label={g} essential class="g-chip" />
              {:else}
                <span class="g-chip">{g}</span>
              {/if}
            {/each}
          </div>
          <button class="round ghost more" onclick={open} aria-label={`More info about ${title}`}>
            <Icon name="chevron-down" size={16} />
          </button>
        </div>
      </div>

      {#if item.position > 0}
        <div
          class="neon-progress prog"
          role="progressbar"
          aria-label={`${title} progress`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(pct)}
        >
          <span style:width={`${pct}%`}></span>
        </div>
      {/if}
    </div>

    <div class="meta">
      <h3 class="title" {title}>{title}</h3>
      {#if item.titleNative && item.titleNative !== title}
        <Ja text={item.titleNative} block class="native" />
      {/if}
      <p class="sub tn-label">
        {epLabel}
        {#if left > 0}
          · <Ja text={jaRemaining(left)} label={`${minsLeft} min left`} essential />
        {:else if item.episodeTitle}
          · <span class="ep-title">{item.episodeTitle}</span>
        {/if}
      </p>
    </div>
  </div>
</article>

<style>
  .tile-wrap {
    flex: none;
    position: relative;
    width: var(--still-w, 300px);
  }
  .tile {
    --rx: 0deg;
    --ry: 0deg;
    position: relative;
    transform-origin: var(--origin, center);
    transition: transform var(--t-med) var(--ease);
  }
  .tile:global(.hot),
  .tile:focus-within {
    z-index: 3;
    transform: perspective(900px) translateY(-6px) scale(1.06) rotateX(var(--rx)) rotateY(var(--ry));
  }
  .art {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-md);
    overflow: hidden;
    background: var(--surface-2);
    box-shadow: var(--shadow-sm);
    transition: box-shadow var(--t-med) var(--ease);
  }
  .tile:global(.hot) .art,
  .tile:focus-within .art {
    box-shadow:
      0 18px 40px -14px color-mix(in srgb, var(--glow) 70%, transparent),
      0 0 28px -4px color-mix(in srgb, var(--glow) 45%, transparent);
  }
  .hit {
    position: absolute;
    inset: 0;
    display: block;
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
  .hit img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(0deg, rgba(5, 6, 12, 0.78) 0%, rgba(5, 6, 12, 0.1) 55%, transparent 100%);
  }
  .prog {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    border-radius: 0;
  }

  .panel {
    position: absolute;
    left: 6px;
    right: 6px;
    bottom: 8px;
    padding: 7px;
    border-radius: 10px;
    background: rgba(12, 14, 24, 0.93);
    border: 1px solid rgba(255, 255, 255, 0.09);
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.55);
    transform: translateY(calc(100% + 10px));
    opacity: 0;
    /* Hidden panel buttons must not be clickable / focusable. */
    visibility: hidden;
    pointer-events: none;
    transition:
      transform var(--t-med) var(--ease),
      opacity var(--t-med) var(--ease),
      visibility 0s linear var(--t-med);
  }
  .tile:global(.hot) .panel,
  .tile:focus-within .panel {
    transform: none;
    opacity: 1;
    visibility: visible;
    pointer-events: auto;
    transition:
      transform var(--t-med) var(--ease),
      opacity var(--t-med) var(--ease),
      visibility 0s;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .round {
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    padding: 0;
    cursor: pointer;
    transition: box-shadow var(--t-fast) var(--ease);
  }
  .round.play {
    border: 0;
    background: var(--primary);
    color: var(--on-primary);
  }
  .round.play :global(svg) {
    margin-left: 2px;
  }
  .round.ghost {
    border: 1px solid var(--glass-hairline);
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  .round:hover {
    box-shadow: var(--neon-glow);
  }
  .round:disabled {
    cursor: wait;
  }
  .facts {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    font-family: var(--font-label);
    font-size: 11px;
    font-weight: 600;
    color: var(--text-2);
    white-space: nowrap;
  }
  .score {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--amber);
  }
  .facts :global(.g-chip) {
    padding: 1px 7px;
    border-radius: var(--r-pill);
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
    font-family: var(--font-jp);
    font-size: 10.5px;
  }

  .meta {
    padding: 8px 2px 0;
    min-width: 0;
  }
  .title {
    font-size: 13.5px;
    font-weight: 600;
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta :global(.native) {
    font-size: 11.5px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    margin-top: 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub :global(.ja) {
    letter-spacing: 0.04em;
    text-transform: none;
  }
  .ep-title {
    text-transform: none;
    letter-spacing: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .tile:global(.hot),
    .tile:focus-within {
      transform: none;
    }
  }
</style>
