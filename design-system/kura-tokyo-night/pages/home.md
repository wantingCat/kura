# Home page overrides (ホーム)

> Overrides `../MASTER.md` for `src/routes/+page.svelte` and its components.

## Structure
1. **Billboard hero** (`HomePanels.svelte`) — `80vh` (min 520px), full-bleed; the floating navbar overlays it (no top padding).
   - Key art = banner, else cover. Vignette fades into `--bg` at bottom + left (uses `color-mix(var(--bg) …)` so the manga theme still works).
   - Ambient radial glow from the featured show's `coverColor` (`--glow`, fallback `--neon-violet`).
   - Content bottom-left: native title (`.neon-text`, decorative `<Ja>`), logo image or big English title (links to `/anime/[id]`),
     meta row (★ score pill `averageScore/10`, `jaSeason`, `jaFormat`, `jaEpisodeCount` — each `<Ja essential label=English>`, 2 English genre tags),
     episode line (if an episode is queued), 1-line synopsis, `.btn-cta` Play/Resume + glass "More Info" (`detailsModal.open`).
   - Decorative `.tategaki` strip of the native title (violet 50%, hidden < 1100px).
   - "JP SUBS" tag is **omitted** — no subtitle-language data on `MediaCard`.
   - Spotlight rail (right, ≤ 5 posters, "Spotlight · 注目"): same candidates as before (continue watching → new episodes → recently added with banners → recently added). Auto-rotates every 9s **only** when the hero is not hovered, has no focus inside, and `prefers-reduced-motion` is off. Hidden < 720px.
2. **Rows** (`Rail.svelte` + tiles), in order:
   Continue Watching · 続きから (StillTile) → New Episodes · 新エピソード (PosterTile + EP badge) →
   Top 10 in Your Library · ランキング → Recently Added · 新着 → up to 4 genre rows.
3. **Your Library · ライブラリ** grid with every existing library control (library chips, review banner, watch tabs + JP sub, format chips, filters panel, sort).

## Decisions
- **Top 10 ranking basis:** AniList `averageScore` (desc), ties broken by `watchedCount`. Only shown when the (grouped) library has ≥ 6 shows; shows without a score are excluded.
- **Recently Added** shown when the library has ≥ 6 shows (up to 16, newest first).
- **Genre rows:** most common genres in the grouped library with a `jaGenre` translation and ≥ 4 shows; Hentai/Ecchi excluded from home rows; each row sorted by score, max 20.
- **"+ My List" button omitted:** local "Planning" is derived from watch progress (`watchBucket`), and the only status setter is `api.anilistUpdateEntry` (requires a connected AniList account, not reflected locally until the next sync). Revisit when a local list API exists.
- **Tiles:** hover intent = `--hover-intent`, lift −6px, scale 1.06, tilt ≤ 4° (`hoverTile` action exported from `PosterTile.svelte`, reused by `StillTile`), glow in `coverColor`. `:focus-within` shows the glass panel for keyboard users. Reduced motion → no transform.
- Edge tiles: rail track has 18px/26px vertical padding and `--origin: left/right` on first/last child.
- Clicking any tile/card opens the details modal; the grid card title is still a link to `/anime/[id]`.

## URL modes (navbar links)
| URL | Grid filter | Header (watermark) | Hero/rows |
|---|---|---|---|
| `/?format=series` | TV, TV_SHORT, ONA, OVA, SPECIAL | Series (シリーズ) | hidden |
| `/?format=movie` | MOVIE | Movies (劇場版) | hidden |
| `/?watch=planning` | selects Planning tab (once per URL change) | My List (マイリスト) | hidden |

Pages without the hero add `calc(var(--nav-h) + var(--nav-inset) + 28px)` top padding so content clears the floating navbar.
