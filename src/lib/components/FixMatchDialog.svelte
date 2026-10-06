<script lang="ts">
  import { onMount } from "svelte";
  import { api, type SearchResult } from "$lib/api";
  import { formatLabel, statusLabel } from "$lib/format";
  import Icon from "./Icon.svelte";

  let {
    initialQuery,
    groupId,
    fromAnilistId,
    subtitle,
    onclose,
    ondone,
  }: {
    initialQuery: string;
    groupId?: number;
    fromAnilistId?: number;
    subtitle?: string;
    onclose: () => void;
    ondone?: (newId: number) => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let query = $state(initialQuery);
  let results = $state<SearchResult[]>([]);
  let loading = $state(false);
  let saving = $state<number | null>(null);
  let error = $state<string | null>(null);
  let input: HTMLInputElement | undefined = $state();
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function search(q: string) {
    if (!q.trim()) {
      results = [];
      return;
    }
    loading = true;
    error = null;
    try {
      results = await api.searchAnilist(q.trim());
    } catch (e) {
      error = `Search failed: ${e}`;
    } finally {
      loading = false;
    }
  }

  function onInput() {
    clearTimeout(timer);
    timer = setTimeout(() => search(query), 400);
  }

  async function pick(r: SearchResult) {
    saving = r.anilistId;
    error = null;
    try {
      await api.fixMatch(r.anilistId, { groupId, fromAnilistId });
      ondone?.(r.anilistId);
      onclose();
    } catch (e) {
      error = `Couldn't apply match: ${e}`;
    } finally {
      saving = null;
    }
  }

  onMount(() => {
    search(initialQuery);
    input?.focus();
    input?.select();
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="fix-title">
  <header>
    <div>
      <h2 id="fix-title">Fix match</h2>
      {#if subtitle}<p class="faint">{subtitle}</p>{/if}
    </div>
    <button class="btn btn-ghost btn-icon" onclick={onclose} aria-label="Close" id="fix-close">
      <Icon name="x" />
    </button>
  </header>

  <div class="search">
    <Icon name="search" size={16} />
    <input
      bind:this={input}
      bind:value={query}
      oninput={onInput}
      onkeydown={(e) => e.key === "Enter" && search(query)}
      class="input"
      placeholder="Search AniList — English, romaji or 日本語"
      id="fix-search"
    />
    {#if loading}<div class="spinner"></div>{/if}
  </div>

  {#if error}<p class="error">{error}</p>{/if}

  <div class="results">
    {#each results as r (r.anilistId)}
      <button class="result" onclick={() => pick(r)} disabled={saving !== null} id={`fix-result-${r.anilistId}`}>
        <div class="thumb">
          {#if r.coverUrl}<img src={r.coverUrl} alt="" loading="lazy" />{/if}
        </div>
        <div class="info">
          <strong>{r.titleEnglish || r.titleRomaji}</strong>
          {#if r.titleEnglish && r.titleRomaji}<span class="faint">{r.titleRomaji}</span>{/if}
          {#if r.titleNative}<span class="faint jp">{r.titleNative}</span>{/if}
          <div class="tags">
            {#if r.format}<span class="badge">{formatLabel(r.format)}</span>{/if}
            {#if r.seasonYear}<span class="badge">{r.seasonYear}</span>{/if}
            {#if r.episodes}<span class="badge">{r.episodes} eps</span>{/if}
            {#if r.status}<span class="badge">{statusLabel(r.status)}</span>{/if}
          </div>
        </div>
        <div class="go">
          {#if saving === r.anilistId}
            <div class="spinner"></div>
          {:else}
            <Icon name="chevron-right" />
          {/if}
        </div>
      </button>
    {:else}
      {#if !loading}
        <p class="empty faint">No results. Try another title or the Japanese name.</p>
      {/if}
    {/each}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--line) 78%, transparent);
    z-index: 50;
    animation: fade-in var(--t-med) var(--ease);
  }
  .dialog {
    position: fixed;
    z-index: 51;
    top: 8vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(680px, calc(100vw - 48px));
    max-height: 84vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-xl);
    box-shadow: 6px 6px 0 var(--coral);
    animation: pop 340ms var(--bounce);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(12px) scale(0.98);
    }
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    padding: 22px 22px 8px 26px;
  }
  h2 {
    font-size: 22px;
  }
  header p {
    margin-top: 4px;
    font-size: 13px;
    word-break: break-all;
  }
  .search {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 10px 22px 6px;
  }
  .search > :global(svg) {
    position: absolute;
    left: 14px;
    color: var(--text-3);
  }
  .search .input {
    flex: 1;
    height: 46px;
    padding-left: 40px;
    font-size: 15px;
  }
  .search .spinner {
    position: absolute;
    right: 14px;
  }
  .error {
    margin: 6px 26px;
    color: var(--danger);
    font-size: 13px;
  }
  .results {
    overflow-y: auto;
    padding: 8px 14px 18px;
  }
  .result {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px;
    border: var(--bw) solid transparent;
    border-radius: var(--r-md);
    background: transparent;
    text-align: left;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
    animation: fade-up 300ms var(--ease) both;
  }
  .result:hover {
    background: var(--surface);
    border-color: var(--line);
  }
  .result:hover .go {
    color: var(--coral);
    transform: translateX(2px);
  }
  .thumb {
    width: 54px;
    height: 78px;
    flex: none;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    overflow: hidden;
    background: var(--surface-2);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .info strong {
    font-weight: 600;
  }
  .info span {
    font-size: 12.5px;
  }
  .tags {
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
    margin-top: 6px;
  }
  .go {
    color: var(--text-3);
    transition: all var(--t-fast) var(--ease);
  }
  .empty {
    padding: 30px;
    text-align: center;
  }
</style>
