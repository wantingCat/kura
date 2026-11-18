<!--
  Ja — renders a Japanese accent span with correct language tagging.

  Usage:
    <Ja text="新着" />                         decorative duplicate of nearby English (aria-hidden)
    <Ja text="月" label="Monday" />            Japanese REPLACES English → English kept for screen readers
    <Ja text="葬送のフリーレン" block class="neon-text" />

  - Always sets lang="ja" so screen readers / Yomitan / :lang(ja) fonts work.
  - Hidden entirely when Settings → 日本語アクセント is off, unless `essential`
    is set (then the English label is shown instead).
-->
<script lang="ts">
  import { app } from "$lib/app.svelte";

  interface Props {
    /** The Japanese text. */
    text: string | null | undefined;
    /** English equivalent. If given, the Japanese is not decorative: screen readers get this. */
    label?: string;
    /** Render as a block-level element. */
    block?: boolean;
    /** When accents are off, show `label` instead of hiding (for text that replaces English). */
    essential?: boolean;
    class?: string;
    title?: string;
  }

  let { text, label, block = false, essential = false, class: cls = "", title }: Props = $props();

  const on = $derived(app.japaneseAccents);
</script>

{#if text}
  {#if on}
    {#if label}
      <svelte:element
        this={block ? "div" : "span"}
        class="ja {cls}"
        lang="ja"
        title={title ?? label}
      ><span aria-hidden="true">{text}</span><span class="sr-only">{label}</span></svelte:element>
    {:else}
      <svelte:element
        this={block ? "div" : "span"}
        class="ja ja-deco {cls}"
        lang="ja"
        aria-hidden="true"
        {title}
      >{text}</svelte:element>
    {/if}
  {:else if essential && label}
    <svelte:element this={block ? "div" : "span"} class={cls}>{label}</svelte:element>
  {/if}
{/if}

<style>
  .ja {
    font-family: var(--font-jp);
  }
</style>
