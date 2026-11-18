<script lang="ts" module>
  /**
   * Shared hover behaviour for Tokyo Night tiles (PosterTile + StillTile).
   * - Adds `.hot` after the `--hover-intent` delay (default 300ms).
   * - Writes `--rx` / `--ry` (≤ 4° tilt toward the cursor) while hot.
   * - Does nothing fancy under prefers-reduced-motion (CSS also guards it).
   * Keyboard users get the same panel through `:focus-within` (pure CSS).
   */
  let reducedMotion = false;
  if (typeof matchMedia !== "undefined") {
    const mq = matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotion = mq.matches;
    mq.addEventListener("change", (e) => {
      reducedMotion = e.matches;
    });
  }

  export function hoverTile(node: HTMLElement) {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let rafId = 0;
    let rect: DOMRect | null = null;
    let lastX = 0;
    let lastY = 0;

    const reset = () => {
      if (rafId) {
        cancelAnimationFrame(rafId);
        rafId = 0;
      }
      rect = null;
      node.style.removeProperty("--rx");
      node.style.removeProperty("--ry");
    };
    const applyTilt = () => {
      rafId = 0;
      if (!rect || rect.width <= 0 || rect.height <= 0) return;
      const px = (lastX - rect.left) / rect.width - 0.5;
      const py = (lastY - rect.top) / rect.height - 0.5;
      node.style.setProperty("--ry", `${(px * 8).toFixed(2)}deg`);
      node.style.setProperty("--rx", `${(-py * 8).toFixed(2)}deg`);
    };
    const enter = () => {
      // Arm only: the intent timer starts on real pointer movement, so tiles
      // scrolling under a stationary cursor (keyboard / wheel) don't pop open.
      clearTimeout(timer);
      timer = undefined;
    };
    const leave = () => {
      clearTimeout(timer);
      timer = undefined;
      node.classList.remove("hot");
      reset();
    };
    const move = (e: PointerEvent) => {
      if (e.pointerType === "mouse" && e.movementX === 0 && e.movementY === 0) return;
      lastX = e.clientX;
      lastY = e.clientY;
      if (!node.classList.contains("hot")) {
        if (timer === undefined) {
          timer = setTimeout(() => {
            node.classList.add("hot");
            if (!reducedMotion) {
              rect = node.getBoundingClientRect();
              applyTilt();
            }
          }, 260);
        }
        return;
      }
      if (reducedMotion) return;
      if (!rect) rect = node.getBoundingClientRect();
      if (!rafId) rafId = requestAnimationFrame(applyTilt);
    };
    node.addEventListener("pointerenter", enter);
    node.addEventListener("pointerleave", leave);
    node.addEventListener("pointermove", move, { passive: true });
    return {
      destroy() {
        clearTimeout(timer);
        if (rafId) cancelAnimationFrame(rafId);
        node.removeEventListener("pointerenter", enter);
        node.removeEventListener("pointerleave", leave);
        node.removeEventListener("pointermove", move);
      },
    };
  }

  /** AniList 0–100 score → "8.7". */
  export const score10 = (s: number | null | undefined) => (s ? (s / 10).toFixed(1) : "");
</script>

<script lang="ts">
  import { img, type MediaCard, type UpNextItem } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { detailsModal } from "$lib/detailsModal.svelte";
  import { displayTitle } from "$lib/format";
  import { jaGenre } from "$lib/ja";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";

  let {
    m,
    item = null,
    rank = null,
    badge = null,
    sub = null,
  }: {
    /** The show. */
    m: MediaCard;
    /** A playable episode (new episode / up next). Enables the white Play button. */
    item?: UpNextItem | null;
    /** Top-10 rank: draws a huge outlined numeral behind the poster. */
    rank?: number | null;
    /** Small corner tag, e.g. "EP 05 +2". */
    badge?: string | null;
    /** Secondary line under the title (defaults to the native title). */
    sub?: string | null;
  } = $props();

  const title = $derived(displayTitle(m));
  const src = $derived(img(m.coverPath, m.coverUrl));
  const glow = $derived(m.coverColor || "var(--neon-violet)");
  const genres = $derived(m.genres.slice(0, 3));
  const launching = $derived(item ? app.isLaunching(item.anilistId, item.epKey) : false);

  function open() {
    detailsModal.open(m.anilistId);
  }
  function play() {
    if (item) app.play(item.anilistId, item.epKey, item.path);
    else open();
  }
</script>

<article class="tile-wrap" class:ranked={rank !== null}>
  {#if rank !== null}
    <span class="rank-num" aria-hidden="true">{rank}</span>
  {/if}
  <div class="tile" use:hoverTile style:--glow={glow}>
    <div class="art">
      <button
        class="hit"
        onclick={open}
        aria-label={`${rank !== null ? `#${rank} · ` : ""}${title} — more info`}
      >
        {#if src}
          <img {src} alt="" loading="lazy" decoding="async" />
        {:else}
          <span class="art-fallback" lang="ja" aria-hidden="true">{m.titleNative?.slice(0, 1) ?? "蔵"}</span>
        {/if}
        <span class="shade" aria-hidden="true"></span>
      </button>
      {#if badge}<span class="tag tn-label">{badge}</span>{/if}

      <div class="panel">
        <div class="actions">
          <button
            class="round play"
            onclick={play}
            disabled={launching}
            aria-label={item ? `Play ${title} episode ${item.number}` : `Open ${title}`}
          >
            {#if launching}<span class="spinner spinner-sm"></span>{:else}<Icon name="play" size={13} fill />{/if}
          </button>
          <span class="facts">
            {#if m.averageScore}
              <span class="score"><Icon name="star" size={10} fill /> {score10(m.averageScore)}</span>
            {/if}
            {#if m.episodes}<span class="eps">{m.episodes} EP</span>{/if}
          </span>
          <button class="round ghost more" onclick={open} aria-label={`More info about ${title}`}>
            <Icon name="chevron-down" size={16} />
          </button>
        </div>
        {#if genres.length}
          <ul class="genres" aria-label="Genres">
            {#each genres as g (g)}
              <li>
                {#if jaGenre(g)}
                  <Ja text={jaGenre(g)} label={g} essential class="g-chip" />
                {:else}
                  <span class="g-chip">{g}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>
    <div class="meta">
      <h3 class="title" {title}>{title}</h3>
      {#if sub}
        <p class="sub">{sub}</p>
      {:else if m.titleNative && m.titleNative !== title}
        <Ja text={m.titleNative} block class="native" />
      {/if}
    </div>
  </div>
</article>

<style>
  .tile-wrap {
    flex: none;
    position: relative;
    width: var(--tile-w, 168px);
  }
  .tile-wrap.ranked {
    width: calc(var(--tile-w, 168px) + 64px);
    padding-left: 64px;
  }
  .rank-num {
    position: absolute;
    left: -4px;
    bottom: 34px;
    font-family: var(--font-display);
    font-size: 168px;
    font-weight: 900;
    line-height: 0.8;
    letter-spacing: -0.08em;
    color: transparent;
    -webkit-text-stroke: 2px color-mix(in srgb, var(--text-2) 70%, transparent);
    user-select: none;
    pointer-events: none;
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
    transform: perspective(800px) translateY(-6px) scale(1.06) rotateX(var(--rx)) rotateY(var(--ry));
  }

  .art {
    position: relative;
    aspect-ratio: 2 / 3;
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
  .art-fallback {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 56px;
    font-weight: 800;
    color: var(--text-3);
  }
  .shade {
    position: absolute;
    inset: 0;
    background: linear-gradient(0deg, rgba(5, 6, 12, 0.55) 0%, transparent 40%);
    opacity: 0;
    transition: opacity var(--t-med) var(--ease);
  }
  .tile:global(.hot) .shade,
  .tile:focus-within .shade {
    opacity: 1;
  }
  .tag {
    position: absolute;
    top: 8px;
    right: 8px;
    padding: 3px 8px;
    border-radius: var(--r-pill);
    background: var(--cta);
    color: var(--on-cta);
    font-size: 10px;
    pointer-events: none;
  }

  .panel {
    position: absolute;
    left: 5px;
    right: 5px;
    bottom: 5px;
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px;
    border-radius: 9px;
    background: rgba(12, 14, 24, 0.93);
    border: 1px solid rgba(255, 255, 255, 0.09);
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.55);
    transform: translateY(calc(100% + 8px));
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
    gap: 6px;
  }
  .round {
    flex: none;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    padding: 0;
    cursor: pointer;
    transition:
      transform var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
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
  .round.more {
    margin-left: auto;
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
    white-space: nowrap;
    font-family: var(--font-label);
    font-size: 10.5px;
    font-weight: 600;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .score {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--amber);
  }
  .genres {
    display: flex;
    flex-wrap: nowrap;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow: hidden;
  }
  .genres :global(.g-chip) {
    display: inline-block;
    padding: 0 6px;
    border-radius: var(--r-pill);
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
    font-size: 10px;
    line-height: 16px;
    font-weight: 600;
    white-space: nowrap;
  }

  .meta {
    padding: 8px 2px 0;
    min-width: 0;
  }
  .title {
    font-size: 13.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta :global(.native),
  .sub {
    margin-top: 1px;
    font-size: 11.5px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  @media (prefers-reduced-motion: reduce) {
    .tile:global(.hot),
    .tile:focus-within {
      transform: none;
    }
  }
</style>
