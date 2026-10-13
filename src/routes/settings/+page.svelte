<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
  import { getVersion } from "@tauri-apps/api/app";
  import {
    api,
    type BackupSummary,
    type DetectedPlayer,
    type Diagnostics,
    type ImportOptions,
    type ImportReport,
    type Library,
    type PlayerKind,
    type SafetyCopy,
  } from "$lib/api";
  import { relativeTime } from "$lib/format";
  import { app } from "$lib/app.svelte";
  import { updater } from "$lib/updater.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import NewLibraryForm from "$lib/components/NewLibraryForm.svelte";
  import { mpvModal } from "$lib/mpvModal.svelte";
  import { changelogModal } from "$lib/changelogModal.svelte";
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
  let version = $state("0.3.7");
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
      version = "0.3.7";
    }
  });

  const found = (k: PlayerKind) => detected.find((d) => d.kind === k) ?? null;
  /** What the backend will actually use when nothing was picked yet. */
  const effective = $derived<PlayerKind>(
    app.playback.player || ((["mpv", "vlc", "mpc"] as PlayerKind[]).find((k) => found(k)) ?? "system"),
  );
  const auto = $derived(!app.playback.player);

  async function choose(k: PlayerKind) {
    if (k === "mpv" && !found("mpv")) {
      mpvModal.open();
    }
    await app.setPlayback("player", k);
    await app.setPlayback("playerPath", "");
  }

  $effect(() => {
    if (!mpvModal.visible) {
      api.detectPlayers().then((d) => (detected = d)).catch(() => {});
    }
  });

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

  // Backup & Restore --------------------------------------------------------
  let safety = $state<SafetyCopy[]>([]);
  let exporting = $state(false);
  let exported = $state<BackupSummary | null>(null);
  let preview = $state<BackupSummary | null>(null);
  let importOpts = $state<ImportOptions>({ settings: true, libraries: true, history: true, trackPrefs: true, matches: true });
  let importing = $state(false);
  let importReport = $state<ImportReport | null>(null);

  const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

  async function loadSafety() {
    try {
      safety = await api.backupStatus();
    } catch {
      safety = [];
    }
  }

  async function doExport() {
    const d = new Date();
    const pad = (n: number) => String(n).padStart(2, "0");
    const path = await save({
      title: "Export Kura backup",
      defaultPath: `kura-backup-${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}.json`,
      filters: [{ name: "Kura backup", extensions: ["json"] }],
    });
    if (!path) return;
    exporting = true;
    try {
      exported = await api.exportBackup(path);
      app.toast("Backup saved.", "info", 4000);
    } catch (e) {
      app.toast(String(e), "error", 8000);
    } finally {
      exporting = false;
    }
  }

  async function pickImport() {
    const picked = await open({
      multiple: false,
      directory: false,
      title: "Choose a Kura backup",
      filters: [{ name: "Kura backup", extensions: ["json"] }],
    });
    if (typeof picked !== "string") return;
    importReport = null;
    try {
      const p = await api.previewBackup(picked);
      importOpts = {
        settings: p.settings > 0,
        libraries: p.libraries.length > 0,
        history: p.watched + p.progress > 0,
        trackPrefs: p.trackPrefs > 0,
        matches: p.manualMatches > 0,
      };
      preview = p;
    } catch (e) {
      preview = null;
      app.toast(String(e), "error", 8000);
    }
  }

  const importRows = $derived.by(() => {
    const p = preview;
    if (!p) return [];
    const folders = p.libraries.reduce((n, l) => n + l.folders, 0);
    return [
      { key: "settings" as const, label: "Settings", count: p.settings, detail: `${plural(p.settings, "preference")} — theme, player, languages…` },
      {
        key: "libraries" as const,
        label: "Libraries & folders",
        count: p.libraries.length,
        detail: p.libraries.length
          ? `${p.libraries.map((l) => l.name).join(", ")} · ${plural(folders, "folder")}`
          : "None in this backup",
      },
      {
        key: "history" as const,
        label: "Watch history & resume points",
        count: p.watched + p.progress,
        detail: `${plural(p.watched, "watched episode")} across ${plural(p.showsWatched, "show")} · ${plural(p.progress, "resume point")}`,
      },
      { key: "trackPrefs" as const, label: "Per-show audio & subtitles", count: p.trackPrefs, detail: plural(p.trackPrefs, "show override") },
      { key: "matches" as const, label: "Fixed matches", count: p.manualMatches, detail: plural(p.manualMatches, "manual match", "manual matches") },
    ];
  });
  const anySelected = $derived(importRows.some((r) => r.count > 0 && importOpts[r.key]));

  async function doImport() {
    if (!preview) return;
    importing = true;
    try {
      importReport = await api.importBackup(preview.path, $state.snapshot(importOpts));
      if (importOpts.settings) await app.loadPrefs();
      await app.refresh();
      preview = null;
      app.toast("Backup imported.", "info", 5000);
      loadSafety();
    } catch (e) {
      app.toast(String(e), "error", 8000);
    } finally {
      importing = false;
    }
  }

  const reportText = (r: ImportReport) => {
    const parts = [
      r.settings && plural(r.settings, "setting"),
      r.librariesAdded && plural(r.librariesAdded, "new library", "new libraries"),
      r.foldersAdded && plural(r.foldersAdded, "folder"),
      r.watchedAdded && plural(r.watchedAdded, "watched episode"),
      r.progressAdded && plural(r.progressAdded, "resume point"),
      r.trackPrefs && plural(r.trackPrefs, "show override"),
      r.matches && plural(r.matches, "fixed match", "fixed matches"),
    ].filter(Boolean);
    return parts.length ? parts.join(", ") : "nothing new — this library already had everything";
  };

  async function reveal(path: string) {
    try {
      await revealItemInDir(path);
    } catch (e) {
      app.toast(String(e), "error", 6000);
    }
  }

  // Categories & search -----------------------------------------------------
  type CatId =
    | "libraries"
    | "appearance"
    | "playback"
    | "tracks"
    | "artwork"
    | "integrations"
    | "backup"
    | "updates"
    | "about";

  const CATEGORIES: { id: CatId; label: string; icon: string }[] = [
    { id: "libraries", label: "Libraries", icon: "library" },
    { id: "appearance", label: "Appearance", icon: "palette" },
    { id: "playback", label: "Playback", icon: "play" },
    { id: "tracks", label: "Audio & Subtitles", icon: "subtitles" },
    { id: "artwork", label: "Artwork & Metadata", icon: "image" },
    { id: "integrations", label: "Integrations", icon: "discord" },
    { id: "backup", label: "Backup & Restore", icon: "archive" },
    { id: "updates", label: "Updates", icon: "refresh" },
    { id: "about", label: "About & Diagnostics", icon: "info" },
  ];
  const catOf = (id: CatId) => CATEGORIES.find((c) => c.id === id)!;

  const CAT_KEY = "kura.settings.category";
  function loadCat(): CatId {
    try {
      const v = localStorage.getItem(CAT_KEY);
      if (v && CATEGORIES.some((c) => c.id === v)) return v as CatId;
    } catch {}
    return "libraries";
  }
  let active = $state<CatId>(loadCat());
  let rootEl = $state<HTMLElement>();
  let searchEl = $state<HTMLInputElement>();

  $effect(() => {
    if (active === "backup") loadSafety();
  });

  // Diagnostics -------------------------------------------------------------
  let diag = $state<Diagnostics | null>(null);
  let diagLoading = $state(false);
  let copied = $state(false);
  let showLog = $state(false);

  async function loadDiag() {
    diagLoading = true;
    try {
      diag = await api.getDiagnostics();
      detected = diag.players;
    } catch (e) {
      app.toast(String(e), "error", 6000);
    } finally {
      diagLoading = false;
    }
  }

  async function copyText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Fallback for webviews without async clipboard access.
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
  }

  async function copyDiag() {
    await loadDiag(); // always copy a fresh report
    if (!diag) return;
    await copyText(diag.report);
    copied = true;
    setTimeout(() => (copied = false), 2200);
  }

  $effect(() => {
    if (active === "about") loadDiag();
  });

  function go(id: CatId) {
    query = "";
    if (active !== id) rootEl?.scrollIntoView({ block: "start" });
    active = id;
    try {
      localStorage.setItem(CAT_KEY, id);
    } catch {}
  }

  /** Searchable index. `target` is an element id; `fallback` is used when the
   *  target is conditionally hidden (e.g. sub-options of a disabled toggle). */
  type Entry = { cat: CatId; label: string; hint: string; target: string; fallback?: string; kw: string };
  const INDEX: Entry[] = [
    { cat: "libraries", label: "New library", hint: "Create an anime library from your folders", target: "settings-new-library", kw: "add create" },
    { cat: "libraries", label: "Library folders", hint: "Add, remove or rescan folders", target: "set-libraries", kw: "folder path rescan rename remove delete drive" },
    { cat: "libraries", label: "Live folder watcher", hint: "Auto-rescan when files change", target: "pref-folder-watcher", kw: "auto rescan watch sync new episodes" },
    { cat: "appearance", label: "Theme", hint: "Comicbook (dark) or Manga (light)", target: "set-theme", kw: "comic manga dark light color colour look" },
    { cat: "appearance", label: "Multi-season shows", hint: "Group seasons into one card or keep them separate", target: "set-seasons", kw: "group grouped separate season movie ova tabs" },
    { cat: "playback", label: "Player", hint: "mpv, VLC, MPC, Memento or system default", target: "set-player", kw: "mpv vlc mpc memento system default video install" },
    { cat: "playback", label: "Program location", hint: "Point Kura at your player's executable", target: "player-path", fallback: "set-player", kw: "path exe browse custom" },
    { cat: "playback", label: "MPC web interface port", hint: "Port MPC-HC / BE listens on", target: "mpc-port", fallback: "set-player", kw: "mpc port web interface" },
    { cat: "playback", label: "Mark as watched at", hint: "How much of an episode counts as watched", target: "pref-threshold", kw: "threshold percent watched complete" },
    { cat: "playback", label: "Resume where I left off", hint: "Continue from the last position", target: "pref-resume", kw: "resume continue position progress" },
    { cat: "playback", label: "Autoplay next episode", hint: "Play the next episode automatically", target: "pref-autoplay", kw: "autoplay next binge" },
    { cat: "tracks", label: "Track presets", hint: "Sub, dub or no-subs presets", target: "set-tracks", kw: "preset audio subtitle sub dub japanese english" },
    { cat: "tracks", label: "Preferred audio", hint: "Audio language priority", target: "pref-audio-lang", kw: "audio language dub japanese english" },
    { cat: "tracks", label: "Primary subtitles", hint: "Subtitle language priority", target: "pref-sub-lang", kw: "subtitle subs language" },
    { cat: "tracks", label: "Fallback subtitles", hint: "Used when the primary language is missing", target: "pref-sub-fallback", kw: "subtitle subs fallback second" },
    { cat: "artwork", label: "Artwork pipeline", hint: "Where posters, backdrops and logos come from", target: "set-pipeline", kw: "poster backdrop logo clearlogo local fanart anilist image" },
    { cat: "artwork", label: "Fanart.tv API key", hint: "Optional personal key", target: "set-fanart", kw: "fanart api key token" },
    { cat: "artwork", label: "Rebuild metadata & artwork", hint: "Re-download everything, keep watch history", target: "set-rebuild", kw: "rebuild refresh redownload reset metadata" },
    { cat: "integrations", label: "Discord Rich Presence", hint: "Show what you're watching on Discord", target: "pref-discord-rpc", kw: "discord rpc activity status presence" },
    { cat: "integrations", label: "Discord spoiler protection", hint: "Hide episode titles on Discord", target: "pref-discord-spoilers", fallback: "pref-discord-rpc", kw: "discord spoiler hide title" },
    { cat: "integrations", label: "Discord profile buttons", hint: "AniList and GitHub buttons", target: "pref-discord-buttons", fallback: "pref-discord-rpc", kw: "discord buttons anilist github" },
    { cat: "backup", label: "Export a backup", hint: "Save settings, libraries and watch history to a file", target: "set-export", kw: "backup export save transfer move computer" },
    { cat: "backup", label: "Restore from a backup", hint: "Import a Kura backup file", target: "set-import", kw: "backup import restore load transfer" },
    { cat: "backup", label: "Automatic safety copy", hint: "Where Kura's automatic backups live", target: "set-safety", kw: "backup automatic safety reinstall" },
    { cat: "updates", label: "Automatic update checks", hint: "Look for new versions on start", target: "pref-auto-update", kw: "update auto check" },
    { cat: "updates", label: "Check for updates", hint: "See if a new version is out", target: "update-check", kw: "update version new release" },
    { cat: "updates", label: "What's New", hint: "Read the changelog", target: "view-changelog", kw: "changelog release notes whats new" },
    { cat: "about", label: "About Kura", hint: "Version, credits and links", target: "set-about", kw: "about version github credits" },
    { cat: "about", label: "Diagnostics", hint: "Copy a report for bug reports", target: "set-diagnostics", kw: "diagnostics debug bug report issue copy log problem help" },
    { cat: "about", label: "Detected players", hint: "Where Kura found mpv, VLC, MPC and Memento", target: "set-players-detected", kw: "detect redetect player mpv vlc mpc memento path found" },
    { cat: "about", label: "Log file", hint: "Open Kura's log folder", target: "set-diagnostics", kw: "log logs file folder kura.log" },
  ];

  let query = $state("");
  const results = $derived.by(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return [];
    return INDEX.filter((e) => {
      const hay = `${e.label} ${e.hint} ${e.kw} ${catOf(e.cat).label}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    });
  });

  async function jump(e: Entry) {
    go(e.cat);
    await tick();
    const el = document.getElementById(e.target) ?? (e.fallback ? document.getElementById(e.fallback) : null);
    if (!el) return;
    const box = (el.closest(".row, .field, .opt-head, .block") as HTMLElement | null) ?? el;
    box.scrollIntoView({ behavior: "smooth", block: "center" });
    box.classList.remove("flash");
    void box.offsetWidth; // restart the animation if it's already flashing
    box.classList.add("flash");
    setTimeout(() => box.classList.remove("flash"), 1700);
  }

  function onWindowKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      searchEl?.focus();
      searchEl?.select();
    }
  }
</script>

<svelte:window onkeydown={onWindowKey} />

<svelte:head><title>Settings · Kura</title></svelte:head>

<div class="page settings-page" bind:this={rootEl}>
  <header class="head">
    <h1>Settings</h1>
    <p class="muted">Tune Kura to the way you watch.</p>
  </header>

  <div class="layout">
  <aside class="side">
    <div class="search">
      <Icon name="search" size={15} />
      <input
        bind:this={searchEl}
        bind:value={query}
        type="search"
        placeholder="Search settings…"
        aria-label="Search settings"
        id="settings-search"
        autocomplete="off"
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === "Escape") query = "";
          if (e.key === "Enter" && results[0]) jump(results[0]);
        }}
      />
      {#if query}
        <button class="clear" onclick={() => (query = "")} aria-label="Clear search" id="settings-search-clear">
          <Icon name="x" size={13} />
        </button>
      {:else}
        <kbd>Ctrl F</kbd>
      {/if}
    </div>
    <nav class="cats" aria-label="Settings categories">
      {#each CATEGORIES as c (c.id)}
        <button
          class="cat"
          class:on={active === c.id && !query}
          aria-current={active === c.id && !query ? "page" : undefined}
          onclick={() => go(c.id)}
          id={`settings-cat-${c.id}`}
        >
          <span class="cat-icon"><Icon name={c.icon} size={16} /></span>
          <span class="cat-label">{c.label}</span>
        </button>
      {/each}
    </nav>
  </aside>

  <div class="content">
  {#if query.trim()}
    <section class="block results">
      <h2 class="section-title">{results.length} result{results.length === 1 ? "" : "s"} for “{query.trim()}”</h2>
      {#if results.length}
        <div class="result-list">
          {#each results as r (r.cat + r.label)}
            <button class="result" onclick={() => jump(r)}>
              <span class="r-icon"><Icon name={catOf(r.cat).icon} size={16} /></span>
              <span class="r-text">
                <strong>{r.label}</strong>
                <small>{catOf(r.cat).label} · {r.hint}</small>
              </span>
              <Icon name="chevron-right" size={16} />
            </button>
          {/each}
        </div>
      {:else}
        <div class="card empty-search">
          <strong>Nothing matches that.</strong>
          <p class="faint">Try a simpler word like “subtitle”, “theme” or “mpv”.</p>
        </div>
      {/if}
    </section>
  {:else}
  {#key active}
  <div class="pane">
  {#if active === "libraries"}
  <section class="block" id="set-libraries">
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

    <div class="card playback watcher-card">
      <div class="rows">
        <div class="row">
          <div class="row-text">
            <strong>Live folder watcher (Auto-rescan)</strong>
            <small>Automatically detect newly added, renamed, or deleted episodes and keep your library in sync without manual rescans.</small>
          </div>
          <button
            class="switch"
            class:on={app.folderWatcher}
            role="switch"
            aria-checked={app.folderWatcher}
            aria-label="Live folder watcher"
            onclick={() => app.setFolderWatcher(!app.folderWatcher)}
            id="pref-folder-watcher"
          ><span></span></button>
        </div>
      </div>
    </div>
  </section>
  {/if}

  {#if active === "appearance"}
  <section class="block" id="set-appearance">
    <h2 class="section-title">Appearance</h2>
    <div class="card display">
      <div class="opt-head" id="set-theme">
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

      <div class="opt-head" id="set-seasons">
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
  {/if}

  {#if active === "artwork"}
  <section class="block" id="set-artwork">
    <h2 class="section-title">Artwork & Metadata</h2>
    <div class="card display">
      <div class="opt-head" id="set-pipeline">
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

      <div class="opt-head" id="set-fanart">
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

      <div class="opt-head" id="set-rebuild">
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
  {/if}

  {#if active === "playback"}
  <section class="block" id="set-playback">
    <h2 class="section-title">Playback</h2>
    <div class="card playback">
      <div class="opt-head" id="set-player">
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

      {#if !found("mpv")}
        <div class="mpv-banner">
          <div class="mpv-banner-left">
            <span class="burst-mini" aria-hidden="true"><span>⚡</span></span>
            <div>
              <strong>MPV Player is recommended for Kura</strong>
              <p class="faint">Unlocks 10-bit playback, smart OP/ED skip, seamless autoplay, and subtitle sync hotkeys.</p>
            </div>
          </div>
          <button class="btn btn-sm btn-accent" onclick={() => mpvModal.open()} id="settings-install-mpv">
            <Icon name="download" size={14} /> Install MPV
          </button>
        </div>
      {/if}

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
  {/if}

  {#if active === "tracks"}
  <section class="block" id="set-tracks-section">
    <h2 class="section-title">Audio & Subtitles</h2>
    <div class="card playback">
      <div class="opt-head" id="set-tracks">
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
  {/if}

  {#if active === "integrations"}
  <section class="block" id="set-integrations">
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
  {/if}

  {#if active === "updates"}
  <section class="block" id="set-updates">
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
          <div class="row-actions">
            <button
              class="btn btn-sm btn-ghost"
              onclick={() => changelogModal.open()}
              id="view-changelog"
              title="View What's New changelog"
            >
              <Icon name="sparkles" size={14} /> What's New
            </button>
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
    </div>
  </section>
  {/if}

  {#if active === "backup"}
  <section class="block" id="set-backup">
    <h2 class="section-title">Backup & Restore</h2>
    <div class="card playback">
      <div class="opt-head" id="set-export">
        <h3>Export a backup</h3>
        <p class="faint">
          Saves your settings, libraries, watch history, resume points, per-show audio &amp; subtitle choices and fixed
          matches into one small file. Artwork and episode info aren't included — Kura re-downloads those.
        </p>
      </div>
      <div class="backup-actions">
        <button class="btn btn-primary" onclick={doExport} disabled={exporting} id="backup-export">
          {#if exporting}<span class="spinner"></span>{:else}<Icon name="download" size={15} />{/if}
          Export backup…
        </button>
        {#if exported}
          <span class="backup-done">
            <Icon name="check" size={14} stroke={3} />
            Saved {plural(exported.watched, "watched episode")} from {plural(exported.showsWatched, "show")} to
            <button class="linkish" onclick={() => exported && reveal(exported.path)} title={exported.path}>{fileName(exported.path)}</button>
          </span>
        {/if}
      </div>

      <div class="opt-divider"></div>

      <div class="opt-head" id="set-import">
        <h3>Restore from a backup</h3>
        <p class="faint">
          Merges a backup into this library — nothing gets deleted. Episodes you've watched here stay watched and the newer
          resume point wins, so it's safe to import the same file twice.
        </p>
      </div>

      {#if !preview}
        <div class="backup-actions">
          <button class="btn" onclick={pickImport} id="backup-import-pick">
            <Icon name="folder-open" size={15} /> Choose backup file…
          </button>
        </div>
      {:else}
        <div class="backup-preview">
          <div class="bp-head">
            <span class="bp-icon"><Icon name="archive" size={18} /></span>
            <div class="bp-title">
              <strong title={preview.path}>{fileName(preview.path)}</strong>
              <small>
                Made {relativeTime(preview.createdAt)}{preview.appVersion ? ` with Kura ${preview.appVersion}` : ""}
                {#if preview.format < 2}· older backup (settings &amp; libraries only){/if}
              </small>
            </div>
            <button class="btn btn-ghost btn-sm btn-icon" onclick={() => (preview = null)} aria-label="Cancel import">
              <Icon name="x" size={14} />
            </button>
          </div>

          <div class="bp-options">
            {#each importRows as row (row.key)}
              <label class="bp-opt" class:off={row.count === 0}>
                <input
                  type="checkbox"
                  bind:checked={importOpts[row.key]}
                  disabled={row.count === 0}
                  id={`backup-opt-${row.key}`}
                />
                <span class="bp-check" aria-hidden="true"><Icon name="check" size={12} stroke={3.5} /></span>
                <span class="bp-text">
                  <strong>{row.label}</strong>
                  <small>{row.count === 0 ? "Not in this backup" : row.detail}</small>
                </span>
              </label>
            {/each}
          </div>

          <div class="backup-actions">
            <button class="btn btn-primary" onclick={doImport} disabled={importing || !anySelected} id="backup-import-confirm">
              {#if importing}<span class="spinner"></span> Importing…{:else}<Icon name="check" size={15} /> Import selected{/if}
            </button>
            <button class="btn btn-ghost" onclick={() => (preview = null)} disabled={importing}>Cancel</button>
          </div>
        </div>
      {/if}

      {#if importReport}
        <div class="note ok-note">
          <Icon name="check" size={15} />
          <div>
            Imported {reportText(importReport)}.
            {#if importReport.librariesAdded || importReport.foldersAdded || importReport.matches}
              A scan is running to pick up the files.
            {/if}
            {#if importReport.matchesSkipped}
              {plural(importReport.matchesSkipped, "fixed match", "fixed matches")} couldn't be placed because the library
              or folder doesn't exist here.
            {/if}
          </div>
        </div>
      {/if}
    </div>

    <div class="card playback safety-card">
      <div class="opt-head" id="set-safety">
        <h3>Automatic safety copy</h3>
        <p class="faint">
          Kura keeps an up-to-date copy of all of the above outside its data folder and uses it to refill an empty
          library after a reinstall. Nothing to do here — it just runs.
        </p>
      </div>
      <div class="folders safety-list">
        {#each safety as s (s.path)}
          <div class="folder">
            <Icon name="file" size={16} />
            <span class="path" title={s.path}>{s.path}</span>
            <span class="badge" class:badge-ok={!!s.modified}>
              {s.modified ? `Updated ${relativeTime(s.modified)}` : "Not written yet"}
            </span>
            {#if s.modified}
              <button class="btn btn-ghost btn-sm btn-icon" onclick={() => reveal(s.path)} title="Show in folder">
                <Icon name="folder-open" size={14} />
              </button>
            {/if}
          </div>
        {:else}
          <p class="faint">Checking…</p>
        {/each}
      </div>
    </div>
  </section>
  {/if}

  {#if active === "about"}
  <section class="block" id="set-about">
    <h2 class="section-title">About</h2>
    <div class="card about">
      <div class="about-hero">
        <img class="about-logo" src="/brand/logo.png" alt="Kura Logo" width="48" height="48" />
        <div class="about-title-block">
          <h3>
            Kura 蔵
            <span class="badge badge-accent">v{version || "0.3.7"}</span>
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
          Advanced library filters, Jimaku subtitle fetching with smart matching, AniList / MyAnimeList sync, and
          Syncplay watch-together sessions.
        </p>
      </div>
    </div>

    <div class="card playback diag-card">
      <div class="opt-head" id="set-diagnostics">
        <h3>Diagnostics</h3>
        <p class="faint">
          Something not working? Copy this report into a GitHub issue. It lists your Kura version, library folders,
          detected players, settings (API keys hidden) and recent log lines — note that it includes folder paths.
        </p>
      </div>
      <div class="backup-actions">
        <button class="btn btn-primary" onclick={copyDiag} disabled={diagLoading && !diag} id="diag-copy">
          {#if copied}<Icon name="check" size={15} stroke={3} /> Copied!{:else}<Icon name="copy" size={15} /> Copy diagnostics{/if}
        </button>
        <button class="btn" onclick={loadDiag} disabled={diagLoading} id="diag-redetect">
          {#if diagLoading}<span class="spinner"></span>{:else}<Icon name="refresh" size={15} />{/if} Re-detect players
        </button>
        {#if diag}
          <button class="btn btn-ghost" onclick={() => diag && reveal(diag.logPath)} id="diag-open-log">
            <Icon name="folder-open" size={15} /> Open log folder
          </button>
        {/if}
      </div>

      <h4 class="diag-sub" id="set-players-detected">Players found on this computer</h4>
      <div class="folders">
        {#each PLAYERS.filter((p) => p.kind !== "system") as p (p.kind)}
          {@const hit = diag?.players.find((d) => d.kind === p.kind)}
          <div class="folder" class:offline={!!diag && !hit}>
            <Icon name={hit ? "check" : "x"} size={15} />
            <strong class="diag-name">{p.name}</strong>
            <span class="path" title={hit?.path}>
              {hit ? hit.path : diag ? "Not found" : "Checking…"}
            </span>
            {#if effective === p.kind}<span class="badge badge-accent">In use</span>{/if}
          </div>
        {/each}
      </div>

      <button class="linkish log-toggle" onclick={() => (showLog = !showLog)} id="diag-toggle-log">
        <Icon name={showLog ? "chevron-down" : "chevron-right"} size={14} />
        {showLog ? "Hide" : "Show"} recent log{diag ? ` (${diag.logTail.length} lines)` : ""}
      </button>
      {#if showLog}
        <pre class="log-view">{diag?.logTail.length ? diag.logTail.join("\n") : "Nothing logged yet."}</pre>
      {/if}
    </div>
  </section>
  {/if}
  </div>
  {/key}
  {/if}
  </div>
  </div>
</div>

<style>
  .settings-page {
    max-width: 1180px;
  }
  .layout {
    display: grid;
    grid-template-columns: 236px minmax(0, 1fr);
    gap: 32px;
    align-items: start;
  }
  .content {
    min-width: 0;
  }
  .pane {
    animation: fade-up 260ms var(--ease);
  }

  /* Sidebar */
  .side {
    position: sticky;
    top: 12px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 10px 0 12px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--bg-elev);
    color: var(--text-3);
    box-shadow: 2px 2px 0 var(--line);
    transition:
      box-shadow var(--t-fast) var(--ease),
      transform var(--t-fast) var(--ease);
  }
  .search:focus-within {
    color: var(--coral);
    transform: translate(-1px, -1px);
    box-shadow: 3px 3px 0 var(--coral);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 14px;
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  .search kbd {
    font-family: var(--font);
    font-size: 10.5px;
    font-weight: 700;
    padding: 2px 6px;
    border-radius: 6px;
    border: 1.5px solid var(--border-strong);
    color: var(--text-3);
    white-space: nowrap;
  }
  .search .clear {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 50%;
    background: var(--surface);
    color: var(--text-2);
    cursor: pointer;
  }
  .search .clear:hover {
    background: var(--coral);
    color: var(--on-coral);
  }
  .cats {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .cat {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 8px 10px;
    border-radius: var(--r-md);
    border: var(--bw) solid transparent;
    background: transparent;
    color: var(--text-2);
    font: inherit;
    font-size: 14px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease),
      transform var(--t-fast) var(--bounce),
      box-shadow var(--t-fast) var(--ease);
  }
  .cat:hover {
    background: var(--surface);
    color: var(--text);
    transform: translateX(2px);
  }
  .cat-icon {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    flex: none;
    border-radius: 9px;
    background: var(--surface);
    color: var(--text-2);
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease);
  }
  .cat-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cat.on {
    background: var(--bg-elev);
    border-color: var(--line);
    color: var(--text);
    box-shadow: 3px 3px 0 var(--line);
    transform: none;
  }
  .cat.on .cat-icon {
    background: var(--coral);
    color: var(--on-coral);
  }

  /* Search results */
  .result-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .result {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 16px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--bg-elev);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    box-shadow: 2px 2px 0 var(--line);
    transition:
      transform var(--t-fast) var(--bounce),
      box-shadow var(--t-fast) var(--ease);
  }
  .result:hover,
  .result:focus-visible {
    transform: translate(-2px, -2px);
    box-shadow: 4px 4px 0 var(--coral);
  }
  .r-icon {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    flex: none;
    border-radius: 10px;
    background: var(--coral);
    color: var(--on-coral);
  }
  .r-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .r-text small {
    color: var(--text-3);
    font-size: 12.5px;
    overflow-wrap: anywhere;
  }
  .empty-search {
    padding: 22px;
  }
  .content :global(.flash) {
    animation: -global-kura-settings-flash 1.6s var(--ease);
    border-radius: var(--r-md);
  }
  @keyframes -global-kura-settings-flash {
    0%,
    60% {
      box-shadow: 0 0 0 3px var(--coral);
      background-color: color-mix(in srgb, var(--coral) 12%, transparent);
    }
    100% {
      box-shadow: 0 0 0 3px transparent;
    }
  }

  @media (max-width: 860px) {
    .layout {
      grid-template-columns: 1fr;
      gap: 20px;
    }
    .side {
      position: static;
    }
    .cats {
      flex-direction: row;
      overflow-x: auto;
      padding-bottom: 6px;
    }
    .cat {
      flex: none;
    }
    .cat:hover {
      transform: none;
    }
  }

  /* Backup & Restore */
  .backup-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 14px;
    margin-top: 14px;
  }
  .backup-done {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: 13px;
    color: var(--text-2);
    min-width: 0;
  }
  .backup-done :global(svg) {
    color: var(--ok);
  }
  .linkish {
    border: 0;
    padding: 0;
    background: none;
    color: var(--coral);
    font: inherit;
    font-weight: 700;
    cursor: pointer;
    overflow-wrap: anywhere;
    text-align: left;
  }
  .linkish:hover {
    text-decoration: underline;
  }
  .backup-preview {
    margin-top: 14px;
    padding: 16px;
    border-radius: var(--r-md);
    border: var(--bw) solid var(--line);
    background: var(--bg);
    box-shadow: 3px 3px 0 var(--line);
    animation: fade-up 260ms var(--ease);
  }
  .bp-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .bp-icon {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    flex: none;
    border-radius: 11px;
    border: var(--bw) solid var(--line);
    background: var(--coral);
    color: var(--on-coral);
    box-shadow: 2px 2px 0 var(--line);
  }
  .bp-title {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .bp-title strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bp-title small {
    color: var(--text-3);
    font-size: 12.5px;
  }
  .bp-options {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 10px;
    margin-top: 14px;
  }
  .bp-opt {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: 11px;
    padding: 11px 12px;
    border-radius: var(--r-sm);
    border: var(--bw) solid var(--border-strong);
    background: var(--bg-elev);
    cursor: pointer;
    transition:
      border-color var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .bp-opt:hover {
    border-color: var(--coral);
  }
  .bp-opt:has(input:checked) {
    border-color: var(--line);
    background: color-mix(in srgb, var(--coral) 10%, var(--bg-elev));
  }
  .bp-opt:has(input:focus-visible) {
    outline: 2px solid var(--coral);
    outline-offset: 2px;
  }
  .bp-opt.off {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .bp-opt input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .bp-check {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    flex: none;
    margin-top: 1px;
    border-radius: 6px;
    border: var(--bw) solid var(--line);
    background: var(--surface);
    color: transparent;
    transition:
      background var(--t-fast) var(--ease),
      color var(--t-fast) var(--ease),
      transform var(--t-fast) var(--bounce);
  }
  .bp-opt:has(input:checked) .bp-check {
    background: var(--coral);
    color: var(--on-coral);
    transform: scale(1.06);
  }
  .bp-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .bp-text strong {
    font-size: 14px;
  }
  .bp-text small {
    color: var(--text-3);
    font-size: 12.5px;
    overflow-wrap: anywhere;
  }
  .ok-note {
    margin-top: 14px;
    border-color: color-mix(in srgb, var(--ok) 60%, var(--line));
  }
  .ok-note :global(svg) {
    color: var(--ok);
  }
  .safety-card {
    margin-top: 18px;
  }
  .safety-list {
    margin-top: 12px;
  }

  /* Diagnostics */
  .diag-card {
    margin-top: 18px;
  }
  .diag-sub {
    margin: 22px 0 10px;
    font-size: 14px;
    color: var(--text-2);
  }
  .diag-name {
    flex: none;
    min-width: 92px;
    font-size: 13.5px;
  }
  .log-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 16px;
    font-size: 13px;
  }
  .log-view {
    margin-top: 10px;
    max-height: 320px;
    overflow: auto;
    padding: 12px 14px;
    border-radius: var(--r-sm);
    border: var(--bw) solid var(--line);
    background: var(--bg);
    color: var(--text-2);
    font-family: ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 11.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    animation: fade-up 220ms var(--ease);
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
  .mpv-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-top: 14px;
    padding: 12px 16px;
    background: var(--surface-2);
    border: var(--bw) solid var(--coral);
    border-radius: var(--r-md);
    box-shadow: var(--sticker);
  }
  .mpv-banner-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .mpv-banner-left strong {
    font-size: 13.5px;
    color: var(--text);
  }
  .mpv-banner-left p {
    font-size: 12px;
    margin-top: 2px;
  }
  .burst-mini {
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    background: var(--amber);
    clip-path: polygon(
      50% 0%, 61% 15%, 79% 7%, 80% 26%, 98% 30%, 88% 46%, 100% 61%, 82% 69%, 85% 88%, 66% 85%, 55% 100%, 44% 86%,
      26% 94%, 23% 75%, 4% 72%, 13% 55%, 0% 40%, 17% 31%, 13% 12%, 33% 14%
    );
    font-size: 13px;
    flex-shrink: 0;
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
  .row-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
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
