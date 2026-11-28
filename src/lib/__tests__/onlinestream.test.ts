import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstalledExtension, MediaDetail } from "../api";

const startWebStreamMock = vi.fn().mockResolvedValue(undefined);
const getMappingMock = vi.fn().mockResolvedValue(null);

vi.mock("../api", () => ({
  api: {
    getOnlinestreamMapping: (...args: unknown[]) => getMappingMock(...args),
    startWebStream: (...args: unknown[]) => startWebStreamMock(...args),
    prefetchWebStreamSubtitles: vi.fn().mockResolvedValue(null),
  },
}));

const getInstalledExtensionsMock = vi.fn<() => Promise<InstalledExtension[]>>();
const getSettingsMock = vi.fn();
const searchMock = vi.fn();
const findEpisodesMock = vi.fn();
const findEpisodeServerMock = vi.fn();

vi.mock("../extensions/host", () => ({
  buildSeanimeMedia: (d: MediaDetail) => ({ id: d.anilistId }),
  getInstalledExtensions: () => getInstalledExtensionsMock(),
  onlinestreamProviderHost: {
    getSettings: (...args: unknown[]) => getSettingsMock(...args),
    search: (...args: unknown[]) => searchMock(...args),
    findEpisodes: (...args: unknown[]) => findEpisodesMock(...args),
    findEpisodeServer: (...args: unknown[]) => findEpisodeServerMock(...args),
  },
}));

import {
  autoPlayWebEpisode,
  invalidateOnlinestreamCache,
  resolveEpisodeOptionsForProvider,
} from "../onlinestream.svelte";

const demoExt: InstalledExtension = {
  id: "demo-stream",
  name: "Demo Stream",
  version: "1.0.0",
  extType: "onlinestream-provider",
  language: "javascript",
  manifestUri: "https://example.org/manifest.json",
  iconUrl: null,
  author: "Tester",
  description: "Test provider",
  enabled: true,
  payload: "",
  userConfig: {},
  updatedAt: 0,
};

const demoDetail = {
  anilistId: 154587,
  idMal: 52991,
  titleRomaji: "Sousou no Frieren",
  titleEnglish: "Frieren: Beyond Journey's End",
  titleNative: "葬送のフリーレン",
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
  status: "FINISHED",
  episodes: 28,
  duration: 24,
  season: "FALL",
  seasonYear: 2023,
  averageScore: 91,
  genres: [],
  tags: [],
  studios: [],
  nextAiringEp: null,
  nextAiringAt: null,
  anilistStatus: null,
  anilistProgress: null,
  anilistScore: null,
  inStreamLibrary: false,
  trackPref: null,
  episodeList: [],
  extras: [],
  groups: [],
  franchise: [],
  relations: [],
  characters: [],
  staff: [],
} as unknown as MediaDetail;

describe("onlinestream resolution & fallback", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invalidateOnlinestreamCache();
    getInstalledExtensionsMock.mockResolvedValue([demoExt]);
    searchMock.mockResolvedValue([
      { id: "frieren-sub", title: "Sousou no Frieren", url: "https://example.org/frieren", subOrDub: "sub" },
    ]);
    findEpisodesMock.mockResolvedValue([
      { id: "ep-1", number: 1, url: "https://example.org/watch/1" },
    ]);
  });

  it("prioritizes Sub servers over Dub when Japanese audio is preferred", async () => {
    getSettingsMock.mockResolvedValue({ episodeServers: ["Dub", "Sub"] });
    findEpisodeServerMock.mockImplementation(async (_ext, _ep, srv: string) => ({
      server: srv,
      headers: { Referer: "https://example.org/" },
      videoSources: [
        {
          url: `https://cdn.example.org/${srv.toLowerCase()}/master.m3u8`,
          type: "m3u8",
          quality: "1080p",
          subtitles:
            srv === "Sub"
              ? [{ id: "en", url: "https://cdn.example.org/en.vtt", language: "eng", isDefault: true }]
              : [],
        },
      ],
    }));

    const res = await resolveEpisodeOptionsForProvider(demoExt, demoDetail, 1, false, true, false);
    expect(res.options).toHaveLength(2);
    expect(res.options[0].server).toBe("Sub");
    expect(res.options[0].isSoftSub).toBe(true);
    expect(res.options[1].server).toBe("Dub");
  });

  it("falls back to playable stream instead of failing when provider omits external subtitle array", async () => {
    getSettingsMock.mockResolvedValue({ episodeServers: ["Sub"] });
    findEpisodeServerMock.mockResolvedValue({
      server: "Sub",
      headers: { Referer: "https://example.org/" },
      videoSources: [
        {
          url: "https://cdn.example.org/hls/index.m3u8",
          type: "m3u8",
          quality: "1080p",
          subtitles: [],
        },
      ],
    });

    // Even when softSubOnly = true, autoPlayWebEpisode falls back to bestHardsubFallback so video plays!
    const picked = await autoPlayWebEpisode(demoDetail, 1, true, false);
    expect(picked.url).toBe("https://cdn.example.org/hls/index.m3u8");
    expect(startWebStreamMock).toHaveBeenCalledTimes(1);
    expect(startWebStreamMock).toHaveBeenCalledWith(
      expect.objectContaining({
        anilistId: 154587,
        epKey: "1",
        url: "https://cdn.example.org/hls/index.m3u8",
      }),
    );
  });
});
