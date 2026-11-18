# Details — modal + full anime page (詳細)

> Page override for `MASTER.md`. Applies to `src/lib/components/DetailsModal.svelte` (quick view) and
> `src/routes/anime/[id]/+page.svelte` (full page). Rules here win over MASTER where they differ.

## Details modal (クイックビュー)

### Shell
| Part | Spec |
|---|---|
| Root | `position: fixed; inset: 0; z-index: 900` (navbar ≈ 800, toasts render later in the DOM and stay above) |
| Scrim | `rgba(5,6,12,.7)` + `blur(8px)`; click closes |
| Ambient glow | radial gradient of the show's `coverColor` (fallback `--neon-violet`) at ~34 % → transparent, blurred 30px, behind the modal |
| Modal | `width: min(880px, 94vw)`, `max-height: calc(100vh - 48px)`, internal scroll, `--glass-tn-strong` + `--blur-tn`, 1px `--glass-hairline`, `--r-xl`, deep shadow + faint cover-colour glow |
| Motion | scrim `fade` 200ms, modal `scale` 0.96→1 + fade 240ms `cubicOut`, both `|local`; durations become 0 under `prefers-reduced-motion` |

### Accessibility
- `role="dialog" aria-modal="true" aria-labelledby="dm-title"`, `aria-busy` while loading.
- Focus moves to the dialog on open; Tab / Shift+Tab are trapped; **Esc** closes; focus restore is done by `detailsModal.close()`.
- `#main-scroll` gets `overflow-y: hidden` while open (restored on close).
- Close = 40px round glass button top-right, `aria-label="Close"`. All round action buttons are 44px with English `aria-label` + `title`.
- Japanese is decorative (`<Ja>` → `aria-hidden`) unless it replaces English (season / format / episode count / status chips use `label` + `essential`, so screen readers and the accents-off mode get English).

### Banner
- Image: `img(bannerPath, bannerUrl) ?? img(coverPath, coverUrl)`; a cover fallback is blurred. Fades into the modal glass from the bottom and left.
- Library card (`app.cards`) fills the banner + title instantly while `getMediaDetail` loads.
- Bottom-left: native title (`.neon-text`, `<Ja block>`), English title (`h2#dm-title`, ≤ 2 lines), then actions:
  - `.btn-cta` pill: `▶ Resume EP 08 · 残り14分` / `▶ Play EP 01` / spinner "Launching…" / equaliser "Now Playing". Next episode = same rule as the full page (partially-watched owned ep → first unwatched owned ep → streaming fallback).
  - If the next episode has no local file (streaming mode) the CTA is a link to `/anime/[id]` ("Stream EP n") — torrents are **not** reimplemented in the modal.
  - Round glass buttons (existing features only): **+ My List** (AniList `PLANNING`, only when AniList is connected and the show has no list status), **Add to library** (streaming mode, nothing owned — `toggleStreamLibrary`), **Open full page** (always).
- Right edge: vertical `.tategaki` native title, 50 % white, decorative.

### Meta + info
- Meta row: `★ 8.4` amber pill · `秋 2023` · `TV`/`劇場版` · `全28話` · studio (`.tn-label`) · status chip `放送中` (green tint) / `完結`.
- Two columns (stack < 720px): synopsis (HTML stripped via `cleanDescription`, 7-line clamp) | `dl` with `.tn-label` terms: Genres (`Action · アクション`), Status, Local files (owned / total), Studio, Subtitles (linked languages), Tracks (custom preference).

### Episodes (エピソード)
- Heading `Episodes` + `.ja-sub エピソード`. Franchise pills (only when `app.groupSeasons` and `franchise.length > 1`): glass segmented control, `第1期 · Season 1`, `劇場版 · Movie`; switching reloads the modal for that entry **without** changing the URL.
- Row (no borders, 14px padding, 4px gap, `content-visibility: auto`): `.tn-label EP 01` + `第1話` · 16:9 still (thumb → banner fallback, greyed when missing) with `.neon-progress` (watched = 100 %, else position / duration) · title + runtime · "Up next"/"Resume" label · 2-line synopsis.
- Next/current row: violet-tinted glass + inset violet hairline + soft glow.
- Owned ep → `<button>` that calls `app.play(anilistId, epKey, files[0].path)`; play glyph (white round) on hover/focus; spinner while launching; equaliser while playing.
- Missing ep: local mode → `aria-disabled` row with "Not in your library" tooltip; streaming mode → link to the full page.
- Specials / extras / files → "on the full page" link.

### Characters (キャラクター)
- Horizontal strip of 64px round avatars, English name + `nameNative` (`lang="ja"`). Max 14. Hidden when the detail has no characters (e.g. mock mode).

### States
- Loading: banner skeleton/card art, `読み込み中…` (decorative) + English "Loading" (`role="status"`), shimmering skeleton lines and 3 skeleton episode rows.
- Error: alert icon, "Couldn't load this anime" + `エラー`, message, glass **Retry** pill.

## Full anime page (`/anime/[id]`)
Style pass only — all script logic and features (AniList tracker, torrents, Jimaku, subtitle matcher, fix match, extras, other files, people modal) are unchanged.
- Banner: shade uses `--bg` mixes (theme-aware), ambient `coverColor` glow bottom-left, decorative `.tategaki` native title on the right; back button sits below the floating navbar (`calc(var(--nav-h) + var(--nav-inset) + 16px)`).
- Header: native title in `.neon-text` (`<Ja block>`), badges carry decorative JP (`劇場版`, `放送中`, `秋`), progress = `.neon-progress`, primary play = `.btn-cta` pill, all action buttons pill-shaped, "More" menu is glass.
- Season tabs: borderless surface tiles, active = violet tint + glow, `第N期` prefix, neon progress bar.
- Section headings: Synopsis あらすじ, Episodes エピソード, Specials 特別編, Characters キャラクター, Staff スタッフ, Related 関連作品 (`.ja-sub`).
- Episodes: rows lose borders; `.tn-label EP 01` + `第1話` above the title; next row = violet glass + glow; neon resume bar; white round play glyph.
- Cards (extras, characters, staff, side cards, AniList tracker, notices) drop grey borders → elevation (`--shadow-sm`) only.
- `.page.body` keeps `padding-top: 32px` so the −250px banner overlap stays stable if `.page` gains nav padding globally.

## Japanese vocabulary used locally (not in `ja.ts`)
| Where | Key | JA | EN |
|---|---|---|---|
| modal | `JA_LOCAL.retry` | 再試行 | Retry |
| modal | `JA_LOCAL.fullPage` | 詳しく見る | Open full page |
| modal (reserved) | `play` / `resume` / `synopsis` / `details` | 再生 / 続きから / あらすじ / 詳細 | |
| page | `JA_SYNOPSIS` | あらすじ | Synopsis |
| page | `JA_SPECIALS` | 特別編 | Specials |
