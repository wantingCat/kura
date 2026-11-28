import { describe, expect, it } from "vitest";
import {
  compareTitles,
  diceBigrams,
  effectiveSeasonNumber,
  extractPartNumber,
  extractSeasonNumber,
  extractYear,
  normalizeTitle,
  parseEpisodeNumber,
  stripRomajiMacrons,
} from "../titleMatch";

describe("titleMatch", () => {
  it("folds Romaji macrons and circumflexes while preserving case", () => {
    expect(stripRomajiMacrons("Sōsō no Frieren")).toBe("Soso no Frieren");
    expect(stripRomajiMacrons("Ōkami to Kōshinryō")).toBe("Okami to Koshinryo");
    expect(stripRomajiMacrons("Yūsha")).toBe("Yusha");
  });

  it("extracts explicit season numbers across English, Roman, and Japanese markers", () => {
    expect(extractSeasonNumber("Sousou no Frieren")).toBe(-1);
    expect(effectiveSeasonNumber("Sousou no Frieren")).toBe(1);
    expect(extractSeasonNumber("Jujutsu Kaisen Season 2")).toBe(2);
    expect(extractSeasonNumber("Mob Psycho 100 II")).toBe(2);
    expect(extractSeasonNumber("Overlord IV")).toBe(4);
    expect(extractSeasonNumber("Kusuriya no Hitorigoto 2nd Season")).toBe(2);
    expect(extractSeasonNumber("Oshi no Ko Second Season")).toBe(2);
    expect(extractSeasonNumber("[SubsPlease] Show Name S03E07 (1080p)")).toBe(3);
    expect(extractSeasonNumber("推しの子 第2期")).toBe(2);
  });

  it("extracts part and cour numbers cleanly", () => {
    expect(extractPartNumber("Attack on Titan: The Final Season Part 2")).toBe(2);
    expect(extractPartNumber("Mushoku Tensei II Cour 2")).toBe(2);
    expect(extractPartNumber("Spy x Family Second Cour")).toBe(2);
    expect(extractPartNumber("無職転生 第2クール")).toBe(2);
    expect(extractPartNumber("Bocchi the Rock!")).toBe(-1);
  });

  it("parses episode numbers while ignoring resolutions, codecs, CRC32, and years", () => {
    expect(
      parseEpisodeNumber("[SubsPlease] Sousou no Frieren - 28 (1080p) [A1B2C3D4]"),
    ).toBe(28);
    expect(
      parseEpisodeNumber("[Erai-raws] Kusuriya no Hitorigoto - 05v2 [1080p][HEVC x265]"),
    ).toBe(5);
    expect(parseEpisodeNumber("Show.Name.S02E11.1080p.WEB-DL.H.264")).toBe(11);
    expect(parseEpisodeNumber("葬送のフリーレン 第14話 「若者の特権」")).toBe(14);
    expect(parseEpisodeNumber("[Group] Standalone Anime Movie (2024) [1080p] [2160p]")).toBeNull();
    expect(extractYear("[Group] Standalone Anime Movie (2024) [1080p]")).toBe(2024);
  });

  it("normalizes base titles and computes similarity scores", () => {
    const norm = normalizeTitle("Kaguya-sama: Love is War - Second Season (TV)");
    expect(norm.season).toBe(2);
    expect(norm.cleanBaseTitle).toBe("Kaguya sama Love is War");
    expect(norm.isMain).toBe(false);

    expect(diceBigrams("frieren", "frieren")).toBe(1);
    expect(compareTitles("Bocchi the Rock! (TV)", "Bocchi the Rock")).toBe(1);
    expect(compareTitles("Sousou no Frieren", "Jujutsu Kaisen")).toBe(0);
  });
});
