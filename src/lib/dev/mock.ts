/**
 * Dev-only browser preview.
 *
 * When the frontend is opened in a normal browser (e.g. http://localhost:1420)
 * instead of inside Tauri, there is no backend to talk to. This module mocks
 * the IPC layer with a small sample library built from live AniList + ani.zip
 * data, so UI work can be done (and screenshotted) without running Rust.
 *
 * Query params:
 *   ?preview=onboarding  → start with no libraries (first-run screen)
 *   ?preview=scanning    → show the scan progress card
 */
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import type { EpisodeRow, Library, MediaCard, MediaDetail, RelationCard, UnmatchedGroup, UpNextItem } from "$lib/api";

const SAMPLE: { id: number; owned: number; watched: number }[] = [
  { id: 154587, owned: 10, watched: 6 }, // Frieren
  { id: 130003, owned: 12, watched: 12 }, // Bocchi the Rock!
  { id: 171018, owned: 2, watched: 0 }, // Dandadan
  { id: 161645, owned: 1, watched: 0 }, // Kusuriya no Hitorigoto
  { id: 21519, owned: 1, watched: 0 }, // Kimi no Na wa.
  { id: 108465, owned: 3, watched: 1 }, // Mushoku Tensei
  { id: 16498, owned: 25, watched: 25 }, // Shingeki no Kyojin
  { id: 20958, owned: 12, watched: 12 }, // Shingeki no Kyojin S2
  { id: 99147, owned: 12, watched: 4 }, // Shingeki no Kyojin S3
  { id: 104578, owned: 10, watched: 0 }, // Shingeki no Kyojin S3 Part 2
  { id: 21355, owned: 25, watched: 0 }, // Re:Zero S1
  { id: 108632, owned: 3, watched: 0 }, // Re:Zero S2
];

const QUERY = `query ($ids: [Int]) {
  Page(perPage: 50) {
    media(id_in: $ids, type: ANIME) {
      id idMal format status episodes duration season seasonYear averageScore genres synonyms
      title { romaji english native }
      coverImage { extraLarge color }
      bannerImage description(asHtml: false)
      startDate { year month day } endDate { year month day }
      studios(isMain: true) { nodes { name } }
      tags { name rank }
      nextAiringEpisode { episode airingAt }
      relations { edges { relationType node { id type format status episodes seasonYear title { romaji english } coverImage { large } } } }
    }
  }
}`;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AniMedia = any;

let media: Map<number, AniMedia> | null = null;
const watched = new Map<number, Set<string>>();

async function loadMedia(): Promise<Map<number, AniMedia>> {
  if (media) return media;
  const res = await fetch("https://graphql.anilist.co", {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify({ query: QUERY, variables: { ids: SAMPLE.map((s) => s.id) } }),
  });
  const json = await res.json();
  media = new Map((json?.data?.Page?.media ?? []).map((m: AniMedia) => [m.id, m]));
  return media;
}

const date = (d: { year: number | null; month: number | null; day: number | null } | null) =>
  d?.year ? `${d.year}-${String(d.month ?? 1).padStart(2, "0")}-${String(d.day ?? 1).padStart(2, "0")}` : null;

const prefs: Record<string, string> = {};

/** Same idea as franchise.rs: connected components over PREQUEL/SEQUEL edges, ordered by start date. */
function franchises(all: Map<number, AniMedia>) {
  const parent = new Map<number, number>();
  const find = (x: number): number => {
    if (!parent.has(x)) parent.set(x, x);
    while (parent.get(x) !== x) x = parent.get(x)!;
    return x;
  };
  for (const m of all.values()) {
    find(m.id);
    for (const e of m.relations?.edges ?? []) {
      if (e.relationType === "PREQUEL" || e.relationType === "SEQUEL") parent.set(find(m.id), find(e.node.id));
    }
  }
  const groups = new Map<number, AniMedia[]>();
  for (const m of all.values()) {
    const r = find(m.id);
    groups.set(r, [...(groups.get(r) ?? []), m]);
  }
  const out = new Map<number, { root: number; index: number; size: number; members: AniMedia[] }>();
  for (const members of groups.values()) {
    members.sort((a, b) => (date(a.startDate) ?? "9999").localeCompare(date(b.startDate) ?? "9999"));
    members.forEach((m, index) => out.set(m.id, { root: members[0].id, index, size: members.length, members }));
  }
  return out;
}

function card(m: AniMedia, i: number): MediaCard {
  const s = SAMPLE.find((x) => x.id === m.id)!;
  return {
    anilistId: m.id,
    titleRomaji: m.title.romaji,
    titleEnglish: m.title.english,
    titleNative: m.title.native,
    synonyms: m.synonyms ?? [],
    format: m.format,
    status: m.status,
    episodes: m.episodes,
    season: m.season,
    seasonYear: m.seasonYear,
    genres: m.genres ?? [],
    averageScore: m.averageScore,
    coverUrl: m.coverImage?.extraLarge ?? null,
    coverPath: null,
    coverColor: m.coverImage?.color ?? null,
    bannerUrl: m.bannerImage,
    bannerPath: null,
    description: m.description,
    ownedCount: s.owned,
    watchedCount: watched.get(m.id)?.size ?? s.watched,
    addedAt: Date.now() - i * 3_600_000,
    lastWatchedAt: s.watched > 0 ? Date.now() - i * 60_000 : null,
    needsReview: m.id === 108632,
    libraryIds: [1],
    franchiseId: (media && franchises(media).get(m.id)?.root) ?? m.id,
    franchiseIndex: (media && franchises(media).get(m.id)?.index) ?? 0,
    franchiseSize: (media && franchises(media).get(m.id)?.size) ?? 1,
  };
}

async function episodes(m: AniMedia): Promise<EpisodeRow[]> {
  const s = SAMPLE.find((x) => x.id === m.id)!;
  const seen = watched.get(m.id) ?? new Set(Array.from({ length: s.watched }, (_, i) => String(i + 1)));
  watched.set(m.id, seen);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let eps: Record<string, any> = {};
  try {
    const r = await fetch(`https://api.ani.zip/mappings?anilist_id=${m.id}`);
    if (r.ok) eps = (await r.json()).episodes ?? {};
  } catch {
    /* offline: placeholder episodes */
  }
  const total = m.episodes ?? Math.max(s.owned, Object.keys(eps).filter((k) => /^\d+$/.test(k)).length);
  return Array.from({ length: total }, (_, i) => {
    const n = i + 1;
    const e = eps[String(n)] ?? {};
    const has = n <= s.owned;
    return {
      epKey: String(n),
      number: n,
      isSpecial: false,
      titleEn: e.title?.en ?? null,
      titleJa: e.title?.ja ?? null,
      titleRomaji: e.title?.["x-jat"] ?? null,
      overview: e.overview ?? e.summary ?? null,
      airDate: e.airdate ?? e.airDate ?? null,
      runtime: e.runtime ?? m.duration ?? null,
      thumbUrl: e.image ?? null,
      thumbPath: null,
      filler: false,
      recap: false,
      files: has ? [{ id: n, path: `D:\\Anime\\${m.title.romaji}\\Episode ${n}.mkv`, fileName: `Episode ${n}.mkv`, size: 1_400_000_000 }] : [],
      watchedAt: seen.has(String(n)) ? Date.now() : null,
      // Pretend the episode after the last watched one was stopped part-way.
      progressPos: has && !seen.has(String(n)) && n === s.watched + 1 && s.watched > 0 ? 754 : null,
      progressDur: has && !seen.has(String(n)) && n === s.watched + 1 && s.watched > 0 ? 1420 : null,
    };
  });
}

/** Home-page items: one in-progress episode per partly watched title, plus a couple of "new" ones. */
async function upNext(kind: "continue" | "new"): Promise<UpNextItem[]> {
  const all = await loadMedia();
  const picks =
    kind === "continue"
      ? SAMPLE.filter((s) => s.watched > 0 && s.watched < s.owned)
      : SAMPLE.filter((s) => s.watched === 0 && s.owned > 1).slice(0, 2);
  const out: UpNextItem[] = [];
  for (const [i, s] of picks.entries()) {
    const m = all.get(s.id);
    if (!m) continue;
    const n = kind === "continue" ? s.watched + 1 : 1;
    let ep: AniMedia = {};
    try {
      const r = await fetch(`https://api.ani.zip/mappings?anilist_id=${m.id}`);
      if (r.ok) ep = (await r.json()).episodes?.[String(n)] ?? {};
    } catch {
      /* offline */
    }
    out.push({
      anilistId: m.id,
      epKey: String(n),
      number: n,
      path: `D:\\Anime\\${m.title.romaji}\\Episode ${n}.mkv`,
      titleRomaji: m.title.romaji,
      titleEnglish: m.title.english,
      titleNative: m.title.native,
      format: m.format,
      episodes: m.episodes,
      coverUrl: m.coverImage?.extraLarge ?? null,
      coverPath: null,
      coverColor: m.coverImage?.color ?? null,
      bannerUrl: m.bannerImage,
      bannerPath: null,
      episodeTitle: ep.title?.en ?? null,
      thumbUrl: ep.image ?? null,
      thumbPath: null,
      runtime: m.duration,
      position: kind === "continue" && i === 0 ? 754 : 0,
      duration: kind === "continue" && i === 0 ? 1420 : 0,
      ownedCount: s.owned,
      watchedCount: s.watched,
      at: Date.now() / 1000 - i * 3600,
      newCount: kind === "new" ? Math.min(2, s.owned) : 0,
    });
  }
  return out;
}

async function detail(id: number): Promise<MediaDetail> {
  const m = (await loadMedia()).get(id);
  if (!m) throw new Error("Not in preview data");
  const all = await loadMedia();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const relations: RelationCard[] = (m.relations?.edges ?? [])
    .filter((e: AniMedia) => e.node.type === "ANIME")
    .slice(0, 8)
    .map((e: AniMedia) => ({
      relatedId: e.node.id,
      relationType: e.relationType,
      title: e.node.title.english ?? e.node.title.romaji,
      format: e.node.format,
      status: e.node.status,
      episodes: e.node.episodes,
      seasonYear: e.node.seasonYear,
      coverUrl: e.node.coverImage?.large ?? null,
      coverPath: null,
      ownedCount: all.has(e.node.id) ? 1 : 0,
    }));
  return {
    ...card(m, 0),
    idMal: m.idMal,
    duration: m.duration,
    startDate: date(m.startDate),
    endDate: date(m.endDate),
    tags: (m.tags ?? []).filter((t: AniMedia) => t.rank >= 60).map((t: AniMedia) => t.name),
    studios: (m.studios?.nodes ?? []).map((s: AniMedia) => s.name),
    nextAiringEpisode: m.nextAiringEpisode?.episode ?? null,
    nextAiringAt: m.nextAiringEpisode?.airingAt ?? null,
    episodesSource: "anizip",
    episodeList: await episodes(m),
    relations,
    extras: [],
    otherFiles: [],
    trackPref: null,
    groups: [{ id: 1, displayName: m.title.romaji, folderPath: `D:\\Anime\\${m.title.romaji}`, confidence: 0.97, manual: false }],
    franchise: (() => {
      const f = franchises(all).get(id);
      if (!f || f.size < 2) return [];
      return f.members.map((x) => {
        const c = card(x, 0);
        return {
          anilistId: x.id,
          titleRomaji: x.title.romaji,
          titleEnglish: x.title.english,
          titleNative: x.title.native,
          format: x.format,
          status: x.status,
          season: x.season,
          seasonYear: x.seasonYear,
          episodes: x.episodes,
          ownedCount: c.ownedCount,
          watchedCount: c.watchedCount,
        };
      });
    })(),
  };
}

export function installMock() {
  const preview = new URLSearchParams(location.search).get("preview");
  const empty = preview === "onboarding";
  const scanning = preview === "scanning";

  const library: Library = {
    id: 1,
    name: "Anime",
    kind: "anime",
    folders: [{ id: 1, path: "D:\\Anime", exists: true }],
    fileCount: 33,
    mediaCount: SAMPLE.length,
  };
  const unmatched: UnmatchedGroup[] = [
    {
      id: 99,
      libraryId: 1,
      displayName: "zzqx random home video",
      folderPath: "D:\\Anime\\zzqx random home video",
      titleGuess: "zzqx random home video",
      fileCount: 1,
      confidence: null,
      attempted: true,
      sampleFiles: ["clip 01.mp4"],
    },
  ];

  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "list_libraries":
          return empty ? [] : [library];
        case "get_library":
          return empty ? [] : [...(await loadMedia()).values()].map(card);
        case "get_unmatched":
          return empty ? [] : unmatched;
        case "is_scanning":
          return scanning;
        case "get_media_detail":
          return detail(Number(a.anilistId));
        case "set_watched": {
          const set = watched.get(Number(a.anilistId)) ?? new Set<string>();
          for (const k of a.epKeys as string[]) (a.watched ? set.add(k) : set.delete(k));
          watched.set(Number(a.anilistId), set);
          return null;
        }
        case "search_anilist":
          return [];
        case "get_prefs":
          return prefs;
        case "set_pref":
          prefs[String(a.key)] = String(a.value);
          return null;
        case "get_up_next":
          return empty ? [] : upNext("continue");
        case "get_new_episodes":
          return empty ? [] : upNext("new");
        case "detect_players":
          return [
            { kind: "mpv", path: "C:\\Program Files\\mpv\\mpv.exe" },
            { kind: "vlc", path: "C:\\Program Files\\VideoLAN\\VLC\\vlc.exe" },
          ];
        case "play_episode":
          setTimeout(
            () =>
              emit("playback", {
                state: "tracking",
                player: prefs.player ?? "mpv",
                anilistId: a.anilistId,
                epKey: a.epKey,
                position: 0,
                duration: 1420,
                hint: null,
              }),
            500,
          );
          return null;
        case "plugin:app|version":
          return "0.3.1";
        default:
          console.info("[preview] ignored command", cmd, a);
          return null;
      }
    },
    { shouldMockEvents: true },
  );

  if (scanning) {
    setTimeout(() => emit("scan-progress", { phase: "metadata", current: 4, total: 9, message: "Sousou no Frieren" }), 600);
  }
  console.info("[preview] Browser preview mode — IPC is mocked with sample data.");
}
