<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import Ja from "./Ja.svelte";

  let {
    title,
    ja = null,
    href = null,
    children,
  }: {
    /** English row title. */
    title: string;
    /** Decorative Japanese subtitle shown after the title (e.g. 新着). */
    ja?: string | null;
    /** Optional "See all" link. */
    href?: string | null;
    children: Snippet;
  } = $props();

  const uid = `rail-${Math.random().toString(36).slice(2, 9)}`;
  let track: HTMLDivElement | undefined = $state();
  let canLeft = $state(false);
  let canRight = $state(false);

  function update() {
    if (!track) return;
    canLeft = track.scrollLeft > 4;
    canRight = track.scrollLeft + track.clientWidth < track.scrollWidth - 4;
  }

  function page(dir: 1 | -1) {
    if (!track) return;
    const reduce = typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
    track.scrollBy({ left: dir * track.clientWidth * 0.9, behavior: reduce ? "auto" : "smooth" });
  }

  $effect(() => {
    if (!track) return;
    update();
    const ro = new ResizeObserver(update);
    ro.observe(track);
    // Items can be added/removed without the track itself resizing.
    const mo = new MutationObserver(update);
    mo.observe(track, { childList: true });
    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  });
</script>

<section class="rail-section" aria-labelledby={uid}>
  <header class="rail-head">
    <h2 id={uid}>{title}{#if ja}<Ja text={ja} class="ja-sub" />{/if}</h2>
    {#if href}<a class="see-all" {href}>See all <Icon name="chevron-right" size={13} /></a>{/if}
  </header>
  <div class="rail-wrap">
    {#if canLeft}
      <button class="nav left glass" onclick={() => page(-1)} aria-label="Scroll left">
        <Icon name="chevron-right" size={20} class="flip" />
      </button>
    {/if}
    <div class="track no-scrollbar" bind:this={track} onscroll={update}>
      {@render children()}
    </div>
    {#if canRight}
      <button class="nav right glass" onclick={() => page(1)} aria-label="Scroll right">
        <Icon name="chevron-right" size={20} />
      </button>
    {/if}
  </div>
</section>

<style>
  .rail-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    padding: 0 var(--gutter);
  }
  h2 {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .see-all {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text-2);
  }
  .see-all:hover {
    color: var(--text);
  }
  .rail-wrap {
    position: relative;
  }
  .track {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    overflow-x: auto;
    overflow-y: hidden;
    /* Vertical room so lifted / scaled hover tiles aren't clipped. */
    padding: 18px var(--gutter) 26px;
    scroll-padding: 0 var(--gutter);
    scroll-snap-type: x proximity;
  }
  .track > :global(*) {
    scroll-snap-align: start;
  }
  /* Edge tiles grow inward instead of past the viewport edge. */
  .track > :global(:first-child) {
    --origin: left center;
  }
  .track > :global(:last-child) {
    --origin: right center;
  }
  .nav {
    position: absolute;
    top: 50%;
    z-index: 5;
    width: 44px;
    height: 44px;
    margin-top: -30px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: var(--text);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .nav :global(.flip) {
    transform: scaleX(-1);
  }
  .rail-wrap:hover .nav,
  .rail-section:focus-within .nav {
    opacity: 1;
  }
  .nav:hover {
    box-shadow: var(--neon-glow);
  }
  .nav.left {
    left: max(6px, calc(var(--gutter) / 2 - 22px));
  }
  .nav.right {
    right: max(6px, calc(var(--gutter) / 2 - 22px));
  }
  @media (hover: none) {
    .nav {
      opacity: 1;
    }
  }
</style>
