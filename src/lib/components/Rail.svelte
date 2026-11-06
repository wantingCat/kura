<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    title,
    href = null,
    children,
  }: { title: string; href?: string | null; children: Snippet } = $props();

  let track: HTMLDivElement | undefined = $state();
  let canLeft = $state(false);
  let canRight = $state(false);

  function update() {
    if (!track) return;
    canLeft = track.scrollLeft > 4;
    canRight = track.scrollLeft + track.clientWidth < track.scrollWidth - 4;
  }

  function page(dir: 1 | -1) {
    track?.scrollBy({ left: dir * track.clientWidth * 0.85, behavior: "smooth" });
  }

  $effect(() => {
    if (!track) return;
    update();
    const ro = new ResizeObserver(update);
    ro.observe(track);
    return () => ro.disconnect();
  });
</script>

<section class="rail-section">
  <header class="rail-head">
    <h2>{title}</h2>
    {#if href}<a class="see-all" {href}>See all</a>{/if}
  </header>
  <div class="rail-wrap">
    {#if canLeft}
      <button class="nav left" onclick={() => page(-1)} aria-label="Scroll left">
        <Icon name="arrow-left" size={18} />
      </button>
    {/if}
    <div class="track" bind:this={track} onscroll={update}>
      {@render children()}
    </div>
    {#if canRight}
      <button class="nav right" onclick={() => page(1)} aria-label="Scroll right">
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
    padding: 0 var(--page-x);
    margin-bottom: 10px;
  }
  h2 {
    font-size: 19px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .see-all {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text-3);
  }
  .see-all:hover {
    color: var(--text);
  }
  .rail-wrap {
    position: relative;
  }
  .track {
    display: flex;
    gap: 16px;
    overflow-x: auto;
    padding: 6px var(--page-x) 8px;
    scroll-padding: 0 var(--page-x);
    scroll-snap-type: x proximity;
    scrollbar-width: none;
  }
  .track::-webkit-scrollbar {
    display: none;
  }
  .track > :global(*) {
    scroll-snap-align: start;
  }
  .nav {
    position: absolute;
    top: 6px;
    bottom: 48px;
    z-index: 2;
    width: calc(var(--page-x) - 6px);
    border: none;
    display: grid;
    place-items: center;
    color: #fff;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }
  .rail-wrap:hover .nav {
    opacity: 1;
  }
  .nav.left {
    left: 0;
    background: linear-gradient(90deg, var(--bg) 20%, transparent);
  }
  .nav.right {
    right: 0;
    background: linear-gradient(270deg, var(--bg) 20%, transparent);
  }
</style>

