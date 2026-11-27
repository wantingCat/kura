import { api, type EpisodeRow, type MediaDetail } from "./api";
import { displayTitle } from "./format";

// ---------------------------------------------------------------------------
// Shared Bounded LRU `MediaDetail` Cache (Max 32 Entries)
// ---------------------------------------------------------------------------

const MAX_CACHE_ENTRIES = 32;
const detailLru = new Map<number, MediaDetail>();

export function getCachedMediaDetail(anilistId: number): MediaDetail | undefined {
  const hit = detailLru.get(anilistId);
  if (!hit) return undefined;
  // Promote to MRU
  detailLru.delete(anilistId);
  detailLru.set(anilistId, hit);
  return hit;
}

export function setCachedMediaDetail(detail: MediaDetail): void {
  if (detailLru.has(detail.anilistId)) {
    detailLru.delete(detail.anilistId);
  }
  detailLru.set(detail.anilistId, detail);
  while (detailLru.size > MAX_CACHE_ENTRIES) {
    const oldestKey = detailLru.keys().next().value;
    if (oldestKey === undefined) break;
    detailLru.delete(oldestKey);
  }
}

export function clearCachedMediaDetails(): void {
  detailLru.clear();
}

export function invalidateCachedMediaDetail(anilistId: number): void {
  detailLru.delete(anilistId);
}

export async function getCachedOrLoadMediaDetail(
  anilistId: number,
  forceRefresh = false,
): Promise<MediaDetail> {
  if (!forceRefresh) {
    const cached = getCachedMediaDetail(anilistId);
    if (cached) return cached;
  }
  const loaded = await api.getMediaDetail(anilistId);
  setCachedMediaDetail(loaded);
  return loaded;
}

// ---------------------------------------------------------------------------
// Shared Detail Helpers (`DetailsModal` & `/anime/[id]`)
// ---------------------------------------------------------------------------

interface AppHooks {
  refresh: () => Promise<void>;
  toast: (text: string, kind?: "info" | "warn" | "error", ms?: number) => void;
}

let appHooks: AppHooks | null = null;

export function registerMediaDetailHooks(hooks: AppHooks): void {
  appHooks = hooks;
}

/** An episode left part-way through wins over the first unwatched one. */
export function computeNextUp(
  detail: MediaDetail | null | undefined,
  appMode: "local" | "streaming",
): EpisodeRow | null {
  if (!detail) return null;
  const regular = detail.episodeList.filter((e) => !e.isSpecial);
  return (
    regular.find((e) => !e.watchedAt && e.files.length > 0 && (e.progressPos ?? 0) > 0) ??
    regular.find((e) => !e.watchedAt && e.files.length > 0) ??
    (appMode === "streaming"
      ? (regular.find((e) => !e.watchedAt && (e.progressPos ?? 0) > 0) ??
          regular.find((e) => !e.watchedAt && e.number > (detail.anilistProgress ?? 0)) ??
          regular[0] ??
          null)
      : null)
  );
}

/** Toggle a show in Kura's stream library, updating the shared cache and refreshing app state. */
export async function toggleMediaStreamLibrary(detail: MediaDetail): Promise<boolean> {
  const next = await api.toggleStreamLibrary(detail.anilistId);
  detail.inStreamLibrary = next;
  setCachedMediaDetail(detail);
  await appHooks?.refresh();
  appHooks?.toast(
    next ? `Added ${displayTitle(detail)} to your library` : "Removed from stream library",
    "info",
    3500,
  );
  return next;
}
