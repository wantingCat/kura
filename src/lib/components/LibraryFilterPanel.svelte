<script lang="ts">
  import type { MediaCard } from "$lib/api";
  import Icon from "$lib/components/Icon.svelte";
  import { AIRING, airingOk, facetCounts, filters, type FacetKey } from "$lib/libraryFilters.svelte";

  let { entries }: { entries: MediaCard[] } = $props();

  const genres = $derived(facetCounts(entries, "genres"));
  const studios = $derived(facetCounts(entries, "studios"));
  const tags = $derived(facetCounts(entries, "tags"));
  const airingCounts = $derived(
    Object.fromEntries(AIRING.map((a) => [a.key, entries.filter((c) => airingOk(c, a.key)).length])),
  );

  let studioQuery = $state("");
  let tagQuery = $state("");
  let showAllStudios = $state(false);
  let showAllTags = $state(false);

  const STUDIO_LIMIT = 14;
  const TAG_LIMIT = 24;

  /** Selected values always stay visible, then search / top-N. */
  function visible(list: [string, number][], facet: FacetKey, q: string, all: boolean, limit: number) {
    const needle = q.trim().toLowerCase();
    const hits = needle ? list.filter(([v]) => v.toLowerCase().includes(needle)) : list;
    const shown = needle || all ? hits : hits.slice(0, limit);
    const missing = filters[facet].filter((v) => !shown.some(([s]) => s === v)).map((v) => [v, 0] as [string, number]);
    return [...missing, ...shown];
  }

  const studioList = $derived(visible(studios, "studios", studioQuery, showAllStudios, STUDIO_LIMIT));
  const tagList = $derived(visible(tags, "tags", tagQuery, showAllTags, TAG_LIMIT));
</script>

<div class="panel card" id="library-filter-panel">
  <div class="panel-head">
    <h3>Filters</h3>
    {#if filters.advancedCount > 0}
      <button class="btn btn-ghost btn-sm" onclick={() => filters.clearAdvanced()} id="filters-clear-advanced">
        Clear filters
      </button>
    {/if}
    <button class="btn btn-ghost btn-sm btn-icon" onclick={() => (filters.panelOpen = false)} aria-label="Close filters">
      <Icon name="x" size={14} />
    </button>
  </div>

  <section class="group">
    <h4>Airing status</h4>
    <div class="pills" role="radiogroup" aria-label="Airing status">
      {#each AIRING as a (a.key)}
        <button
          class="pill"
          class:on={filters.airing === a.key}
          role="radio"
          aria-checked={filters.airing === a.key}
          onclick={() => (filters.airing = a.key)}
          id={`filter-airing-${a.key}`}
        >
          {a.label}<span class="n">{airingCounts[a.key]}</span>
        </button>
      {/each}
    </div>
  </section>

  {#if genres.length}
    <section class="group">
      <h4>Genres <small>match all selected</small></h4>
      <div class="pills">
        {#each genres as [g, n] (g)}
          <button
            class="pill"
            class:on={filters.has("genres", g)}
            aria-pressed={filters.has("genres", g)}
            onclick={() => filters.toggle("genres", g)}
          >
            {g}<span class="n">{n}</span>
          </button>
        {/each}
      </div>
    </section>
  {/if}

  {#if studios.length}
    <section class="group">
      <div class="group-head">
        <h4>Studios <small>match any selected</small></h4>
        <label class="mini-search">
          <Icon name="search" size={13} />
          <input bind:value={studioQuery} placeholder="Find a studio…" aria-label="Find a studio" id="filter-studio-search" />
        </label>
      </div>
      <div class="pills">
        {#each studioList as [s, n] (s)}
          <button
            class="pill"
            class:on={filters.has("studios", s)}
            aria-pressed={filters.has("studios", s)}
            onclick={() => filters.toggle("studios", s)}
          >
            {s}{#if n}<span class="n">{n}</span>{/if}
          </button>
        {:else}
          <span class="none">No studio matches “{studioQuery}”.</span>
        {/each}
        {#if !studioQuery && studios.length > STUDIO_LIMIT}
          <button class="more" onclick={() => (showAllStudios = !showAllStudios)}>
            {showAllStudios ? "Show fewer" : `+${studios.length - STUDIO_LIMIT} more`}
          </button>
        {/if}
      </div>
    </section>
  {/if}

  {#if tags.length}
    <section class="group">
      <div class="group-head">
        <h4>Tags <small>match all selected</small></h4>
        <label class="mini-search">
          <Icon name="search" size={13} />
          <input bind:value={tagQuery} placeholder="Find a tag…" aria-label="Find a tag" id="filter-tag-search" />
        </label>
      </div>
      <div class="pills">
        {#each tagList as [t, n] (t)}
          <button
            class="pill"
            class:on={filters.has("tags", t)}
            aria-pressed={filters.has("tags", t)}
            onclick={() => filters.toggle("tags", t)}
          >
            {t}{#if n}<span class="n">{n}</span>{/if}
          </button>
        {:else}
          <span class="none">No tag matches “{tagQuery}”.</span>
        {/each}
        {#if !tagQuery && tags.length > TAG_LIMIT}
          <button class="more" onclick={() => (showAllTags = !showAllTags)}>
            {showAllTags ? "Show fewer" : `+${tags.length - TAG_LIMIT} more`}
          </button>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .panel {
    padding: 18px 20px 6px;
    margin-bottom: 18px;
    animation: fade-up 240ms var(--ease);
  }
  .panel-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }
  .panel-head h3 {
    font-size: 17px;
    margin-right: auto;
  }
  .group {
    padding: 12px 0 14px;
    border-top: 1.5px dashed var(--border);
  }
  .group:first-of-type {
    border-top: 0;
  }
  .group-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  h4 {
    font-size: 13.5px;
    margin-bottom: 10px;
    color: var(--text);
  }
  h4 small {
    margin-left: 6px;
    font-family: var(--font);
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-3);
  }
  .group-head h4 {
    margin-bottom: 0;
  }
  .group-head + .pills {
    margin-top: 10px;
  }
  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    padding: 5px 11px;
    border-radius: 999px;
    border: 1.5px solid var(--border-strong);
    background: var(--bg-elev);
    color: var(--text-2);
    font: inherit;
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    overflow-wrap: anywhere;
    text-align: left;
    transition:
      background var(--t-fast) var(--ease),
      border-color var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease),
      transform var(--t-fast) var(--bounce);
  }
  .pill:hover {
    border-color: var(--coral);
    color: var(--text);
    transform: translateY(-1px);
  }
  .pill.on {
    background: var(--coral);
    border-color: var(--line);
    color: var(--on-coral);
    box-shadow: 2px 2px 0 var(--line);
  }
  .n {
    font-size: 10.5px;
    font-weight: 800;
    opacity: 0.65;
  }
  .more {
    padding: 5px 8px;
    border: 0;
    background: none;
    color: var(--coral);
    font: inherit;
    font-size: 12.5px;
    font-weight: 700;
    cursor: pointer;
  }
  .more:hover {
    text-decoration: underline;
  }
  .none {
    font-size: 12.5px;
    color: var(--text-3);
  }
  .mini-search {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1.5px solid var(--border-strong);
    background: var(--bg-elev);
    color: var(--text-3);
  }
  .mini-search:focus-within {
    border-color: var(--coral);
    color: var(--coral);
  }
  .mini-search input {
    width: 150px;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
  }
</style>
