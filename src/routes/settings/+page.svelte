<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, type Library } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import NewLibraryForm from "$lib/components/NewLibraryForm.svelte";

  let showNew = $state(false);
  let renaming = $state<number | null>(null);
  let renameValue = $state("");
  let confirmDelete = $state<number | null>(null);

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
    <p class="muted">Manage your libraries and the folders Kura scans.</p>
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
    <h2 class="section-title">About</h2>
    <div class="card about">
      <p>
        <strong>Kura</strong> <span class="faint">v0.1.1-alpha</span> · a local-first anime library. Metadata from
        <strong>AniList</strong>, episode info from <strong>ani.zip</strong> and <strong>Jikan (MyAnimeList)</strong>.
        Everything is cached locally, and your media files never leave your computer.
      </p>
      <p class="faint coming">Coming next: AniList account sync, opening episodes in Memento / mpv, and marking episodes watched automatically.</p>
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
