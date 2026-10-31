// Library filter & sort state.
//
// Lives in a module (not in the page) so filters survive opening a show and coming back.
// Sort choice is also remembered across restarts.

import type { MediaCard } from "$lib/api";

export type FormatFilter = "all" | "tv" | "movie" | "extra";
export type WatchFilter = "all" | "unwatched" | "watching" | "completed";
export type AiringFilter = "all" | "airing" | "finished" | "upcoming";
export type SortKey = "added" | "watched" | "title" | "year" | "score" | "progress";
export type FacetKey = "genres" | "tags" | "studios";

export const SORTS: { key: SortKey; label: string; desc: boolean }[] = [
  { key: "added", label: "Date added", desc: true },
  { key: "watched", label: "Recently watched", desc: true },
  { key: "title", label: "Title", desc: false },
  { key: "year", label: "Release year", desc: true },
  { key: "score", label: "Score", desc: true },
  { key: "progress", label: "Progress", desc: true },
];

export const AIRING: { key: AiringFilter; label: string }[] = [
  { key: "all", label: "Any" },
  { key: "airing", label: "Airing" },
  { key: "finished", label: "Finished" },
  { key: "upcoming", label: "Not yet aired" },
];

export function airingOk(c: MediaCard, f: AiringFilter): boolean {
  switch (f) {
    case "airing":
      return c.status === "RELEASING" || c.status === "HIATUS";
    case "finished":
      return c.status === "FINISHED" || c.status === "CANCELLED";
    case "upcoming":
      return c.status === "NOT_YET_RELEASED";
    default:
      return true;
  }
}

const SORT_KEY = "kura.library.sort";

function loadSort(): { sort: SortKey; desc: boolean } {
  try {
    const v = JSON.parse(localStorage.getItem(SORT_KEY) ?? "null");
    if (v && SORTS.some((s) => s.key === v.sort) && typeof v.desc === "boolean") return v;
  } catch {
    /* ignore */
  }
  return { sort: "added", desc: true };
}

class LibraryFilters {
  format = $state<FormatFilter>("all");
  watch = $state<WatchFilter>("all");
  airing = $state<AiringFilter>("all");
  genres = $state<string[]>([]);
  tags = $state<string[]>([]);
  studios = $state<string[]>([]);
  sort = $state<SortKey>("added");
  desc = $state(true);
  panelOpen = $state(false);

  constructor() {
    const s = loadSort();
    this.sort = s.sort;
    this.desc = s.desc;
  }

  /** Filters that live in the "Filters" panel. */
  get advancedCount() {
    return this.genres.length + this.tags.length + this.studios.length + (this.airing !== "all" ? 1 : 0);
  }

  get active() {
    return this.format !== "all" || this.watch !== "all" || this.advancedCount > 0;
  }

  has(facet: FacetKey, value: string) {
    return this[facet].includes(value);
  }

  toggle(facet: FacetKey, value: string) {
    const list = this[facet];
    this[facet] = list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
  }

  clearAdvanced() {
    this.genres = [];
    this.tags = [];
    this.studios = [];
    this.airing = "all";
  }

  clearAll() {
    this.clearAdvanced();
    this.format = "all";
    this.watch = "all";
  }

  setSort(key: SortKey) {
    this.sort = key;
    this.desc = SORTS.find((s) => s.key === key)?.desc ?? true;
    this.persist();
  }

  flipDirection() {
    this.desc = !this.desc;
    this.persist();
  }

  /** Entry-level check for the panel filters (genres: all must match, studios: any, tags: all). */
  matches(c: MediaCard) {
    if (!airingOk(c, this.airing)) return false;
    if (this.genres.length && !this.genres.every((g) => c.genres.includes(g))) return false;
    if (this.studios.length && !this.studios.some((s) => c.studios.includes(s))) return false;
    if (this.tags.length && !this.tags.every((t) => c.tags.includes(t))) return false;
    return true;
  }

  private persist() {
    try {
      localStorage.setItem(SORT_KEY, JSON.stringify({ sort: this.sort, desc: this.desc }));
    } catch {
      /* ignore */
    }
  }
}

export const filters = new LibraryFilters();

/** Value → how many entries have it, most common first. */
export function facetCounts(entries: MediaCard[], facet: FacetKey): [string, number][] {
  const m = new Map<string, number>();
  for (const c of entries) for (const v of c[facet] ?? []) m.set(v, (m.get(v) ?? 0) + 1);
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}
