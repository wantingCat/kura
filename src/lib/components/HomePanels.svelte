<!--
  Home "comic page": a big Continue-watching panel plus side panels for new
  episodes and nearly finished shows. Panels without content are hidden.

  Tone it down by lowering `--tilt` (0 = no rotated panels) or swapping
  `--panel-border` for the regular `--bw`.
-->
<script lang="ts">
  import { api, img, type MediaCard, type UpNextItem } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { clock, displayTitle, totalEpisodes } from "$lib/format";
  import Icon from "./Icon.svelte";

  let { scoped }: { scoped: MediaCard[] } = $props();

  let upNext = $state<UpNextItem[]>([]);
  let fresh = $state<UpNextItem[]>([]);
  let pick = $state(0);

  $effect(() => {
    void app.version; // re-fetch whenever the library / watch state changes
    Promise.all([api.getUpNext(6), api.getNewEpisodes(4)])
      .then(([u, n]) => {
        upNext = u;
        fresh = n;
      })
      .catch((e) => console.warn("[kura] home panels", e));
  });

  const ids = $derived(new Set(scoped.map((c) => c.anilistId)));
  const cont = $derived(upNext.filter((i) => ids.has(i.anilistId)));
  const featured = $derived(cont.length ? cont[Math.min(pick, cont.length - 1)] : null);
  const queue = $derived(cont.filter((i) => i !== featured).slice(0, 4));

  const newItem = $derived(
    fresh.find((i) => ids.has(i.anilistId) && !(featured && i.anilistId === featured.anilistId)) ?? null,
  );

  /** A show you're into with only a few owned episodes left. */
  const finish = $derived.by(() => {
    const taken = new Set([featured?.anilistId, newItem?.anilistId]);
    return (
      scoped
        .map((c) => ({ c, total: totalEpisodes(c), left: (totalEpisodes(c) ?? 0) - c.watchedCount }))
        .filter(
          ({ c, total, left }) =>
            total !== null && total > 1 && c.watchedCount > 0 && left > 0 && left <= 3 && c.ownedCount > c.watchedCount && !taken.has(c.anilistId),
        )
        .sort((a, b) => a.left - b.left || (b.c.lastWatchedAt ?? 0) - (a.c.lastWatchedAt ?? 0))[0] ?? null
    );
  });

  const art = (i: UpNextItem) => img(i.thumbPath, i.thumbUrl) ?? img(i.bannerPath, i.bannerUrl) ?? img(i.coverPath, i.coverUrl);
  const wide = (i: UpNextItem) => img(i.bannerPath, i.bannerUrl) ?? img(i.thumbPath, i.thumbUrl) ?? img(i.coverPath, i.coverUrl);
  const pct = (i: UpNextItem) => (i.duration > 0 ? Math.min(100, (i.position / i.duration) * 100) : 0);
  const epLabel = (i: UpNextItem) => (i.epKey.startsWith("S") ? `Special ${i.number}` : `Episode ${i.number}`);

  function select(i: UpNextItem) {
    pick = cont.indexOf(i);
  }
</script>

{#if featured || newItem || finish}
  <section class="panels" class:solo={!featured} class:no-side={!newItem && !finish} aria-label="Pick up where you left off">
    {#if featured}
      {#key featured.anilistId + featured.epKey}
        <article class="panel main">
          <img class="mascot cat" src="/brand/cat.png" alt="" width="104" />
          <div class="art">
            {#if wide(featured)}<img src={wide(featured)} alt="" />{:else}<img class="kanji" src="/brand/kanji.png" alt="" />{/if}
            <span class="dots" aria-hidden="true"></span>
          </div>

          <span class="caption">Continue watching</span>
          <span class="bubble">{featured.position > 0 ? "Where were we?" : "Up next!"}</span>

          <div class="info">
            <a class="title" href={`/anime/${featured.anilistId}`}>{displayTitle(featured)}</a>
            <p class="ep">
              <strong>{epLabel(featured)}</strong>{#if featured.episodeTitle}<span> · {featured.episodeTitle}</span>{/if}
            </p>
            {#if featured.position > 0}
              <div class="prog">
                <span class="bar"><span class="fill" style:width={`${pct(featured)}%`}></span></span>
                <span class="time">{clock(featured.position)} / {clock(featured.duration)}</span>
              </div>
            {/if}
            <div class="actions">
              <button
                class="btn btn-primary play"
                onclick={() => app.play(featured.anilistId, featured.epKey, featured.path)}
                id="home-resume"
              >
                <Icon name="play" size={16} fill />
                {featured.position > 0 ? `Resume ${clock(featured.position)}` : `Play episode ${featured.number}`}
              </button>
              <a class="btn" href={`/anime/${featured.anilistId}`} id="home-details">Details</a>
            </div>
          </div>

          {#if queue.length}
            <div class="queue" aria-label="Also watching">
              {#each queue as q (q.anilistId)}
                <button class="q" onclick={() => select(q)} title={`${displayTitle(q)} · ${epLabel(q)}`}>
                  {#if art(q)}<img src={art(q)} alt="" />{/if}
                  <span class="q-ep">Ep {q.number}</span>
                  {#if q.position > 0}<span class="q-bar" style:width={`${pct(q)}%`}></span>{/if}
                </button>
              {/each}
            </div>
          {/if}
        </article>
      {/key}
    {/if}

    {#if newItem || finish}
      <div class="side">
        {#if newItem}
          <article class="panel new">
            <span class="burst" aria-hidden="true"><span>NEW!</span></span>
            <a class="thumb" href={`/anime/${newItem.anilistId}`}>
              {#if art(newItem)}<img src={art(newItem)} alt="" />{/if}
            </a>
            <div class="side-body">
              <span class="kicker">Just landed</span>
              <a class="side-title" href={`/anime/${newItem.anilistId}`}>{displayTitle(newItem)}</a>
              <p class="side-sub">
                {epLabel(newItem)}{newItem.newCount > 1 ? ` + ${newItem.newCount - 1} more` : ""}
              </p>
              <button
                class="btn btn-sm"
                onclick={() => app.play(newItem.anilistId, newItem.epKey, newItem.path)}
                id="home-play-new"
              >
                <Icon name="play" size={13} fill /> Play
              </button>
            </div>
          </article>
        {/if}

        {#if finish}
          <a class="panel finish" href={`/anime/${finish.c.anilistId}`} id="home-finish">
            <span class="poster">
              {#if img(finish.c.coverPath, finish.c.coverUrl)}<img src={img(finish.c.coverPath, finish.c.coverUrl)} alt="" />{/if}
            </span>
            <div class="side-body">
              <span class="kicker amber">Finish this!</span>
              <span class="side-title">{displayTitle(finish.c)}</span>
              <p class="left">
                Only <strong>{finish.left}</strong> episode{finish.left === 1 ? "" : "s"} left
              </p>
              <span class="bar small"
                ><span class="fill amber" style:width={`${(finish.c.watchedCount / (finish.total ?? 1)) * 100}%`}></span></span
              >
            </div>
          </a>
        {/if}
      </div>
    {/if}
  </section>
{/if}

<style>
  .panels {
    --tilt: 1;
    --panel-border: 3px;
    display: grid;
    grid-template-columns: minmax(0, 1.9fr) minmax(300px, 1fr);
    gap: 24px;
    margin: 0 0 40px;
    padding-top: 46px; /* room for the cat */
  }
  .panels.no-side {
    grid-template-columns: 1fr;
  }
  .panels.solo {
    padding-top: 6px;
    grid-template-columns: 1fr;
  }
  .panels.solo .side {
    flex-direction: row;
  }
  .panels.solo .side > :global(*) {
    flex: 1;
  }

  .panel {
    position: relative;
    border: var(--panel-border) solid var(--line);
    border-radius: var(--r-lg);
    background: var(--surface);
    box-shadow: var(--sticker-lg);
    animation: fade-in 400ms var(--ease) both;
  }

  /* Main panel ----------------------------------------------------------- */
  .main {
    min-height: 360px;
    display: flex;
    align-items: flex-end;
    padding: 22px;
  }
  .cat {
    position: absolute;
    top: -46px;
    right: 64px;
    z-index: 0;
    animation: peek 600ms var(--bounce) both 150ms;
  }
  .art {
    position: absolute;
    inset: 0;
    z-index: 1;
    border-radius: calc(var(--r-lg) - var(--panel-border));
    overflow: hidden;
    background: var(--coral);
  }
  .art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 30%;
    animation: art-in 900ms var(--ease) both;
  }
  .art .kanji {
    width: 40%;
    height: auto;
    margin: 40px auto;
    object-fit: contain;
  }
  @keyframes art-in {
    from {
      opacity: 0;
      transform: scale(1.04);
    }
  }
  /* Comic halftone wash so the text boxes sit on the art more naturally. */
  .dots {
    position: absolute;
    inset: 0;
    background-image: radial-gradient(color-mix(in srgb, var(--line) 55%, transparent) 1.2px, transparent 1.6px);
    background-size: 7px 7px;
    mask-image: linear-gradient(to top, #000 0%, transparent 70%);
    opacity: 0.9;
  }

  .caption {
    position: absolute;
    top: 18px;
    left: 18px;
    z-index: 2;
    padding: 6px 12px;
    border: var(--bw) solid var(--line);
    border-radius: 8px;
    background: var(--amber);
    color: var(--on-coral);
    font-family: var(--font-display);
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    box-shadow: 2px 2px 0 var(--line);
    transform: rotate(calc(-2deg * var(--tilt)));
  }
  .bubble {
    position: absolute;
    top: 22px;
    right: 190px;
    z-index: 2;
    padding: 8px 16px;
    border: var(--bw) solid var(--line);
    border-radius: 999px;
    background: var(--cream);
    color: var(--on-coral);
    font-family: var(--font-display);
    font-size: 17px;
    font-weight: 700;
    text-transform: uppercase;
    box-shadow: 2px 2px 0 var(--line);
    transform: rotate(calc(3deg * var(--tilt)));
    animation: pop 500ms var(--bounce) both 450ms;
  }
  .bubble::after {
    content: "";
    position: absolute;
    right: -9px;
    top: 50%;
    width: 14px;
    height: 14px;
    margin-top: -7px;
    background: var(--cream);
    border-right: var(--bw) solid var(--line);
    border-top: var(--bw) solid var(--line);
    transform: rotate(45deg) skew(12deg, 12deg);
  }
  @keyframes pop {
    from {
      opacity: 0;
      scale: 0.6;
    }
  }

  .info {
    position: relative;
    z-index: 2;
    max-width: min(560px, 68%);
    padding: 18px 20px;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-md);
    background: var(--surface-3);
    box-shadow: var(--sticker);
  }
  .title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-family: var(--font-display);
    font-size: 28px;
    font-weight: 600;
    line-height: 1.1;
  }
  .title:hover {
    color: var(--coral-hi);
  }
  .ep {
    margin-top: 6px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ep strong {
    color: var(--coral);
    font-weight: 800;
  }
  .prog {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 12px;
  }
  .bar {
    flex: 1;
    display: block;
    height: 10px;
    border-radius: 99px;
    border: var(--bw) solid var(--line);
    background: var(--bg);
    overflow: hidden;
  }
  .bar.small {
    height: 8px;
    margin-top: 8px;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--coral);
    border-radius: 99px;
  }
  .fill.amber {
    background: var(--amber);
  }
  .time {
    font-size: 12.5px;
    font-weight: 700;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .actions {
    display: flex;
    gap: 10px;
    margin-top: 16px;
  }
  .play {
    height: 46px;
    padding: 0 22px;
    font-size: 16px;
  }
  .actions .btn:not(.play) {
    height: 46px;
  }

  .queue {
    position: absolute;
    right: 18px;
    bottom: 18px;
    z-index: 2;
    display: flex;
    gap: 10px;
  }
  .q {
    position: relative;
    width: 104px;
    aspect-ratio: 16 / 9;
    padding: 0;
    border: var(--bw) solid var(--line);
    border-radius: 10px;
    background: var(--surface-2);
    overflow: hidden;
    cursor: pointer;
    box-shadow: 2px 2px 0 var(--line);
    transition:
      transform var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .q:hover {
    transform: translate(-1px, -2px) rotate(calc(-2deg * var(--tilt)));
    box-shadow: 3px 4px 0 var(--line);
  }
  .q img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .q-ep {
    position: absolute;
    left: 5px;
    bottom: 5px;
    padding: 0 6px;
    border-radius: 6px;
    background: var(--plum);
    color: var(--cream);
    font-size: 11px;
    font-weight: 800;
  }
  .q-bar {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 4px;
    background: var(--coral);
  }

  /* Side panels ---------------------------------------------------------- */
  .side {
    display: flex;
    flex-direction: column;
    gap: 24px;
    min-width: 0;
  }
  .new,
  .finish {
    flex: 1;
    display: flex;
    gap: 16px;
    padding: 14px;
    min-height: 150px;
    transition:
      transform var(--t-med) var(--bounce),
      box-shadow var(--t-fast) var(--ease);
  }
  .new {
    transform: rotate(calc(0.8deg * var(--tilt)));
  }
  .finish {
    transform: rotate(calc(-0.7deg * var(--tilt)));
  }
  .new:hover,
  .finish:hover {
    transform: rotate(0) translate(-2px, -2px);
    box-shadow: 7px 7px 0 var(--line);
  }
  .burst {
    position: absolute;
    top: -22px;
    right: -16px;
    z-index: 3;
    width: 70px;
    height: 70px;
    display: grid;
    place-items: center;
    background: var(--coral);
    clip-path: polygon(
      50% 0%, 61% 15%, 79% 7%, 80% 26%, 98% 30%, 88% 46%, 100% 61%, 82% 69%, 85% 88%, 66% 85%, 55% 100%, 44% 86%,
      26% 94%, 23% 75%, 4% 72%, 13% 55%, 0% 40%, 17% 31%, 13% 12%, 33% 14%
    );
    filter: drop-shadow(2px 2px 0 var(--line));
    transform: rotate(calc(12deg * var(--tilt)));
    animation: pop 500ms var(--bounce) both 600ms;
  }
  .burst span {
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 700;
    color: var(--on-coral);
  }
  .thumb {
    flex: none;
    width: 46%;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-sm);
    overflow: hidden;
    background: var(--surface-2);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .poster {
    flex: none;
    width: 84px;
    aspect-ratio: 2 / 3;
    border: var(--bw) solid var(--line);
    border-radius: var(--r-sm);
    overflow: hidden;
    background: var(--surface-2);
    align-self: center;
  }
  .poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .side-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: flex-start;
    gap: 3px;
  }
  .kicker {
    font-family: var(--font-display);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--coral);
  }
  .kicker.amber {
    color: var(--amber);
  }
  .side-title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-family: var(--font-display);
    font-size: 17px;
    font-weight: 600;
    line-height: 1.2;
  }
  .side-sub {
    color: var(--text-2);
    font-size: 13px;
    margin-bottom: 8px;
  }
  .left {
    color: var(--text-2);
    font-size: 13.5px;
  }
  .left strong {
    color: var(--amber);
    font-size: 17px;
    font-family: var(--font-display);
  }
  .finish .side-body {
    align-items: stretch;
  }

  @media (max-width: 1150px) {
    .panels {
      grid-template-columns: 1fr;
    }
    .side {
      flex-direction: row;
    }
    .side > :global(*) {
      flex: 1;
    }
    .bubble {
      right: 160px;
    }
    .queue {
      display: none;
    }
    .info {
      max-width: 100%;
    }
  }
</style>
