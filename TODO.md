# Kura Master Roadmap & To-Do List

This document serves as the master planning roadmap for Kura, organizing proposed features into prioritized milestones, technical specifications, and implementation checklists.

---

## 📍 Milestone Overview

| Milestone | Target Focus | Status |
| :--- | :--- | :--- |
| **v0.3.2** | **Visual & Artwork Overhaul**: Manga Mode (Light Theme) + Priority Artwork Engine (ClearLogos & Fanart) | ✅ Completed |
| **v0.3.3** | **Social & Real-Time Tracking**: Discord Rich Presence (RPC) + Live Folder Watcher | ✅ Completed |
| **v0.4.0** | **Embedded Player Engine & Smart Skip**: In-App libmpv Player, Smart OP/ED Skip + Rich Cast & Voice Actors | 💡 Planned |
| **v0.5.0** | **Cloud & Account Integration**: AniList & MyAnimeList Two-Way Scrobbling / List Sync | 💡 Planned |
| **v0.6.0** | **Japanese Immersion**: Native In-Player Subtitle HUD & Yomitan Dictionary Integration (Direct in Embedded Player) | 💡 Planned |
| **v1.0.0 (Beta)** | **Library Power-User**: Advanced Filtering, Library Backup/Export, Performance Polish | 💡 Future |

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

## 🎬 Phase 5: Built-in Video Player Engine & Smart OP/ED Skip
*Target: v0.4.0*

Embed a hardware-accelerated video player directly inside Kura while preserving full freedom for users to pick external players (mpv, VLC, MPC-HC, or System Default) in Settings anytime. This also acts as the direct stepping stone making Japanese Immersion (Phase 8) far simpler, native, and robust.

### Checklist
- [ ] **Embedded `libmpv` Player Engine (In-App Player)**
  - [ ] Cross-platform embedded player core in Rust (`libmpv` / native render surface) with full hardware decoding (DirectX / Vulkan / Metal).
  - [ ] Full `libass` support for anime stylized typesetting, custom fonts, karaoke effects, and signs.
  - [ ] Seamless in-app player view with theater mode and fullscreen support (`F` / double click).
  - [ ] **Custom Kura Player UI (Svelte & CSS)**:
    - [ ] Glassmorphic control bar: play/pause, seek scrubber with hover preview, volume, and playback speed (0.5x - 2.0x).
    - [ ] Dynamic playlist / episode drawer (quick switch to previous/next episodes).
    - [ ] Audio track switcher and Subtitle track selector honoring user's global and per-anime track defaults.
  - [ ] **Player Preference in Settings**:
    - [ ] Add "Embedded Player (Kura Internal)" as a first-class choice under **Settings → Playback**.
    - [ ] Users can freely toggle between Embedded, mpv, VLC, MPC-HC, or System Default whenever they like.
- [ ] **Smart Skip OP/ED Engine (Chapters & AniSkip)**
  - [ ] **Offline-First Schema**: `CREATE TABLE skip_times (anilist_id INTEGER, ep_key TEXT, op_start REAL, op_end REAL, ed_start REAL, ed_end REAL, source TEXT, PRIMARY KEY (anilist_id, ep_key))`.
  - [ ] **Tier 1 (File Chapters)**: Detect embedded MKV/MP4 chapter markers (`OP`, `Opening`, `ED`, `Ending`) on launch.
  - [ ] **Tier 2 (Cached AniSkip)**: Pre-fetch and cache AniSkip timestamps (`api.aniskip.com`) during library scan for 100% offline usage.
  - [ ] **Interactive On-Screen Skip Button**:
    - [ ] Animated slide-in button: **`[ ⏭ Skip Opening (Tab) ]`** / **`[ ⏭ Skip Ending (Tab) ]`**.
    - [ ] One-click or `Tab` keypress jumps to the end of the intro/outro.
    - [ ] Optional setting toggle: *Auto-skip Openings & Endings* for users who prefer hands-free playback.
  - [ ] For external players (mpv/VLC/MPC): support floating skip pill or mpv OSD script and hotkey.
- [ ] **Stepping Stone to Japanese Immersion (Phase 8)**
  - [ ] Direct access to rendered subtitle text streams without external window hooking.
  - [ ] Clickable Japanese subtitle tokens directly in the DOM for instant Yomitan lookups.

---

## 🎭 Phase 6: Rich Cast, Voice Actors & Staff Metadata
*Target: v0.4.0*

Deepen the anime experience with voice actor guides, character portraits, and production credits.

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

---

## 🔄 Phase 7: AniList & MyAnimeList Account Sync
*Target: v0.5.0*

Two-way synchronization between your local Kura library and your online anime tracking profiles.

### Checklist
- [ ] **OAuth2 Authentication**
  - [ ] AniList OAuth2 login flow via secure local loopback redirect (`http://localhost:port/oauth/callback`).
  - [ ] Secure token storage in SQLite settings.
- [ ] **Two-Way Synchronization Engine**
  - [ ] Import user's list on connect (Watching, Completed, Planning, Paused, Dropped).
  - [ ] Auto-scrobble: when an episode reaches 90% in Kura, update episode progress on AniList.
  - [ ] Safe merge rules: ensure local and remote never accidentally regress progress.
  - [ ] Rate limit compliance (AniList 90 req/min).

---

## 🎌 Phase 8: Japanese Immersion & Yomitan Subtitle HUD
*Target: v0.6.0*

A blazing-fast, native Japanese immersion environment. Render interactive Japanese subtitles directly in Kura's embedded player (with mpv fallback) for instant Yomitan dictionary lookups.

### Checklist
- [ ] **Native Subtitle Rendering with Interactive Tokens**
  - [ ] Stream active Japanese subtitle strings, start/end timestamps, and track IDs to the player.
  - [ ] Crisp manga-styled Japanese subtitle rendering with customizable font size, outline, and position.
  - [ ] Hoverable words and de-inflected tokens with instant visual feedback.
- [ ] **Yomitan Dictionary Engine (Rust + SQLite)**
  - [ ] **Dictionary Importer**: Drag-and-drop standard Yomitan `.zip` archives (JMdict, KANJIDIC, Daijirin, etc.) in **Settings → Japanese Immersion**.
  - [ ] **Fast SQLite Index**: High-speed term lookup table indexing headwords, readings, definitions, and pitch accents (< 1ms query time).
  - [ ] **Deinflection Engine**: Break down conjugated verbs and adjectives (e.g. `食べられなかった` → `食べる`).
- [ ] **Manga-Styled Yomitan Popover Card**
  - [ ] Displays word reading (hiragana/katakana furigana), pitch accent markers, and concise definitions.
  - [ ] Lightweight, pure instant lookup — zero mining bloat, no Anki sync baggage, instant response.
- [ ] **Settings Integration**
  - [ ] "Japanese Immersion Mode" toggle under **Settings → Playback**.
  - [ ] Dictionary management UI (install, delete, view active dictionaries).

---

## ⚡ Phase 9: Library Power-User Tools & Backup
*Target: v1.0.0 (Beta)*

- [ ] **Advanced Filtering & Library Search**
  - [ ] Filter library by genres, tags, studios, format (TV, Movie, OVA, Special), airing status, and voice actors.
  - [ ] Quick filter pills on the library home page.
  - [ ] Multi-criteria sorting (Score, Date Added, Release Year, Title, Progress).
- [ ] **Library Export & Portable Backup**
  - [ ] One-click export of watch history, manual match overrides, and track preferences to a single `.json` file.
  - [ ] One-click restore option in Settings.
