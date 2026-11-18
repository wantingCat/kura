<script lang="ts">
  import "@fontsource-variable/inter";
  import "@fontsource-variable/noto-sans-jp";
  import "@fontsource-variable/space-grotesk";
  import "$lib/styles/global.css";
  import { onMount } from "svelte";
  import { afterNavigate } from "$app/navigation";
  import { page } from "$app/state";
  import { app } from "$lib/app.svelte";
  import { detailsModal } from "$lib/detailsModal.svelte";
  import TopBar from "$lib/components/TopBar.svelte";
  import UpdateBanner from "$lib/components/UpdateBanner.svelte";
  import Toasts from "$lib/components/Toasts.svelte";
  import MpvInstallModal from "$lib/components/MpvInstallModal.svelte";
  import BuiltinPlayerModal from "$lib/components/BuiltinPlayerModal.svelte";
  import DetailsModal from "$lib/components/DetailsModal.svelte";
  import { mpvModal } from "$lib/mpvModal.svelte";
  import ChangelogModal from "$lib/components/ChangelogModal.svelte";
  import { changelogModal } from "$lib/changelogModal.svelte";

  let { children } = $props();
  let main: HTMLElement | undefined = $state();

  onMount(async () => {
    // (Browser-preview IPC mock is installed in +layout.ts load, before any page renders.)
    await app.init();
    await changelogModal.checkOnLaunch();
    mpvModal.checkInitialLaunch();
  });

  afterNavigate(({ type, from, to }) => {
    // Opening/closing the details modal only changes `?anime=`; don't jump the page.
    const samePath =
      !!from?.url && !!to?.url && from.url.pathname === to.url.pathname;
    if (type !== "popstate" && !samePath)
      main?.scrollTo({ top: 0, behavior: "instant" });
  });

  // Keep the details modal in sync with `?anime=<id>` (Back button closes it).
  $effect(() => {
    detailsModal.syncFromUrl(page.url);
  });

  $effect(() => {
    const activeLibmpv = Boolean(
      app.builtinPlayer?.active && app.builtinPlayer.backend === "libmpv",
    );
    document.documentElement.classList.toggle("libmpv-active", activeLibmpv);
    return () => {
      document.documentElement.classList.remove("libmpv-active");
    };
  });
</script>

<div
  class="shell"
  class:player-hidden={app.builtinPlayer?.active &&
    app.builtinPlayer.backend === "libmpv"}
>
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

<DetailsModal />
<Toasts />
<MpvInstallModal />
<ChangelogModal />

<style>
  /* Tokyo Night: floating glass navbar overlays the page (position: fixed inside
     TopBar), so the shell is a single scrolling column. Pages that don't start
     with a full-bleed hero add top padding via `.page` (see global.css). */
  .shell {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
    background: transparent;
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
