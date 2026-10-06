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
