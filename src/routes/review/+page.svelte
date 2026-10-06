<script lang="ts">
  import { img } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, formatLabel } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import FixMatchDialog from "$lib/components/FixMatchDialog.svelte";

  let fixing = $state<{ query: string; groupId?: number; fromAnilistId?: number; subtitle: string } | null>(null);
  const uncertain = $derived(app.cards.filter((c) => c.needsReview));
</script>

<svelte:head><title>Needs review · Kura</title></svelte:head>

<div class="page">
  <header class="head">
    <h1>Needs review</h1>
    <p class="muted">Folders Kura couldn't identify, and matches it isn't sure about. Fix them once and Kura keeps your choice.</p>
  </header>

  {#if app.unmatched.length === 0 && uncertain.length === 0}
    <div class="empty">
      <img class="mascot" src="/brand/cat.png" alt="" width="150" />
      <h2>All clear</h2>
      <p class="muted">Every folder has been matched.</p>
    </div>
  {/if}

  {#if app.unmatched.length > 0}
    <section class="block">
      <h2 class="section-title">Unmatched <span class="count">{app.unmatched.length}</span></h2>
      <div class="list">
        {#each app.unmatched as g, i (g.id)}
          <div class="item card" style:--i={i}>
            <div class="icon warn"><Icon name="folder" size={20} /></div>
            <div class="info">
              <strong>{g.displayName}</strong>
              <span class="faint">
                {g.fileCount} file{g.fileCount === 1 ? "" : "s"} · searched as “{g.titleGuess}”
                {#if !g.attempted} · not searched yet (offline?){/if}
              </span>
              <div class="samples">
                {#each g.sampleFiles as f (f)}<code>{f}</code>{/each}
              </div>
            </div>
            <button
              class="btn btn-primary"
              onclick={() => (fixing = { query: g.titleGuess, groupId: g.id, subtitle: g.folderPath ?? g.displayName })}
              id={`review-match-${g.id}`}
            >
              <Icon name="search" size={15} /> Find match
            </button>
          </div>
        {/each}
      </div>
    </section>
  {/if}

  {#if uncertain.length > 0}
    <section class="block">
      <h2 class="section-title">Low confidence <span class="count">{uncertain.length}</span></h2>
      <div class="list">
        {#each uncertain as m, i (m.anilistId)}
          {@const cover = img(m.coverPath, m.coverUrl)}
          <div class="item card" style:--i={i}>
            <a class="cover" href={`/anime/${m.anilistId}`}>{#if cover}<img src={cover} alt="" />{/if}</a>
            <div class="info">
              <a href={`/anime/${m.anilistId}`}><strong>{displayTitle(m)}</strong></a>
              <span class="faint">{[formatLabel(m.format), m.seasonYear, `${m.ownedCount} files`].filter(Boolean).join(" · ")}</span>
            </div>
            <button
              class="btn"
              onclick={() =>
                (fixing = {
                  query: m.titleRomaji ?? displayTitle(m),
                  fromAnilistId: m.anilistId,
                  subtitle: `Currently matched to “${displayTitle(m)}”`,
                })}
            >
              <Icon name="link" size={15} /> Change
            </button>
          </div>
        {/each}
      </div>
    </section>
  {/if}
</div>

{#if fixing}
  <FixMatchDialog
    initialQuery={fixing.query}
    groupId={fixing.groupId}
    fromAnilistId={fixing.fromAnilistId}
    subtitle={fixing.subtitle}
    onclose={() => (fixing = null)}
  />
{/if}

<style>
  .head {
    margin: 12px 0 32px;
  }
  .head h1 {
    font-size: 34px;
    font-weight: 600;
  }
  .head p {
    margin-top: 6px;
    max-width: 640px;
  }
  .block {
    margin-bottom: 36px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 16px;
    animation: fade-up 360ms var(--ease) both;
    animation-delay: calc(var(--i) * 30ms);
  }
  .icon {
    width: 44px;
    height: 44px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: var(--r-md);
  }
  .icon.warn {
    border: var(--bw) solid var(--line);
    background: var(--amber);
    color: var(--on-coral);
    box-shadow: 2px 2px 0 var(--line);
  }
  .cover {
    width: 44px;
    height: 64px;
    flex: none;
    border-radius: 8px;
    border: 1.5px solid var(--line);
    overflow: hidden;
    background: var(--surface-2);
  }
  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .info strong {
    font-weight: 600;
  }
  .info a:hover strong {
    color: var(--coral);
  }
  .info .faint {
    font-size: 12.5px;
  }
  .samples {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 6px;
  }
  code {
    font-family: ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 11.5px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 60px 20px 80px;
    text-align: center;
  }
  .empty .mascot {
    margin-bottom: 4px;
    animation: peek 600ms var(--bounce) both;
  }
  .empty h2 {
    font-size: 24px;
  }
</style>
