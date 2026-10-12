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
| **v0.4.0** | **Anime Knowledge & Airing**: Rich Cast, Seiyuu Guides + Seasonal Airing Calendar & Countdown | 💡 Planned |
| **v0.5.0** | **Cloud Sync & Subtitles**: AniList / MAL Two-Way Sync + Jimaku.cc Japanese Subtitle Fetcher | 💡 Planned |
| **v0.6.0** | **Immersion & Analytics**: Yomitan Subtitle HUD + Immersion Hours Tracker & Watch Stats | 💡 Planned |
| **v0.7.0** | **Automation & Social**: Smart RSS Watchlist Downloader (qBittorrent) + SyncPlay Watch Party | 💡 Planned |
| **v0.8.0** | **Media Creativity & Music**: Manga Screenshot & Comic Strip Generator + AnimeThemes OP/ED Jukebox | 💡 Planned |
| **v1.0.0 (Beta)** | **Library Power-User**: Advanced Filtering, Portable Backup/Restore, Performance Polish | 💡 Future |

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

## 🎭 Phase 5: Rich Cast, Voice Actors & Seasonal Airing Calendar
*Target: v0.4.0*

Deepen the anime experience with voice actor guides, character portraits, and an interactive seasonal broadcast calendar.

### Checklist
- [ ] **AniList GraphQL Expansion**
  - [ ] Query `characters(sort: [ROLE, RELEVANCE])` with character name, role (`MAIN`, `SUPPORTING`), image, and voice actors (`voiceActors(language: JAPANESE)`).
  - [ ] Query `staff` for Director, Series Composition, Music Composer, Character Designer, and Original Author.
  - [ ] Cache character and voice actor data locally in SQLite.
- [ ] **Anime Details UI**
  - [ ] Add a **Characters & Cast** tab / drawer on anime detail pages.
  - [ ] Character cards with portrait, character name, Japanese voice actor (seiyuu), and English dub VA toggle.
  - [ ] **Staff Credits Row**: Director, Studio, and Composer badges.
  - [ ] Clickable voice actors / staff showing other anime in your library they worked on.
- [ ] **Seasonal Airing Calendar & Simulcast Countdown**
  - [ ] **Weekly Broadcast Grid (Mon – Sun)**: Interactive weekly calendar showing when currently airing series release in your local timezone.
  - [ ] **Library Highlighting**: Visual indicators distinguishing shows already tracked in your library vs. new seasonal discoveries.
  - [ ] **Episode Countdown Timers**: Live countdown display (*"Episode 7 airs in 3 hours 15 mins"*) powered by AniList's next-airing schedule API.
  - [ ] **Airing Desktop Notifications**: Optional native notification when a tracked show finishes airing or when the folder watcher detects its file drop.

---

## 🔄 Phase 6: Cloud Sync & Japanese Subtitle Auto-Fetcher
*Target: v0.5.0*

Two-way synchronization with AniList/MAL plus automatic Japanese subtitle fetching and audio auto-sync.

### Checklist
- [ ] **OAuth2 Authentication**
  - [ ] AniList OAuth2 login flow via secure local loopback redirect (`http://localhost:port/oauth/callback`).
  - [ ] Secure token storage in SQLite settings.
- [ ] **Two-Way Synchronization Engine**
  - [ ] Import user's list on connect (Watching, Completed, Planning, Paused, Dropped).
  - [ ] Auto-scrobble: when an episode reaches 90% in Kura, update episode progress on AniList.
  - [ ] Safe merge rules: ensure local and remote never accidentally regress progress.
  - [ ] Rate limit compliance (AniList 90 req/min).
- [ ] **Japanese Subtitle Auto-Fetcher & Syncer (Jimaku.cc & Kitsunekko)**
  - [ ] Direct [Jimaku.cc](https://jimaku.cc) API integration: query and match Japanese subtitles by AniList/MAL ID and episode number.
  - [ ] Kitsunekko & OpenSubtitles fallback for older series, movies, and specials.
  - [ ] **One-Click Subtitle Drop**: Preview and save matching `.ass` / `.srt` Japanese subtitles directly alongside media files or into Kura's cache.
  - [ ] **Audio-Based Auto-Sync (`alass` / `ffsubsync`)**: One-click timing alignment that synchronizes subtitle timestamps to the audio waveform automatically.

---

## 🎌 Phase 7: Japanese Immersion HUD & Watch-Time Analytics
*Target: v0.6.0*

A blazing-fast native alternative to Memento paired with offline Japanese immersion hours tracking and analytics.

### Checklist
- [ ] **mpv Real-Time Subtitle Streaming (`sub-text`)**
  - [ ] Subscribe to mpv's `sub-text` property over the existing IPC socket in `player.rs`.
  - [ ] Stream active Japanese subtitle strings, start/end timestamps, and track IDs to Kura in real-time.
  - [ ] Hide mpv's internal text rendering when immersion overlay is active (`--sub-font-size=0`).
- [ ] **Transparent Floating Subtitle HUD Window**
  - [ ] Lightweight, borderless, transparent Tauri window snapped over the bottom of mpv (windowed or fullscreen).
  - [ ] Crisp manga-styled Japanese subtitle rendering with customizable font size, outline, and position.
  - [ ] Mouse-interactive words: automatically becomes hoverable when video is paused or mouse enters the HUD.
- [ ] **Yomitan Dictionary Engine (Rust + SQLite)**
  - [ ] **Dictionary Importer**: Drag-and-drop standard Yomitan `.zip` archives (JMdict, KANJIDIC, Daijirin, etc.) in **Settings → Japanese Immersion**.
  - [ ] **Fast SQLite Index**: High-speed term lookup table indexing headwords, readings, definitions, and pitch accents (< 1ms query time).
  - [ ] **Deinflection Engine**: Break down conjugated verbs and adjectives (e.g. `食べられなかった` → `食べる`).
- [ ] **Manga-Styled Yomitan Popover Card**
  - [ ] Displays word reading (hiragana/katakana furigana), pitch accent markers, and concise definitions.
  - [ ] Lightweight, pure instant lookup — zero mining bloat, no Anki sync baggage, instant response.
- [ ] **Immersion Hours & Analytics ("Kura Wrapped")**
  - [ ] **Japanese Immersion Hours Counter**: Track total hours watched specifically with Japanese audio + Japanese subtitles.
  - [ ] **Watch Time Statistics**: Total days, hours, and episodes watched across your entire library.
  - [ ] **GitHub-Style Yearly Activity Heatmap**: Visualize daily watch streaks, activity frequency, and consistency.
  - [ ] **Studio & Genre Breakdown**: Comic-styled breakdown charts of top studios (*ufotable, Kyoto Animation, MAPPA*) and genres.
  - [ ] **Exportable "Manga Stats Card"**: Generate a stylish shareable graphic summarizing your monthly or yearly anime stats.

---

## 🚀 Phase 8: Smart RSS Watchlist Downloader & SyncPlay
*Target: v0.7.0*

End-to-end automated episode acquisition combined with synchronized watch party rooms.

### Checklist
- [ ] **Smart RSS Watchlist Downloader (qBittorrent / Transmission / Deluge)**
  - [ ] **Auto-Watch Tag**: Mark airing shows to automatically fetch new episodes the moment they are released.
  - [ ] **RSS Feed Monitor**: Monitor anime RSS feeds (Nyaa / Tokyo Toshokan) for preferred fansub release groups (`[SubsPlease]`, `[Erai-raws]`, `[Judas]`, etc.).
  - [ ] **Torrent Client WebUI API**: Send magnet links / `.torrent` files directly to local client (qBittorrent, Transmission, or Deluge) via API.
  - [ ] **Zero-Touch Pipeline**: RSS auto-downloads episode $\to$ Folder Watcher detects completion $\to$ AniList matches $\to$ Discord RPC tracks $\to$ Instant playback.
- [ ] **SyncPlay / Watch Together**
  - [ ] **Built-in SyncPlay Room Launcher**: Start or join a synchronized MPV playback room directly from Kura with custom nickname and room name.
  - [ ] **Lockstep Sync**: Synchronized playback, pause, and seek locked in frame-accurate sync with friends across the internet.
  - [ ] **In-App Room Controls**: Nickname, room name, server selection, and participant status badge.

---

## 🎨 Phase 9: Manga Panel Screenshot Maker & AnimeThemes Jukebox
*Target: v0.8.0*

Creative media capture and a dedicated anime music jukebox.

### Checklist
- [ ] **Manga Panel Screenshot & Comic Strip Generator**
  - [ ] **Clean Subtitle-Free Frame Grab**: Press a shortcut in MPV to save clean lossless 1080p/4K PNG screenshots without on-screen clutter via MPV IPC (`screenshot-to-file`).
  - [ ] **Manga Comic Strip Creator**: Select 2 to 4 frame grabs from an episode and lay them out into a comic strip with authentic halftone screentone patterns, black gutters, and dialogue speech bubbles.
  - [ ] **Clip & GIF Exporter**: Trim 3 to 10-second video moments right from the episode details drawer into lightweight animated WebP / GIF stickers.
- [ ] **Clean OP/ED Jukebox & Anime Radio (AnimeThemes.moe)**
  - [ ] **Theme Songs Drawer**: Browse and stream clean creditless NCOP and NCED videos powered by the [AnimeThemes.moe](https://animethemes.moe) API.
  - [ ] **Top-Bar Mini OST Player / Radio**: Subtle background audio player shuffling theme songs from all anime in your collection while browsing.

---

## ⚡ Phase 10: Library Power-User Tools & Portable Backup
*Target: v1.0.0 (Beta)*

- [ ] **Advanced Filtering & Library Search**
  - [ ] Filter library by genres, tags, studios, format (TV, Movie, OVA, Special), airing status, and voice actors.
  - [ ] Quick filter pills on the library home page.
  - [ ] Multi-criteria sorting (Score, Date Added, Release Year, Title, Progress).
- [ ] **Library Export & Portable Backup**
  - [ ] One-click export of watch history, manual match overrides, and track preferences to a single `.json` file.
  - [ ] One-click restore option in Settings.
