# Kura Tokyo Night — MASTER Design System (蔵 · 東京ナイト)

> Global source of truth. Page-specific overrides live in `pages/<page>.md` and win over this file.
> Built with the ui-ux-pro-max skill (style: Dark/OLED + Glassmorphism, typography: Inter, JP: Noto Sans JP).

## Concept
A late-night Shinjuku cinema for a local anime library. Indigo-black canvas, frosted glass, a violet→pink neon used **sparingly**, and a Japanese accent layer sprinkled through an **English** UI. Netflix-like structure: floating navbar → billboard hero → horizontal rows → hover cards → details modal.

## Tokens (defined in `src/lib/styles/global.css`)
| Token | Value | Rule |
|---|---|---|
| `--bg` / `--bg-deep` | `#0B0D14` / `#080910` | Page background. Never flat gray. |
| `--surface` / `--surface-2` / `--surface-3` | `#12141F` / `#181B29` / `#222537` | Cards, menus, raised items |
| `--glass-tn` (+ `--blur-tn`, `--glass-hairline`) | `rgba(18,20,31,.78)` + blur(12px) | **Only** navbar, dropdowns, and small overlays — never on repeated card panels or full-screen modal containers. |
| `--text` / `--text-2` / `--text-3` | `#F4F2FF` / `#9A9CB8` / `#5C5F7A` | `--text-3` is ~3:1 → large/decorative text only |
| `--neon` | `linear-gradient(90deg,#8B5CF6,#F472B6)` | Progress bars, active nav underline, focus glow, gradient native titles (`.neon-text`). Non-text. |
| `--neon-violet` / `--neon-pink` | `#8B5CF6` / `#F472B6` | Solid accents, rings, dots |
| `--neon-glow` / `--neon-glow-strong` | violet box-shadow glows | Only on hovered/focused interactive elements |
| `--cta` / `--cta-hover` / `--on-cta` | `#7C3AED→#BE185D` / white | **Primary Play button fill** (`.btn-cta`). Darkened so white text ≥ 4.5:1. |
| `--primary` / `--on-primary` | `#F4F2FF` / `#0B0D14` | White round in-card play buttons |
| Radius | 12px cards (`--r-md`), 14px nav, 16px modal (`--r-xl`), pills (`--r-pill`) | |
| Fonts | `--font` Inter · `--font-jp` Noto Sans JP · `--font-label` Space Grotesk | `.tn-label` = 11px uppercase letter-spaced label ("EP 07 · 12M LEFT") |
| Motion | `--t-med` 250ms, `--ease`, `--hover-intent` 300ms | Lift −6px, scale 1.06, tilt ≤ 4°. All disabled under `prefers-reduced-motion`. |
| Layout | `--nav-h` 60px, `--nav-inset` 16px, `--gutter` 4vw | Rows/hero content use `--gutter` side padding |

## Hard rules
1. **No grey borders around cards.** Separation = elevation, shadow, spacing. A `--glass-hairline` 1px edge is allowed only on glass panels.
2. Neon/gradient is for: progress, primary Play, focus, active nav, native-title gradient text. Never for body text, headings, or decoration.
3. Every icon-only button has an English `aria-label`. No emojis as icons — use `Icon.svelte`.
4. Hover-revealed actions must also be reachable by click/keyboard (cards are links/buttons; click opens the details modal).
5. `prefers-reduced-motion`: no tilt/lift/parallax/auto-rotation.
6. `backdrop-filter` only on navbar, dropdowns, and small floating pills — never on repeated card panels or large scrollable modal bodies (Tauri's transparent WebView2 window over `libmpv` makes large/repeated blur passes costly).
7. Lazy-load images below the fold (`loading="lazy"`, `decoding="async"`).
8. Keep ALL existing logic: `api.*` calls, `app.*` actions (e.g. `app.play`), Tauri IPC, player and scanning code are untouched. Redesign = markup + styles + presentational components.
9. Tokyo Night is Kura's single canonical dark theme — always use semantic CSS tokens rather than hard-coded hex values.

## Japanese accent layer (日本語アクセント)
- Vocabulary + helpers: `src/lib/ja.ts` (`JA`, `jaWeekday`, `jaWeekdayLong`, `jaDate`, `jaTime`, `jaEpisode`, `jaEpisodeCount`, `jaRemaining`, `jaCour`, `jaSeason`, `jaFormat`, `jaStatus`, `jaWatchStatus`, `jaGenre`, `jaStreak`, `jaDuration`).
- Component: `src/lib/components/Ja.svelte`
  - `<Ja text="新着" />` → decorative duplicate (aria-hidden, hidden when accents off)
  - `<Ja text="月" label="Monday" essential />` → Japanese replaces English; English kept for screen readers and shown instead when accents are off
- Toggle: `app.japaneseAccents` / `app.setJapaneseAccents(on)`; `html.ja-off` hides `.ja-deco`.
- CSS helpers: `.ja-sub` (small JP subtitle after English heading), `.kanji-watermark` (huge faint page kanji, aria-hidden), `.neon-text`, `.tn-label`.
- Native titles: every card type has `titleNative` — show it above/below English titles on hero, hover cards, modal.
- Placeholders in Japanese (`JA.placeholders.*`) MUST be paired with an English `aria-label`.
- The app language stays English: Japanese never replaces a functional English label unless its meaning is obvious and the English is kept accessibly.

## Components (target)
- **Floating glass navbar**: fixed, inset 16px, 60px tall, 14px radius, more opaque when `#main-scroll` scrolls > 8px. 蔵 mark + KURA, links Home / Series / Movies / My List with neon underline on active, search pill (`作品を検索…  ⌘K`), bell, avatar menu (Discover 発見, Calendar 放送, Immersion 没入, Review 復習, Settings 設定), torrent/now-playing chips.
- **Hero**: 80vh, key art with vignette + bottom/left fade into `--bg`, ambient glow from `coverColor`, native title in `.neon-text`, vertical `.tategaki` strip, ★ score pill, `秋 2023 · TV · 全28話`, 1-line synopsis, `.btn-cta` ▶ Play, glass More Info, spotlight rail.
- **Rows**: English title + `.ja-sub`, hidden scrollbars, glass round chevron buttons, 12px gaps.
- **Poster tile (2:3)** / **Still tile (16:9)**: 12px radius, hover lift/tilt/glow in `coverColor`, glass info panel (▶ / + My List / details, ★ score, JP genre chips), neon progress.
- **Details modal**: 880px glass, scrim + blur, `?anime=<id>`, banner + native title, `.btn-cta` Resume, `第1期` season pills, episode list with `第N話`, neon progress; "Open full page" → `/anime/[id]`.
