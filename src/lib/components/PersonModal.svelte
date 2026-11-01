<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, img, type PersonDetail } from "$lib/api";
  import { displayTitle } from "$lib/format";
  import Icon from "./Icon.svelte";

  let {
    staffId,
    initialName,
    onclose,
  }: {
    staffId: number;
    initialName: string;
    onclose: () => void;
  } = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);
  let person = $state<PersonDetail | null>(null);
  let onlyLibrary = $state(false);

  onMount(async () => {
    try {
      person = await api.getPersonWorks(staffId);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  const libraryCount = $derived(person?.works.filter((w) => w.inLibrary).length ?? 0);
  const visibleWorks = $derived(
    person
      ? onlyLibrary
        ? person.works.filter((w) => w.inLibrary)
        : person.works
      : [],
  );

  function openShow(anilistId: number) {
    onclose();
    goto(`/anime/${anilistId}`);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="person-modal-title">
  <header>
    <div class="person-head">
      {#if person?.imageUrl}
        <img class="avatar" src={person.imageUrl} alt={person.nameFull} />
      {/if}
      <div>
        <h2 id="person-modal-title">{person?.nameFull || initialName}</h2>
        {#if person?.nameNative}
          <p class="native">{person.nameNative}</p>
        {/if}
        {#if person?.occupations.length}
          <div class="occ-pills">
            {#each person.occupations as occ (occ)}
              <span class="badge">{occ}</span>
            {/each}
          </div>
        {/if}
      </div>
    </div>
    <button class="btn btn-ghost btn-icon" onclick={onclose} aria-label="Close">
      <Icon name="x" />
    </button>
  </header>

  <div class="body">
    {#if loading}
      <div class="state-box">
        <div class="spinner"></div>
        <p>Loading credits for <strong>{initialName}</strong>…</p>
      </div>
    {:else if error}
      <div class="state-box">
        <p class="muted">{error}</p>
      </div>
    {:else if person}
      <div class="filter-bar">
        <span class="count-label">
          <strong>{visibleWorks.length}</strong> anime credit{visibleWorks.length === 1 ? "" : "s"}
          {#if libraryCount > 0}
            · <span class="lib-count">{libraryCount} in your library</span>
          {/if}
        </span>
        {#if libraryCount > 0 && person.works.length > libraryCount}
          <label class="toggle-chip">
            <input type="checkbox" bind:checked={onlyLibrary} />
            <span>In Library Only ({libraryCount})</span>
          </label>
        {/if}
      </div>

      {#if visibleWorks.length === 0}
        <div class="state-box">
          <p class="muted">No matching anime works found.</p>
        </div>
      {:else}
        <div class="works-grid">
          {#each visibleWorks as w (`${w.anilistId}-${w.role}-${w.characterName ?? ""}`)}
            {@const poster = img(w.coverPath, w.coverUrl)}
            <button
              type="button"
              class="work-card"
              class:in-lib={w.inLibrary}
              onclick={() => openShow(w.anilistId)}
            >
              <div class="work-poster">
                {#if poster}
                  <img src={poster} alt="" loading="lazy" />
                {/if}
                {#if w.inLibrary}
                  <span class="lib-tag">IN LIBRARY</span>
                {/if}
              </div>
              <div class="work-info">
                <strong class="work-title">{displayTitle(w)}</strong>
                <span class="work-meta">
                  {#if w.seasonYear}{w.seasonYear} · {/if}{w.role}
                </span>
                {#if w.characterName}
                  <div class="char-chip">
                    {#if w.characterImage}
                      <img src={w.characterImage} alt="" loading="lazy" />
                    {/if}
                    <span>as <strong>{w.characterName}</strong></span>
                  </div>
                {/if}
              </div>
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  </div>

  <footer>
    <button
      class="btn btn-ghost btn-sm"
      onclick={() => openUrl(`https://anilist.co/staff/${staffId}`)}
    >
      <Icon name="external" size={14} /> View on AniList
    </button>
    <div class="spacer"></div>
    <button class="btn" onclick={onclose}>Close</button>
  </footer>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: hsl(0 0% 0% / 0.65);
    backdrop-filter: blur(4px);
  }
  .dialog {
    position: fixed;
    z-index: 51;
    top: 7vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(760px, calc(100vw - 40px));
    max-height: 86vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-xl);
    box-shadow: 6px 6px 0 var(--coral);
  }
  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 20px 24px 16px;
    border-bottom: 1.5px solid var(--line-soft);
  }
  .person-head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .avatar {
    width: 60px;
    height: 60px;
    border-radius: 50%;
    object-fit: cover;
    border: 2px solid var(--line);
  }
  .native {
    font-size: 13px;
    color: var(--fg-muted);
    margin-top: 2px;
  }
  .occ-pills {
    display: flex;
    gap: 6px;
    margin-top: 6px;
    flex-wrap: wrap;
  }
  .body {
    padding: 18px 24px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .filter-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
  }
  .lib-count {
    color: var(--coral);
    font-weight: 800;
  }
  .toggle-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 700;
    padding: 4px 10px;
    border-radius: var(--r-pill);
    background: var(--bg-soft);
    border: 1.5px solid var(--line-soft);
    cursor: pointer;
  }
  .works-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(310px, 1fr));
    gap: 10px;
  }
  .work-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 11px;
    text-align: left;
    background: var(--bg-card);
    border: 1.5px solid var(--line-soft);
    border-radius: var(--r-md);
    color: var(--fg);
    cursor: pointer;
    transition: transform 140ms var(--ease), border-color 140ms var(--ease);
  }
  .work-card:hover {
    transform: translateY(-2px);
    border-color: var(--coral);
  }
  .work-card.in-lib {
    border-color: var(--coral);
  }
  .work-poster {
    position: relative;
    width: 48px;
    height: 68px;
    border-radius: var(--r-sm);
    overflow: hidden;
    background: var(--bg-soft);
    flex-shrink: 0;
  }
  .work-poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .lib-tag {
    position: absolute;
    bottom: 2px;
    left: 2px;
    right: 2px;
    font-size: 8px;
    font-weight: 900;
    text-align: center;
    background: var(--coral);
    color: var(--plum);
    border-radius: 3px;
    padding: 1px 2px;
  }
  .work-info {
    min-width: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .work-title {
    font-size: 13px;
    line-height: 1.25;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .work-meta {
    font-size: 11px;
    color: var(--fg-muted);
  }
  .char-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    font-size: 11.5px;
    color: var(--fg-muted);
  }
  .char-chip img {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    object-fit: cover;
  }
  .state-box {
    padding: 28px 16px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 24px;
    border-top: 1.5px solid var(--line-soft);
  }
  .spacer {
    flex: 1;
  }
</style>

