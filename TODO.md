# Kura Master Roadmap & To-Do List

This document serves as the master planning roadmap for Kura, organizing proposed features into prioritized milestones, technical specifications, and implementation checklists.

---

## 📍 Milestone Overview

| Milestone | Target Focus | Status |
| :--- | :--- | :--- |
| **v0.3.2** | **Visual & Artwork Overhaul**: Manga Mode (Light Theme) + Priority Artwork Engine (ClearLogos & Fanart) | 📋 Ready to Plan |
| **v0.3.3** | **Social & Real-Time Tracking**: Discord Rich Presence (RPC) + Live Folder Watcher | 💡 Planned |
| **v0.4.0** | **Deeper Anime Knowledge**: Rich Cast, Voice Actors (Seiyuu), Character Portraits & Staff Guides | 💡 Planned |
| **v0.5.0** | **Japanese Immersion**: Native mpv + Floating Yomitan Subtitle HUD (Lightweight Memento Alternative) | 💡 Planned |
| **v0.6.0** | **Cloud & Account Integration**: AniList & MyAnimeList Two-Way Scrobbling / List Sync | 💡 Planned |
| **v1.0.0 (Beta)** | **Library Power-User**: Advanced Filtering, Library Backup/Export, Performance Polish | 💡 Future |

---

## 🎨 Phase 1: Manga Mode (Light Theme — Ink & Paper)
*Target: v0.3.2*

Transform Kura's visual identity from the current plum/coral "Comicbook Dark" mode into an authentic Japanese printed manga aesthetic inspired by *Tankōbon* volumes and Shōnen Jump.

### Checklist
- [ ] **Design Tokens & Theme Engine (`[data-theme="manga"]`)**
  - [ ] **Backgrounds**: Warm off-white newsprint / manga paper (`#f5f2eb` page background, `#ffffff` card panels).
  - [ ] **Sumi Ink Outlines**: Deep black line art (`#111111`) with crisp 2px solid sticker drop shadows (`3px 3px 0 #111111`).
  - [ ] **Option A Accent — Vermilion Red Stamp**: Authentic Japanese red seal (`#e53935` / `#eb4d3d`) for play buttons, active season tabs, and key highlight badges.
  - [ ] **Screentone Midtones**: Subtle wash surfaces (`#ece8df`, `#ded9cd`) for secondary cards, inputs, and tab bars.
  - [ ] **Monochrome Icons & Typography**: High-contrast dark ink text with native `color-scheme: light` support.
- [ ] **Settings & Boot Persistence**
  - [ ] Add `pref.theme` setting in SQLite `settings` table (`"comic"` vs `"manga"`).
  - [ ] Add Theme / Appearance radio card picker under **Settings → Display**.
  - [ ] Initialize theme class on `document.documentElement` during early boot to prevent any Flash of Unstyled Content (FOUC).

---

## 🖼️ Phase 2: Priority Artwork Engine (`Local > Fanart > AniList`)
*Target: v0.3.2*

Eliminate missing or blurred backdrops and introduce transparent anime title logos (ClearLogos) across the app using a strict priority pipeline.

### Priority Hierarchy
$$\mathbf{1.\ Local\ Folder\ Assets} \;\longrightarrow\; \mathbf{2.\ Fanart.tv\ /\ ani.zip\ Fanart} \;\longrightarrow\; \mathbf{3.\ AniList\ Default}$$

### Checklist
- [ ] **Local Asset Scanner (`scan.rs`)**
  - [ ] Scan anime folder for local backdrops: `fanart.jpg`, `fanart.png`, `backdrop.jpg`, `backdrop.png`, `background.jpg`, `art.jpg`.
  - [ ] Scan anime folder for local transparent logos: `clearlogo.png`, `logo.png`, `clearart.png`.
  - [ ] Scan anime folder for local posters: `poster.jpg`, `cover.jpg`, `folder.jpg`.
  - [ ] Use local file paths directly when present, bypassing remote downloads.
- [ ] **Zero-Config Built-in Fanart & ClearLogo Integration**
  - [ ] Update `anizip_episodes` parser to extract the `images` array from `api.ani.zip/mappings`:
    - `coverType: "Fanart"` $\to$ 1080p TheTVDB/Fanart widescreen background.
    - `coverType: "Clearlogo"` $\to$ Transparent PNG title logo.
    - `coverType: "Banner"` $\to$ Graphical banner.
  - [ ] Update SQLite schema: add `logo_url` and `logo_path` columns to `media` table.
  - [ ] Download and cache ClearLogos locally in `<app_data>/cache/images/logos/` for 100% offline usage.
- [ ] **Fanart.tv API Key Support (Settings)**
  - [ ] Add `fanart_api_key` preference in **Settings → Artwork & Metadata**.
  - [ ] When a key is configured, query `https://webservice.fanart.tv/v3/tv/{thetvdb_id}?api_key=...` (or `/movies/{themoviedb_id}`) using mappings from ani.zip.
  - [ ] Fetch community-curated HD ClearLogos (`hdtvlogo`), high-res posters, and backgrounds (`showbackground`).
- [ ] **Cinematic Header with Floating ClearLogos (`anime/[id]/+page.svelte`)**
  - [ ] Display transparent PNG title logo floating over the widescreen Fanart backdrop with drop shadow (`filter: drop-shadow(0 4px 14px rgba(0,0,0,0.7))`).
  - [ ] Subtitle underneath with Japanese kanji/kana and romaji titles.
  - [ ] Seamless fallback to styled typographic title if no ClearLogo exists for that title.

---

## 🎮 Phase 3: Discord Rich Presence (RPC)
*Target: v0.3.3*

Show off what you are currently watching to friends on Discord with live episode progress.

### Checklist
- [ ] **Discord IPC Client in Rust (`src-tauri`)**
  - [ ] Integrate lightweight Rust IPC client (`discord-rich-presence`) connecting to Discord's local socket (`\\.\pipe\discord-ipc-0` / `/tmp/discord-ipc-0`).
  - [ ] Register Kura's Discord Application ID with default art assets (Kura logo, play/pause badges).
- [ ] **Real-Time Playback Synchronization**
  - [ ] Hook into `player.rs` event loop to update Discord presence on play, pause, seek, and finish.
  - [ ] **Activity State**: Display anime title, episode number, and episode title.
  - [ ] **Live Progress Bar**: Display remaining/elapsed time using Discord timestamp timestamps.
  - [ ] **Artwork**: Display anime poster / cover art as large image with tooltip.
  - [ ] **Action Button**: Optional "View on AniList" button linking to the series page.
  - [ ] **Idle State**: Show "Browsing Library" or clear status when player closes.
- [ ] **Privacy & Settings**
  - [ ] Toggle in **Settings → Playback / Privacy**:
    - *Enable Discord Rich Presence* (On/Off).
    - *Spoiler Protection*: Option to hide episode title and number from Discord status.

---

## 📂 Phase 4: Folder Watcher & Background Rescan
*Target: v0.3.3*

Automatically detect new anime episodes and movies the moment they finish downloading.

### Checklist
- [ ] **Filesystem Watcher (`notify` crate)**
  - [ ] Watch all configured library folder roots recursively.
  - [ ] Filter for media file extensions (`.mkv`, `.mp4`, `.avi`, `.webm`).
  - [ ] Debounce events (wait 3-5 seconds after file lock releases to ensure download/copy is 100% finished).
- [ ] **Targeted Background Ingestion**
  - [ ] Ingest only the newly detected file instead of rescanning the entire library.
  - [ ] Parse filename, match to existing series in SQLite, and download episode thumbnail.
  - [ ] Emit Tauri event `library-changed` to automatically update the frontend without reloading.

---

## 🎭 Phase 5: Rich Cast, Voice Actors & Staff Metadata
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

## 🎌 Phase 6: Japanese Immersion & Yomitan Subtitle HUD
*Target: v0.5.0*

A blazing-fast, lightweight alternative to Memento. Combines native mpv 4K video playback with a transparent Tauri floating subtitle overlay for instant Yomitan dictionary lookups.

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
- [ ] **Settings Integration**
  - [ ] "Japanese Immersion Mode" toggle under **Settings → Playback** (enabled when mpv is selected).
  - [ ] Dictionary management UI (install, delete, view active dictionaries).

---

## 🔄 Phase 7: AniList & MyAnimeList Account Sync
*Target: v0.6.0*

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

## ⚡ Phase 8: Library Power-User Tools & Backup
*Target: v1.0.0 (Beta)*

- [ ] **Advanced Filtering & Library Search**
  - [ ] Filter library by genres, tags, studios, format (TV, Movie, OVA, Special), airing status, and voice actors.
  - [ ] Quick filter pills on the library home page.
  - [ ] Multi-criteria sorting (Score, Date Added, Release Year, Title, Progress).
- [ ] **Library Export & Portable Backup**
  - [ ] One-click export of watch history, manual match overrides, and track preferences to a single `.json` file.
  - [ ] One-click restore option in Settings.
