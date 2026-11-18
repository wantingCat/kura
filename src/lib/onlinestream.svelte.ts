import {
  api,
  type InstalledExtension,
  type MediaDetail,
  type WebStreamSubtitleTrack,
} from "./api";
import {
  buildSeanimeMedia,
  onlinestreamProviderHost,
  type OnlinestreamEpisodeDetails,
  type OnlinestreamEpisodeServer,
  type OnlinestreamSearchResult,
  type OnlinestreamVideoSource,
} from "./extensions/host";

// ---------------------------------------------------------------------------
// Caches (24h Episode List TTL, 12m Resolved Stream TTL)
// ---------------------------------------------------------------------------

const EP_LIST_TTL_MS = 24 * 60 * 60 * 1000;
const SERVER_SOURCE_TTL_MS = 12 * 60 * 1000;

interface CachedEpisodeContainer {
  fetchedAt: number;
  matchedId: string;
  matchedTitle: string;
  episodes: OnlinestreamEpisodeDetails[];
}

interface CachedServerResult {
  fetchedAt: number;
  serverData: OnlinestreamEpisodeServer;
}

const episodeListCache = new Map<string, CachedEpisodeContainer>();
const serverSourceCache = new Map<string, CachedServerResult>();
const mediaDetailCache = new Map<number, MediaDetail>();

export function invalidateOnlinestreamCache(anilistId?: number) {
  if (anilistId === undefined) {
    episodeListCache.clear();
    serverSourceCache.clear();
    return;
  }
  const needle = `:${anilistId}`;
  for (const k of episodeListCache.keys()) {
    if (k.includes(needle)) episodeListCache.delete(k);
  }
  for (const k of serverSourceCache.keys()) {
    if (k.includes(`${needle}:`)) serverSourceCache.delete(k);
  }
}

// ---------------------------------------------------------------------------
// Title Similarity Scorer (AniList -> Provider Search Result)
// ---------------------------------------------------------------------------

function normalizeMatchText(s: string): string {
  return s
    .toLowerCase()
    .replace(/[āâ]/g, "a")
    .replace(/[īî]/g, "i")
    .replace(/[ūû]/g, "u")
    .replace(/[ēê]/g, "e")
    .replace(/[ōô]/g, "o")
    .replace(/['’]s\b/g, "")
    .replace(/\((?:tv|ona|ova|movie|special|sub|dub)\)/g, " ")
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function extractSeasonTag(s: string): number {
  const m1 = s.match(/\b(?:season\s*|s)(\d{1,2})\b/i);
  if (m1) return Number(m1[1]);
  const m2 = s.match(/\b(\d{1,2})(?:st|nd|rd|th)\s+season\b/i);
  if (m2) return Number(m2[1]);
  const m3 = s.match(/\s+(II|III|IV|V)\s*$/);
  if (m3) return { II: 2, III: 3, IV: 4, V: 5 }[m3[1]] ?? 1;
  return 1;
}

function diceBigrams(a: string, b: string): number {
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

export function scoreProviderCandidate(candTitle: string, detail: MediaDetail): number {
  const targets = [
    detail.titleRomaji,
    detail.titleEnglish,
    ...(detail.synonyms ?? []).slice(0, 5),
  ].filter((t): t is string => Boolean(t && t.trim()));

  if (targets.length === 0) return 0;
  const candNorm = normalizeMatchText(candTitle);
  const candSeason = extractSeasonTag(candTitle);
  const targetSeason = Math.max(
    extractSeasonTag(detail.titleRomaji ?? ""),
    extractSeasonTag(detail.titleEnglish ?? ""),
  );

  let best = 0;
  for (const t of targets) {
    const tNorm = normalizeMatchText(t);
    if (!tNorm) continue;
    let sim = diceBigrams(candNorm, tNorm);
    if (candNorm === tNorm) sim += 0.25;
    if (sim > best) best = sim;
  }

  // Penalize season mismatch strongly so Season 2 never locks onto Season 1!
  if (candSeason !== targetSeason) {
    best -= 0.35;
  }
  if (detail.seasonYear && candTitle.includes(String(detail.seasonYear))) {
    best += 0.08;
  }
  return best;
}

// ---------------------------------------------------------------------------
// Soft-Sub Inspection & Quality Ranking
// ---------------------------------------------------------------------------

export function extractValidSubtitles(src: OnlinestreamVideoSource): WebStreamSubtitleTrack[] {
  const out: WebStreamSubtitleTrack[] = [];
  for (const s of src.subtitles ?? []) {
    if (!s?.url) continue;
    const langLow = (s.language ?? "").toLowerCase();
    const urlLow = s.url.toLowerCase();
    if (langLow.includes("thumb") || langLow.includes("sprite") || urlLow.includes("thumbnails")) {
      continue;
    }
    out.push({
      url: s.url,
      language: s.language || "eng",
      label: s.language || "Subtitles",
      isDefault: Boolean(s.isDefault),
    });
  }
  return out;
}

export function isCleanSoftSubSource(src: OnlinestreamVideoSource): boolean {
  return extractValidSubtitles(src).length > 0;
}

function qualityScore(q: string): number {
  const low = (q ?? "").toLowerCase();
  if (low.includes("1080")) return 400;
  if (low.includes("auto") || low.includes("default") || low.includes("master")) return 350;
  if (low.includes("720")) return 250;
  if (low.includes("2160") || low.includes("4k")) return 300;
  if (low.includes("480")) return 120;
  return 200;
}

// ---------------------------------------------------------------------------
// Provider Discovery, Matching & Stream Resolution
// ---------------------------------------------------------------------------

export interface ResolvedWebStreamOption {
  providerId: string;
  providerName: string;
  matchedAnimeId: string;
  matchedAnimeTitle: string;
  server: string;
  url: string;
  streamType: string;
  quality: string;
  label: string;
  headers: Record<string, string>;
  subtitles: WebStreamSubtitleTrack[];
  isSoftSub: boolean;
}

export async function listEnabledOnlinestreamProviders(): Promise<InstalledExtension[]> {
  try {
    const all = await api.listExtensions();
    return all.filter((e) => e.enabled && e.extType === "onlinestream-provider");
  } catch {
    return [];
  }
}

export async function searchProviderAnime(
  ext: InstalledExtension,
  detail: MediaDetail,
  customQuery?: string,
): Promise<OnlinestreamSearchResult[]> {
  const media = buildSeanimeMedia(detail);
  const queries = customQuery?.trim()
    ? [customQuery.trim()]
    : Array.from(
        new Set(
          [detail.titleRomaji, detail.titleEnglish]
            .map((s) => s?.trim())
            .filter((s): s is string => Boolean(s)),
        ),
      );

  const settled = await Promise.allSettled(
    queries.map((q) =>
      onlinestreamProviderHost.search(ext, {
        media,
        query: q,
        dub: false,
        year: detail.seasonYear ?? undefined,
      }),
    ),
  );

  const byId = new Map<string, OnlinestreamSearchResult>();
  for (const res of settled) {
    if (res.status !== "fulfilled" || !Array.isArray(res.value)) continue;
    for (const item of res.value) {
      if (!item?.id || item.subOrDub === "dub") continue;
      if (!byId.has(item.id)) byId.set(item.id, item);
    }
  }

  const list = Array.from(byId.values());
  if (!customQuery?.trim()) {
    list.sort((a, b) => scoreProviderCandidate(b.title, detail) - scoreProviderCandidate(a.title, detail));
  }
  return list;
}

export async function resolveProviderEpisodes(
  ext: InstalledExtension,
  detail: MediaDetail,
  forceRefresh = false,
): Promise<CachedEpisodeContainer> {
  mediaDetailCache.set(detail.anilistId, detail);
  const cacheKey = `${ext.id}:${detail.anilistId}`;
  if (!forceRefresh) {
    const hit = episodeListCache.get(cacheKey);
    if (hit && Date.now() - hit.fetchedAt < EP_LIST_TTL_MS) {
      return hit;
    }
  }

  // 1. Check SQLite manual mapping override first
  const manual = await api.getOnlinestreamMapping(ext.id, detail.anilistId);
  let matchedId = manual?.providerAnimeId ?? "";
  let matchedTitle = manual?.providerTitle ?? "";

  if (!matchedId) {
    const results = await searchProviderAnime(ext, detail);
    if (results.length === 0) {
      throw new Error(`${ext.name}: Could not find "${detail.titleEnglish ?? detail.titleRomaji}"`);
    }
    matchedId = results[0].id;
    matchedTitle = results[0].title;
  }

  const episodes = (await onlinestreamProviderHost.findEpisodes(ext, matchedId)) ?? [];
  const container: CachedEpisodeContainer = {
    fetchedAt: Date.now(),
    matchedId,
    matchedTitle: matchedTitle || matchedId,
    episodes,
  };
  episodeListCache.set(cacheKey, container);
  return container;
}

export async function resolveEpisodeOptionsForProvider(
  ext: InstalledExtension,
  detail: MediaDetail,
  episodeNum: number,
  softSubOnly: boolean,
  forceRefresh = false,
): Promise<{
  matchedId: string;
  matchedTitle: string;
  options: ResolvedWebStreamOption[];
  hardsubSkippedCount: number;
}> {
  const container = await resolveProviderEpisodes(ext, detail, forceRefresh);
  const epDetails =
    container.episodes.find((e) => Number(e.number) === episodeNum) ??
    (detail.format === "MOVIE" && episodeNum === 1 ? container.episodes[0] : undefined);

  if (!epDetails) {
    throw new Error(`${ext.name}: Episode ${episodeNum} not available on "${container.matchedTitle}"`);
  }

  let servers: string[] = [];
  try {
    const settings = await onlinestreamProviderHost.getSettings(ext);
    servers = (settings?.episodeServers ?? []).filter(Boolean);
  } catch {
    servers = [];
  }
  if (servers.length === 0) servers = ["default"];

  const options: ResolvedWebStreamOption[] = [];
  let hardsubSkippedCount = 0;

  // Query servers sequentially with early satisfaction so we don't hammer CDNs unnecessarily,
  // or query up to 3 servers in parallel for fast picker results!
  const topServers = servers.slice(0, 4);
  const serverResults = await Promise.allSettled(
    topServers.map(async (srv) => {
      const srvKey = `${ext.id}:${detail.anilistId}:${episodeNum}:${srv}`;
      if (!forceRefresh) {
        const hit = serverSourceCache.get(srvKey);
        if (hit && Date.now() - hit.fetchedAt < SERVER_SOURCE_TTL_MS) {
          return hit.serverData;
        }
      }
      const data = await onlinestreamProviderHost.findEpisodeServer(ext, epDetails, srv);
      if (data?.videoSources?.length) {
        serverSourceCache.set(srvKey, { fetchedAt: Date.now(), serverData: data });
      }
      return data;
    }),
  );

  for (let i = 0; i < serverResults.length; i++) {
    const r = serverResults[i];
    const srvName = topServers[i];
    if (r.status !== "fulfilled" || !r.value?.videoSources) continue;
    const headers = r.value.headers ?? {};

    for (const vs of r.value.videoSources) {
      if (!vs?.url) continue;
      const subs = extractValidSubtitles(vs);
      const isSoft = subs.length > 0;
      if (softSubOnly && !isSoft) {
        hardsubSkippedCount++;
        continue;
      }
      options.push({
        providerId: ext.id,
        providerName: ext.name,
        matchedAnimeId: container.matchedId,
        matchedAnimeTitle: container.matchedTitle,
        server: r.value.server || srvName,
        url: vs.url,
        streamType: vs.type || (vs.url.includes(".m3u8") ? "m3u8" : "mp4"),
        quality: vs.quality || "auto",
        label: vs.label || `${r.value.server || srvName} · ${vs.quality || "auto"}`,
        headers,
        subtitles: subs,
        isSoftSub: isSoft,
      });
    }
  }

  options.sort((a, b) => {
    if (a.isSoftSub !== b.isSoftSub) return a.isSoftSub ? -1 : 1;
    return qualityScore(b.quality) - qualityScore(a.quality);
  });

  return {
    matchedId: container.matchedId,
    matchedTitle: container.matchedTitle,
    options,
    hardsubSkippedCount,
  };
}

/**
 * Resolve the best stream across all enabled `onlinestream-provider` extensions and immediately
 * start playback in native `libmpv` (with `.vtt` -> Jimaku auto-retiming).
 */
export async function autoPlayWebEpisode(
  detail: MediaDetail,
  episodeNum: number,
  softSubOnly = true,
): Promise<ResolvedWebStreamOption> {
  mediaDetailCache.set(detail.anilistId, detail);
  const providers = await listEnabledOnlinestreamProviders();
  if (providers.length === 0) {
    throw new Error(
      "NO_WEB_EXTENSIONS: No Web Streaming extensions installed yet. Add a Seanime streaming extension in Settings → Extensions.",
    );
  }

  const errors: string[] = [];
  let totalHardsubSkipped = 0;

  for (const ext of providers) {
    try {
      const res = await resolveEpisodeOptionsForProvider(ext, detail, episodeNum, softSubOnly);
      totalHardsubSkipped += res.hardsubSkippedCount;
      if (res.options.length > 0) {
        const picked = res.options[0];
        await launchResolvedWebOption(detail.anilistId, String(episodeNum), picked);
        return picked;
      }
    } catch (e) {
      errors.push(e instanceof Error ? e.message : String(e));
    }
  }

  if (softSubOnly && totalHardsubSkipped > 0) {
    throw new Error(
      `HARD_SUB_ONLY: Found ${totalHardsubSkipped} stream(s), but all had burned-in hardsubs (skipped by Soft-Sub Only mode). Open Web Sources to allow hardsub fallback or switch to Torrent.`,
    );
  }
  throw new Error(errors[0] || "No working web stream sources found for this episode.");
}

export async function launchResolvedWebOption(
  anilistId: number,
  epKey: string,
  opt: ResolvedWebStreamOption,
) {
  await api.startWebStream({
    anilistId,
    epKey,
    url: opt.url,
    streamType: opt.streamType,
    headers: opt.headers,
    subtitles: opt.subtitles,
    providerName: opt.providerName,
    serverName: opt.server,
  });
}

/**
 * Phase 5: Called at 85% playback progress — pre-resolves Episode `N+1`'s `.m3u8` server URLs
 * into cache AND pre-downloads + piecewise-retimes Episode `N+1`'s Jimaku Japanese subtitles!
 */
export async function preloadNextWebEpisode(
  anilistId: number,
  nextEpisode: number,
  softSubOnly = true,
): Promise<boolean> {
  try {
    const detail = mediaDetailCache.get(anilistId) ?? (await api.getMediaDetail(anilistId));
    mediaDetailCache.set(anilistId, detail);
    const providers = await listEnabledOnlinestreamProviders();
    for (const ext of providers) {
      try {
        const res = await resolveEpisodeOptionsForProvider(ext, detail, nextEpisode, softSubOnly);
        if (res.options.length > 0) {
          const best = res.options[0];
          const engRef =
            best.subtitles.find((s) => /eng|^en$/i.test(s.language)) ?? best.subtitles[0];
          await api.prefetchWebStreamSubtitles({
            anilistId,
            episode: nextEpisode,
            refSubUrl: engRef?.url ?? null,
            headers: best.headers,
          });
          return true;
        }
      } catch {
        /* try next provider */
      }
    }
  } catch {
    /* ignore background preload failure */
  }
  return false;
}

export async function getCachedOrLoadMediaDetail(anilistId: number): Promise<MediaDetail> {
  return mediaDetailCache.get(anilistId) ?? (await api.getMediaDetail(anilistId));
}
