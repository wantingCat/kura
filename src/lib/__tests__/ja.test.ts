import { describe, expect, it } from "vitest";
import {
  enWeekdayShort,
  isMoraHigh,
  jaEpisode,
  jaEpisodeCount,
  jaFormat,
  jaGenre,
  jaRemaining,
  jaSeason,
  jaStatus,
  jaWeekday,
  splitMoras,
} from "../ja";

describe("ja helpers & pitch-accent mora tools", () => {
  it("splits hiragana and katakana readings into moras including yotacized digraphs", () => {
    expect(splitMoras("きょうと")).toEqual(["きょ", "う", "と"]);
    expect(splitMoras("パーティー")).toEqual(["パ", "ー", "ティ", "ー"]);
    expect(splitMoras("シークヮーサー")).toEqual(["シ", "ー", "クヮ", "ー", "サ", "ー"]);
    expect(splitMoras("がっこう")).toEqual(["が", "っ", "こ", "う"]);
  });

  it("computes Tokyo standard pitch contours (heiban, atamadaka, nakadaka, odaka)", () => {
    // 0 = heiban (L-H-H-H)
    expect([0, 1, 2, 3].map((i) => isMoraHigh(i, 0))).toEqual([false, true, true, true]);
    // 1 = atamadaka (H-L-L-L)
    expect([0, 1, 2, 3].map((i) => isMoraHigh(i, 1))).toEqual([true, false, false, false]);
    // 2 = nakadaka downstep after 2nd mora (L-H-L-L)
    expect([0, 1, 2, 3].map((i) => isMoraHigh(i, 2))).toEqual([false, true, false, false]);
    // 3 = downstep after 3rd mora (L-H-H-L)
    expect([0, 1, 2, 3].map((i) => isMoraHigh(i, 3))).toEqual([false, true, true, false]);
  });

  it("formats Tokyo Night Japanese labels accurately", () => {
    expect(jaWeekday(0)).toBe("日");
    expect(jaWeekday(-1)).toBe("土");
    expect(enWeekdayShort(1)).toBe("MON");
    expect(jaEpisode(8)).toBe("第8話");
    expect(jaEpisode("")).toBe("");
    expect(jaEpisodeCount(28)).toBe("全28話");
    expect(jaRemaining(720)).toBe("残り12分");
    expect(jaRemaining(5400)).toBe("残り1時間30分");
    expect(jaSeason("FALL", 2023)).toBe("秋 2023");
    expect(jaFormat("MOVIE")).toBe("劇場版");
    expect(jaStatus("RELEASING")).toBe("放送中");
    expect(jaGenre("Slice of Life")).toBe("日常");
  });
});
