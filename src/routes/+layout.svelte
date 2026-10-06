<script lang="ts">
  import "@fontsource-variable/fredoka";
  import "@fontsource-variable/nunito";
  import "$lib/styles/global.css";
  import { onMount } from "svelte";
  import { afterNavigate } from "$app/navigation";
  import { app } from "$lib/app.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";

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
  <Sidebar />
  <main bind:this={main} id="main-scroll">
    {@render children()}
  </main>
</div>

<style>
  .shell {
    display: flex;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
  }
  main {
    flex: 1;
    min-width: 0;
    height: 100vh;
    overflow-y: auto;
    overflow-x: hidden;
    position: relative;
    scroll-behavior: smooth;
  }
</style>
