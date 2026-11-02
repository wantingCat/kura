<script lang="ts">
  import type { LookupResult } from "$lib/api";

  let {
    result,
    anchorX,
    anchorY,
    pinned = false,
    onClose,
  }: {
    result: LookupResult;
    anchorX: number;
    anchorY: number;
    pinned?: boolean;
    onClose: () => void;
  } = $props();

  let activeTab = $state<"terms" | "kanji">("terms");

  // Reset tab when lookup result changes
  $effect(() => {
    void result.matchedText;
    if (result.terms.length > 0) {
      activeTab = "terms";
    } else if (result.kanji.length > 0) {
      activeTab = "kanji";
    }
  });

  // Compute clamped popup position so it never overflows the viewport
  let popupLeft = $derived.by(() => {
    const vw = typeof window !== "undefined" ? window.innerWidth : 1280;
    const width = 430;
    return Math.max(16, Math.min(anchorX - width / 2, vw - width - 16));
  });

  let popupBottom = $derived.by(() => {
    const vh = typeof window !== "undefined" ? window.innerHeight : 720;
    // Place above the subtitle line by default
    const fromBottom = vh - anchorY + 18;
    return Math.max(24, Math.min(fromBottom, vh - 360));
  });

  function splitMoras(reading: string): string[] {
    const small = new Set(["ゃ", "ゅ", "ょ", "ぁ", "ぃ", "ぅ", "ぇ", "ぉ", "ャ", "ュ", "ョ", "ァ", "ィ", "ゥ", "ェ", "ォ"]);
    const chars = Array.from(reading);
    const out: string[] = [];
    for (let i = 0; i < chars.length; i++) {
      if (i + 1 < chars.length && small.has(chars[i + 1])) {
        out.push(chars[i] + chars[i + 1]);
        i++;
      } else {
        out.push(chars[i]);
      }
    }
    return out;
  }

  function isMoraHigh(moraIndex: number, downstep: number): boolean {
    if (downstep === 0) {
      return moraIndex > 0;
    }
    if (downstep === 1) {
      return moraIndex === 0;
    }
    return moraIndex > 0 && moraIndex < downstep;
  }

  function speakJapanese(text: string) {
    if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
    try {
      window.speechSynthesis.cancel();
      const u = new SpeechSynthesisUtterance(text);
      u.lang = "ja-JP";
      u.rate = 0.95;
      window.speechSynthesis.speak(u);
    } catch {
      // ignore if speech synthesis unavailable
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="yomitan-popup"
  class:pinned
  style="left: {popupLeft}px; bottom: {popupBottom}px;"
  onmousedown={(e) => e.stopPropagation()}
  onclick={(e) => e.stopPropagation()}
  onwheel={(e) => e.stopPropagation()}
>
  <header class="yt-header">
    <div class="yt-tabs">
      <button
        type="button"
        class="yt-tab"
        class:active={activeTab === "terms"}
        onclick={() => (activeTab = "terms")}
      >
        Terms <span class="badge">{result.terms.length}</span>
      </button>
      <button
        type="button"
        class="yt-tab"
        class:active={activeTab === "kanji"}
        onclick={() => (activeTab = "kanji")}
      >
        Kanji <span class="badge">{result.kanji.length}</span>
      </button>
    </div>

    <div class="yt-status">
      <span class="pin-hint">{pinned ? "Pinned · Esc" : "Click word to pin"}</span>
      {#if pinned}
        <button type="button" class="yt-close" onclick={onClose} title="Close (Esc)">✕</button>
      {/if}
    </div>
  </header>

  <div class="yt-body">
    {#if activeTab === "terms"}
      {#if result.terms.length === 0}
        <div class="yt-empty">
          No term entries found for <strong>「{result.matchedText}」</strong>.
        </div>
      {:else}
        {#each result.terms as term}
          <article class="yt-entry">
            <div class="yt-entry-top">
              <div class="yt-headword">
                {#each term.furigana as seg}
                  {#if seg.reading}
                    <ruby>{seg.text}<rt>{seg.reading}</rt></ruby>
                  {:else}
                    <span>{seg.text}</span>
                  {/if}
                {/each}
              </div>

              <div class="yt-actions">
                <button
                  type="button"
                  class="yt-icon-btn"
                  title="Pronounce"
                  onclick={() => speakJapanese(term.reading || term.expression)}
                >
                  🔊
                </button>
              </div>
            </div>

            {#if term.deinflectionReasons.length > 0}
              <div class="yt-reasons">
                <span class="matched-surface">{term.matchedText}</span>
                {#each term.deinflectionReasons as reason}
                  <span class="reason-arrow">→</span>
                  <span class="reason-pill">{reason}</span>
                {/each}
              </div>
            {/if}

            <div class="yt-tags-row">
              {#each term.definitionTags as tag}
                <span class="yt-tag pos">{tag}</span>
              {/each}
              {#each term.frequencies as freq}
                <span class="yt-tag freq">Freq: {freq}</span>
              {/each}
              {#if term.pitches.length > 0}
                {#each term.pitches.slice(0, 2) as pitch}
                  {@const moras = splitMoras(pitch.reading || term.reading || term.expression)}
                  <span class="yt-pitch" title="Pitch accent [{pitch.downstep}]">
                    {#each moras as mora, mIdx}
                      {@const high = isMoraHigh(mIdx, pitch.downstep)}
                      {@const nextHigh = mIdx + 1 < moras.length ? isMoraHigh(mIdx + 1, pitch.downstep) : false}
                      <span
                        class="mora"
                        class:high
                        class:low={!high}
                        class:drop={high && !nextHigh && pitch.downstep === mIdx + 1}
                      >{mora}</span>
                    {/each}
                    <sup class="pitch-num">[{pitch.downstep}]</sup>
                  </span>
                {/each}
              {/if}
              <span class="yt-tag dict">{term.dictionaryTitle}</span>
            </div>

            <ol class="yt-glossary">
              {#each term.glossaries as gloss}
                <li>{gloss}</li>
              {/each}
            </ol>
          </article>
        {/each}
      {/if}
    {:else}
      {#if result.kanji.length === 0}
        <div class="yt-empty">No Kanji entries in matched text.</div>
      {:else}
        {#each result.kanji as k}
          <article class="yt-kanji-card">
            <div class="yt-kanji-char">{k.character}</div>
            <div class="yt-kanji-info">
              <div class="yt-kanji-meanings">{k.meanings.join(", ")}</div>
              {#if k.onyomi.length > 0}
                <div class="yt-reading-row">
                  <span class="r-label">On</span>
                  <span class="r-val">{k.onyomi.join(" · ")}</span>
                </div>
              {/if}
              {#if k.kunyomi.length > 0}
                <div class="yt-reading-row">
                  <span class="r-label">Kun</span>
                  <span class="r-val">{k.kunyomi.join(" · ")}</span>
                </div>
              {/if}
              <div class="yt-tags-row">
                {#each k.tags as tag}
                  <span class="yt-tag pos">{tag}</span>
                {/each}
                {#if k.stats.strokes}
                  <span class="yt-tag freq">{k.stats.strokes} strokes</span>
                {/if}
                {#if k.stats.jlpt}
                  <span class="yt-tag freq">JLPT {k.stats.jlpt}</span>
                {/if}
                <span class="yt-tag dict">{k.dictionaryTitle}</span>
              </div>
            </div>
          </article>
        {/each}
      {/if}
    {/if}
  </div>
</div>

<style>
  .yomitan-popup {
    position: fixed;
    z-index: 9600;
    width: 430px;
    max-width: calc(100vw - 32px);
    max-height: 340px;
    display: flex;
    flex-direction: column;
    background: rgba(20, 12, 13, 0.96);
    backdrop-filter: blur(18px);
    border: 1px solid rgba(244, 162, 97, 0.35);
    border-radius: 12px;
    box-shadow:
      0 20px 50px rgba(0, 0, 0, 0.78),
      0 0 0 1px rgba(255, 255, 255, 0.05);
    color: #f7ede2;
    overflow: hidden;
    user-select: text;
    animation: yt-pop 110ms cubic-bezier(0.16, 1, 0.3, 1);
  }

  .yomitan-popup.pinned {
    border-color: rgba(231, 111, 81, 0.75);
    box-shadow:
      0 22px 56px rgba(0, 0, 0, 0.85),
      0 0 0 1px rgba(231, 111, 81, 0.35);
  }

  @keyframes yt-pop {
    from {
      opacity: 0;
      transform: translateY(6px) scale(0.98);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .yt-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 7px 12px;
    background: rgba(255, 255, 255, 0.03);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    flex-shrink: 0;
  }

  .yt-tabs {
    display: flex;
    gap: 6px;
  }

  .yt-tab {
    background: transparent;
    border: none;
    color: rgba(247, 237, 226, 0.6);
    font-size: 12px;
    font-weight: 600;
    padding: 4px 9px;
    border-radius: 6px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    transition: all 120ms ease;
  }

  .yt-tab:hover {
    color: #f7ede2;
    background: rgba(255, 255, 255, 0.06);
  }

  .yt-tab.active {
    color: #140c0d;
    background: #f4a261;
  }

  .yt-tab .badge {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 99px;
    background: rgba(0, 0, 0, 0.22);
  }

  .yt-status {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .pin-hint {
    font-size: 11px;
    color: rgba(247, 237, 226, 0.5);
  }

  .yt-close {
    background: rgba(255, 255, 255, 0.08);
    border: none;
    color: #f7ede2;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    font-size: 11px;
    cursor: pointer;
    display: grid;
    place-items: center;
  }

  .yt-close:hover {
    background: rgba(231, 111, 81, 0.35);
  }

  .yt-body {
    overflow-y: auto;
    padding: 10px 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .yt-entry {
    padding-bottom: 10px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.07);
  }

  .yt-entry:last-child {
    padding-bottom: 0;
    border-bottom: none;
  }

  .yt-entry-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }

  .yt-headword {
    font-size: 24px;
    font-weight: 700;
    line-height: 1.35;
    color: #fff;
    letter-spacing: 0.02em;
  }

  .yt-headword ruby {
    ruby-align: center;
  }

  .yt-headword rt {
    font-size: 11px;
    font-weight: 500;
    color: #f4a261;
  }

  .yt-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .yt-icon-btn {
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid rgba(255, 255, 255, 0.1);
    color: #f7ede2;
    width: 28px;
    height: 26px;
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
  }

  .yt-icon-btn:hover {
    background: rgba(255, 255, 255, 0.14);
  }

  .yt-reasons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
    font-size: 11px;
  }

  .matched-surface {
    color: rgba(247, 237, 226, 0.65);
  }

  .reason-arrow {
    color: rgba(247, 237, 226, 0.4);
  }

  .reason-pill {
    background: rgba(244, 162, 97, 0.16);
    border: 1px solid rgba(244, 162, 97, 0.35);
    color: #f4a261;
    padding: 1px 6px;
    border-radius: 4px;
    font-weight: 600;
  }

  .yt-tags-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    margin-top: 6px;
  }

  .yt-tag {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .yt-tag.pos {
    background: rgba(231, 111, 81, 0.2);
    color: #f4a261;
  }

  .yt-tag.freq {
    background: rgba(132, 169, 140, 0.22);
    color: #a3d9a5;
  }

  .yt-tag.dict {
    background: rgba(255, 255, 255, 0.07);
    color: rgba(247, 237, 226, 0.6);
  }

  .yt-pitch {
    display: inline-flex;
    align-items: baseline;
    background: rgba(255, 255, 255, 0.06);
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 12px;
  }

  .yt-pitch .mora {
    position: relative;
    padding: 0 1px;
  }

  .yt-pitch .mora.high {
    border-top: 2px solid #f4a261;
  }

  .yt-pitch .mora.low {
    border-bottom: 1px solid rgba(247, 237, 226, 0.35);
  }

  .yt-pitch .mora.drop {
    border-right: 2px solid #f4a261;
  }

  .pitch-num {
    font-size: 9px;
    color: #f4a261;
    margin-left: 3px;
  }

  .yt-glossary {
    margin: 7px 0 0;
    padding-left: 18px;
    font-size: 13px;
    line-height: 1.48;
    color: rgba(247, 237, 226, 0.92);
  }

  .yt-glossary li + li {
    margin-top: 3px;
  }

  .yt-kanji-card {
    display: flex;
    gap: 14px;
    align-items: flex-start;
    padding-bottom: 10px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.07);
  }

  .yt-kanji-card:last-child {
    border-bottom: none;
    padding-bottom: 0;
  }

  .yt-kanji-char {
    font-size: 42px;
    line-height: 1;
    font-weight: 700;
    color: #fff;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 10px;
    width: 64px;
    height: 64px;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }

  .yt-kanji-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .yt-kanji-meanings {
    font-size: 14px;
    font-weight: 600;
    color: #fff;
  }

  .yt-reading-row {
    font-size: 12px;
    display: flex;
    gap: 6px;
    align-items: baseline;
  }

  .r-label {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    color: #f4a261;
    min-width: 28px;
  }

  .r-val {
    color: rgba(247, 237, 226, 0.85);
  }

  .yt-empty {
    font-size: 13px;
    color: rgba(247, 237, 226, 0.65);
    padding: 12px 0;
    text-align: center;
  }
</style>

