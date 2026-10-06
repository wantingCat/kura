<script lang="ts">
  import "@fontsource-variable/fredoka";
  import "@fontsource-variable/nunito";
  import "$lib/styles/global.css";
  import { onMount } from "svelte";
  import { afterNavigate } from "$app/navigation";
  import { app } from "$lib/app.svelte";
  import TopBar from "$lib/components/TopBar.svelte";
  import UpdateBanner from "$lib/components/UpdateBanner.svelte";
  import Toasts from "$lib/components/Toasts.svelte";

  let { children } = $props();
  let main: HTMLElement | undefined = $state();

  onMount(async () => {
    // Opened in a plain browser during development → use the sample-data preview.
    if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
      (await import("$lib/dev/mock")).installMock();
    }
    app.init();
  });

  // Each page starts at the top (main is the scroll container, not the window).
  afterNavigate(({ type }) => {
    if (type !== "popstate") main?.scrollTo({ top: 0 });
  });
</script>

<div class="shell">
  {#if app.loaded && app.libraries.length > 0}
    <TopBar />
  {/if}
  <main bind:this={main} id="main-scroll">
    <UpdateBanner />
    {@render children()}
  </main>
  <Toasts />
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
  }
  main {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    position: relative;
    scroll-behavior: smooth;
  }
</style>
