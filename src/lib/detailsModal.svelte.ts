/**
 * Global state for the Tokyo Night details modal (anime quick view).
 *
 * Open it from anywhere (hover cards, hero "More Info", search results):
 *   detailsModal.open(anilistId)
 * The modal mirrors its state into the URL (`?anime=<id>`) so the browser /
 * mouse Back button closes it, and deep links reopen it.
 */
import { goto } from "$app/navigation";
import { page } from "$app/state";

class DetailsModalStore {
  /** AniList id of the anime currently shown, or null when closed. */
  anilistId = $state<number | null>(null);
  /** Element that had focus before opening, restored on close. */
  #returnFocus: HTMLElement | null = null;

  get isOpen() {
    return this.anilistId !== null;
  }

  open(anilistId: number) {
    if (typeof document !== "undefined") {
      this.#returnFocus = document.activeElement as HTMLElement | null;
    }
    this.anilistId = anilistId;
    try {
      const url = new URL(page.url);
      if (url.searchParams.get("anime") !== String(anilistId)) {
        url.searchParams.set("anime", String(anilistId));
        goto(url, { keepFocus: true, noScroll: true, replaceState: false });
      }
    } catch {
      /* not in a routed context (e.g. tests) */
    }
  }

  close() {
    if (this.anilistId === null) return;
    this.anilistId = null;
    try {
      const url = new URL(page.url);
      if (url.searchParams.has("anime")) {
        url.searchParams.delete("anime");
        goto(url, { keepFocus: true, noScroll: true, replaceState: true });
      }
    } catch {
      /* ignore */
    }
    const el = this.#returnFocus;
    this.#returnFocus = null;
    queueMicrotask(() => el?.focus?.());
  }

  /** Keep store in sync when the URL changes (Back / Forward / deep link). */
  syncFromUrl(url: URL) {
    const raw = url.searchParams.get("anime");
    const id = raw ? Number(raw) : NaN;
    if (Number.isFinite(id) && id > 0) {
      if (this.anilistId !== id) this.anilistId = id;
    } else if (this.anilistId !== null) {
      this.anilistId = null;
    }
  }
}

export const detailsModal = new DetailsModalStore();
