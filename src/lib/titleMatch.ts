/**
 * Shared anime title normalization, Romaji macron stripping, season/part extraction,
 * and release episode parsing.
 *
 * Pure TypeScript with zero DOM or Tauri imports so it runs identically in both the
 * main thread (`onlinestream.svelte.ts`) and extension Web Workers (`worker.ts`).
 */

const STOP_WORDS = new Set([
  "the",
  "a",
  "an",
  "and",
  "of",
  "in",
  "on",
  "to",
  "for",
  "tv",
  "animation",
]);

const WORD_ORDINALS: Record<string, number> = {
  first: 1,
  second: 2,
  third: 3,
  fourth: 4,
  fifth: 5,
  sixth: 6,
};

const ROMAN_NUMERALS: Record<string, number> = {
  II: 2,
  III: 3,
  IV: 4,
  V: 5,
};

const RESOLUTION_OR_CODEC_NUMBERS = new Set([
  264,
  265,
  480,
  540,
  576,
  720,
  1080,
  1440,
  2160,
]);

/** Fold Romaji macrons and circumflexes (`ō` -> `o`, `ū` -> `u`, etc.) preserving case. */
export function stripRomajiMacrons(input: string): string {
  return String(input ?? "")
    .replace(/[āâàáä]/g, "a")
    .replace(/[ĀÂÀÁÄ]/g, "A")
    .replace(/[īîìíï]/g, "i")
    .replace(/[ĪÎÌÍÏ]/g, "I")
    .replace(/[ūûùúü]/g, "u")
    .replace(/[ŪÛÙÚÜ]/g, "U")
    .replace(/[ēêèéë]/g, "e")
    .replace(/[ĒÊÈÉË]/g, "E")
    .replace(/[ōôòóö]/g, "o")
    .replace(/[ŌÔÒÓÖ]/g, "O");
}

/**
 * Extract explicit season number (`1..99`) from an anime title or release name.
 * Returns `-1` when no explicit season marker is present.
 *
 * Handles `"Season 2"`, `"2nd Season"`, `"Second Season"`..`"Sixth Season"`,
 * `"S2"` / `"S02E05"`, `"第2期"`, `"2期"`, `"Season II"`, and trailing Roman numerals `II..V`.
 */
export function extractSeasonNumber(title: string): number {
  const s = String(title ?? "").normalize("NFKC");
  if (!s) return -1;

  const mSeason = s.match(/\b(?:Season\s*(\d{1,2})|S(\d{1,2})(?:E\d{1,4}|\b))/i);
  if (mSeason) {
    const n = Number(mSeason[1] ?? mSeason[2]);
    if (n > 0) return n;
  }

  const mNumOrd = s.match(/\b(\d{1,2})(?:st|nd|rd|th)\s+Season\b/i);
  if (mNumOrd) {
    const n = Number(mNumOrd[1]);
    if (n > 0) return n;
  }

  const mWordOrd = s.match(/\b(first|second|third|fourth|fifth|sixth)\s+season\b/i);
  if (mWordOrd) {
    return WORD_ORDINALS[mWordOrd[1].toLowerCase()] ?? -1;
  }

  const mSeasonRoman = s.match(/\bSeason\s+(II|III|IV|V)\b/i);
  if (mSeasonRoman) {
    return ROMAN_NUMERALS[mSeasonRoman[1].toUpperCase()] ?? -1;
  }

  const mJa = s.match(/(?:第\s*)?(\d{1,2})\s*期/);
  if (mJa) {
    const n = Number(mJa[1]);
    if (n > 0) return n;
  }

  const mTrailingRoman = s.match(/(?:^|\s)(II|III|IV|V)(?:\s*$|\s*[-:(\[])/);
  if (mTrailingRoman) {
    return ROMAN_NUMERALS[mTrailingRoman[1]] ?? -1;
  }

  return -1;
}

/** Same as `extractSeasonNumber`, defaulting to `1` when no explicit season tag is present. */
export function effectiveSeasonNumber(title: string): number {
  const s = extractSeasonNumber(title);
  return s > 0 ? s : 1;
}

export function extractPartNumber(title: string): number {
  const s = String(title ?? "").normalize("NFKC");
  const m1 = s.match(/\b(?:Part|Cour)\s*(\d{1,2})\b/i);
  if (m1) return Number(m1[1]);
  const m2 = s.match(/\b(\d{1,2})(?:st|nd|rd|th)\s+(?:Part|Cour)\b/i);
  if (m2) return Number(m2[1]);
  const mWord = s.match(/\b(first|second|third|fourth|fifth|sixth)\s+(?:part|cour)\b/i);
  if (mWord) return WORD_ORDINALS[mWord[1].toLowerCase()] ?? -1;
  const m3 = s.match(/\b(?:Part|Cour)\s+(II|III|IV)\b/i);
  if (m3) return ROMAN_NUMERALS[m3[1].toUpperCase()] ?? -1;
  const mJa = s.match(/(?:第\s*)?(\d{1,2})\s*クール/);
  if (mJa) return Number(mJa[1]);
  return -1;
}

export function extractYear(title: string): number {
  const m = String(title ?? "").match(/\b(19[7-9]\d|20\d{2})\b/);
  return m ? Number(m[1]) : -1;
}

function isResolutionOrYearNumber(n: number): boolean {
  if (!Number.isFinite(n) || n < 0) return true;
  if (RESOLUTION_OR_CODEC_NUMBERS.has(n)) return true;
  if (n >= 1970 && n <= 2099) return true;
  return false;
}

/**
 * Parse an episode number from a release filename or title, guarding dash/bracket numbers
 * against `264|265|480|540|576|720|1080|1440|2160` resolutions/codecs and `1970..2099` years.
 */
export function parseEpisodeNumber(withoutExt: string): number | null {
  const s = String(withoutExt ?? "").normalize("NFKC").trim();
  if (!s) return null;

  // 1. Explicit S01E05 / 1x05
  const mSe = s.match(/\bS\d{1,2}E(\d{1,4})(?:v\d)?\b/i) ?? s.match(/\b\d{1,2}x(\d{1,4})(?:v\d)?\b/i);
  if (mSe) return Number(mSe[1]);

  // 2. Explicit EP 05 / Episode 05 / 第05話
  const mEp =
    s.match(/\b(?:EP|Episode)\s*(\d{1,4})(?:v\d)?\b/i) ??
    s.match(/第\s*(\d{1,4})\s*話/);
  if (mEp) return Number(mEp[1]);

  // 3. Dash-separated `- 05` / `- 05v2` (reject resolutions like `1080p`, codecs `264`/`265`, and years `1970..2099`)
  const dashRe = /-\s*(\d{1,4})(?:v\d)?(?=$|[\s\[\(_.-])/g;
  let mDash: RegExpExecArray | null;
  while ((mDash = dashRe.exec(s)) !== null) {
    const num = Number(mDash[1]);
    const tail = s.slice(mDash.index + mDash[0].length);
    if (/^[a-zA-Z]/.test(tail)) continue;
    if (isResolutionOrYearNumber(num)) continue;
    return num;
  }

  // 4. Non-leading bracketed `[05]` / `[05v2]` (skip leading `[Group]` and `[CRC32]`)
  const strippedLead = s.replace(/^\[[^\]]+\]\s*/, "");
  const bracketRe = /\[(\d{1,4})(?:v\d)?\]/g;
  let mBr: RegExpExecArray | null;
  while ((mBr = bracketRe.exec(strippedLead)) !== null) {
    const num = Number(mBr[1]);
    if (isResolutionOrYearNumber(num)) continue;
    return num;
  }

  return null;
}

export function sanitizeQuery(query: string): string {
  return stripRomajiMacrons(String(query ?? "").normalize("NFKC"))
    .replace(/['’]s\b/gi, "")
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function stripSeasonAndFormatNoise(raw: string): string {
  return String(raw ?? "")
    .normalize("NFKC")
    .replace(/\b(?:Season\s*\d{1,2}|\d{1,2}(?:st|nd|rd|th)\s+Season|S\d{1,2})\b/gi, " ")
    .replace(/\b(?:first|second|third|fourth|fifth|sixth)\s+season\b/gi, " ")
    .replace(/\bSeason\s+(?:II|III|IV|V)\b/gi, " ")
    .replace(/(?:第\s*)?\d{1,2}\s*期/g, " ")
    .replace(/\b(?:Part|Cour)\s*(?:\d{1,2}|II|III|IV)\b/gi, " ")
    .replace(/\b\d{1,2}(?:st|nd|rd|th)\s+(?:Part|Cour)\b/gi, " ")
    .replace(/\b(?:first|second|third|fourth|fifth|sixth)\s+(?:part|cour)\b/gi, " ")
    .replace(/(?:第\s*)?\d{1,2}\s*クール/g, " ")
    .replace(/\((?:TV|ONA|OVA|Movie|Special|Sub|Dub|19[7-9]\d|20\d{2})\)/gi, " ")
    .replace(/(?:^|\s)(?:II|III|IV|V)\s*$/, " ");
}

export function normalizeMatchText(s: string): string {
  return sanitizeQuery(
    String(s ?? "")
      .normalize("NFKC")
      .replace(/\((?:tv|ona|ova|movie|special|sub|dub)\)/gi, " "),
  ).toLowerCase();
}

export function normalizeTitle(title: string) {
  const original = String(title ?? "");
  const season = extractSeasonNumber(original);
  const part = extractPartNumber(original);
  const year = extractYear(original);
  const cleanBase = sanitizeQuery(stripSeasonAndFormatNoise(original));
  const normalized = sanitizeQuery(original).toLowerCase();
  const tokens = normalized.split(/\s+/).filter((t) => t.length > 1 && !STOP_WORDS.has(t));
  return {
    original,
    normalized,
    cleanBaseTitle: cleanBase,
    denoisedTitle: cleanBase,
    tokens,
    season,
    part,
    year,
    isMain: season <= 1 && part <= 1,
  };
}

export function diceBigrams(a: string, b: string): number {
  if (a === b) return 1;
  if (a.length < 2 || b.length < 2) return 0;
  const counts = new Map<string, number>();
  for (let i = 0; i < a.length - 1; i++) {
    const bg = a.slice(i, i + 2);
    counts.set(bg, (counts.get(bg) ?? 0) + 1);
  }
  let inter = 0;
  for (let i = 0; i < b.length - 1; i++) {
    const bg = b.slice(i, i + 2);
    const c = counts.get(bg) ?? 0;
    if (c > 0) {
      counts.set(bg, c - 1);
      inter++;
    }
  }
  return (2 * inter) / (a.length - 1 + (b.length - 1));
}

export function compareTitles(t1: string, t2: string): number {
  const a = normalizeTitle(t1);
  const b = normalizeTitle(t2);
  if (!a.tokens.length || !b.tokens.length) return a.normalized === b.normalized ? 1 : 0;
  const setB = new Set(b.tokens);
  let hits = 0;
  for (const tok of a.tokens) {
    if (setB.has(tok)) hits++;
  }
  return (2 * hits) / (a.tokens.length + b.tokens.length);
}
