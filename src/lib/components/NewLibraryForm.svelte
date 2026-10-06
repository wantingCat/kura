<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import Icon from "./Icon.svelte";

  let { oncreated, compact = false }: { oncreated?: (id: number) => void; compact?: boolean } = $props();

  let name = $state("Anime");
  let folders = $state<string[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function pickFolders() {
    const picked = await open({ directory: true, multiple: true, title: "Choose anime folders" });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    folders = [...new Set([...folders, ...list])];
  }

  async function create() {
    if (!name.trim() || folders.length === 0) return;
    busy = true;
    error = null;
    try {
      const id = await api.createLibrary(name.trim(), folders);
      await app.refresh();
      await app.scan(id);
      oncreated?.(id);
      folders = [];
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="form" class:compact>
  <label class="field">
    <span>Library name</span>
    <input class="input" bind:value={name} placeholder="Anime" id="new-library-name" />
  </label>

  <div class="field">
    <span>Folders</span>
    <div class="folders">
      {#each folders as f (f)}
        <div class="folder">
          <Icon name="folder" size={16} />
          <span class="path" title={f}>{f}</span>
          <button
            class="btn btn-ghost btn-sm btn-icon"
            onclick={() => (folders = folders.filter((x) => x !== f))}
            aria-label="Remove folder"
          >
            <Icon name="x" size={14} />
          </button>
        </div>
      {/each}
      <button class="add-folder" onclick={pickFolders} id="new-library-add-folder">
        <Icon name="plus" size={16} />
        {folders.length ? "Add another folder" : "Choose folder…"}
      </button>
    </div>
  </div>

  {#if error}<p class="error">{error}</p>{/if}

  <button
    class="btn btn-primary create"
    disabled={busy || !name.trim() || folders.length === 0}
    onclick={create}
    id="new-library-create"
  >
    {#if busy}<div class="spinner"></div>{:else}<Icon name="check" size={16} stroke={3} />{/if}
    Create library & scan
  </button>
</div>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .field > span {
    font-family: var(--font-display);
    font-size: 12.5px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-3);
  }
  .folders {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 14px;
    border-radius: var(--r-md);
    background: var(--surface-2);
    border: var(--bw) solid var(--line);
    color: var(--text-2);
    animation: fade-up 240ms var(--ease);
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
  .add-folder {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 52px;
    border-radius: var(--r-md);
    border: var(--bw) dashed var(--border-strong);
    background: transparent;
    color: var(--text-2);
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }
  .add-folder:hover {
    border-color: var(--coral);
    color: var(--coral);
    background: color-mix(in srgb, var(--coral) 6%, transparent);
  }
  .create {
    height: 44px;
    align-self: flex-start;
    padding: 0 22px;
  }
  .compact .create {
    height: 38px;
  }
  .error {
    color: var(--danger);
    font-size: 13px;
  }
</style>
