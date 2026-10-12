# Kura Master Roadmap & To-Do List

This document serves as the master planning roadmap for Kura, organizing proposed features into prioritized milestones, technical specifications, and implementation checklists.

---

## 📍 Milestone Overview

| Milestone | Target Focus | Status |
| :--- | :--- | :--- |
| **v0.3.2** | **Visual & Artwork Overhaul**: Manga Mode (Light Theme) + Priority Artwork Engine (ClearLogos & Fanart) | ✅ Completed |
| **v0.3.3** | **Social & Real-Time Tracking**: Discord Rich Presence (RPC) + Live Folder Watcher | ✅ Completed |
| **v0.3.4** | **Playback & Immersion**: Interactive MPV Skip OP/ED Overlay, AniSkip, Changelog Popup & Settings Persistence | ✅ Completed |
| **v0.3.5** | **Bug Fix**: Reliable MPV detection after winget install (registry, WinGet folders, fresh PATH) | ✅ Completed |
| **v0.3.6** | **Bug Fix**: Skip button no longer triggers on prologues (length-matched AniSkip, stricter chapter detection) + redesigned skip button & styled MPV messages | ✅ Completed |
| **v0.4.0** | **Foundations**: Settings Overhaul (sidebar + search), Full Backup & Restore, Diagnostics, CI Checks, Advanced Filtering | 💡 Planned |
| **v0.5.0** | **Sync & Airing**: AniList Two-Way Sync + Seasonal Airing Calendar & Countdown | 💡 Planned |
| **v0.6.0** | **Subtitles & Cast**: Jimaku.cc Japanese Subtitle Fetcher + Smart Subtitle Matcher + Characters, Seiyuu & Staff | 💡 Planned |
| **v0.7.0** | **Watch Analytics**: Watch-Time & Immersion Hours Tracker + Yearly Activity Heatmap | 💡 Planned |
| **v0.8.0** | **Social & Music**: Syncplay Auto-Install & Launcher + AnimeThemes OP/ED Drawer + Linux/macOS Polish | 💡 Planned |
| **v1.0.0 (Beta)** | **Polish & Trust**: Performance, Windows Code Signing, Japanese UI Language | 💡 Future |

---

## 🎨 Phase 1: Manga Mode (Light Theme — Ink & Paper)
*Target: v0.3.2* — **Completed**

Transform Kura's visual identity from the current plum/coral "Comicbook Dark" mode into an authentic Japanese printed manga aesthetic inspired by *Tankōbon* volumes and Shōnen Jump.

### Checklist
- [x] **Design Tokens & Theme Engine (`[data-theme="manga"]`)**
  - [x] **Backgrounds**: Warm off-white newsprint / manga paper (`#f5f2eb` page background, `#ffffff` card panels).
  - [x] **Sumi Ink Outlines**: Deep black sumi line art (`#12100e`) with crisp 2px solid sticker drop shadows (`3px 3px 0 #12100e`).
  - [x] **Option A Accent — Vermilion Red Stamp**: Authentic Japanese red seal (`#e53935` / `#eb4d3d`) for play buttons, active season tabs, and key highlight badges.
  - [x] **Screentone Midtones**: Subtle wash surfaces (`#ece8df`, `#ded8cb`) for secondary cards, inputs, and tab bars.
  - [x] **Monochrome Icons & Typography**: High-contrast dark ink text with native `color-scheme: light` support.
- [x] **Settings & Boot Persistence**
  - [x] Add `pref.theme` setting in SQLite `settings` table (`"comic"` vs `"manga"`).
  - [x] Add Theme / Appearance radio card picker under **Settings → Display**.
  - [x] Initialize theme class on `document.documentElement` during early boot to prevent any Flash of Unstyled Content (FOUC).

---

## 🖼️ Phase 2: Priority Artwork Engine (`Local > Fanart > AniList`)
*Target: v0.3.2* — **Completed**

Eliminate missing or blurred backdrops and introduce transparent anime title logos (ClearLogos) across the app using a strict priority pipeline.

### Priority Hierarchy
$$\mathbf{1.\ Local\ Folder\ Assets} \;\longrightarrow\; \mathbf{2.\ Fanart.tv\ /\ ani.zip\ Fanart} \;\longrightarrow\; \mathbf{3.\ AniList\ Default}$$

### Checklist
- [x] **Local Asset Scanner (`scan.rs`)**
  - [x] Scan anime folder for local backdrops: `fanart.jpg`, `fanart.png`, `backdrop.jpg`, `backdrop.png`, `background.jpg`, `art.jpg`.
  - [x] Scan anime folder for local transparent logos: `clearlogo.png`, `logo.png`, `clearart.png`.
  - [x] Scan anime folder for local posters: `poster.jpg`, `cover.jpg`, `folder.jpg`.
  - [x] Use local file paths directly when present, bypassing remote downloads.
- [x] **Zero-Config Built-in Fanart & ClearLogo Integration**
  - [x] Update `anizip_episodes` parser to extract the `images` array from `api.ani.zip/mappings`:
    - `coverType: "Fanart"` $\to$ 1080p TheTVDB/Fanart widescreen background.
    - `coverType: "Clearlogo"` $\to$ Transparent PNG title logo.
    - `coverType: "Banner"` $\to$ Graphical banner.
  - [x] Update SQLite schema: add `logo_url` and `logo_path` columns to `media` table.
  - [x] Download and cache ClearLogos locally in `<app_data>/cache/images/logos/` for 100% offline usage.
- [x] **Fanart.tv API Key Support (Settings)**
  - [x] Add `fanart_api_key` preference in **Settings → Artwork & Metadata**.
  - [x] When a key is configured, query `https://webservice.fanart.tv/v3/tv/{thetvdb_id}?api_key=...` (or `/movies/{themoviedb_id}`) using mappings from ani.zip.
  - [x] Fetch community-curated HD ClearLogos (`hdtvlogo`), high-res posters, and backgrounds (`showbackground`).
- [x] **Cinematic Header with Floating ClearLogos (`anime/[id]/+page.svelte`)**
  - [x] Display transparent PNG title logo floating over the widescreen Fanart backdrop with drop shadow (`filter: drop-shadow(0 4px 14px rgba(0,0,0,0.7))`).
  - [x] Subtitle underneath with Japanese kanji/kana and romaji titles.
  - [x] Seamless fallback to styled typographic title if no ClearLogo exists for that title.
- [x] **Artwork & Metadata Rebuild Utility (`Settings → Artwork & Metadata`)**
  - [x] One-click button to purge cached metadata, episode guides, and artwork while strictly preserving all watch states (`watch_state`), playback resume positions (`watch_progress`), track preferences (`media_track_prefs`), and libraries.
  - [x] Automatically triggers background scan to fetch ani.zip / Fanart.tv ClearLogos and 1080p Fanarts for all owned anime.

---

## 🎮 Phase 3: Discord Rich Presence (RPC)
*Target: v0.3.3* — **Completed**

Show off what you are currently watching to friends on Discord with live episode progress and Activity Type 3 ("Watching").

### Checklist
- [x] **Discord IPC Client in Rust (`src-tauri/src/discord.rs`)**
  - [x] Native async Tokio client connecting to Discord's local socket (`\\.\pipe\discord-ipc-0..9` on Windows, `$XDG_RUNTIME_DIR/discord-ipc-0` / Flatpak paths on Linux, and `$TMPDIR` on macOS).
  - [x] Configured default Kura Discord Application ID (`1558103009396002816`).
  - [x] Emits Activity Type `3` so Discord displays **`WATCHING Kura`**.
  - [x] Resilient background worker: auto-reconnects smoothly if Discord is opened after Kura or restarted.
- [x] **Real-Time Playback Synchronization (`player.rs`)**
  - [x] Hook into `player.rs` event loop to update Discord presence on play, pause, seek, and finish.
  - [x] **Activity State**: Display anime title, episode number, and episode title.
  - [x] **Live Progress Bar**: Interactive countdown timestamps (`start` and `end`) showing remaining/elapsed time.
  - [x] **Artwork**: High-res anime poster fetched via AniList CDN URL with Kura logo badge.
  - [x] **Action Buttons**: "View on AniList" (series page) and "Get Kura on GitHub" (repository download).
  - [x] Clean exit: clears Discord presence immediately when player closes or stops.
- [x] **Privacy & Settings (`Settings → Discord Rich Presence`)**
  - [x] Toggle: *Show activity on Discord* (On/Off).
  - [x] Toggle: *Spoiler Protection* (hides episode title, showing only episode number).
  - [x] Toggle: *Interactive profile buttons* (toggle AniList and GitHub buttons).

---

## 📂 Phase 4: Folder Watcher & Background Rescan
*Target: v0.3.3* — **Completed**

Automatically detect new anime episodes and movies the moment they finish downloading.

### Checklist
- [x] **Filesystem Watcher (`notify` crate)**
  - [x] Watch all configured library folder roots recursively.
  - [x] Filter for media file extensions (`.mkv`, `.mp4`, `.avi`, `.webm`).
  - [x] Debounce events (wait 3-5 seconds after file lock releases to ensure download/copy is 100% finished).
- [x] **Targeted Background Ingestion**
  - [x] Ingest only the newly detected file instead of rescanning the entire library.
  - [x] Parse filename, match to existing series in SQLite, and download episode thumbnail.
  - [x] Emit Tauri event `library-changed` to automatically update the frontend without reloading.

---

## 🎬 MPV Integration & Smart Auto-Installer
*Target: v0.3.4* — **Completed**

- [x] **Smart First-Launch MPV Detection & Auto-Installer**
  - [x] On first launch, detect whether mpv is installed.
  - [x] If missing, present a comic/manga styled modal explaining Kura's 10-bit playback, smart OP/ED skip, and immersion features.
  - [x] One-click automated installation via `winget` (`shinchiro.mpv`) or Chocolatey (`mpvio`), with automatic fallback to the official `mpv.io` installation website.
  - [x] Live installation progress spinner and automatic re-detection and configuration of `pref.player = "mpv"`.
  - [x] Dedicated MPV recommendation banner in **Settings → Playback**.
- [x] **Interactive Clickable Skip Button & AniSkip Integration**
  - [x] On-screen interactive `[ ⏭ SKIP OPENING (Click or Tab) ]` button rendered directly over MPV with mouse collision tracking and click-to-skip.
  - [x] Frame-accurate AniSkip API integration with smart ~90s duration check ignoring cold opens, avant, and prologues.
- [x] **Changelog Popup & Version Update Notice**
  - [x] Automatic "What's New in Kura" modal triggered when launching a newly installed version.
  - [x] Version history tabs with categorized badges (New, Improved, Fixed) and GitHub releases link.
  - [x] Manual "What's New" button in **Settings → Updates** to view release notes anytime.
- [x] **Settings & Library Persistence Across Upgrades**
  - [x] Fixed WiX bundle `upgradeCode` GUID to preserve existing installations on Windows Installer.
  - [x] Persistent JSON backup (`kura_backup.json`) in AppData and user profile that auto-restores libraries and preferences if database is ever recreated or refreshed.
  - [x] Seeded default preferences to eliminate missing keys on startup.
  - [x] Migrated `mpv_prompt_dismissed` and `theme` from fragile webview LocalStorage into persistent SQLite `settings`.

---

## 🧱 Phase 5: Foundations — Settings, Backup, Diagnostics & Filtering
*Target: v0.4.0*

Groundwork that makes every later feature easier to ship, support and debug.

### Checklist
- [ ] **Settings Overhaul**
  - [ ] Replace the single long page with a **left sidebar of categories**: General, Library, Playback, Subtitles, Artwork & Metadata, Integrations (Discord / AniList / Jimaku), Backup, Updates, About & Diagnostics.
  - [ ] **Settings search bar** that filters every option by label/keyword and jumps to it.
  - [ ] Each category is its own component so new options don't bloat one file.
  - [ ] Remember the last opened category.
- [ ] **Full Backup & Restore** (extends the existing `kura_backup.json`)
  - [ ] Include **watch history** (`watch_state`), resume positions (`watch_progress`), manual match overrides and track preferences (`media_track_prefs`) — not just settings and libraries.
  - [ ] Debounced auto-backup after watch progress changes (not on every tick).
  - [ ] One-click **Export** to a chosen `.json` file and **Import** with a confirmation preview ("312 episodes, 4 libraries").
  - [ ] Versioned backup format so older backups still import after schema changes.
- [ ] **Diagnostics**
  - [ ] Rolling log file in the app data dir (`kura.log`, size-capped).
  - [ ] **"Copy diagnostics"** button: app version, OS, detected players + paths, library count, last errors — paste-ready for GitHub issues.
  - [ ] **Player detection list**: show every mpv / VLC / Syncplay path Kura found and where it found it (registry, PATH, WinGet, etc.), plus a "Re-detect" button.
- [ ] **CI Checks**
  - [ ] GitHub Actions workflow running `cargo check` + `cargo test` on Windows, Ubuntu and macOS, plus `npm run check`, on every push/PR — so Linux/macOS breakages are caught before a release tag.
- [ ] **Advanced Filtering & Library Search**
  - [ ] Filter by genres, tags, studios, format (TV, Movie, OVA, Special), airing status and watch status.
  - [ ] Quick filter pills on the library home page.
  - [ ] Multi-criteria sorting (Score, Date Added, Release Year, Title, Progress).

---

## 🔄 Phase 6: AniList Two-Way Sync & Seasonal Airing Calendar
*Target: v0.5.0*

### Checklist
- [ ] **AniList Login**
  - [ ] AniList OAuth2 (implicit grant) via local loopback redirect (`http://localhost:port/oauth/callback`).
  - [ ] Token stored in SQLite settings; "Disconnect" button in **Settings → Integrations**.
- [ ] **Two-Way Synchronization Engine**
  - [ ] Import the user's list on connect (Watching, Completed, Planning, Paused, Dropped).
  - [ ] Auto-scrobble: when an episode reaches 90% in Kura, update progress on AniList.
  - [ ] Safe merge rules: never regress progress on either side.
  - [ ] Offline queue: scrobbles made offline are sent when back online.
  - [ ] Rate limit compliance (AniList 90 req/min).
- [ ] **Seasonal Airing Calendar & Countdown**
  - [ ] **Weekly Broadcast Grid (Mon – Sun)** in your local timezone.
  - [ ] **Library Highlighting**: shows already in your library vs. new seasonal discoveries.
  - [ ] **Episode Countdown**: *"Episode 7 airs in 3h 15m"* via AniList's `nextAiringEpisode`.
  - [ ] **Desktop Notifications** (optional): when a tracked show airs or the folder watcher picks up its file.

---

## 💬 Phase 7: Jimaku Subtitle Fetcher, Subtitle Matcher & Characters / Staff
*Target: v0.6.0*

### Feasibility notes (checked against the Jimaku API)
- Jimaku has an official REST API: `GET /api/entries/search?anilist_id=…` → `GET /api/entries/{id}/files?episode=N`. Kura already stores AniList IDs, so matching is direct — no fuzzy title search needed.
- **Requires a free Jimaku account + API key** (sent in the `Authorization` header) → add a key field in **Settings → Integrations**, same pattern as the Fanart.tv key.
- Rate limited per IP (~25 requests per window, `429` + `x-ratelimit-reset-after`) → cache results per series and back off.
- Episode filtering is best-effort (filename based) and some uploads are `.zip` / `.rar` packs → always show a picker; extract `.zip` in Rust, skip `.rar` with a "download manually" link.
- Should set a descriptive `User-Agent` (`Kura/x.y.z`).

### Checklist
- [ ] **Jimaku Subtitle Fetcher**
  - [ ] "Find Japanese subtitles" button on the episode row and in the player menu.
  - [ ] Picker listing matching files (name, size, date), best match first.
  - [ ] Save next to the video (`<video>.ja.ass`) or into Kura's cache, then load into a running mpv via IPC `sub-add`.
  - [ ] Optional **auto-fetch** for series marked "Japanese subs" when a new episode is detected.
  - [ ] Timing fixes use the existing `z` / `x` delay shortcuts; audio-based auto-sync (`alass`) is a stretch goal only if users ask for it.
- [ ] **Smart Subtitle Matcher** (Rust port of the Jellyfin Anime Subtitle Renamer v3 script)
  - [ ] **Why**: mpv only auto-loads subtitles whose name starts with the video's name, so files like `薬屋のひとりごと.S02E25.猫猫と毛毛.WEBRip.Netflix.ja[cc].srt` are ignored.
  - [ ] **Episode number styles** (mixed freely in one folder): `S01E01`, absolute `S02E25` (= S02E01), bare `25.srt` / `25.ja.srt`, `#08`, `E08`, `Show - 01` / `Show – 12 END`.
  - [ ] **Uses Kura's own data**: match against each episode's known season/episode from the library (no `SxxExx` video names required), and resolve absolute numbers automatically from the stored `absolute_number` (ani.zip) instead of asking for "first absolute episode".
  - [ ] **Language detection**: `ja` / `jpn` / `jp` / `japanese` / `日本語`, `en` / `eng`, `zh` / `chs` / `cht` / `sc` / `tc`, `ko`, `de`, `fr`, `es`; preserve `[cc]` / SDH as `.cc`; fallback language dropdown ("apply to all undetected").
  - [ ] **Preview dialog**: every match (`source → target`), plus a skipped list with the reason (no episode number, ambiguous, absolute number before season start, target exists).
  - [ ] **Two modes**: *Link without renaming* (default, stored in SQLite, passed to mpv as `--sub-files` at launch) or *Rename for Jellyfin/Plex* (`<video>.ja.cc.srt`). Never overwrite, never touch video files.
  - [ ] Entry points: **"Match subtitles"** button on the series page, and used automatically to split **Jimaku season packs** into per-episode subtitles.
  - [ ] Port fixes: dash pattern must not read `- 1080p` as episode 1080 (`(?![\dp])`); `cc` detection limited to tag positions (`[cc]`, `.cc.`, `SDH`) instead of any word "cc".
  - [ ] Unit tests using the script's examples (Apothecary Diaries absolute `S02E25`, Azumanga Daioh `#08`, bare `25.ja.srt`, mixed folders).
- [ ] **Characters, Seiyuu & Staff**
  - [ ] AniList `characters(sort: [ROLE, RELEVANCE])` with Japanese voice actors, plus key `staff` (Director, Series Composition, Music, Character Design, Original Creator).
  - [ ] Cached locally in SQLite for offline viewing.
  - [ ] **Characters & Cast** tab on anime detail pages; staff credit badges.
  - [ ] Click a voice actor / staff member to see other anime **in your library** they worked on.

---

## 📊 Phase 8: Watch-Time & Immersion Analytics
*Target: v0.7.0*

### Checklist
- [ ] **Viewing Sessions Table**: new SQLite table recording start/end, episode, audio language and subtitle language for each play session (needed for accurate stats).
- [ ] **Watch Time Statistics**: total hours, days and episodes watched.
- [ ] **Japanese Immersion Hours**: time watched with Japanese audio, split by Japanese subs / no subs / other subs.
- [ ] **GitHub-Style Yearly Activity Heatmap**: daily watch streaks and consistency.
- [ ] **Studio & Genre Breakdown**: comic-styled charts of top studios and genres.

---

## 🎉 Phase 9: Syncplay, AnimeThemes & Cross-Platform Polish
*Target: v0.8.0*

### Checklist
- [ ] **Syncplay Integration (lightweight)**
  - [ ] Detect an existing Syncplay install (same approach as mpv detection: registry, PATH, WinGet).
  - [ ] **One-click install** in **Settings → Playback**, like mpv: `winget install --id Syncplay.Syncplay` on Windows, package-manager hints on Linux/macOS.
  - [ ] Save server, room and nickname in settings.
  - [ ] **"Watch with friends"** button on episodes that launches Syncplay with the file, mpv path, room and nickname pre-filled (Syncplay handles all the actual syncing).
- [ ] **AnimeThemes OP/ED Drawer**
  - [ ] Browse and stream creditless NCOP / NCED videos for each series via the [AnimeThemes.moe](https://animethemes.moe) API.
- [ ] **Linux / macOS Polish**
  - [ ] mpv detection for Flatpak (`io.mpv.Mpv`), Snap and Homebrew paths.
  - [ ] Distro-specific install hints in the mpv / Syncplay prompts (apt, dnf, pacman, brew).

---

## ⚡ Phase 10: Polish & Trust
*Target: v1.0.0 (Beta)*

- [ ] **Performance**: virtualized library grid for large collections, image cache cleanup, faster startup.
- [ ] **Windows Code Signing**: remove SmartScreen "unknown publisher" warnings.
- [ ] **Japanese UI Language**: optional Japanese interface (i18n groundwork for other languages).
- [ ] **Embedded Player (research only)**: investigate embedding libmpv inside a Kura window. Note: a plain webview `<video>` player is *not* viable — no reliable MKV, HEVC 10-bit or ASS subtitle support — so this would have to be libmpv-based.

---

## 🗑️ Cut / Not Planned

| Idea | Why it was cut |
| :--- | :--- |
| **Yomitan Subtitle HUD & built-in dictionary** | Main motivation was Memento's slow startup, now much less painful thanks to the player-launching indicator. Huge effort to rebuild what Yomitan/Memento already do well. |
| **Exportable "Manga Stats Card"** | Not needed. |
| **Smart RSS / torrent auto-downloader** | Out of scope for a library app, legal grey area, and Sonarr/qBittorrent RSS already do it better. |
| **Built-in SyncPlay rooms** | Replaced by lightweight auto-install + launch of the real Syncplay app (Phase 9). |
| **Comic strip / GIF / clip maker** | Fun but niche; mpv already takes screenshots. |
| **Top-bar anime radio** | Background audio inside a library app adds little; the OP/ED drawer covers the useful part. |
| **MAL sync** | AniList first; MAL only if users ask for it. |
