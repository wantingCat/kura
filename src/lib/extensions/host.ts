import {
  api,
  type InstalledExtension,
  type MediaDetail,
  type TorrentCandidate,
} from "../api";

// ---------------------------------------------------------------------------
// Seanime Extension Type Contracts (`online-streaming-provider.d.ts` & `anime-torrent-provider.d.ts`)
// ---------------------------------------------------------------------------

export interface SeanimeFuzzyDate {
  year: number;
  month?: number;
  day?: number;
}

export interface SeanimeMedia {
  id: number;
  idMal?: number;
  status?: string;
  format?: string;
  englishTitle?: string;
  romajiTitle?: string;
  episodeCount?: number;
  absoluteSeasonOffset?: number;
  synonyms: string[];
  isAdult: boolean;
  startDate?: SeanimeFuzzyDate;
}

export interface OnlinestreamSearchResult {
  id: string;
  title: string;
  url: string;
  subOrDub: "sub" | "dub" | "both" | string;
}

export interface OnlinestreamEpisodeDetails {
  id: string;
  number: number;
  url: string;
  title?: string;
}

export interface OnlinestreamVideoSubtitle {
  id?: string;
  url: string;
  language: string;
  isDefault?: boolean;
}

export interface OnlinestreamVideoSource {
  url: string;
  type: "mp4" | "m3u8" | "unknown" | string;
  quality: string;
  label?: string;
  subtitles?: OnlinestreamVideoSubtitle[];
}

export interface OnlinestreamEpisodeServer {
  server: string;
  headers?: Record<string, string>;
  videoSources: OnlinestreamVideoSource[];
}

export interface OnlinestreamSettings {
  episodeServers: string[];
  supportsDub: boolean;
}

export interface SeanimeTorrentProviderSettings {
  canSmartSearch: boolean;
  smartSearchFilters?: string[];
  supportsAdult?: boolean;
  type?: "main" | "special" | string;
}

export interface SeanimeAnimeTorrent {
  name: string;
  date?: string;
  size?: number;
  formattedSize?: string;
  seeders?: number;
  leechers?: number;
  downloadCount?: number;
  link?: string;
  downloadUrl?: string;
  magnetLink?: string;
  infoHash?: string;
  resolution?: string;
  isBatch?: boolean;
  episodeNumber?: number;
  releaseGroup?: string;
  isBestRelease?: boolean;
  confirmed?: boolean;
}

// ---------------------------------------------------------------------------
// Worker Lifecycle & RPC Manager
// ---------------------------------------------------------------------------

interface WorkerSlot {
  extId: string;
  updatedAt: number;
  worker: Worker;
  ready: Promise<void>;
  nextCallId: number;
  pending: Map<
    number,
    { resolve: (val: unknown) => void; reject: (err: Error) => void; timer: ReturnType<typeof setTimeout> }
  >;
}

const slots = new Map<string, WorkerSlot>();

function resolveEffectiveUserPrefs(ext: InstalledExtension): Record<string, string> {
  const out: Record<string, string> = {};
  const fields = ext.userConfig?.definition?.fields ?? [];
  for (const f of fields) {
    if (f?.name && f.default !== undefined && f.default !== null) {
      out[f.name] = String(f.default);
    }
  }
  const saved = ext.userConfig?.values ?? {};
  for (const [k, v] of Object.entries(saved)) {
    if (v !== undefined && v !== null) {
      out[k] = String(v);
    }
  }
  return out;
}

function getOrSpawnWorker(ext: InstalledExtension): WorkerSlot {
  const existing = slots.get(ext.id);
  if (existing && existing.updatedAt === ext.updatedAt) {
    return existing;
  }
  if (existing) {
    terminateWorkerSlot(ext.id);
  }

  const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
  const pending: WorkerSlot["pending"] = new Map();
  const slot: WorkerSlot = {
    extId: ext.id,
    updatedAt: ext.updatedAt,
    worker,
    ready: Promise.resolve(),
    nextCallId: 2,
    pending,
  };

  worker.onmessage = async (ev) => {
    const msg = ev.data;
    if (!msg) return;

    if (msg.kind === "http_fetch") {
      try {
        const resp = await api.extensionHttpFetch(msg.req);
        worker.postMessage({ kind: "http_fetch_done", reqId: msg.reqId, ok: true, resp });
      } catch (e) {
        worker.postMessage({
          kind: "http_fetch_done",
          reqId: msg.reqId,
          ok: false,
          error: e instanceof Error ? e.message : String(e),
        });
      }
      return;
    }

    if (msg.kind === "result") {
      const entry = pending.get(msg.callId);
      if (!entry) return;
      pending.delete(msg.callId);
      clearTimeout(entry.timer);
      if (msg.ok) {
        entry.resolve(msg.value);
      } else {
        entry.reject(new Error(msg.error ?? "Extension method failed"));
      }
    }
  };

  worker.onerror = (ev) => {
    const err = new Error(ev.message || `Worker crashed for extension '${ext.id}'`);
    for (const [, p] of pending) {
      clearTimeout(p.timer);
      p.reject(err);
    }
    pending.clear();
  };

  slot.ready = new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(1);
      reject(new Error(`Extension '${ext.name}' timed out during initialization`));
    }, 15_000);
    pending.set(1, {
      resolve: () => resolve(),
      reject,
      timer,
    });
    worker.postMessage({
      kind: "init",
      callId: 1,
      data: {
        id: ext.id,
        language: ext.language,
        payload: ext.payload,
        userPrefs: resolveEffectiveUserPrefs(ext),
      },
    });
  });

  slots.set(ext.id, slot);
  return slot;
}

export function terminateWorkerSlot(extId: string) {
  const existing = slots.get(extId);
  if (!existing) return;
  slots.delete(extId);
  for (const [, p] of existing.pending) {
    clearTimeout(p.timer);
    p.reject(new Error("Extension worker terminated"));
  }
  existing.pending.clear();
  existing.worker.terminate();
}

export async function callExtensionMethod<T = unknown>(
  ext: InstalledExtension,
  method: string,
  args: unknown[] = [],
  timeoutMs = 30_000,
): Promise<T> {
  const slot = getOrSpawnWorker(ext);
  await slot.ready;

  const callId = slot.nextCallId++;
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      slot.pending.delete(callId);
      reject(new Error(`${ext.name}.${method}() timed out after ${Math.round(timeoutMs / 1000)}s`));
    }, timeoutMs);

    slot.pending.set(callId, {
      resolve: (val) => resolve(val as T),
      reject,
      timer,
    });
    slot.worker.postMessage({
      kind: "call",
      callId,
      method,
      args,
    });
  });
}

// ---------------------------------------------------------------------------
// Media Conversion & High-Level Wrappers
// ---------------------------------------------------------------------------

export function buildSeanimeMedia(detail: MediaDetail): SeanimeMedia {
  let startDate: SeanimeFuzzyDate | undefined;
  if (detail.startDate) {
    const parts = detail.startDate.split("-").map((x) => Number(x));
    if (parts[0] > 1900) {
      startDate = {
        year: parts[0],
        month: parts[1] || undefined,
        day: parts[2] || undefined,
      };
    }
  } else if (detail.seasonYear) {
    startDate = { year: detail.seasonYear };
  }

  return {
    id: detail.anilistId,
    idMal: detail.idMal ?? undefined,
    status: detail.status ?? undefined,
    format: detail.format ?? undefined,
    englishTitle: detail.titleEnglish ?? undefined,
    romajiTitle: detail.titleRomaji ?? undefined,
    episodeCount: detail.episodes ?? undefined,
    synonyms: detail.synonyms ?? [],
    isAdult: false,
    startDate,
  };
}

export const onlinestreamProviderHost = {
  getSettings: (ext: InstalledExtension) =>
    callExtensionMethod<OnlinestreamSettings>(ext, "getSettings", [], 10_000),
  search: (
    ext: InstalledExtension,
    opts: { media: SeanimeMedia; query: string; dub: boolean; year?: number },
  ) => callExtensionMethod<OnlinestreamSearchResult[]>(ext, "search", [opts], 25_000),
  findEpisodes: (ext: InstalledExtension, providerAnimeId: string) =>
    callExtensionMethod<OnlinestreamEpisodeDetails[]>(ext, "findEpisodes", [providerAnimeId], 25_000),
  findEpisodeServer: (
    ext: InstalledExtension,
    episode: OnlinestreamEpisodeDetails,
    server: string,
  ) => callExtensionMethod<OnlinestreamEpisodeServer>(ext, "findEpisodeServer", [episode, server], 25_000),
};

function parseClientReleaseBadges(title: string) {
  const codec = /\bav1\b/i.test(title)
    ? "AV1"
    : /\b(hevc|x265|h\.?265)\b/i.test(title)
      ? "HEVC"
      : /\b(avc|x264|h\.?264)\b/i.test(title)
        ? "AVC"
        : null;
  const bitDepth = /\b(10[\s\-_]*bit|hi10p|yuv420p10)\b/i.test(title) ? "10-bit" : null;
  const isDualAudio = /\b(dual[\s_\-]*audio|multi[\s_\-]*audio|ja[\s_\-+/]+en|en[\s_\-+/]+ja)\b/i.test(title);
  return { codec, bitDepth, isDualAudio };
}

/**
 * Query all enabled Seanime `anime-torrent-provider` extensions in parallel and normalize their
 * results into Kura `TorrentCandidate[]` rows so they merge cleanly into `TorrentPickerDialog`.
 */
export async function searchInstalledTorrentExtensions(
  detail: MediaDetail,
  episode: number,
  resolution: string,
  customQuery?: string | null,
): Promise<TorrentCandidate[]> {
  let exts: InstalledExtension[] = [];
  try {
    exts = (await api.listExtensions()).filter(
      (e) => e.enabled && e.extType === "anime-torrent-provider",
    );
  } catch {
    return [];
  }
  if (exts.length === 0) return [];

  const media = buildSeanimeMedia(detail);
  const baseQuery =
    customQuery?.trim() ||
    detail.titleRomaji ||
    detail.titleEnglish ||
    "Anime";
  const effectiveRes = resolution === "ask" ? "" : resolution.replace(/p$/i, "");

  let anidbAID = 0;
  let anidbEID = 0;
  if (!customQuery?.trim()) {
    try {
      const [aid, eid] = await api.getAnidbIds(detail.anilistId, episode);
      anidbAID = aid ?? 0;
      anidbEID = eid ?? 0;
    } catch {
      /* optional */
    }
  }

  const perProvider = await Promise.allSettled(
    exts.map(async (ext) => {
      let settings: SeanimeTorrentProviderSettings = { canSmartSearch: false };
      try {
        settings = await callExtensionMethod<SeanimeTorrentProviderSettings>(
          ext,
          "getSettings",
          [],
          8_000,
        );
      } catch {
        /* fallback to basic search */
      }

      let items: SeanimeAnimeTorrent[] = [];
      if (settings?.canSmartSearch && !customQuery?.trim()) {
        items =
          (await callExtensionMethod<SeanimeAnimeTorrent[]>(
            ext,
            "smartSearch",
            [
              {
                media,
                query: baseQuery,
                batch: false,
                episodeNumber: episode,
                resolution: effectiveRes,
                anidbAID,
                anidbEID,
                bestReleases: false,
              },
            ],
            25_000,
          )) ?? [];
      } else {
        const searchQ = customQuery?.trim()
          ? customQuery.trim()
          : `${baseQuery} ${String(episode).padStart(2, "0")}`;
        items =
          (await callExtensionMethod<SeanimeAnimeTorrent[]>(
            ext,
            "search",
            [{ media, query: searchQ }],
            25_000,
          )) ?? [];
      }

      const mapped: TorrentCandidate[] = [];
      for (const t of items.slice(0, 40)) {
        if (!t?.name) continue;
        let magnet = t.magnetLink || null;
        let hash = t.infoHash || null;
        if (!magnet && !t.downloadUrl) {
          try {
            magnet = await callExtensionMethod<string>(ext, "getTorrentMagnetLink", [t], 8_000);
          } catch {}
        }
        if (!hash && magnet?.includes("urn:btih:")) {
          hash = magnet.split("urn:btih:")[1]?.split("&")[0]?.trim()?.toLowerCase() ?? null;
        }
        const seeders = Number(t.seeders ?? 0);
        const res = t.resolution
          ? t.resolution.endsWith("p")
            ? t.resolution
            : `${t.resolution}p`
          : (t.name.match(/\b(2160p|1080p|720p|480p)\b/i)?.[1]?.toLowerCase() ?? null);
        const group =
          t.releaseGroup || (t.name.match(/^\[([^\]]+)\]/)?.[1]?.trim() ?? null);
        const { codec, bitDepth, isDualAudio } = parseClientReleaseBadges(t.name);
        const baseScore =
          (t.isBestRelease ? 400 : 0) +
          (t.confirmed ? 150 : 0) +
          (res === resolution ? 400 : res === "1080p" ? 250 : 100) +
          Math.min(seeders, 200) * 8 +
          (t.isBatch ? 200 : 900);

        mapped.push({
          title: t.name,
          group,
          resolution: res,
          size: Number(t.size ?? 0),
          seeders,
          leechers: Number(t.leechers ?? 0),
          torrentUrl: t.downloadUrl && t.downloadUrl.startsWith("http") ? t.downloadUrl : null,
          magnetUri: magnet,
          infoHash: hash,
          publishedAt: null,
          source: t.isBestRelease ? `${ext.name} ★ Best` : ext.name,
          isBatch: Boolean(t.isBatch),
          score: baseScore,
          refSubtitleUrl: null,
          codec,
          bitDepth,
          isDualAudio,
          isBestRelease: Boolean(t.isBestRelease),
        });
      }
      return mapped;
    }),
  );

  const combined: TorrentCandidate[] = [];
  for (const res of perProvider) {
    if (res.status === "fulfilled") {
      combined.push(...res.value);
    }
  }
  return combined;
}
