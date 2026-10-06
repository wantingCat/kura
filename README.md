# Kura 蔵

<p align="center">
  <img src="assets/brand/logo.png" alt="Kura Logo" width="128" />
  <br>
  <strong>A local-first, manga-styled desktop anime media library.</strong>
  <br>
  <em>Current Release: <code>v0.1.0-alpha</code> · Built with AI assistance</em>
</p>

---

Point **Kura** at your local anime folders and it takes care of the rest: parses complex fansub release names, matches series to [AniList](https://anilist.co), pulls rich episode metadata and thumbnails via [ani.zip](https://api.ani.zip) (with [Jikan](https://jikan.moe) fallback), and organizes everything into an offline-ready, comic-inspired library interface.

---

## 📸 Screenshots

### Home & Library View
Browse your collection, view featured recently-added releases, search across English/Romaji/Japanese titles, and filter by series, movies, and specials:

![Kura Home & Library](assets/screenshots/home-library.png)

### Anime Details & Information
Backdrop banners, official poster art, format tags, season/year, studio info, ratings, synopsis, and related franchise entries:

![Kura Anime Details](assets/screenshots/anime-details.png)

### Episode List & Metadata
Individual episode cards with thumbnail previews, Japanese/English titles, plot summaries, air dates, file sizes, watch toggles, and "Up Next" indicators:

![Kura Episode List](assets/screenshots/episodes-list.png)

---

## ✨ Features in `v0.1.0-alpha`

- **Intelligent Filename Parsing**: Handles real-world release formats, fansub group tags `[Group]`, scene conventions, `SxxEyy`, `- 05v2`, batch folders, multi-part titles (`Part 2`), OVA/SP/Movie markings, creditless OP/EDs (`NCOP`/`NCED`), and Japanese numbering (`第N話`).
- **Season & Sequel Chain Resolution**: Accurately maps absolute episode numbering (e.g. episode `35`) or multi-season releases to the correct sequel/prequel AniList entry.
- **Rich Episode Details**: Synopses, episode titles (English + Kanji/Kana), thumbnails, and air dates pulled from [ani.zip](https://api.ani.zip) with [Jikan](https://jikan.moe) fallback.
- **Local-First & Offline Ready**: All metadata and cached artworks are saved locally using SQLite. Fast, lightweight, and works without an internet connection once scanned.
- **One-Click Playback**: Click any episode or the "Start / Up Next" button to launch the file directly in your operating system's default media player.
- **Review Queue & Manual Matcher**: "Needs review" queue and manual "Fix match" search modal for any unrecognized or ambiguous files.
- **Watched Progress Tracking**: Mark episodes as watched individually or in bulk ("Up to here"), with active progress bars on anime cards.

---

## 🗺️ Roadmap (Upcoming Features)

- [ ] **AniList Account Integration**: OAuth login, list import, and safe two-way watch progress synchronization (ensures local and remote progress never accidentally regress).
- [ ] **External Player Integrations**:
  - Direct integration with [mpv](https://mpv.io), [MPC-HC](https://github.com/clsid2/mpc-hc), [VLC](https://www.videolan.org/vlc/), and [Memento](https://github.com/ripose-jp/Memento) (ideal for language immersion).
  - Accurate watch-time and completion detection via player IPC.
- [ ] **Folder Watcher / Auto-Rescan**: Automatic background scanning triggered when new episode downloads or torrents complete in monitored directories.
- [ ] **Advanced Filtering & Search**: Filter by genres, tags, studios, release seasons, voice actors, and airing status.
- [ ] **Cross-Platform Releases**: Pre-built packages for macOS (Apple Silicon & Intel DMG) and Linux (Flatpak / AppImage).
- [ ] **Library Export / Backup**: Backup watch history and metadata cache to JSON or portable archives.

---

## 📦 Download & Installation

### Windows (v0.1.0-alpha)
Download the latest pre-compiled Windows installer (`.exe` setup or `.msi`) from the **[GitHub Releases](https://github.com/wantingCat/kura/releases)** page.

---

## 🛠️ Building from Source

### Prerequisites
- [Node.js](https://nodejs.org) 20+
- [Rust](https://rustup.rs) (stable)
- OS-specific [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) (on Windows: C++ Build Tools & WebView2)

### Setup & Run
```sh
# Install dependencies
npm install

# Start development app with hot reload
npm run tauri dev
```

### Useful Commands
| Command | Purpose |
| --- | --- |
| `npm run check` | Svelte & TypeScript diagnostics |
| `cargo test --lib` (in `src-tauri`) | Filename parser & matcher unit tests |
| `npm run tauri build` | Build optimized production executable and installers |

---

## 🧱 Tech Stack

- **Desktop Framework**: [Tauri v2](https://v2.tauri.app)
- **Backend / Core**: Rust (`rusqlite`, `reqwest`, `tokio`, custom filename regex parser)
- **Frontend**: [SvelteKit](https://kit.svelte.dev) (Svelte 5, static SPA adapter)
- **Language**: TypeScript & Rust
- **Styling**: Vanilla CSS (comicbook / pop dark aesthetic)
- **Metadata Sources**: [AniList GraphQL API](https://anilist.gitbook.io/anilist-apiv2-docs), [ani.zip](https://api.ani.zip), [Jikan / MAL](https://jikan.moe)

---

## 📄 License

This project is licensed under the [GNU General Public License v3.0 or later (GPL-3.0-or-later)](LICENSE).
Forks, derivative works, and redistributed versions must remain free and open source under the same license.
