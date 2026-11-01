<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    type SubtitleMatchPreview,
  } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import Icon from "./Icon.svelte";

  let {
    anilistId,
    title,
    onclose,
    onapplied,
  }: {
    anilistId: number;
    title: string;
    onclose: () => void;
    onapplied?: () => void;
  } = $props();

  let loading = $state(true);
  let applying = $state(false);
  let error = $state<string | null>(null);
  let preview = $state<SubtitleMatchPreview | null>(null);
  let defaultLang = $state("ja");
  let mode = $state<"link" | "rename">("link");
  let autoRetime = $state(app.jimakuAutoRetime);

  const FALLBACK_LANGS = [
    { value: "ja", label: "Japanese (ja)" },
    { value: "en", label: "English (en)" },
    { value: "zh", label: "Chinese (zh)" },
    { value: "ko", label: "Korean (ko)" },
    { value: "de", label: "German (de)" },
    { value: "fr", label: "French (fr)" },
    { value: "es", label: "Spanish (es)" },
  ];

  async function loadPreview() {
    loading = true;
    error = null;
    try {
      preview = await api.previewSubtitleMatches(anilistId, defaultLang);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    loadPreview();
  });

  async function apply() {
    if (!preview || preview.matches.length === 0 || applying) return;
    applying = true;
    error = null;
    try {
      const rep = await api.applySubtitleMatches({
        anilistId,
        mode,
        items: preview.matches,
        autoRetime,
      });
      let msg = `Linked ${rep.linked} subtitle${rep.linked === 1 ? "" : "s"}`;
      if (rep.renamed > 0) msg += ` (renamed ${rep.renamed})`;
      if (rep.retimed > 0) msg += ` · Auto-retimed ${rep.retimed} to video`;
      app.toast(msg + "!", "info", 5500);
      onapplied?.();
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      applying = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="sub-matcher-title">
  <header>
    <div>
      <h2 id="sub-matcher-title">Smart Subtitle Matcher</h2>
      <p class="faint">{title}</p>
    </div>
    <button class="btn btn-ghost btn-icon" onclick={onclose} aria-label="Close">
      <Icon name="x" />
    </button>
  </header>

  <div class="body">
    <div class="controls-bar">
      <div class="ctrl-group">
        <span class="ctrl-label">Action Mode</span>
        <div class="segmented">
          <button
            type="button"
            class="seg-btn"
            class:active={mode === "link"}
            onclick={() => (mode = "link")}
          >
            Link Only (Keep Filenames)
          </button>
          <button
            type="button"
            class="seg-btn"
            class:active={mode === "rename"}
            onclick={() => (mode = "rename")}
          >
            Rename for Jellyfin / Plex
          </button>
        </div>
      </div>

      <div class="ctrl-group">
        <label class="ctrl-label" for="sub-default-lang">Fallback Language</label>
        <select
          id="sub-default-lang"
          class="input select select-sm"
          bind:value={defaultLang}
          onchange={() => loadPreview()}
        >
          {#each FALLBACK_LANGS as l (l.value)}
            <option value={l.value}>{l.label}</option>
          {/each}
        </select>
      </div>

      <label class="retime-check" title="Automatically synchronize subtitle timestamps against each episode's video using Kura's Alass-style dynamic programming engine">
        <input type="checkbox" bind:checked={autoRetime} />
        <span>Auto-Retime to Video</span>
      </label>
    </div>

    {#if loading}
      <div class="state-box">
        <div class="spinner"></div>
        <p>Scanning series folders for subtitle files…</p>
      </div>
    {:else if error}
      <div class="error-box">
        <Icon name="alert" size={18} />
        <span>{error}</span>
      </div>
    {:else if preview}
      {#if preview.matches.length === 0}
        <div class="state-box">
          <Icon name="subtitles" size={28} />
          <h3>No Loose Subtitles Found</h3>
          <p class="muted">
            Drop `.srt`, `.ass`, or `.vtt` files (even with absolute numbering like `S02E25` or bare `25.ja.srt`) into your series folder or a `Subs` subfolder, or fetch them directly via <strong>Jimaku</strong>.
          </p>
        </div>
      {:else}
        <div class="section-head">
          <strong>Matched Subtitles ({preview.matches.length})</strong>
          <span class="faint">
            {mode === "link"
              ? "Passed directly to mpv via --sub-file without renaming your files"
              : "Renamed next to each episode video file"}
          </span>
        </div>

        <div class="match-list">
          {#each preview.matches as m (m.sourcePath)}
            <div class="match-row">
              <div class="ep-col">
                <span class="ep-badge">Ep {m.episodeNumber}</span>
                {#if m.resolutionNote}
                  <span class="abs-note">{m.resolutionNote}</span>
                {/if}
              </div>
              <div class="paths-col">
                <div class="src-line" title={m.sourcePath}>
                  <span class="lbl">Source:</span>
                  <code>{m.sourceName}</code>
                </div>
                <div class="dst-line" title={m.targetPath}>
                  <span class="lbl">{mode === "rename" ? "Rename →" : "Video →"}</span>
                  <code>{mode === "rename" ? m.targetName : m.videoName}</code>
                </div>
              </div>
              <div class="tags-col">
                <span class="lang-pill">{m.lang.toUpperCase()}{m.cc ? " · CC" : ""}</span>
                {#if m.alreadyLinked}
                  <span class="linked-pill">Linked</span>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {/if}

      {#if preview.skipped.length > 0}
        <details class="skipped-box">
          <summary>Skipped files ({preview.skipped.length})</summary>
          <div class="skipped-list">
            {#each preview.skipped as s (s.sourcePath)}
              <div class="skip-row">
                <code>{s.sourceName}</code>
                <span class="muted">{s.reason}</span>
              </div>
            {/each}
          </div>
        </details>
      {/if}
    {/if}
  </div>

  <footer>
    <button class="btn btn-ghost" onclick={onclose} disabled={applying}>Cancel</button>
    <div class="spacer"></div>
    <button
      class="btn btn-primary"
      disabled={applying || !preview || preview.matches.length === 0}
      onclick={apply}
    >
      {#if applying}
        <span class="spinner spinner-xs"></span>
        {autoRetime ? "Linking & Auto-Retiming…" : "Applying…"}
      {:else}
        <Icon name="check" size={15} />
        {mode === "rename" ? "Rename & Link" : "Link"} {preview?.matches.length ?? 0} Subtitle{(preview?.matches.length ?? 0) === 1 ? "" : "s"}
      {/if}
    </button>
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
    width: min(740px, calc(100vw - 40px));
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
    padding: 20px 24px 16px;
    border-bottom: 1.5px solid var(--line-soft);
  }
  .body {
    padding: 18px 24px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .controls-bar {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 14px;
    flex-wrap: wrap;
    padding: 12px 14px;
    background: var(--bg-card);
    border: 1.5px solid var(--line-soft);
    border-radius: var(--r-md);
  }
  .ctrl-group {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .ctrl-label {
    font-size: 11px;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--fg-muted);
  }
  .segmented {
    display: inline-flex;
    background: var(--bg-soft);
    border: 1.5px solid var(--line-soft);
    border-radius: var(--r-md);
    padding: 2px;
  }
  .seg-btn {
    border: 0;
    background: transparent;
    color: var(--fg-muted);
    font-size: 12px;
    font-weight: 700;
    padding: 5px 10px;
    border-radius: calc(var(--r-md) - 3px);
    cursor: pointer;
  }
  .seg-btn.active {
    background: var(--coral);
    color: var(--plum);
  }
  .select-sm {
    padding: 5px 10px;
    font-size: 12.5px;
    height: 32px;
  }
  .retime-check {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12.5px;
    font-weight: 700;
    padding: 6px 12px;
    border-radius: var(--r-pill);
    background: var(--bg-soft);
    border: 1.5px solid var(--coral);
    cursor: pointer;
    user-select: none;
  }
  .section-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    font-size: 13px;
  }
  .match-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .match-row {
    display: grid;
    grid-template-columns: 96px 1fr auto;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    background: var(--bg-card);
    border: 1.5px solid var(--line-soft);
    border-radius: var(--r-md);
  }
  .ep-col {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .ep-badge {
    display: inline-block;
    width: fit-content;
    font-size: 12px;
    font-weight: 800;
    padding: 2px 8px;
    border-radius: var(--r-pill);
    background: var(--coral);
    color: var(--plum);
  }
  .abs-note {
    font-size: 10.5px;
    color: var(--amber);
    font-weight: 700;
  }
  .paths-col {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 12px;
  }
  .src-line,
  .dst-line {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .lbl {
    color: var(--fg-muted);
    font-size: 11px;
    width: 58px;
    flex-shrink: 0;
  }
  .paths-col code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
  }
  .tags-col {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
  }
  .lang-pill {
    font-size: 11px;
    font-weight: 800;
    padding: 2px 8px;
    border-radius: var(--r-pill);
    background: var(--bg-soft);
    border: 1px solid var(--line-soft);
  }
  .linked-pill {
    font-size: 10.5px;
    font-weight: 700;
    color: var(--ok);
  }
  .skipped-box {
    font-size: 12.5px;
    padding: 10px 14px;
    border-radius: var(--r-md);
    background: var(--bg-soft);
    border: 1px solid var(--line-soft);
  }
  .skipped-box summary {
    cursor: pointer;
    font-weight: 700;
    color: var(--fg-muted);
  }
  .skipped-list {
    margin-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .skip-row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
  }
  .state-box {
    padding: 30px 18px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
  }
  .error-box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    background: hsl(0 70% 50% / 0.15);
    border: 1.5px solid var(--danger);
    color: var(--danger);
    font-size: 13px;
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

