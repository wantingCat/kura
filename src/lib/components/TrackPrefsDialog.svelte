<script lang="ts">
  import { onMount } from "svelte";
  import { api, type MediaTrackPref } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import {
    AUDIO_LANG_OPTIONS,
    SUB_LANG_OPTIONS,
    SUB_FALLBACK_OPTIONS,
    TRACK_PRESETS,
    describeTrackSummary,
    matchingPresetId,
    type TrackPreset,
  } from "$lib/languages";
  import Icon from "./Icon.svelte";

  let {
    anilistId,
    title,
    initialPref,
    onclose,
    onsave,
  }: {
    anilistId: number;
    title: string;
    initialPref: MediaTrackPref | null;
    onclose: () => void;
    onsave: (pref: MediaTrackPref | null) => void;
  } = $props();

  // Mode: either following global default or custom per-show override
  // svelte-ignore state_referenced_locally
  let isOverride = $state(initialPref !== null);
  // svelte-ignore state_referenced_locally
  let audio = $state(initialPref?.audioPref ?? app.playback.audioLang);
  // svelte-ignore state_referenced_locally
  let sub = $state(initialPref?.subPref ?? app.playback.subLang);
  // svelte-ignore state_referenced_locally
  let fallback = $state(initialPref?.subFallback ?? app.playback.subFallback);
  let saving = $state(false);
  let error = $state<string | null>(null);

  const globalSummary = $derived(
    describeTrackSummary(app.playback.audioLang, app.playback.subLang, app.playback.subFallback)
  );

  const currentPresetId = $derived(matchingPresetId(audio, sub, fallback));

  function applyPreset(p: TrackPreset) {
    isOverride = true;
    audio = p.audio;
    sub = p.sub;
    fallback = p.fallback;
  }

  async function save() {
    saving = true;
    error = null;
    try {
      if (!isOverride) {
        await api.setMediaTrackPref(anilistId, null);
        onsave(null);
      } else {
        const payload: MediaTrackPref = {
          audioPref: audio,
          subPref: sub,
          subFallback: sub === "off" ? "none" : fallback,
        };
        await api.setMediaTrackPref(anilistId, payload);
        onsave(payload);
      }
      onclose();
    } catch (e) {
      error = `Failed to save preferences: ${e}`;
    } finally {
      saving = false;
    }
  }

  async function resetToGlobal() {
    isOverride = false;
    audio = app.playback.audioLang;
    sub = app.playback.subLang;
    fallback = app.playback.subFallback;
    await save();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="track-pref-title">
  <header>
    <div>
      <h2 id="track-pref-title">Audio & Subtitles</h2>
      <p class="faint">{title}</p>
    </div>
    <button class="btn btn-ghost btn-icon" onclick={onclose} aria-label="Close" id="track-pref-close">
      <Icon name="x" />
    </button>
  </header>

  <div class="body">
    <!-- Mode selector: Global default vs Show-specific override -->
    <div class="mode-switch-group">
      <button
        class="mode-btn"
        class:active={!isOverride}
        onclick={() => (isOverride = false)}
        type="button"
        id="track-pref-mode-global"
      >
        <Icon name="volume" size={16} />
        <div class="mode-info">
          <strong>Use Global Default</strong>
          <small>{globalSummary}</small>
        </div>
      </button>

      <button
        class="mode-btn"
        class:active={isOverride}
        onclick={() => (isOverride = true)}
        type="button"
        id="track-pref-mode-override"
      >
        <Icon name="subtitles" size={16} />
        <div class="mode-info">
          <strong>Override For This Show</strong>
          <small>Customize audio and subtitle priorities for this series only.</small>
        </div>
      </button>
    </div>

    {#if isOverride}
      <div class="override-panel">
        <div class="subhead">
          <span class="subhead-title">Presets</span>
          <span class="faint">Quick selection</span>
        </div>

        <div class="preset-pills">
          {#each TRACK_PRESETS as p (p.id)}
            <button
              class="pill"
              class:on={currentPresetId === p.id}
              onclick={() => applyPreset(p)}
              type="button"
            >
              {p.name}
            </button>
          {/each}
        </div>

        <div class="fields-grid">
          <div class="field">
            <label for="track-dialog-audio">Audio Language</label>
            <select id="track-dialog-audio" class="input select" bind:value={audio}>
              {#each AUDIO_LANG_OPTIONS as opt}
                <option value={opt.value}>{opt.label}</option>
              {/each}
            </select>
          </div>

          <div class="field">
            <label for="track-dialog-sub">Primary Subtitles</label>
            <select id="track-dialog-sub" class="input select" bind:value={sub}>
              {#each SUB_LANG_OPTIONS as opt}
                <option value={opt.value}>{opt.label}</option>
              {/each}
            </select>
          </div>

          <div class="field" class:disabled={sub === "off"}>
            <label for="track-dialog-fallback">Fallback Subtitles</label>
            <select
              id="track-dialog-fallback"
              class="input select"
              disabled={sub === "off"}
              bind:value={fallback}
            >
              {#each SUB_FALLBACK_OPTIONS as opt}
                <option value={opt.value}>{opt.label}</option>
              {/each}
            </select>
            <small class="faint">Used if primary subtitle isn't found.</small>
          </div>
        </div>

        <div class="summary-box">
          <span class="summary-label">Selected Track Priority:</span>
          <strong>{describeTrackSummary(audio, sub, fallback)}</strong>
        </div>
      </div>
    {/if}

    {#if error}
      <div class="error">{error}</div>
    {/if}
  </div>

  <footer>
    {#if initialPref !== null}
      <button class="btn btn-ghost" onclick={resetToGlobal} disabled={saving} id="track-pref-reset">
        Reset to Global Default
      </button>
    {/if}
    <div class="footer-spacer"></div>
    <button class="btn btn-ghost" onclick={onclose} disabled={saving}>Cancel</button>
    <button class="btn btn-primary" onclick={save} disabled={saving} id="track-pref-save">
      {#if saving}Saving…{:else}Apply Preferences{/if}
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
    animation: fade-in 200ms var(--ease);
  }
  .dialog {
    position: fixed;
    z-index: 51;
    top: 10vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(600px, calc(100vw - 48px));
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
    padding: 22px 24px 14px;
    border-bottom: 1.5px solid var(--border);
  }
  h2 {
    font-size: 20px;
  }
  header p {
    margin-top: 3px;
    font-size: 13px;
    color: var(--text-2);
  }
  .body {
    padding: 20px 24px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .mode-switch-group {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  @media (max-width: 540px) {
    .mode-switch-group {
      grid-template-columns: 1fr;
    }
  }
  .mode-btn {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 14px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--surface);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .mode-btn:hover {
    border-color: var(--coral);
    transform: translate(-1px, -1px);
    box-shadow: 2px 2px 0 var(--line);
  }
  .mode-btn.active {
    background: var(--surface-3);
    border-color: var(--line);
    box-shadow: 3px 3px 0 var(--coral);
    transform: translate(-2px, -2px);
  }
  .mode-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .mode-info strong {
    font-size: 14.5px;
    font-weight: 700;
    font-family: var(--font-display);
  }
  .mode-info small {
    font-size: 12px;
    color: var(--text-3);
    line-height: 1.35;
  }
  .override-panel {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px;
    background: var(--surface-2);
    border: var(--bw) solid var(--line);
    border-radius: var(--r-lg);
    animation: fade-up 240ms var(--ease);
  }
  .subhead {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .subhead-title {
    font-family: var(--font-display);
    font-size: 13.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-2);
  }
  .preset-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .pill {
    padding: 6px 12px;
    border-radius: 99px;
    border: var(--bw) solid var(--line);
    background: var(--bg-elev);
    color: var(--text-2);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .pill:hover {
    border-color: var(--coral);
    color: var(--text);
  }
  .pill.on {
    background: var(--coral);
    color: var(--on-coral);
    border-color: var(--line);
    box-shadow: 2px 2px 0 var(--line);
  }
  .fields-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 12px;
    margin-top: 4px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field.disabled {
    opacity: 0.45;
    pointer-events: none;
  }
  .field label {
    font-size: 13px;
    font-weight: 600;
    font-family: var(--font-display);
  }
  .field select {
    width: 100%;
    cursor: pointer;
  }
  .field small {
    font-size: 11px;
    color: var(--text-3);
  }
  .summary-box {
    margin-top: 6px;
    padding: 10px 14px;
    border-radius: var(--r-md);
    background: var(--surface);
    border: 1.5px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .summary-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-3);
    font-weight: 700;
  }
  .summary-box strong {
    font-size: 13.5px;
    color: var(--coral);
  }
  .error {
    color: var(--danger);
    font-size: 13px;
    padding: 8px 12px;
    border-radius: var(--r-md);
    background: hsl(0 80% 50% / 0.1);
    border: 1px solid var(--danger);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px 24px;
    border-top: 1.5px solid var(--border);
    background: var(--bg);
    border-bottom-left-radius: var(--r-xl);
    border-bottom-right-radius: var(--r-xl);
  }
  .footer-spacer {
    flex: 1;
  }
</style>
