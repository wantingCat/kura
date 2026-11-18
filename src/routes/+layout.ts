// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;

/**
 * Browser-preview mode (vite dev outside Tauri): install the IPC mock BEFORE any
 * page renders. Doing it in +layout's onMount was too late — child pages' own
 * onMount (e.g. /calendar) ran first and hit `invoke` with no backend.
 */
export async function load() {
  if (import.meta.env.DEV && typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window)) {
    // mockIPC defines window.__TAURI_INTERNALS__, so record preview mode explicitly first.
    (window as unknown as { __KURA_PREVIEW__?: boolean }).__KURA_PREVIEW__ = true;
    (await import("$lib/dev/mock")).installMock();
  }
  return {};
}
