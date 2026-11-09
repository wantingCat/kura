<script lang="ts">
  import "@fontsource-variable/nunito";
  import "$lib/styles/global.css";
  import { onMount } from "svelte";
  import { afterNavigate } from "$app/navigation";
  import { app } from "$lib/app.svelte";
  import TopBar from "$lib/components/TopBar.svelte";
  import UpdateBanner from "$lib/components/UpdateBanner.svelte";
  import Toasts from "$lib/components/Toasts.svelte";
  import MpvInstallModal from "$lib/components/MpvInstallModal.svelte";
  import BuiltinPlayerModal from "$lib/components/BuiltinPlayerModal.svelte";
  import { mpvModal } from "$lib/mpvModal.svelte";
  import ChangelogModal from "$lib/components/ChangelogModal.svelte";
  import { changelogModal } from "$lib/changelogModal.svelte";

  let { children } = $props();
  let main: HTMLElement | undefined = $state();

  onMount(async () => {
    if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
      (await import("$lib/dev/mock")).installMock();
    }
    await app.init();
    await changelogModal.checkOnLaunch();
    mpvModal.checkInitialLaunch();
  });

  afterNavigate(({ type }) => {
    if (type !== "popstate") main?.scrollTo({ top: 0 });
  });

  $effect(() => {
    const activeLibmpv = Boolean(
      app.builtinPlayer?.active && app.builtinPlayer.backend === "libmpv"
    );
    document.documentElement.classList.toggle("libmpv-active", activeLibmpv);
    return () => {
      document.documentElement.classList.remove("libmpv-active");
    };
  });
</script>

<div class="shell" class:player-hidden={app.builtinPlayer?.active && app.builtinPlayer.backend === "libmpv"}>
  {#if app.loaded && (app.libraries.length > 0 || app.appMode === "streaming")}
    <TopBar />
  {/if}
  <main bind:this={main} id="main-scroll">
    <UpdateBanner />
    {@render children()}
  </main>
</div>

{#if app.builtinPlayer?.active}
  <BuiltinPlayerModal playerState={app.builtinPlayer} />
{/if}

<Toasts />
<MpvInstallModal />
<ChangelogModal />

<style>
  .shell {
    display: flex;
    flex-direction: row;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
    background: var(--bg);
  }

  .shell.player-hidden {
    visibility: hidden;
    pointer-events: none;
    background: transparent;
  }

  main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    position: relative;
    scroll-behavior: smooth;
  }
</style>
