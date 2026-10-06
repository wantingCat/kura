<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getVersion } from "@tauri-apps/api/app";
  import { api, type DetectedPlayer, type Library, type PlayerKind } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import { updater } from "$lib/updater.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import NewLibraryForm from "$lib/components/NewLibraryForm.svelte";

  let showNew = $state(false);
  let renaming = $state<number | null>(null);
  let renameValue = $state("");
  let confirmDelete = $state<number | null>(null);

  // Playback ----------------------------------------------------------------
  const PLAYERS: { kind: PlayerKind; name: string; blurb: string }[] = [
    { kind: "mpv", name: "mpv", blurb: "Full tracking + autoplay" },
    { kind: "vlc", name: "VLC", blurb: "Full tracking + autoplay" },
    { kind: "mpc", name: "MPC-HC / BE", blurb: "Needs its web interface on" },
    { kind: "memento", name: "Memento", blurb: "Tracked if it speaks mpv IPC" },
    { kind: "system", name: "System default", blurb: "Opens normally, no tracking" },
  ];
  let detected = $state<DetectedPlayer[]>([]);
  let version = $state("");
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
      version = "";
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
      <p>
        <strong>Kura</strong> <span class="faint">v0.2.0-alpha</span> · a local-first anime library. Metadata from
        <strong>AniList</strong>, episode info from <strong>ani.zip</strong> and <strong>Jikan (MyAnimeList)</strong>.
        Everything is cached locally, and your media files never leave your computer.
      </p>
      <p class="faint coming">Coming next: AniList account sync and a built-in Memento integration.</p>
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
    padding: 20px;
    color: var(--text-2);
    line-height: 1.7;
  }
  .coming {
    margin-top: 10px;
    font-size: 13px;
  }

  /* Display preferences ---------------------------------------------------- */
  .display {
    padding: 20px;
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
</style>
