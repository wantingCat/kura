import { beforeEach, describe, expect, it } from "vitest";
import type { EpisodeRow, MediaDetail } from "../api";
import {
  clearCachedMediaDetails,
  computeNextUp,
  getCachedMediaDetail,
  invalidateCachedMediaDetail,
  setCachedMediaDetail,
} from "../mediaDetail.svelte";

function makeEp(overrides: Partial<EpisodeRow> & { number: number }): EpisodeRow {
  const { number, ...rest } = overrides;
  return {
    epKey: String(number),
    number,
    isSpecial: false,
    titleEn: `Episode ${number}`,
    titleJa: null,
    titleRomaji: null,
    overview: null,
    airDate: null,
    runtime: 24,
    thumbUrl: null,
    thumbPath: null,
    filler: false,
    recap: false,
    watchedAt: null,
    progressPos: null,
    progressDur: null,
    files: [],
    subtitles: [],
    ...rest,
  };
}

function makeDetail(id: number, episodeList: EpisodeRow[] = [], anilistProgress: number | null = null): MediaDetail {
  return {
    anilistId: id,
    idMal: null,
    titleRomaji: `Anime ${id}`,
    titleEnglish: `Anime ${id}`,
    titleNative: null,
    synonyms: [],
    description: null,
    coverUrl: null,
    coverPath: null,
    bannerUrl: null,
    bannerPath: null,
    logoUrl: null,
    logoPath: null,
    coverColor: null,
    format: "TV",
    status: "RELEASING",
    episodes: 12,
    duration: 24,
    season: "FALL",
    seasonYear: 2025,
    averageScore: 85,
    genres: [],
    tags: [],
    studios: [],
    nextAiringEp: null,
    nextAiringAt: null,
    anilistStatus: null,
    anilistProgress,
    anilistScore: null,
    inStreamLibrary: false,
    trackPref: null,
    episodeList,
    extras: [],
    groups: [],
    franchise: [],
    relations: [],
    characters: [],
    staff: [],
  } as unknown as MediaDetail;
}

describe("mediaDetail cache & computeNextUp", () => {
  beforeEach(() => {
    clearCachedMediaDetails();
  });

  it("enforces bounded 32-entry LRU eviction and MRU promotion", () => {
    for (let i = 1; i <= 32; i++) {
      setCachedMediaDetail(makeDetail(i));
    }
    // Touch entry 1 so it becomes MRU; entry 2 is now LRU
    expect(getCachedMediaDetail(1)?.anilistId).toBe(1);

    // Inserting entry 33 should evict entry 2, keeping entry 1
    setCachedMediaDetail(makeDetail(33));
    expect(getCachedMediaDetail(2)).toBeUndefined();
    expect(getCachedMediaDetail(1)?.anilistId).toBe(1);
    expect(getCachedMediaDetail(33)?.anilistId).toBe(33);

    invalidateCachedMediaDetail(1);
    expect(getCachedMediaDetail(1)).toBeUndefined();
  });

  it("prioritizes partially-watched local files, then unwatched local, and supports streaming mode", () => {
    const localFile = { id: 1, path: "/anime/ep.mkv", fileName: "ep.mkv", size: 1000 };
    const detail = makeDetail(
      100,
      [
        makeEp({ number: 1, watchedAt: 1700000000, files: [localFile] }),
        makeEp({ number: 2, files: [localFile] }),
        makeEp({ number: 3, progressPos: 420, progressDur: 1440, files: [localFile] }),
      ],
      1,
    );
    // Episode 3 has in-progress resume position, so it wins over unwatched Episode 2
    expect(computeNextUp(detail, "local")?.number).toBe(3);

    // Unowned show: returns null in strictly local mode, but picks next episode after AniList progress in streaming mode
    const streamShow = makeDetail(
      200,
      [makeEp({ number: 1 }), makeEp({ number: 2 }), makeEp({ number: 3 })],
      2,
    );
    expect(computeNextUp(streamShow, "local")).toBeNull();
    expect(computeNextUp(streamShow, "streaming")?.number).toBe(3);
  });
});
