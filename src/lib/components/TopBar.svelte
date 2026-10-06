<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { app } from "$lib/app.svelte";
  import { clock } from "$lib/format";
  import Icon from "./Icon.svelte";

  const PHASES: Record<string, string> = {
    discover: "Scanning folders",
    match: "Identifying anime",
    resolve: "Mapping episodes",
    metadata: "Fetching metadata",
    artwork: "Downloading artwork",
    done: "Up to date",
  };

  const path = $derived(page.url.pathname);
  const pct = $derived(
    app.progress && app.progress.total > 0 ? Math.min(100, (app.progress.current / app.progress.total) * 100) : null,
  );
  const playing = $derived(app.nowPlaying);
  const playingCard = $derived(playing ? app.cards.find((c) => c.anilistId === playing.anilistId) : null);

  let searchEl: HTMLInputElement | undefined = $state();

  function onInput() {
    // Searching always happens on the library page.
    if (path !== "/") goto(page.url.searchParams.get("lib") ? `/?lib=${page.url.searchParams.get("lib")}` : "/");
  }

  function onKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      searchEl?.focus();
      searchEl?.select();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<header class="topbar">
  <a class="brand" href="/" aria-label="Kura home" id="nav-home" onclick={() => (app.query = "")}>
    <img class="brand-mark" src="/brand/logo.png" alt="" width="38" height="38" />
    <img class="brand-word" src="/brand/wordmark-white.png" alt="Kura" height="18" />
  </a>

  <div class="search">
    <Icon name="search" size={16} />
    <input
      bind:this={searchEl}
      bind:value={app.query}
      oninput={onInput}
      onkeydown={(e) => e.key === "Escape" && ((app.query = ""), searchEl?.blur())}
      class="input"
      placeholder="Search your library — English, romaji, 日本語…"
      id="library-search"
      aria-label="Search library"
    />
    {#if app.query}
      <button class="clear" onclick={() => (app.query = "")} aria-label="Clear search"><Icon name="x" size={14} /></button>
    {:else}
      <kbd>Ctrl K</kbd>
    {/if}
  </div>

  <div class="actions">
    {#if playing}
      <a class="now" href={`/anime/${playing.anilistId}`} id="now-playing" title="Kura is tracking this episode">
        <span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>
        <span class="now-text">
          {#if playingCard}<span class="now-title">{playingCard.titleEnglish ?? playingCard.titleRomaji}</span>{/if}
          <span class="now-ep">Ep {playing.epKey}{playing.duration > 0 ? ` · ${clock(playing.position)}` : ""}</span>
        </span>
      </a>
    {/if}

    {#if app.reviewCount > 0}
      <a class="review" class:active={path.startsWith("/review")} href="/review" id="nav-review">
        <Icon name="alert" size={15} stroke={2.5} />
        {app.reviewCount} need review
      </a>
    {/if}

    {#if app.progress}
      <div class="scan" class:done={!app.scanning} title={app.progress.message}>
        {#if app.scanning}
          <span class="spinner"></span>
        {:else}
          <span class="ok"><Icon name="check" size={11} stroke={3.5} /></span>
        {/if}
        <span class="scan-text">
          <span class="scan-phase">
            {PHASES[app.progress.phase] ?? app.progress.phase}
            {#if pct !== null && app.scanning}<span class="scan-num">{app.progress.current}/{app.progress.total}</span>{/if}
          </span>
          <span class="bar"
            ><span
              class="fill"
              class:indeterminate={pct === null && app.scanning}
              style:width={pct === null ? (app.scanning ? "35%" : "100%") : `${pct}%`}
            ></span></span
          >
        </span>
      </div>
    {:else if app.libraries.length > 0}
      <button class="icon-btn" onclick={() => app.scan()} id="nav-rescan" title="Rescan libraries" aria-label="Rescan libraries">
        <Icon name="refresh" size={18} />
      </button>
    {/if}

    <a
      class="icon-btn"
      class:active={path.startsWith("/settings")}
      href="/settings"
      id="nav-settings"
      title="Settings"
      aria-label="Settings"
    >
      <Icon name="settings" size={19} />
    </a>
  </div>
</header>

<style>
  .topbar {
    position: relative;
    z-index: 20;
    flex: none;
    display: grid;
    grid-template-columns: 1fr minmax(260px, 560px) 1fr;
    align-items: center;
    gap: 20px;
    height: 68px;
    padding: 0 24px;
    background: var(--bg-elev);
    border-bottom: var(--bw) solid var(--line);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 11px;
    justify-self: start;
  }
  .brand-mark {
    border-radius: 11px;
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
    transition: transform var(--t-med) var(--bounce);
  }
  .brand:hover .brand-mark {
    transform: rotate(-6deg) scale(1.06);
  }
  .brand-word {
    height: 18px;
    width: auto;
  }

  .search {
    position: relative;
    display: flex;
    align-items: center;
  }
  .search > :global(svg) {
    position: absolute;
    left: 13px;
    color: var(--text-3);
    pointer-events: none;
  }
  .search .input {
    width: 100%;
    padding-left: 38px;
    padding-right: 64px;
  }
  kbd {
    position: absolute;
    right: 10px;
    font-family: var(--font);
    font-size: 11px;
    font-weight: 700;
    color: var(--text-3);
    border: 1.5px solid var(--border-strong);
    border-bottom-width: 3px;
    border-radius: 6px;
    padding: 1px 6px;
    background: var(--surface);
    pointer-events: none;
  }
  .clear {
    position: absolute;
    right: 8px;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 6px;
    background: var(--surface-3);
    cursor: pointer;
  }

  .actions {
    justify-self: end;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: var(--r-md);
    border: var(--bw) solid transparent;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease),
      transform var(--t-med) var(--bounce);
  }
  .icon-btn:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  #nav-rescan:hover :global(svg) {
    transform: rotate(90deg);
    transition: transform var(--t-med) var(--bounce);
  }
  .icon-btn.active {
    background: var(--coral);
    color: var(--on-coral);
    border-color: var(--line);
    box-shadow: var(--sticker);
  }

  .review {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 34px;
    padding: 0 13px;
    border-radius: 99px;
    border: var(--bw) solid var(--line);
    background: var(--amber);
    color: var(--on-coral);
    font-family: var(--font-display);
    font-size: 13.5px;
    font-weight: 600;
    white-space: nowrap;
    box-shadow: 2px 2px 0 var(--line);
    transition: transform var(--t-fast) var(--ease);
  }
  .review:hover {
    transform: translate(-1px, -1px) rotate(-1.5deg);
  }

  .now {
    display: inline-flex;
    align-items: center;
    gap: 9px;
    height: 38px;
    max-width: 260px;
    padding: 0 13px 0 11px;
    border-radius: 99px;
    border: var(--bw) solid var(--line);
    background: var(--surface-2);
    box-shadow: 2px 2px 0 var(--line);
    animation: fade-in var(--t-med) var(--ease);
  }
  .now:hover {
    background: var(--surface-3);
  }
  .now-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.15;
  }
  .now-title {
    font-family: var(--font-display);
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .now-ep {
    font-size: 11px;
    font-weight: 700;
    color: var(--coral);
    font-variant-numeric: tabular-nums;
  }
  .eq {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 14px;
    flex: none;
  }
  .eq i {
    width: 3px;
    background: var(--coral);
    border-radius: 2px;
    animation: eq 900ms ease-in-out infinite;
  }
  .eq i:nth-child(2) {
    animation-delay: -300ms;
  }
  .eq i:nth-child(3) {
    animation-delay: -600ms;
  }
  @keyframes eq {
    0%,
    100% {
      height: 4px;
    }
    50% {
      height: 14px;
    }
  }

  .scan {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 38px;
    padding: 0 12px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--surface);
    box-shadow: 2px 2px 0 var(--line);
    animation: fade-in var(--t-med) var(--ease);
  }
  .scan-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 150px;
  }
  .scan-phase {
    display: flex;
    justify-content: space-between;
    font-family: var(--font-display);
    font-size: 12.5px;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
  }
  .scan-num {
    font-family: var(--font);
    font-size: 11px;
    font-weight: 700;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .ok {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--ok);
    color: var(--on-coral);
  }
  .bar {
    display: block;
    height: 6px;
    border-radius: 99px;
    background: var(--bg);
    border: 1.5px solid var(--line);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: 99px;
    background: var(--coral);
    transition: width var(--t-med) var(--ease);
  }
  .scan.done .fill {
    background: var(--ok);
  }
  .fill.indeterminate {
    animation: slide 1.2s var(--ease) infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(300%);
    }
  }

  @media (max-width: 1180px) {
    .now-title {
      display: none;
    }
    .brand-word {
      display: none;
    }
  }
</style>
