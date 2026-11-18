# Calendar page — 放送 (Tokyo Night override)

> Overrides `../MASTER.md` for `src/routes/calendar/+page.svelte`.

## Header
- `.kanji-watermark` 放送 (aria-hidden via `<Ja>`) behind "Seasonal Airing Calendar" + `.ja-sub` 放送.
- Season picker = surface pill; season tabs show English + decorative 冬/春/夏/秋 (`jaSeason`); active tab = `--primary` fill.
- Filter input: rounded pill, placeholder `JA.placeholders.calendarFilter` (番組を絞り込む…) when accents are on, English otherwise; **always** `aria-label="Filter calendar by title, studio or genre"`; focus = violet ring + `--neon-glow`.

## Week grid
- 7 columns (Mon→Sun), `--surface` cards, **no borders**, `--shadow-sm`.
- Column header:
  - Big weekday kanji 月 火 水 木 金 土 日 (`jaWeekday(jsDay)`, 30px/700) rendered with `<Ja text label="Monday" essential>` → screen readers hear English; with accents off the English day name is shown instead.
  - Small `.tn-label` English short day (MON) under the kanji (decorative, hidden when accents off).
  - Meta: `月曜日 · Monday` (decorative, accents on only) + date of the next occurrence of that weekday as `jaDate` (11月17日) with English (`Nov 17`) as the accessible label.
- **Today**: violet-tinted surface, 1px violet ring + `--neon-glow`, header divider becomes a 2px `--neon` underline with glow, kanji gets a soft violet text-shadow, `今日` chip (`--cta` fill, English "Today" kept for SR / when accents off); column gets `aria-current="date"`.
- Weekends are styled exactly like weekdays (user decision — no blue Saturday / red Sunday).

## Entries
- `--surface-2` tiles, no border; hover = lift −2px + `--surface-3` + `--neon-glow`; focus-visible = violet outline.
- In-library entries: subtle green tint + 3px inset left accent (`--ok`), not a border.
- Time pill on the cover: **local time, 24-hour** via `jaTime` (e.g. 23:30), Space Grotesk tabular numerals.
- Chips: `放送中` (status RELEASING, `jaStatus`, English "Airing" accessible), library/AniList status badge, countdown `次回 Ep 5 in 2d` (次回 decorative).
- Native title (`titleNative`) one line under the English title, `--text-2`, Noto Sans JP, ellipsis.
- Unscheduled/TBA grid reuses the same tile style on `--surface`, with status chips via `jaStatus` and section `.ja-sub` `jaSeason(season, year)`.

## Motion / a11y
- Hover lift disabled under `prefers-reduced-motion`.
- Every Japanese string that replaces English carries the English label; decorative Japanese is aria-hidden and disappears with `html.ja-off`.
