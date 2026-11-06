# Kura 蔵

<p align="center">
  <img src="assets/brand/logo.png" alt="Kura logo" width="128" />
  <br>
  <strong>A Japanese immersion suite for anime.</strong>
  <br>
  <em>Watch with Japanese subtitles, look up any word on hover, and keep your learning in one place.</em>
</p>

---

Kura is a desktop app for learning Japanese through anime. It plays episodes in a built-in player with Japanese subtitles you can hover for instant dictionary lookups, finds and syncs those subtitles for you, and tracks what you watch — whether the episodes live in your own folders or are streamed through sources you add yourself.

## Why Kura

Immersion usually means juggling a player, a subtitle site, a retiming tool, a dictionary extension and a tracker. Kura puts that whole loop in one window:

1. **Find something to watch** — your local library, your AniList lists, or the full AniList catalog.
2. **Get Japanese subtitles automatically** — fetched from [Jimaku](https://jimaku.cc) and retimed to your exact video.
3. **Watch and look things up** — pause, hover a word, and see its reading, meaning, pitch accent and kanji.
4. **Keep going** — progress syncs to AniList, and Kura tells you when the next episode and its Japanese subtitles are out.

## Features

### Immersion
- **Hover dictionary (Yomitan-style)** — pause and hover any word in the subtitles. Conjugations are deinflected (`食べさせられた` → `食べる`), with furigana, definitions, pitch accent, frequency, audio and kanji breakdowns.
- **Bring your own dictionaries** — one-click JMdict + KANJIDIC download, or import any Yomitan `.zip`.
- **Japanese subtitles from Jimaku** — search and download per episode or as season packs. A 日本語 tag on the show page tells you which episodes already have subtitles.
- **Automatic subtitle retiming** — Jimaku subtitles are often timed for TV broadcasts. Kura aligns them to your video's own dialogue track, handling intro bumpers and commercial-break shifts, even while an episode is still streaming.
- **Subtitle matcher** — link or rename a folder of subtitle files to the right episodes, including absolute numbering (`S02E25` → `S02E01`).
- **Episode & subtitle alerts** — get notified when an episode on your Watching list airs and again when its Japanese subtitles appear on Jimaku (which can take anywhere from hours to a week). Kura can stay in the system tray to keep checking.

### Watching
- **Built-in player** — powered by mpv with GPU 10-bit HEVC decoding, embedded subtitle and chapter support, instant audio-track switching, OP/ED skip via AniSkip, and resume.
- **Or use your own player** — mpv, VLC and MPC-HC/BE are detected automatically.
- **Local library** — point Kura at your folders. It understands real-world release names, groups seasons into one show, and fetches artwork and episode details.
- **Streaming (optional)** — add your own torrent sources in **Settings → Extensions** and stream episodes you don't have locally. Kura ships with **no built-in sources**.

### Tracking
- **AniList two-way sync** — imports your lists and updates progress the moment you mark an episode watched, with an offline queue.
- **Seasonal calendar** — this week's broadcasts in your timezone, with countdowns.
- **Global search** (`Ctrl K`) — your library first, then all of AniList.
- **Discord Rich Presence**, **backup & restore**, and **automatic updates**.

## Download

Get the latest build from **[GitHub Releases](https://github.com/wantingCat/kura/releases)**:

- **Windows** — `.exe` installer (recommended) or `.msi`
- **macOS** — `.dmg` (`aarch64` for Apple Silicon, `x64` for Intel)
- **Linux** — `.deb`, `.rpm` or `.AppImage`

Kura updates itself — when a new version is out you'll see an **Update & restart** banner.

> [!NOTE]
> Builds aren't code-signed yet. On Windows, if SmartScreen appears, click **More info → Run anyway**. On macOS, right-click the app and choose **Open** the first time.

### Getting started
1. Add a folder of anime in the first-run screen, or switch to **Streaming & Hybrid Mode** in **Settings → Playback**.
2. Add a free [Jimaku API key](https://jimaku.cc/account) in **Settings → Integrations** for Japanese subtitles.
3. Install the full dictionaries in **Settings → Audio & Subtitles**.
4. Optionally connect AniList in **Settings → Integrations**.

## Building from source

**Prerequisites:** [Node.js](https://nodejs.org) 20+, [Rust](https://rustup.rs) (stable), and the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
npm install
npm run tauri dev
```

| Command | Purpose |
| --- | --- |
| `npm run check` | Svelte & TypeScript checks |
| `cargo test --lib` (in `src-tauri`) | Rust unit tests |
| `npm run tauri build` | Production build and installers |

**Stack:** Tauri v2 · Rust (`tokio`, `rusqlite`, `reqwest`) · SvelteKit / Svelte 5 · mpv. Metadata from [AniList](https://anilist.co), [ani.zip](https://api.ani.zip) and [Jikan](https://jikan.moe); subtitles from [Jimaku](https://jimaku.cc).

## License

[GPL-3.0-or-later](LICENSE). Forks and redistributed versions must remain free and open source under the same license.
