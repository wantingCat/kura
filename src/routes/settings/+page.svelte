<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { getVersion } from "@tauri-apps/api/app";
  import { api, type DetectedPlayer, type Library, type PlayerKind } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { updater } from "$lib/updater.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import NewLibraryForm from "$lib/components/NewLibraryForm.svelte";
  import {
    AUDIO_LANG_OPTIONS,
    SUB_LANG_OPTIONS,
    SUB_FALLBACK_OPTIONS,
    TRACK_PRESETS,
    describeTrackSummary,
    matchingPresetId,
    type TrackPreset,
  } from "$lib/languages";

  let showNew = $state(false);
  let renaming = $state<number | null>(null);
  let renameValue = $state("");
  let confirmDelete = $state<number | null>(null);

  // Artwork & Metadata ------------------------------------------------------
  let fanartKeyInput = $state(app.fanartApiKey);
  let fanartSaved = $state(false);

  $effect(() => {
    fanartKeyInput = app.fanartApiKey;
  });

  async function saveFanartKey() {
    await app.setFanartApiKey(fanartKeyInput);
    fanartSaved = true;
    setTimeout(() => (fanartSaved = false), 2000);
  }

  async function clearFanartKey() {
    fanartKeyInput = "";
    await app.setFanartApiKey("");
    fanartSaved = true;
    setTimeout(() => (fanartSaved = false), 2000);
  }

  let rebuilding = $state(false);
  let rebuildMsg = $state<string | null>(null);

  async function triggerRebuild() {
    if (rebuilding || app.scanning) return;
    rebuilding = true;
    rebuildMsg = "Wiping cached metadata and starting fresh scan…";
    try {
      await api.rebuildMetadata();
      app.toast("Library metadata rebuild initiated.", "info", 5000);
      rebuildMsg = "Scan running in background. ClearLogos and fanart backdrops will populate automatically.";
      setTimeout(() => (rebuildMsg = null), 8000);
    } catch (e) {
      app.toast(String(e), "error", 8000);
      rebuildMsg = `Error: ${e}`;
    } finally {
      rebuilding = false;
    }
  }

  // Playback ----------------------------------------------------------------
  const PLAYERS: { kind: PlayerKind; name: string; blurb: string }[] = [
    { kind: "mpv", name: "mpv", blurb: "Full tracking + autoplay" },
    { kind: "vlc", name: "VLC", blurb: "Full tracking + autoplay" },
    { kind: "mpc", name: "MPC-HC / BE", blurb: "Needs its web interface on" },
    { kind: "memento", name: "Memento", blurb: "Full tracking + autoplay" },
    { kind: "system", name: "System default", blurb: "Opens normally, no tracking" },
  ];
  let detected = $state<DetectedPlayer[]>([]);
  let version = $state("0.3.2");
  let threshold = $state(app.playback.threshold);
  $effect(() => {
    threshold = app.playback.threshold;
  });

  onMount(async () => {
    try {
      detected = await api.detectPlayers();
    } catch {
      detected = [];
    }
    try {
      version = await getVersion();
    } catch {
      version = "0.3.2";
    }
  });

  const found = (k: PlayerKind) => detected.find((d) => d.kind === k) ?? null;
  /** What the backend will actually use when nothing was picked yet. */
  const effective = $derived<PlayerKind>(
    app.playback.player || ((["mpv", "vlc", "mpc"] as PlayerKind[]).find((k) => found(k)) ?? "system"),
  );
  const auto = $derived(!app.playback.player);

  async function choose(k: PlayerKind) {
    await app.setPlayback("player", k);
    await app.setPlayback("playerPath", "");
  }

  // Audio & Subtitles -------------------------------------------------------
  const currentTrackPreset = $derived(
    matchingPresetId(app.playback.audioLang, app.playback.subLang, app.playback.subFallback)
  );
  const trackSummary = $derived(
    describeTrackSummary(app.playback.audioLang, app.playback.subLang, app.playback.subFallback)
  );

  async function applyTrackPreset(p: TrackPreset) {
    await app.setPlayback("audioLang", p.audio);
    await app.setPlayback("subLang", p.sub);
    await app.setPlayback("subFallback", p.fallback);
  }

  async function browse() {
    const isWin = navigator.userAgent.includes("Windows");
    const picked = await open({
      multiple: false,
      directory: false,
      title: "Choose the player's program file",
      filters: isWin ? [{ name: "Programs", extensions: ["exe"] }] : undefined,
    });
    if (typeof picked === "string") {
      if (!app.playback.player) await app.setPlayback("player", effective);
      await app.setPlayback("playerPath", picked);
    }
  }

  async function addFolder(lib: Library) {
    const picked = await open({ directory: true, multiple: true, title: `Add folders to ${lib.name}` });
    if (!picked) return;
    for (const p of Array.isArray(picked) ? picked : [picked]) await api.addFolder(lib.id, p);
    await app.refresh();
    await app.scan(lib.id);
  }

  async function removeFolder(folderId: number) {
    await api.removeFolder(folderId);
    await app.refresh();
  }

  function startRename(lib: Library) {
    renaming = lib.id;
    renameValue = lib.name;
  }

  async function saveRename(lib: Library) {
    if (renameValue.trim() && renameValue.trim() !== lib.name) await api.renameLibrary(lib.id, renameValue.trim());
    renaming = null;
    await app.refresh();
  }

  async function del(lib: Library) {
    await api.deleteLibrary(lib.id);
    confirmDelete = null;
    await app.refresh();
  }
</script>

<svelte:head><title>Settings · Kura</title></svelte:head>

<div class="page narrow">
  <header class="head">
    <h1>Settings</h1>
    <p class="muted">Libraries, playback and updates.</p>
  </header>

  <section class="block">
    <div class="block-head">
      <h2 class="section-title">Libraries</h2>
      {#if !showNew}
        <button class="btn btn-primary btn-sm" onclick={() => (showNew = true)} id="settings-new-library">
          <Icon name="plus" size={14} /> New library
        </button>
      {/if}
    </div>

    {#if showNew}
      <div class="card new-card">
        <div class="new-head">
          <h3>New anime library</h3>
          <button class="btn btn-ghost btn-sm btn-icon" onclick={() => (showNew = false)} aria-label="Cancel">
            <Icon name="x" size={14} />
          </button>
        </div>
        <NewLibraryForm compact oncreated={() => (showNew = false)} />
      </div>
    {/if}

    {#each app.libraries as lib (lib.id)}
      <div class="card lib">
        <div class="lib-head">
          <div class="lib-icon"><Icon name="library" size={20} /></div>
          <div class="lib-title">
            {#if renaming === lib.id}
              <!-- svelte-ignore a11y_autofocus -->
              <input
                class="input rename"
                bind:value={renameValue}
                autofocus
                onkeydown={(e) => {
                  if (e.key === "Enter") saveRename(lib);
                  if (e.key === "Escape") renaming = null;
                }}
                onblur={() => saveRename(lib)}
              />
            {:else}
              <h3>{lib.name}</h3>
            {/if}
            <span class="faint">Anime · {lib.mediaCount} titles · {lib.fileCount} files</span>
          </div>
          <div class="lib-actions">
            <button class="btn btn-sm" onclick={() => app.scan(lib.id)} disabled={app.scanning} id={`lib-scan-${lib.id}`}>
              <Icon name="refresh" size={14} /> Rescan
            </button>
            <button class="btn btn-ghost btn-sm btn-icon" onclick={() => startRename(lib)} title="Rename">
              <Icon name="edit" size={14} />
            </button>
            {#if confirmDelete === lib.id}
              <button class="btn btn-sm btn-danger" onclick={() => del(lib)} id={`lib-delete-confirm-${lib.id}`}>Remove library?</button>
              <button class="btn btn-ghost btn-sm" onclick={() => (confirmDelete = null)}>Cancel</button>
            {:else}
              <button class="btn btn-ghost btn-sm btn-icon btn-danger" onclick={() => (confirmDelete = lib.id)} title="Remove library">
                <Icon name="trash" size={14} />
              </button>
            {/if}
          </div>
        </div>

        <div class="folders">
          {#each lib.folders as f (f.id)}
            <div class="folder" class:offline={!f.exists}>
              <Icon name="folder" size={16} />
              <span class="path" title={f.path}>{f.path}</span>
              {#if !f.exists}<span class="badge badge-warn">Not found</span>{/if}
              <button class="btn btn-ghost btn-sm btn-icon" onclick={() => removeFolder(f.id)} title="Remove folder">
                <Icon name="x" size={14} />
              </button>
            </div>
          {/each}
          <button class="add" onclick={() => addFolder(lib)} id={`lib-add-folder-${lib.id}`}>
            <Icon name="plus" size={14} /> Add folder
          </button>
        </div>

        <div class="providers">
          <span class="faint">Metadata</span>
          <span class="badge badge-accent">AniList</span>
          <span class="faint">episodes via</span>
          <span class="badge">ani.zip</span>
          <span class="badge">Jikan</span>
        </div>
      </div>
    {:else}
      {#if !showNew}
        <p class="muted">No libraries yet.</p>
      {/if}
    {/each}
  </section>

  <section class="block">
    <h2 class="section-title">Display</h2>
    <div class="card display">
      <div class="opt-head">
        <h3>Theme</h3>
        <p class="faint">Choose between the classic dark comicbook look and the printed manga ink & paper aesthetic.</p>
      </div>
      <div class="choices" role="radiogroup" aria-label="Theme">
        <button
          class="choice theme-choice"
          class:on={app.theme === "comic"}
          role="radio"
          aria-checked={app.theme === "comic"}
          onclick={() => app.setTheme("comic")}
          id="pref-theme-comic"
        >
          <span class="theme-preview comic-preview" aria-hidden="true">
            <span class="preview-shell">
              <span class="preview-badge"></span>
              <span class="preview-bar"></span>
              <span class="preview-sub"></span>
            </span>
          </span>
          <span class="choice-text">
            <strong>Comicbook <span class="rec">Dark</span></strong>
            <small>Deep plum-brown ink, warm coral accents, and chunky sticker shadows.</small>
          </span>
        </button>
        <button
          class="choice theme-choice"
          class:on={app.theme === "manga"}
          role="radio"
          aria-checked={app.theme === "manga"}
          onclick={() => app.setTheme("manga")}
          id="pref-theme-manga"
        >
          <span class="theme-preview manga-preview" aria-hidden="true">
            <span class="preview-shell">
              <span class="preview-badge"></span>
              <span class="preview-bar"></span>
              <span class="preview-sub"></span>
            </span>
          </span>
          <span class="choice-text">
            <strong>Manga <span class="rec manga-rec">Light</span></strong>
            <small>Authentic printed tankōbon — warm newsprint paper, sumi ink linework, and vermilion stamp seals.</small>
          </span>
        </button>
      </div>

      <div class="opt-divider"></div>

      <div class="opt-head">
        <h3>Multi-season shows</h3>
        <p class="faint">AniList lists every season, movie and OVA as a separate entry.</p>
      </div>
      <div class="choices" role="radiogroup" aria-label="Multi-season shows">
        <button
          class="choice"
          class:on={app.groupSeasons}
          role="radio"
          aria-checked={app.groupSeasons}
          onclick={() => app.setGroupSeasons(true)}
          id="pref-group-seasons"
        >
          <span class="illo grouped" aria-hidden="true"><i></i><i></i><i></i></span>
          <span class="choice-text">
            <strong>Grouped <span class="rec">Default</span></strong>
            <small>One card per show. Seasons become tabs on the show's page.</small>
          </span>
        </button>
        <button
          class="choice"
          class:on={!app.groupSeasons}
          role="radio"
          aria-checked={!app.groupSeasons}
          onclick={() => app.setGroupSeasons(false)}
          id="pref-separate-seasons"
        >
          <span class="illo separate" aria-hidden="true"><i></i><i></i><i></i></span>
          <span class="choice-text">
            <strong>Separate</strong>
            <small>Every season, movie and OVA gets its own card, like on AniList.</small>
          </span>
        </button>
      </div>
    </div>
  </section>

  <section class="block">
    <h2 class="section-title">Artwork & Metadata</h2>
    <div class="card display">
      <div class="opt-head">
        <h3>Priority Artwork Pipeline</h3>
        <p class="faint">Kura resolves posters, backdrops, and transparent ClearLogos using a three-tiered hierarchy.</p>
      </div>

      <div class="pipeline-tiers">
        <div class="tier">
          <div class="tier-num">1</div>
          <div class="tier-content">
            <div class="tier-title">
              <strong>Local Folder Assets</strong>
              <span class="badge badge-accent">Highest Priority</span>
            </div>
            <p class="faint">
              Files placed inside your series folders take complete precedence and work 100% offline.
            </p>
            <div class="tier-tags">
              <code>fanart.jpg</code>
              <code>backdrop.png</code>
              <code>clearlogo.png</code>
              <code>logo.png</code>
              <code>poster.jpg</code>
              <code>cover.jpg</code>
            </div>
          </div>
        </div>

        <div class="tier">
          <div class="tier-num">2</div>
          <div class="tier-content">
            <div class="tier-title">
              <strong>Fanart.tv & ani.zip</strong>
              <span class="badge badge-ok">High Definition</span>
            </div>
            <p class="faint">
              1080p widescreen backdrops and transparent PNG ClearLogos matched via TheTVDB & ani.zip mappings.
            </p>
          </div>
        </div>

        <div class="tier">
          <div class="tier-num">3</div>
          <div class="tier-content">
            <div class="tier-title">
              <strong>AniList CDN</strong>
              <span class="badge">Standard Fallback</span>
            </div>
            <p class="faint">
              Standard covers and banners retrieved directly during initial catalog matching.
            </p>
          </div>
        </div>
      </div>

      <div class="opt-divider"></div>

      <div class="opt-head">
        <h3>Fanart.tv Personal API Key</h3>
        <p class="faint">
          Optional personal API key for high-rate ClearLogo and backdrop downloads. Without a key, Kura uses bundled ani.zip mappings.
        </p>
      </div>

      <div class="field">
        <div class="fanart-key-row">
          <input
            id="pref-fanart-key"
            type="password"
            class="input fanart-key-input"
            placeholder="Enter Fanart.tv project API key…"
            bind:value={fanartKeyInput}
            onblur={saveFanartKey}
          />
          <button class="btn btn-sm" onclick={saveFanartKey} id="save-fanart-key">
            {fanartSaved ? "Saved" : "Save key"}
          </button>
          {#if app.fanartApiKey}
            <button class="btn btn-ghost btn-sm" onclick={clearFanartKey} id="clear-fanart-key">
              Clear
            </button>
          {/if}
        </div>
        <div class="fanart-help">
          <small class="faint">
            Don't have one? Get a free personal project key from
            <a
              href="https://fanart.tv/get-an-api-key/"
              target="_blank"
              rel="noreferrer"
              onclick={(e) => { e.preventDefault(); openUrl("https://fanart.tv/get-an-api-key/"); }}
            >fanart.tv</a>.
          </small>
        </div>
      </div>

      <div class="opt-divider"></div>

      <div class="opt-head">
        <h3>Rebuild Metadata & Artwork</h3>
        <p class="faint">
          Re-downloads all series metadata, 1080p Fanart backdrops, transparent ClearLogos, and episode lists from scratch.
          <strong>Your watch history, watched status, and playback progress are completely preserved.</strong>
        </p>
      </div>

      <div class="rebuild-box">
        <button
          class="btn btn-primary"
          onclick={triggerRebuild}
          disabled={rebuilding || app.scanning}
          id="btn-rebuild-metadata"
        >
          {#if rebuilding || app.scanning}
            <div class="spinner"></div>
            <span>Rebuilding library…</span>
          {:else}
            <Icon name="refresh" size={15} />
            <span>Rebuild Metadata & Artwork</span>
          {/if}
        </button>
        {#if rebuildMsg}
          <span class="rebuild-note">{rebuildMsg}</span>
        {/if}
      </div>
    </div>
  </section>

  <section class="block">
    <h2 class="section-title">Playback</h2>
    <div class="card playback">
      <div class="opt-head">
        <h3>Player</h3>
        <p class="faint">Kura opens episodes in your own player and follows along to remember where you stopped.</p>
      </div>
      <div class="players" role="radiogroup" aria-label="Player">
        {#each PLAYERS as p (p.kind)}
          {@const hit = p.kind === "system" ? true : !!found(p.kind)}
          <button
            class="player"
            class:on={effective === p.kind}
            role="radio"
            aria-checked={effective === p.kind}
            onclick={() => choose(p.kind)}
            id={`player-${p.kind}`}
          >
            <span class="p-name">
              {p.name}
              {#if effective === p.kind && auto}<span class="rec">Auto</span>{/if}
            </span>
            <small>{p.blurb}</small>
            {#if p.kind !== "system"}
              <span class="badge" class:badge-ok={hit}>{hit ? "Detected" : "Not found"}</span>
            {/if}
          </button>
        {/each}
      </div>

      {#if effective !== "system"}
        <div class="field">
          <label for="player-path">Program location</label>
          <div class="path-row">
            <input
              id="player-path"
              class="input"
              value={app.playback.playerPath}
              placeholder={found(effective)?.path ?? "Not found — browse to the player's program file"}
              onchange={(e) => app.setPlayback("playerPath", e.currentTarget.value.trim())}
            />
            <button class="btn btn-sm" onclick={browse} id="player-browse"><Icon name="folder-open" size={14} /> Browse…</button>
          </div>
          {#if !app.playback.playerPath && found(effective)}
            <small class="faint">Using the detected copy. Pick a different file to override it.</small>
          {/if}
        </div>
      {/if}

      {#if effective === "mpc"}
        <div class="note">
          <Icon name="alert" size={15} />
          <div>
            In MPC go to <strong>Options → Player → Web Interface</strong> and tick <strong>Listen on port</strong>.
            <label class="port">
              Port
              <input
                class="input"
                type="number"
                min="1"
                max="65535"
                value={app.playback.mpcPort}
                onchange={(e) => app.setPlayback("mpcPort", Number(e.currentTarget.value) || 13579)}
                id="mpc-port"
              />
            </label>
          </div>
        </div>
      {:else if effective === "system"}
        <div class="note">
          <Icon name="alert" size={15} />
          <div>With the system default player Kura can't see playback, so progress, auto-watched and autoplay are off.</div>
        </div>
      {/if}

      <div class="rows" class:disabled={effective === "system"}>
        <div class="row">
          <div class="row-text">
            <strong>Mark as watched at <span class="pct">{threshold}%</span></strong>
            <small>Once you've seen this much of an episode it counts as watched — so skipping the ending still counts.</small>
          </div>
          <input
            class="slider"
            type="range"
            min="50"
            max="100"
            step="5"
            bind:value={threshold}
            onchange={() => app.setPlayback("threshold", threshold)}
            style:--fill={`${((threshold - 50) / 50) * 100}%`}
            id="pref-threshold"
            aria-label="Mark as watched at"
          />
        </div>
        <div class="row">
          <div class="row-text">
            <strong>Resume where I left off</strong>
            <small>Pick up episodes from the last position instead of the start.</small>
          </div>
          <button
            class="switch"
            class:on={app.playback.resume}
            role="switch"
            aria-checked={app.playback.resume}
            aria-label="Resume where I left off"
            onclick={() => app.setPlayback("resume", !app.playback.resume)}
            id="pref-resume"
          ><span></span></button>
        </div>
        <div class="row">
          <div class="row-text">
            <strong>Autoplay next episode</strong>
            <small>When an episode ends, play the next one you own in the same window — including the next season.</small>
          </div>
          <button
            class="switch"
            class:on={app.playback.autoplay}
            role="switch"
            aria-checked={app.playback.autoplay}
            aria-label="Autoplay next episode"
            onclick={() => app.setPlayback("autoplay", !app.playback.autoplay)}
            id="pref-autoplay"
          ><span></span></button>
        </div>
      </div>
    </div>
  </section>

  <section class="block">
    <h2 class="section-title">Audio & Subtitles</h2>
    <div class="card playback">
      <div class="opt-head">
        <h3>Default track preferences</h3>
        <p class="faint">Kura tells mpv, Memento, and VLC your language priorities when launching episodes. You can also override this for individual shows.</p>
      </div>

      <div class="choices presets-grid" role="radiogroup" aria-label="Audio and subtitle presets">
        {#each TRACK_PRESETS as p (p.id)}
          <button
            class="choice"
            class:on={currentTrackPreset === p.id}
            role="radio"
            aria-checked={currentTrackPreset === p.id}
            onclick={() => applyTrackPreset(p)}
            id={`track-preset-${p.id}`}
          >
            <span class="choice-text">
              <strong>
                {p.name}
                {#if p.id === "jp-jp-en"}<span class="rec">Default</span>{/if}
              </strong>
              <small>{p.blurb}</small>
            </span>
          </button>
        {/each}
        <button
          class="choice"
          class:on={currentTrackPreset === "custom"}
          role="radio"
          aria-checked={currentTrackPreset === "custom"}
          onclick={() => {}}
          id="track-preset-custom"
        >
          <span class="choice-text">
            <strong>Custom priority</strong>
            <small>Fine-tune languages using the selectors below.</small>
          </span>
        </button>
      </div>

      <div class="track-fields">
        <div class="field">
          <label for="pref-audio-lang">Preferred audio</label>
          <select
            id="pref-audio-lang"
            class="input select"
            value={app.playback.audioLang}
            onchange={(e) => app.setPlayback("audioLang", e.currentTarget.value)}
          >
            {#each AUDIO_LANG_OPTIONS as opt}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="field">
          <label for="pref-sub-lang">Primary subtitles</label>
          <select
            id="pref-sub-lang"
            class="input select"
            value={app.playback.subLang}
            onchange={(e) => app.setPlayback("subLang", e.currentTarget.value)}
          >
            {#each SUB_LANG_OPTIONS as opt}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>

        <div class="field" class:disabled={app.playback.subLang === "off"}>
          <label for="pref-sub-fallback">Fallback subtitles</label>
          <select
            id="pref-sub-fallback"
            class="input select"
            disabled={app.playback.subLang === "off"}
            value={app.playback.subFallback}
            onchange={(e) => app.setPlayback("subFallback", e.currentTarget.value)}
          >
            {#each SUB_FALLBACK_OPTIONS as opt}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
          <small class="faint">Selected if the primary subtitle language isn't present in the file.</small>
        </div>
      </div>

      <div class="track-summary-pill">
        <Icon name="languages" size={15} />
        <span>Active priority: <strong>{trackSummary}</strong></span>
      </div>
    </div>
  </section>

  <section class="block">
    <h2 class="section-title">Discord Rich Presence</h2>
    <div class="card playback">
      <div class="rows">
        <div class="row">
          <div class="row-text">
            <strong>Show activity on Discord</strong>
            <small>Displays "Watching Kura" on your Discord profile with anime title, episode details, and a live progress bar.</small>
          </div>
          <button
            class="switch"
            class:on={app.discord.enabled}
            role="switch"
            aria-checked={app.discord.enabled}
            aria-label="Show activity on Discord"
            onclick={() => app.setDiscord("enabled", !app.discord.enabled)}
            id="pref-discord-rpc"
          ><span></span></button>
        </div>

        {#if app.discord.enabled}
          <div class="row">
            <div class="row-text">
              <strong>Spoiler protection</strong>
              <small>Hides episode titles on Discord, showing only the episode number (e.g. "Episode 04").</small>
            </div>
            <button
              class="switch"
              class:on={app.discord.spoilers}
              role="switch"
              aria-checked={app.discord.spoilers}
              aria-label="Spoiler protection"
              onclick={() => app.setDiscord("spoilers", !app.discord.spoilers)}
              id="pref-discord-spoilers"
            ><span></span></button>
          </div>

          <div class="row">
            <div class="row-text">
              <strong>Interactive profile buttons</strong>
              <small>Include "View on AniList" and "Get Kura on GitHub" action buttons on your Discord activity card.</small>
            </div>
            <button
              class="switch"
              class:on={app.discord.buttons}
              role="switch"
              aria-checked={app.discord.buttons}
              aria-label="Interactive profile buttons"
              onclick={() => app.setDiscord("buttons", !app.discord.buttons)}
              id="pref-discord-buttons"
            ><span></span></button>
          </div>
        {/if}
      </div>
    </div>
  </section>

  <section class="block">
    <h2 class="section-title">Updates</h2>
    <div class="card playback">
      <div class="rows">
        <div class="row">
          <div class="row-text">
            <strong>Check for updates automatically</strong>
            <small>Kura looks for a new version on GitHub when it starts. Nothing is installed without asking.</small>
          </div>
          <button
            class="switch"
            class:on={app.autoUpdate}
            role="switch"
            aria-checked={app.autoUpdate}
            aria-label="Check for updates automatically"
            onclick={() => app.setAutoUpdate(!app.autoUpdate)}
            id="pref-auto-update"
          ><span></span></button>
        </div>
        <div class="row">
          <div class="row-text">
            <strong>Version {version || "—"}</strong>
            <small>
              {#if updater.status === "checking"}Checking…
              {:else if updater.status === "none"}You're on the latest version.
              {:else if updater.status === "available"}v{updater.version} is available — see the banner at the top.
              {:else if updater.status === "error"}Couldn't check: {updater.error}
              {:else}Updates come from the GitHub releases page.{/if}
            </small>
          </div>
          <button
            class="btn btn-sm"
            onclick={() => ((updater.dismissed = false), updater.check({ silent: false }))}
            disabled={updater.status === "checking" || updater.status === "downloading"}
            id="update-check"
          >
            {#if updater.status === "checking"}<span class="spinner"></span>{:else}<Icon name="refresh" size={14} />{/if}
            Check now
          </button>
        </div>
      </div>
    </div>
  </section>

  <section class="block">
    <h2 class="section-title">About</h2>
    <div class="card about">
      <div class="about-hero">
        <img class="about-logo" src="/brand/logo.png" alt="Kura Logo" width="48" height="48" />
        <div class="about-title-block">
          <h3>
            Kura 蔵
            <span class="badge badge-accent">v{version || "0.3.2"}</span>
          </h3>
          <p class="faint">A local-first, manga-styled desktop anime media library.</p>
        </div>
      </div>

      <div class="about-body">
        <p>
          Point Kura at your folders and it parses release filenames, matches series to AniList,
          fetches episode guides and artworks via ani.zip, and plays episodes directly in your preferred video player.
          All metadata, watch history, and image caches remain stored locally in SQLite — your files never leave your computer.
        </p>
      </div>

      <div class="about-grid">
        <div class="about-item">
          <strong>Metadata Providers</strong>
          <small>AniList API · ani.zip · Jikan (MyAnimeList) · Fanart.tv</small>
        </div>
        <div class="about-item">
          <strong>Player Integrations</strong>
          <small>mpv · Memento · VLC · MPC-HC / MPC-BE</small>
        </div>
      </div>

      <div class="about-actions">
        <button
          class="btn btn-sm"
          onclick={() => openUrl("https://github.com/wantingCat/kura")}
          id="about-github"
        >
          <Icon name="external" size={14} /> GitHub Repository
        </button>
        <button
          class="btn btn-sm btn-ghost"
          onclick={() => openUrl("https://github.com/wantingCat/kura/releases")}
          id="about-releases"
        >
          Releases & Changelog
        </button>
      </div>

      <div class="about-coming">
        <strong>Coming up in future releases:</strong>
        <p class="faint">
          Automatic background folder watcher, 2-way AniList & MyAnimeList account sync, and rich cast/voice actors & staff metadata.
        </p>
      </div>
    </div>
  </section>
</div>

<style>
  .narrow {
    max-width: 920px;
  }
  .head {
    margin: 12px 0 32px;
  }
  .head h1 {
    font-size: 34px;
    font-weight: 600;
  }
  .head p {
    margin-top: 6px;
  }
  .block {
    margin-bottom: 40px;
  }
  .block-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 14px;
  }
  .block-head .section-title {
    margin: 0;
  }
  .new-card {
    padding: 22px;
    margin-bottom: 14px;
    animation: fade-up 300ms var(--ease);
  }
  .new-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 16px;
  }
  .new-head h3 {
    font-size: 17px;
  }
  .lib {
    padding: 18px;
    margin-bottom: 14px;
  }
  .lib-head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .lib-icon {
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--coral);
    color: var(--on-coral);
    box-shadow: 2px 2px 0 var(--line);
    flex: none;
  }
  .lib-title {
    flex: 1;
    min-width: 0;
  }
  .lib-title h3 {
    font-size: 17px;
  }
  .lib-title .faint {
    font-size: 12.5px;
  }
  .rename {
    height: 32px;
    width: 260px;
  }
  .lib-actions {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .folders {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 16px;
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 6px 6px 12px;
    border-radius: var(--r-md);
    background: var(--bg-elev);
    border: var(--bw) solid var(--line);
    color: var(--text-3);
  }
  .folder.offline {
    border-left: 6px solid var(--amber);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
  }
  .add {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 38px;
    padding: 0 12px;
    border-radius: var(--r-md);
    border: var(--bw) dashed var(--border-strong);
    background: transparent;
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 14px;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .add:hover {
    border-color: var(--coral);
    color: var(--coral);
  }
  .providers {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 16px;
    font-size: 12.5px;
  }
  .about {
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    color: var(--text-2);
  }
  .about-hero {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .about-logo {
    width: 48px;
    height: 48px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--bg-elev);
    box-shadow: 2px 2px 0 var(--line);
    object-fit: contain;
    flex: none;
  }
  .about-title-block h3 {
    font-size: 19px;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .about-body p {
    line-height: 1.65;
    font-size: 14px;
  }
  .about-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 12px;
  }
  .about-item {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    background: var(--surface);
    border: var(--bw) solid var(--line);
  }
  .about-item strong {
    font-size: 13px;
    font-family: var(--font-display);
    color: var(--text);
  }
  .about-item small {
    font-size: 12px;
    color: var(--text-3);
  }
  .about-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .about-coming {
    padding-top: 14px;
    border-top: 1.5px solid var(--border);
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .about-coming strong {
    font-family: var(--font-display);
    color: var(--text);
  }

  /* Display preferences ---------------------------------------------------- */
  .display {
    padding: 20px;
  }
  .opt-divider {
    height: 1.5px;
    background: var(--border);
    margin: 22px 0;
  }
  .theme-preview {
    position: relative;
    flex: none;
    width: 62px;
    height: 48px;
    border-radius: 8px;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .comic-preview {
    background: #1d1415;
    border: 2px solid #140c0d;
    box-shadow: 2px 2px 0 #140c0d;
  }
  .manga-preview {
    background: #f5f2eb;
    border: 2px solid #12100e;
    box-shadow: 2px 2px 0 #12100e;
    background-image: radial-gradient(#dcd6c9 1px, transparent 1px);
    background-size: 6px 6px;
  }
  .preview-shell {
    width: 44px;
    height: 32px;
    border-radius: 5px;
    padding: 5px;
    display: flex;
    flex-direction: column;
    gap: 3.5px;
  }
  .comic-preview .preview-shell {
    background: #2c1f21;
    border: 1.5px solid #140c0d;
  }
  .manga-preview .preview-shell {
    background: #ffffff;
    border: 1.5px solid #12100e;
  }
  .preview-badge {
    width: 14px;
    height: 4px;
    border-radius: 2px;
  }
  .comic-preview .preview-badge {
    background: #f76d4d;
  }
  .manga-preview .preview-badge {
    background: #e53935;
  }
  .preview-bar {
    width: 28px;
    height: 3px;
    border-radius: 2px;
  }
  .comic-preview .preview-bar {
    background: #f7ede2;
  }
  .manga-preview .preview-bar {
    background: #141210;
  }
  .preview-sub {
    width: 18px;
    height: 2.5px;
    border-radius: 2px;
  }
  .comic-preview .preview-sub {
    background: #cdbab0;
  }
  .manga-preview .preview-sub {
    background: #7c766d;
  }
  .manga-rec {
    background: #e53935;
    color: #ffffff;
  }

  /* Playback / updates ----------------------------------------------------- */
  .playback {
    padding: 20px;
  }
  .players {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 12px;
    margin-top: 16px;
  }
  .player {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: 12px 14px;
    text-align: left;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--bg-elev);
    color: var(--text);
    cursor: pointer;
    transition:
      transform var(--t-fast) var(--bounce),
      box-shadow var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .player:hover {
    transform: translate(-2px, -2px);
    box-shadow: 4px 4px 0 var(--line);
  }
  .player.on {
    background: var(--surface-3);
    box-shadow: 4px 4px 0 var(--coral);
    transform: translate(-2px, -2px);
  }
  .p-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-family: var(--font-display);
    font-size: 15.5px;
    font-weight: 600;
  }
  .player small {
    color: var(--text-3);
    font-size: 12px;
    font-weight: 600;
    line-height: 1.35;
  }
  .player .badge {
    margin-top: 6px;
    height: 20px;
    font-size: 10px;
  }
  .field {
    margin-top: 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field label {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 14px;
  }
  .field small {
    font-size: 12px;
  }
  .path-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .path-row .input {
    flex: 1;
    min-width: 0;
    height: 36px;
    font-size: 13px;
  }
  .note {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    margin-top: 16px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    border-left: 6px solid var(--amber);
    background: var(--bg-elev);
    color: var(--text-2);
    font-size: 13px;
  }
  .note :global(svg) {
    flex: none;
    color: var(--amber);
    margin-top: 2px;
  }
  .port {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin-left: 10px;
    font-weight: 700;
  }
  .port .input {
    width: 96px;
    height: 30px;
    padding: 0 10px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    margin-top: 10px;
  }
  .rows.disabled {
    opacity: 0.5;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 14px 0;
    border-top: 1.5px solid var(--border);
  }
  .rows .row:first-child {
    border-top: none;
  }
  .row-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row-text strong {
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 600;
  }
  .row-text small {
    color: var(--text-3);
    font-size: 12.5px;
    font-weight: 600;
  }
  .pct {
    color: var(--coral);
  }
  .slider {
    -webkit-appearance: none;
    appearance: none;
    width: 220px;
    height: 12px;
    border-radius: 99px;
    border: var(--bw) solid var(--line);
    background: linear-gradient(to right, var(--coral) var(--fill), var(--bg) var(--fill));
    cursor: pointer;
  }
  .slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: var(--bw) solid var(--line);
    background: var(--cream);
    box-shadow: 2px 2px 0 var(--line);
    transition: transform var(--t-fast) var(--bounce);
  }
  .slider:active::-webkit-slider-thumb {
    transform: scale(1.15);
  }
  .switch {
    position: relative;
    flex: none;
    width: 50px;
    height: 28px;
    padding: 0;
    border-radius: 99px;
    border: var(--bw) solid var(--line);
    background: var(--bg);
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }
  .switch span {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: var(--bw) solid var(--line);
    background: var(--text-3);
    transition:
      transform var(--t-med) var(--bounce),
      background var(--t-fast) var(--ease);
  }
  .switch.on {
    background: var(--coral);
  }
  .switch.on span {
    transform: translateX(22px);
    background: var(--cream);
  }
  .opt-head h3 {
    font-size: 17px;
  }
  .opt-head p {
    margin-top: 2px;
    font-size: 13px;
  }
  .choices {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
    margin-top: 16px;
  }
  @media (max-width: 700px) {
    .choices {
      grid-template-columns: 1fr;
    }
  }
  .choice {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 16px;
    text-align: left;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--bg-elev);
    color: var(--text);
    cursor: pointer;
    transition:
      transform var(--t-fast) var(--bounce),
      box-shadow var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .choice:hover {
    transform: translate(-2px, -2px);
    box-shadow: 4px 4px 0 var(--line);
  }
  .choice.on {
    background: var(--surface-3);
    box-shadow: 4px 4px 0 var(--coral);
    transform: translate(-2px, -2px);
  }
  .choice-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .choice-text strong {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--font-display);
    font-size: 15.5px;
    font-weight: 600;
  }
  .choice-text small {
    color: var(--text-3);
    font-size: 12.5px;
    font-weight: 600;
    line-height: 1.45;
  }
  .rec {
    padding: 1px 7px;
    border-radius: 99px;
    border: 1.5px solid var(--line);
    background: var(--coral);
    color: var(--on-coral);
    font-family: var(--font);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  /* Tiny poster illustrations: a stacked pile vs. three loose cards. */
  .illo {
    position: relative;
    flex: none;
    width: 62px;
    height: 48px;
  }
  .illo i {
    position: absolute;
    width: 22px;
    height: 32px;
    border-radius: 5px;
    border: 2px solid var(--line);
    background: var(--surface-2);
  }
  .choice.on .illo i:last-child {
    background: var(--coral);
  }
  .grouped i:nth-child(1) {
    left: 24px;
    top: 4px;
    transform: rotate(10deg);
  }
  .grouped i:nth-child(2) {
    left: 20px;
    top: 7px;
    transform: rotate(4deg);
  }
  .grouped i:nth-child(3) {
    left: 16px;
    top: 10px;
  }
  .separate i {
    top: 8px;
  }
  .separate i:nth-child(1) {
    left: 0;
  }
  .separate i:nth-child(2) {
    left: 20px;
  }
  .separate i:nth-child(3) {
    left: 40px;
  }

  .presets-grid {
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 12px;
  }
  .track-fields {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 16px;
    margin-top: 20px;
    padding-top: 18px;
    border-top: 1.5px solid var(--border);
  }
  .track-fields select {
    cursor: pointer;
  }
  .track-summary-pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    background: var(--surface);
    border: var(--bw) solid var(--line);
    border-radius: 99px;
    font-size: 13px;
    color: var(--text-2);
    margin-top: 18px;
  }
  .track-summary-pill strong {
    color: var(--coral);
  }
  .field.disabled {
    opacity: 0.45;
    pointer-events: none;
  }

  /* Artwork pipeline & Fanart.tv settings ---------------------------------- */
  .pipeline-tiers {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 14px;
  }
  .tier {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 12px 14px;
    border-radius: var(--r-md);
    background: var(--surface);
    border: var(--bw) solid var(--line);
  }
  .tier-num {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--surface-3);
    border: var(--bw) solid var(--line);
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 14px;
    color: var(--text);
    flex: none;
  }
  .tier:first-child .tier-num {
    background: var(--coral);
    color: var(--on-coral);
  }
  .tier-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tier-title {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .tier-title strong {
    font-size: 14px;
    font-family: var(--font-display);
    color: var(--text);
  }
  .tier-content p {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.4;
  }
  .tier-tags {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 4px;
  }
  .tier-tags code {
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    font-size: 11px;
    font-family: monospace;
    color: var(--text-2);
  }
  .fanart-key-row {
    display: flex;
    gap: 8px;
    align-items: center;
    max-width: 520px;
    margin-top: 8px;
  }
  .fanart-key-input {
    flex: 1;
    min-width: 0;
    font-family: monospace;
    letter-spacing: 0.05em;
  }
  .fanart-help {
    margin-top: 6px;
  }
  .fanart-help a {
    color: var(--coral);
    text-decoration: underline;
    cursor: pointer;
  }
  .rebuild-box {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 14px;
    flex-wrap: wrap;
  }
  .rebuild-note {
    font-size: 13px;
    color: var(--ok);
    font-weight: 500;
  }
</style>
