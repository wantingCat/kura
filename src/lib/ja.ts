/**
 * Japanese accent layer (日本語アクセント) & pitch-accent / TTS utilities for Kura's Tokyo Night UI.
 *
 * Kura stays an English app; this module supplies the Japanese labels that are
 * sprinkled on top of it (row subtitles, calendar weekdays, episode / season /
 * status labels, placeholders, page kanji). Keep every translation here so the
 * vocabulary stays consistent and easy to extend.
 *
 * Accessibility rules (WCAG 3.1.2 Language of Parts):
 *  - Render Japanese inside an element with `lang="ja"` (see Ja.svelte).
 *  - If Japanese only DUPLICATES visible English, mark it decorative (aria-hidden).
 *  - If Japanese REPLACES English (weekday headers, placeholders), keep the
 *    English in aria-label / <abbr title> / .sr-only.
 */

// ---------------------------------------------------------------------------
// Static vocabulary
// ---------------------------------------------------------------------------

export const JA = {
  brandMark: "蔵",
  // Navigation / pages (used as kanji watermarks + menu accents)
  pages: {
    home: { ja: "蔵", en: "Home" },
    series: { ja: "シリーズ", en: "Series" },
    movies: { ja: "劇場版", en: "Movies" },
    myList: { ja: "マイリスト", en: "My List" },
    discover: { ja: "発見", en: "Discover" },
    calendar: { ja: "放送", en: "Calendar" },
    immersion: { ja: "没入", en: "Immersion" },
    review: { ja: "確認", en: "Review" },
    settings: { ja: "設定", en: "Settings" },
  },
  // Row / section subtitles
  rows: {
    continueWatching: "続きから",
    newEpisodes: "新エピソード",
    recentlyAdded: "新着",
    top10: "ランキング",
    trending: "人気",
    myList: "マイリスト",
    library: "ライブラリ",
    episodes: "エピソード",
    characters: "キャラクター",
    staff: "スタッフ",
    related: "関連作品",
    upNext: "次はこちら",
    spotlight: "注目",
  },
  // Placeholders (always pair with an English aria-label)
  placeholders: {
    search: "作品を検索…",
    calendarFilter: "番組を絞り込む…",
    libraryFilter: "ライブラリを絞り込む…",
    discover: "アニメを探す…",
  },
  // Generic UI states
  states: {
    loading: "読み込み中…",
    empty: "まだ何もありません",
    done: "完了",
    error: "エラー",
    notice: "お知らせ",
    warning: "注意",
    today: "今日",
    next: "次回",
    nowPlaying: "再生中",
  },
  // Immersion / review stats
  stats: {
    today: "今日",
    week: "今週",
    month: "今月",
    total: "合計",
    streak: "連続",
    words: "単語",
    lookups: "検索",
    hours: "時間",
    correct: "正解",
    again: "もう一度",
  },
  // Settings section accents
  settings: {
    playback: "再生",
    subtitles: "字幕",
    library: "ライブラリ",
    sync: "同期",
    appearance: "外観",
    notifications: "通知",
    streaming: "配信",
    advanced: "詳細",
    about: "情報",
    japaneseAccents: "日本語アクセント",
  },
} as const;

// ---------------------------------------------------------------------------
// Weekdays & dates
// ---------------------------------------------------------------------------

/** Index = JS `Date#getDay()` (0 = Sunday). */
const WEEKDAY_SHORT = ["日", "月", "火", "水", "木", "金", "土"] as const;
const WEEKDAY_EN_SHORT = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"] as const;

/** 月 / 火 / … for a JS weekday index (0 = Sunday). */
export function jaWeekday(day: number): string {
  return WEEKDAY_SHORT[((day % 7) + 7) % 7];
}

/** 月曜日 / 火曜日 / … */
export function jaWeekdayLong(day: number): string {
  return `${jaWeekday(day)}曜日`;
}

/** English helper so callers can keep accessible names next to Japanese. */
export function enWeekdayShort(day: number): string {
  return WEEKDAY_EN_SHORT[((day % 7) + 7) % 7];
}

/** 11月17日 */
export function jaDate(d: Date): string {
  return `${d.getMonth() + 1}月${d.getDate()}日`;
}

/** 23:30 — local time, 24-hour (Japanese TV-listing style). */
export function jaTime(d: Date): string {
  const h = String(d.getHours()).padStart(2, "0");
  const m = String(d.getMinutes()).padStart(2, "0");
  return `${h}:${m}`;
}

// ---------------------------------------------------------------------------
// Episodes, seasons, formats, status
// ---------------------------------------------------------------------------

/** 第8話 */
export function jaEpisode(n: number | string | null | undefined): string {
  if (n === null || n === undefined || n === "") return "";
  const num = typeof n === "string" ? Number(n) : n;
  return Number.isFinite(num) ? `第${num}話` : String(n);
}

/** 全28話 (total episode count) */
export function jaEpisodeCount(n: number | null | undefined): string {
  return n ? `全${n}話` : "";
}

/** 残り12分 (time remaining). Accepts seconds. */
export function jaRemaining(seconds: number | null | undefined): string {
  if (!seconds || seconds <= 0) return "";
  const mins = Math.max(1, Math.round(seconds / 60));
  if (mins >= 60) {
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return m ? `残り${h}時間${m}分` : `残り${h}時間`;
  }
  return `残り${mins}分`;
}

/** 第1期 (season / cour number). */
export function jaCour(n: number): string {
  return `第${n}期`;
}

const SEASONS: Record<string, string> = {
  WINTER: "冬",
  SPRING: "春",
  SUMMER: "夏",
  FALL: "秋",
};

/** "FALL", 2023 → 秋 2023 */
export function jaSeason(season: string | null | undefined, year?: number | null): string {
  const s = season ? SEASONS[season.toUpperCase()] ?? "" : "";
  if (s && year) return `${s} ${year}`;
  if (s) return s;
  return year ? `${year}年` : "";
}

const FORMATS: Record<string, string> = {
  TV: "TV",
  TV_SHORT: "ショート",
  MOVIE: "劇場版",
  SPECIAL: "特別編",
  OVA: "OVA",
  ONA: "ONA",
  MUSIC: "MV",
};

/** MOVIE → 劇場版 */
export function jaFormat(format: string | null | undefined): string {
  if (!format) return "";
  return FORMATS[format.toUpperCase()] ?? format;
}

const STATUSES: Record<string, string> = {
  RELEASING: "放送中",
  FINISHED: "完結",
  NOT_YET_RELEASED: "未放送",
  CANCELLED: "中止",
  HIATUS: "休止",
};

/** RELEASING → 放送中 */
export function jaStatus(status: string | null | undefined): string {
  if (!status) return "";
  return STATUSES[status.toUpperCase()] ?? "";
}

const WATCH_STATUSES: Record<string, string> = {
  watching: "視聴中",
  current: "視聴中",
  planning: "予定",
  completed: "視聴済み",
  paused: "一時停止",
  dropped: "中断",
  repeating: "再視聴",
  all: "すべて",
};

/** Library watch bucket / AniList list status → Japanese. */
export function jaWatchStatus(key: string | null | undefined): string {
  if (!key) return "";
  return WATCH_STATUSES[key.toLowerCase()] ?? "";
}

// ---------------------------------------------------------------------------
// Genres
// ---------------------------------------------------------------------------

const GENRES: Record<string, string> = {
  Action: "アクション",
  Adventure: "冒険",
  Comedy: "コメディ",
  Drama: "ドラマ",
  Ecchi: "エッチ",
  Fantasy: "ファンタジー",
  Horror: "ホラー",
  "Mahou Shoujo": "魔法少女",
  Mecha: "メカ",
  Music: "音楽",
  Mystery: "ミステリー",
  Psychological: "サイコ",
  Romance: "恋愛",
  "Sci-Fi": "SF",
  "Slice of Life": "日常",
  Sports: "スポーツ",
  Supernatural: "超自然",
  Thriller: "スリラー",
  Hentai: "成人向け",
};

/** "Slice of Life" → 日常 (falls back to "" for unknown genres). */
export function jaGenre(genre: string | null | undefined): string {
  if (!genre) return "";
  return GENRES[genre] ?? "";
}

// ---------------------------------------------------------------------------
// Stats / counts
// ---------------------------------------------------------------------------

/** 12日連続 (streak). */
export function jaStreak(days: number): string {
  return `${days}日連続`;
}

/** 1時間24分 from seconds. */
export function jaDuration(seconds: number): string {
  const mins = Math.round(seconds / 60);
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  if (h && m) return `${h}時間${m}分`;
  if (h) return `${h}時間`;
  return `${m}分`;
}

// ---------------------------------------------------------------------------
// Pitch-Accent Mora Splitter, Contour & Japanese TTS
// ---------------------------------------------------------------------------

const SMALL_YOTACIZED_KANA = new Set([
  "ゃ",
  "ゅ",
  "ょ",
  "ぁ",
  "ぃ",
  "ぅ",
  "ぇ",
  "ぉ",
  "ゎ",
  "ャ",
  "ュ",
  "ョ",
  "ァ",
  "ィ",
  "ゥ",
  "ェ",
  "ォ",
  "ヮ",
]);

/** Split a hiragana/katakana reading into moras (combining digraphs such as きゃ, ティ, くゎ, グヮ). */
export function splitMoras(reading: string): string[] {
  const chars = Array.from(reading);
  const out: string[] = [];
  for (let i = 0; i < chars.length; i++) {
    if (i + 1 < chars.length && SMALL_YOTACIZED_KANA.has(chars[i + 1])) {
      out.push(chars[i] + chars[i + 1]);
      i++;
    } else {
      out.push(chars[i]);
    }
  }
  return out;
}

/** Standard Tokyo pitch-accent high/low contour for mora index given NHK/Yomitan downstep index. */
export function isMoraHigh(moraIndex: number, downstep: number): boolean {
  if (downstep === 0) return moraIndex > 0;
  if (downstep === 1) return moraIndex === 0;
  return moraIndex > 0 && moraIndex < downstep;
}

/** Speak Japanese text using Web Speech synthesis when available. */
export function speakJapanese(text: string, e?: Event): void {
  e?.stopPropagation();
  if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
  try {
    window.speechSynthesis.cancel();
    const u = new SpeechSynthesisUtterance(text);
    u.lang = "ja-JP";
    u.rate = 0.95;
    window.speechSynthesis.speak(u);
  } catch {
    // ignore if speech synthesis unavailable
  }
}
