<script lang="ts">
  import { page } from "$app/state";
  import { app } from "$lib/app.svelte";
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
  const libParam = $derived(page.url.searchParams.get("lib"));
  const pct = $derived(
    app.progress && app.progress.total > 0 ? Math.min(100, (app.progress.current / app.progress.total) * 100) : null,
  );
</script>

<aside class="sidebar">
  <a class="brand" href="/" aria-label="Kura home">
    <img class="brand-mark" src="/brand/logo.png" alt="" width="42" height="42" />
    <img class="brand-word" src="/brand/wordmark-white.png" alt="Kura" height="20" />
  </a>

  <nav>
    <a class="nav-item" class:active={path === "/" && !libParam} href="/" id="nav-home">
      <Icon name="home" />
      <span>Home</span>
    </a>

    {#if app.libraries.length > 0}
      <div class="nav-label">Libraries</div>
      {#each app.libraries as lib (lib.id)}
        <a
          class="nav-item"
          class:active={path === "/" && libParam === String(lib.id)}
          href={`/?lib=${lib.id}`}
          id={`nav-lib-${lib.id}`}
        >
          <Icon name="library" />
          <span class="truncate">{lib.name}</span>
          <span class="count">{lib.mediaCount}</span>
        </a>
      {/each}
    {/if}

    <div class="nav-label">Manage</div>
    <a class="nav-item" class:active={path.startsWith("/review")} href="/review" id="nav-review">
      <Icon name="alert" />
      <span>Needs review</span>
      {#if app.reviewCount > 0}
        <span class="pill">{app.reviewCount}</span>
      {/if}
    </a>
    <a class="nav-item" class:active={path.startsWith("/settings")} href="/settings" id="nav-settings">
      <Icon name="settings" />
      <span>Settings</span>
    </a>
  </nav>

  <div class="spacer"></div>

  {#if app.progress}
    <div class="scan-wrap">
      {#if app.scanning}
        <img class="mascot scan-cat" src="/brand/cat.png" alt="" width="92" />
      {/if}
      <div class="scan" class:done={!app.scanning}>
        <div class="scan-head">
          {#if app.scanning}
            <div class="spinner"></div>
          {:else}
            <span class="ok"><Icon name="check" size={12} stroke={3.5} /></span>
          {/if}
          <span class="scan-phase">{PHASES[app.progress.phase] ?? app.progress.phase}</span>
          {#if pct !== null && app.scanning}
            <span class="scan-num">{app.progress.current}/{app.progress.total}</span>
          {/if}
        </div>
        <div class="bar"><div class="fill" style:width={pct === null ? (app.scanning ? "30%" : "100%") : `${pct}%`} class:indeterminate={pct === null && app.scanning}></div></div>
        <p class="scan-msg" title={app.progress.message}>{app.progress.message}</p>
      </div>
    </div>
  {:else if app.libraries.length > 0}
    <button class="btn btn-ghost rescan" onclick={() => app.scan()} id="sidebar-rescan">
      <Icon name="refresh" size={16} />
      Rescan libraries
    </button>
  {/if}
</aside>

<style>
  .sidebar {
    width: var(--sidebar-w);
    flex: none;
    height: 100vh;
    display: flex;
    flex-direction: column;
    padding: 20px 16px;
    background: var(--bg-elev);
    border-right: var(--bw) solid var(--line);
    gap: 4px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 2px 4px 24px;
  }
  .brand-mark {
    border-radius: 12px;
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
    transition: transform var(--t-med) var(--bounce);
  }
  .brand:hover .brand-mark {
    transform: rotate(-6deg) scale(1.05);
  }
  .brand-word {
    height: 20px;
    width: auto;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .nav-label {
    margin: 18px 10px 6px;
    font-family: var(--font-display);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--text-3);
  }
  .nav-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 11px;
    height: 40px;
    padding: 0 12px;
    border-radius: var(--r-md);
    border: var(--bw) solid transparent;
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 500;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease),
      transform var(--t-fast) var(--ease);
  }
  .nav-item:hover {
    background: var(--surface);
    color: var(--text);
  }
  .nav-item.active {
    background: var(--coral);
    border-color: var(--line);
    color: var(--on-coral);
    box-shadow: var(--sticker);
  }
  .nav-item.active .count {
    color: color-mix(in srgb, var(--on-coral) 65%, transparent);
  }
  .truncate {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-family: var(--font);
    font-size: 12px;
    font-weight: 700;
    color: var(--text-3);
  }
  .pill {
    margin-left: auto;
    min-width: 22px;
    height: 20px;
    padding: 0 7px;
    border-radius: 99px;
    background: var(--amber);
    color: var(--on-coral);
    font-family: var(--font);
    font-size: 11px;
    font-weight: 800;
    display: grid;
    place-items: center;
  }
  .nav-item.active .pill {
    background: var(--on-coral);
    color: var(--coral);
  }
  .spacer {
    flex: 1;
  }
  .rescan {
    justify-content: flex-start;
    width: 100%;
  }
  .scan-wrap {
    position: relative;
    padding-top: 44px;
    animation: fade-up var(--t-med) var(--ease);
  }
  .scan-cat {
    position: absolute;
    top: 0;
    left: 50%;
    margin-left: -46px;
    z-index: 0;
    animation: peek 520ms var(--bounce) both;
  }
  .scan {
    position: relative;
    z-index: 1;
    padding: 12px;
    border-radius: var(--r-md);
    background: var(--surface);
    border: var(--bw) solid var(--line);
    box-shadow: var(--sticker);
  }
  .scan-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--font-display);
    font-size: 14px;
    font-weight: 600;
  }
  .scan-phase {
    flex: 1;
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
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--ok);
    color: var(--on-coral);
  }
  .bar {
    margin: 10px 0 8px;
    height: 8px;
    border-radius: 99px;
    background: var(--bg);
    border: 1.5px solid var(--line);
    overflow: hidden;
  }
  .fill {
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
      transform: translateX(340%);
    }
  }
  .scan-msg {
    font-size: 12px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
