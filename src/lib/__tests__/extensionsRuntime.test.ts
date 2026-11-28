import CryptoJS from "crypto-js";
import { describe, expect, it } from "vitest";
import { __workerTestHooks } from "../extensions/worker";

const {
  makeCaseInsensitiveMap,
  $habari,
  $goFeed,
  $torrentUtils,
  KuraBuffer,
  LoadDoc,
  handleWorkerMessage,
} = __workerTestHooks;

function asciiToBase64(str: string): string {
  return KuraBuffer.from(str, "utf8").toString("base64");
}

describe("Seanime sandboxed worker runtime", () => {
  it("supports both bracket access and Headers methods (.get, .has, .forEach) case-insensitively", () => {
    const hdrs = makeCaseInsensitiveMap({
      "Content-Type": "application/vnd.apple.mpegurl",
      "Set-Cookie": "sid=abc123; Path=/",
    }) as Record<string, string> & {
      get: (k: string) => string | null;
      has: (k: string) => boolean;
      forEach: (cb: (v: string, k: string) => void) => void;
    };

    expect(hdrs["content-type"]).toBe("application/vnd.apple.mpegurl");
    expect(hdrs["Content-Type"]).toBe("application/vnd.apple.mpegurl");
    expect(hdrs.get("SET-COOKIE")).toBe("sid=abc123; Path=/");
    expect(hdrs.get("x-missing")).toBeNull();
    expect(hdrs.has("set-cookie")).toBe(true);
    expect(hdrs.has("authorization")).toBe(false);
    expect(Object.keys(hdrs).sort()).toEqual(["content-type", "set-cookie"]);

    const seen: [string, string][] = [];
    hdrs.forEach((v, k) => seen.push([k, v]));
    expect(seen).toHaveLength(2);
  });

  it("parses anime filenames via $habari and RSS XML via $goFeed", () => {
    const parsed = $habari.parse(
      "[SubsPlease] Kusuriya no Hitorigoto S2 - 09 (1080p) [DEAD1234].mkv",
    );
    expect(parsed.release_group).toBe("SubsPlease");
    expect(parsed.video_resolution).toBe("1080p");
    expect(parsed.file_checksum).toBe("DEAD1234");
    expect(parsed.file_extension).toBe("mkv");
    expect(parsed.season_number).toEqual(["2"]);
    expect(parsed.episode_number).toEqual(["9"]);

    const rss = `<?xml version="1.0" encoding="UTF-8"?>
      <rss version="2.0" xmlns:nyaa="https://example.org/xmlns/nyaa">
        <channel>
          <title>Demo Feed</title>
          <item>
            <title>[DemoGroup] Show - 01 (1080p).mkv</title>
            <link>https://example.org/download/1.torrent</link>
            <nyaa:seeders>142</nyaa:seeders>
            <enclosure url="https://example.org/1.torrent" type="application/x-bittorrent" />
          </item>
        </channel>
      </rss>`;
    const feed = $goFeed.parse(rss);
    expect(feed.title).toBe("Demo Feed");
    expect(feed.items).toHaveLength(1);
    expect(feed.items[0].custom.seeders).toBe("142");
    expect(feed.items[0].enclosures[0].url).toBe("https://example.org/1.torrent");
  });

  it("computes exact root-dict SHA-1 btih magnet links and ignores nested fake 4:info keys", () => {
    const infoDict = "d6:lengthi1048576e4:name8:test.mkv12:piece lengthi262144e6:pieces20:01234567890123456789e";
    // Root dictionary contains a nested fake `4:info` inside `7:comment` BEFORE the real root `4:info`!
    const torrentBencode = `d7:commentd4:info5:boguse4:info${infoDict}e`;
    const expectedHash = CryptoJS.SHA1(CryptoJS.enc.Utf8.parse(infoDict)).toString(
      CryptoJS.enc.Hex,
    );

    const magnet = $torrentUtils.getMagnetLinkFromTorrentData(asciiToBase64(torrentBencode));
    expect(magnet).toBe(`magnet:?xt=urn:btih:${expectedHash}`);
  });

  it("handles KuraBuffer encodings and LoadDoc HTML scraping", () => {
    const buf = KuraBuffer.from("蔵 Kura", "utf8");
    const b64 = buf.toString("base64");
    expect(KuraBuffer.from(b64, "base64").toString("utf8")).toBe("蔵 Kura");

    const $ = LoadDoc(`<ul><li data-ep="1"><a href="/watch/1">Ep 1</a></li><li data-ep="2"><a href="/watch/2">Ep 2</a></li></ul>`);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const eps = $("li").map((_: number, el: any) => ({
      num: Number(el.attr("data-ep")),
      href: el.find("a").attr("href"),
    }));
    expect(eps).toEqual([
      { num: 1, href: "/watch/1" },
      { num: 2, href: "/watch/2" },
    ]);
  });

  it("transpiles and executes a TypeScript Seanime onlinestream Provider end-to-end", async () => {
    const tsPayload = `
      class Provider {
        getSettings(): Settings {
          return { episodeServers: ["Sub", "Dub"], supportsDub: true };
        }
        async search(opts: SearchOptions): Promise<SearchResult[]> {
          return [{ id: "anime-101", title: opts.query, url: "https://example.org/a/101", subOrDub: "sub" }];
        }
        async findEpisodes(id: string): Promise<EpisodeDetails[]> {
          return [{ id: id + "$ep=1", number: 1, url: "https://example.org/watch/1" }];
        }
        async findEpisodeServer(ep: EpisodeDetails, server: string): Promise<EpisodeServer> {
          return {
            server,
            headers: { Referer: "https://example.org/" },
            videoSources: [
              {
                url: "https://cdn.example.org/hls/master.m3u8",
                type: "m3u8",
                quality: "1080p",
                subtitles: [{ id: "en", url: "https://cdn.example.org/subs/en.vtt", language: "English", isDefault: true }],
              },
            ],
          };
        }
      }
    `;

    await handleWorkerMessage({
      data: {
        kind: "init",
        callId: 1,
        data: { id: "demo-ts", language: "typescript", payload: tsPayload, userPrefs: {} },
      },
    } as MessageEvent);

    // Verify Provider was compiled and registered cleanly by invoking methods via call
    await expect(
      handleWorkerMessage({
        data: { kind: "call", callId: 2, method: "findEpisodeServer", args: [{ id: "anime-101$ep=1", number: 1, url: "" }, "Sub"] },
      } as MessageEvent),
    ).resolves.toBeUndefined();
  });
});
