import type { MediaCard, MediaDetail } from "./api";

type Titled = Pick<MediaCard, "titleEnglish" | "titleRomaji" | "titleNative">;

export function displayTitle(m: Titled): string {
  return m.titleEnglish || m.titleRomaji || m.titleNative || "Untitled";
}

/** Secondary title shown under the main one (romaji if the main one is English). */
export function subTitle(m: Titled): string | null {
  const main = displayTitle(m);
  if (m.titleRomaji && m.titleRomaji !== main) return m.titleRomaji;
  return null;
}

const FORMATS: Record<string, string> = {
  TV: "TV",
  TV_SHORT: "TV Short",
  MOVIE: "Movie",
  SPECIAL: "Special",
  OVA: "OVA",
  ONA: "ONA",
  MUSIC: "Music",
};
export const formatLabel = (f: string | null | undefined) => (f ? (FORMATS[f] ?? f) : "");

const STATUSES: Record<string, string> = {
  FINISHED: "Finished",
  RELEASING: "Airing",
  NOT_YET_RELEASED: "Upcoming",
  CANCELLED: "Cancelled",
  HIATUS: "Hiatus",
};
export const statusLabel = (s: string | null | undefined) => (s ? (STATUSES[s] ?? s) : "");

const RELATIONS: Record<string, string> = {
  PREQUEL: "Prequel",
  SEQUEL: "Sequel",
  PARENT: "Parent story",
  SIDE_STORY: "Side story",
  SPIN_OFF: "Spin-off",
  ALTERNATIVE: "Alternative",
  SUMMARY: "Summary",
  ADAPTATION: "Adaptation",
  SOURCE: "Source",
  COMPILATION: "Compilation",
  CONTAINS: "Contains",
  OTHER: "Other",
};
export const relationLabel = (r: string) => RELATIONS[r] ?? r;

export const seasonLabel = (season: string | null, year: number | null) => {
  if (!season && !year) return "";
  const s = season ? season.charAt(0) + season.slice(1).toLowerCase() : "";
  return [s, year].filter(Boolean).join(" ");
};

/** AniList descriptions contain light HTML (<br>, <i>) even in plain mode. */
export function cleanDescription(d: string | null | undefined): string {
  if (!d) return "";
  const withBreaks = d.replace(/<br\s*\/?>/gi, "\n").replace(/<\/p>/gi, "\n");
  const doc = new DOMParser().parseFromString(withBreaks, "text/html");
  const text = doc.body.textContent ?? "";
  return text
    .replace(/\(Source:[^)]*\)/gi, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

export function totalEpisodes(m: Pick<MediaCard, "episodes" | "format"> & { ownedCount?: number }): number | null {
  if (m.episodes) return m.episodes;
  if (m.format === "MOVIE") return 1;
  return null;
}

// ---------------------------------------------------------------------------
// Franchise grouping
// ---------------------------------------------------------------------------

const SERIES_FORMATS = ["TV", "TV_SHORT", "ONA"];
export const isSeriesFormat = (f: string | null | undefined) => SERIES_FORMATS.includes(f ?? "");

/** A library card that may stand for several AniList entries of one franchise. */
export interface LibraryItem extends MediaCard {
  /** Entries merged into this card (1 for a standalone title). */
  members: MediaCard[];
  /** Where clicking the card goes — the season you're currently on. */
  linkId: number;
  yearEnd: number | null;
}

const isDone = (m: MediaCard) => {
  const t = totalEpisodes(m);
  return t !== null && t > 0 && m.watchedCount >= t;
};

function single(m: MediaCard): LibraryItem {
  return { ...m, members: [m], linkId: m.anilistId, yearEnd: null };
}

function merge(members: MediaCard[]): LibraryItem {
  if (members.length === 1) return single(members[0]);
  const ordered = [...members].sort((a, b) => a.franchiseIndex - b.franchiseIndex);
  const base = ordered[0];
  const totals = ordered.map(totalEpisodes);
  const years = ordered.map((m) => m.seasonYear).filter((y): y is number => y !== null);

  // Resume where you left off: the latest entry with progress, or the next one if that's finished.
  let linkId = base.anilistId;
  let last = -1;
  ordered.forEach((m, i) => {
    if (m.watchedCount > 0) last = i;
  });
  if (last >= 0) linkId = (isDone(ordered[last]) && ordered[last + 1] ? ordered[last + 1] : ordered[last]).anilistId;

  return {
    ...base,
    members: ordered,
    linkId,
    episodes: totals.every((t) => t !== null) ? totals.reduce<number>((s, t) => s + (t ?? 0), 0) : null,
    // A movie on its own still reads as "1 file"; a mixed franchise is counted in episodes.
    format: ordered.every((m) => m.format === base.format) ? base.format : "TV",
    status: ordered.some((m) => m.status === "RELEASING") ? "RELEASING" : ordered[ordered.length - 1].status,
    ownedCount: ordered.reduce((s, m) => s + m.ownedCount, 0),
    watchedCount: ordered.reduce((s, m) => s + m.watchedCount, 0),
    addedAt: Math.max(...ordered.map((m) => m.addedAt)),
    lastWatchedAt: Math.max(0, ...ordered.map((m) => m.lastWatchedAt ?? 0)) || null,
    needsReview: ordered.some((m) => m.needsReview),
    libraryIds: [...new Set(ordered.flatMap((m) => m.libraryIds))],
    averageScore: base.averageScore,
    seasonYear: years.length ? Math.min(...years) : null,
    yearEnd: years.length ? Math.max(...years) : null,
  };
}

/** Collapse cards into one item per franchise (or wrap them 1:1 when grouping is off). */
export function groupCards(cards: MediaCard[], grouped: boolean): LibraryItem[] {
  if (!grouped) return cards.map(single);
  const byFranchise = new Map<number, MediaCard[]>();
  for (const c of cards) {
    const list = byFranchise.get(c.franchiseId);
    if (list) list.push(c);
    else byFranchise.set(c.franchiseId, [c]);
  }
  return [...byFranchise.values()].map(merge);
}

/** "3 seasons", "2 seasons + 1", "4 parts" — short badge for grouped cards. */
export function franchiseBadge(item: LibraryItem): string | null {
  const n = item.members.length;
  if (n < 2) return null;
  const series = item.members.filter((m) => isSeriesFormat(m.format)).length;
  if (series === n) return `${n} seasons`;
  if (series === 0) return `${n} parts`;
  return `${series} season${series === 1 ? "" : "s"} + ${n - series}`;
}

type Labelled = Titled & { anilistId: number; format: string | null };

function stripPrefix(title: string | null, base: string | null): string | null {
  if (!title || !base || title.length <= base.length) return null;
  if (!title.toLowerCase().startsWith(base.toLowerCase())) return null;
  const rest = title
    .slice(base.length)
    .replace(/^[\s:\-–—~,.!]+/, "")
    .replace(/[\s\-–—~]+$/, "")
    .trim();
  if (!/[\p{L}\p{N}]/u.test(rest)) return null; // "Love is War?" vs "Love is War"
  return /^\d+$/.test(rest) ? `Season ${rest}` : rest;
}

const MARKER =
  /\b(final season(?: part \d+)?|(?:season|cour) \d+(?: part \d+)?|\d+(?:st|nd|rd|th) season(?: part \d+)?|part \d+)\b/i;

function seasonMarker(m: Titled): string | null {
  for (const t of [m.titleEnglish, m.titleRomaji]) {
    const hit = t?.match(MARKER)?.[1];
    if (hit) return hit.replace(/\b\p{L}/gu, (c) => c.toUpperCase());
  }
  return null;
}

/**
 * Short tab label for an entry inside its franchise: strips the shared franchise
 * title so "Attack on Titan Season 3 Part 2" becomes "Season 3 Part 2". Falls back
 * to the full title rather than guessing a season number.
 */
export function franchiseLabel(entry: Labelled, root: Labelled): string {
  if (entry.anilistId === root.anilistId) {
    const marker = seasonMarker(entry);
    if (marker) return marker;
    // The earliest *owned* entry. Only call it Season 1 when nothing says otherwise.
    if (isSeriesFormat(entry.format)) return "Season 1";
    return formatLabel(entry.format) || displayTitle(entry);
  }
  return (
    stripPrefix(entry.titleEnglish, root.titleEnglish) ??
    stripPrefix(entry.titleRomaji, root.titleRomaji) ??
    seasonMarker(entry) ??
    displayTitle(entry)
  );
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

export function formatDate(d: string | null | undefined): string {
  if (!d) return "";
  const date = new Date(d.length === 10 ? d + "T00:00:00" : d);
  if (isNaN(date.getTime())) return d;
  return date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

/** Playback position as m:ss (or h:mm:ss). */
export function clock(seconds: number): string {
  const t = Math.max(0, Math.floor(seconds));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = String(t % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

export function relativeTime(unixSecs: number): string {
  const diff = Date.now() / 1000 - unixSecs;
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (diff < 60) return "just now";
  if (diff < 3600) return rtf.format(-Math.round(diff / 60), "minute");
  if (diff < 86400) return rtf.format(-Math.round(diff / 3600), "hour");
  if (diff < 86400 * 30) return rtf.format(-Math.round(diff / 86400), "day");
  return new Date(unixSecs * 1000).toLocaleDateString();
}

export function untilTime(unixSecs: number): string {
  const diff = unixSecs - Date.now() / 1000;
  if (diff <= 0) return "soon";
  const d = Math.floor(diff / 86400);
  const h = Math.floor((diff % 86400) / 3600);
  if (d > 0) return `${d}d ${h}h`;
  const m = Math.floor((diff % 3600) / 60);
  return `${h}h ${m}m`;
}

/** Normalise for search: lowercase, strip punctuation, keep CJK. */
export function norm(s: string): string {
  return s
    .toLowerCase()
    .normalize("NFKC")
    .replace(/['’]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

export function searchHaystack(m: MediaCard | MediaDetail): string {
  return norm([m.titleEnglish, m.titleRomaji, m.titleNative, ...m.synonyms].filter(Boolean).join(" | "));
}

/** Readable text colour on top of AniList's cover colour. */
export function accentFrom(hex: string | null | undefined): string {
  if (!hex || !/^#[0-9a-f]{6}$/i.test(hex)) return "hsl(340 100% 74%)";
  const r = parseInt(hex.slice(1, 3), 16) / 255;
  const g = parseInt(hex.slice(3, 5), 16) / 255;
  const b = parseInt(hex.slice(5, 7), 16) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  let h = 0;
  const l = (max + min) / 2;
  const d = max - min;
  const s = d === 0 ? 0 : d / (1 - Math.abs(2 * l - 1));
  if (d !== 0) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
  }
  h = Math.round(h * 60);
  if (h < 0) h += 360;
  // Force a bright, saturated variant so it reads well on dark backgrounds.
  return `hsl(${h} ${Math.max(55, Math.round(s * 100))}% 72%)`;
}
