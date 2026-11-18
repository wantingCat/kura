<script lang="ts">
  import { img } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { displayTitle, formatLabel } from "$lib/format";
  import Icon from "$lib/components/Icon.svelte";
  import FixMatchDialog from "$lib/components/FixMatchDialog.svelte";
  import Ja from "$lib/components/Ja.svelte";
  import { JA } from "$lib/ja";

  // Local accent vocabulary for this page (not in $lib/ja.ts).
  const JA_REVIEW = { unmatched: "未照合", lowConfidence: "要確認", allClear: "完了" } as const;

  let fixing = $state<{ query: string; groupId?: number; fromAnilistId?: number; subtitle: string } | null>(null);
  const uncertain = $derived(app.cards.filter((c) => c.needsReview));
</script>

<svelte:head><title>Needs review · Kura</title></svelte:head>

<div class="page review-page">
  <header class="head">
    <Ja text={JA.pages.review.ja} class="kanji-watermark" />
    <h1>Needs review <Ja text={JA.pages.review.ja} class="ja-sub" /></h1>
    <p class="muted">Folders Kura couldn't identify, and matches it isn't sure about. Fix them once and Kura keeps your choice.</p>
  </header>

  {#if app.unmatched.length === 0 && uncertain.length === 0}
    <div class="empty">
      <img class="mascot" src="/brand/cat.png" alt="" width="150" />
      <h2>All clear <Ja text={JA_REVIEW.allClear} class="ja-sub" /></h2>
      <p class="muted">Every folder has been matched.</p>
    </div>
  {/if}

  {#if app.unmatched.length > 0}
    <section class="block">
      <h2 class="section-title">Unmatched <Ja text={JA_REVIEW.unmatched} class="ja-sub" /> <span class="count">{app.unmatched.length}</span></h2>
      <div class="list">
        {#each app.unmatched as g, i (g.id)}
          <div class="item" style:--i={i}>
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
      <h2 class="section-title">Low confidence <Ja text={JA_REVIEW.lowConfidence} class="ja-sub" /> <span class="count">{uncertain.length}</span></h2>
      <div class="list">
        {#each uncertain as m, i (m.anilistId)}
          {@const cover = img(m.coverPath, m.coverUrl)}
          <div class="item" style:--i={i}>
            <a class="cover" href={`/anime/${m.anilistId}`} aria-label={displayTitle(m)} tabindex="-1">{#if cover}<img src={cover} alt="" loading="lazy" decoding="async" />{/if}</a>
            <div class="info">
              <a href={`/anime/${m.anilistId}`}><strong>{displayTitle(m)}</strong></a>
              {#if m.titleNative && m.titleNative !== displayTitle(m)}
                <Ja text={m.titleNative} block class="native" />
              {/if}
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
  .review-page {
    padding-top: calc(var(--nav-h) + var(--nav-inset) + 24px);
  }
  .head {
    position: relative;
    overflow: hidden;
    isolation: isolate;
    margin: 0 0 32px;
    padding-top: 12px;
  }
  .head > h1,
  .head > p {
    position: relative;
    z-index: 1;
  }
  .head h1 {
    font-size: 34px;
    font-weight: 700;
    letter-spacing: -0.02em;
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
    border-radius: var(--r-md);
    background: var(--surface);
    box-shadow: var(--shadow-sm);
    animation: fade-up 360ms var(--ease) both;
    animation-delay: calc(var(--i) * 30ms);
    transition: background var(--t-med) var(--ease), box-shadow var(--t-med) var(--ease);
  }
  .item:hover,
  .item:focus-within {
    background: var(--surface-2);
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
    background: color-mix(in srgb, var(--amber) 16%, var(--surface-2));
    color: var(--amber);
  }
  .cover {
    width: 44px;
    height: 64px;
    flex: none;
    border-radius: 8px;
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
    overflow-wrap: anywhere;
  }
  .info a:hover strong {
    color: var(--coral-hi);
  }
  .info a:focus-visible {
    outline: 2px solid var(--neon-violet);
    outline-offset: 2px;
    border-radius: 4px;
  }
  .info :global(.native) {
    font-size: 12px;
    color: var(--text-2);
  }
  .info .faint {
    font-size: 12.5px;
    color: var(--text-2);
    overflow-wrap: anywhere;
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
  @media (prefers-reduced-motion: reduce) {
    .item {
      animation: none;
    }
  }
</style>
