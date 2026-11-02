export type ChangeType = "new" | "improvement" | "fix";

export interface ChangelogItem {
  type: ChangeType;
  text: string;
}

export interface ChangelogRelease {
  version: string;
  title: string;
  date: string;
  badge?: string;
  summary: string;
  items: ChangelogItem[];
}

export const CHANGELOG: ChangelogRelease[] = [
  {
    version: "0.6",
    title: "Built-In Memento Player, Hover Yomitan Dictionary & Modular Torrent Extensions",
    date: "November 2026",
    badge: "Latest",
    summary:
      "Kura v0.6 introduces the built-in Memento-style video player with pause-to-hover Yomitan Japanese dictionary lookup, transitions torrent streaming to a 100% modular bring-your-own-extension architecture (compatible with Hayase index.json manifests, RSS feeds, SubsPlease, and SeaDex), upgrades the BitTorrent engine with DHT/PEX/WebSeeds and a live player HUD, and adds live subtitle auto-sync for streaming episodes with distributor intro bumpers.",
    items: [
      {
        type: "new",
        text: "Kura Built-In (Memento) Video Player: watch episodes directly inside Kura with hardware-accelerated playback, live H.264/AAC stream transcoding fallback for 10-bit HEVC/AC3, embedded MKV subtitle & chapter extraction, AniSkip OP/ED skip, and a live top-bar torrent stats HUD (downloaded %, MB, download speed, upload speed, and peers).",
      },
      {
        type: "new",
        text: "Pause-to-Hover Yomitan Japanese Dictionary: pause the built-in player and hover over any Japanese subtitle word to deinflect conjugated verbs/adjectives and view furigana readings, pitch accent, word frequencies, definitions, audio pronunciation, and Kanji breakdowns — with a 1-Click Full JMdict + KANJIDIC downloader and Yomitan .zip importer in Settings → Audio & Subtitles.",
      },
      {
        type: "new",
        text: "Modular Torrent Extensions (Zero Built-In Piracy): add Hayase-compatible extension manifests (index.json), Kura JSON manifests, or custom RSS / SubsPlease / SeaDex endpoints in Settings → Playback while keeping Kura completely clean of hardcoded piracy domains.",
      },
      {
        type: "improvement",
        text: "Sub-Second Concurrent Torrent Search & Swarm Engine: queries all enabled extensions concurrently in a single wave with early straggler cancellation, highlights ★ SeaDex Best releases, and adds Mainline DHT (BEP 5), Peer Exchange (BEP 11), Tracker Scrape (BEP 48), and WebSeed (BEP 19) support.",
      },
      {
        type: "improvement",
        text: "Live Streaming Subtitle Auto-Sync & AniList Status Tabs: automatically aligns external Jimaku Japanese subtitles against embedded MKV dialogue cues during live sequential streaming (compensating for Viz Media / Crunchyroll / Netflix intro bumpers), plus adds Watching / Planning / Completed / Paused / Dropped / All tabs to the library.",
      },
    ],
  },
  {
    version: "0.5",
    title: "2-Mode Streaming & Hybrid Architecture, Jimaku Subtitles & Auto-Retiming",
    date: "November 2026",
    summary:
      "A massive leap forward for Kura: switch between classic Offline Library Mode and Hayase-style Streaming & Hybrid Mode with built-in Nyaa BitTorrent sequential streaming into mpv & Memento, browse the full AniList catalog and Voice Actors (Seiyuu) with infinite scroll, plus the complete Japanese Immersion Suite — Jimaku.cc subtitle fetching, Smart Subtitle Matcher, and automatic Alass-style subtitle retiming.",
    items: [
      {
        type: "new",
        text: "2-Mode Operating Architecture: choose between Mode 1 (Offline Library Mode) and Mode 2 (Streaming & Hybrid Mode) in Settings → Playback, with configurable torrent actions (Stream Only in Temp Cache, Stream & Save to Library, or Download to Library First) and optional peer seeding.",
      },
      {
        type: "new",
        text: "Built-in Nyaa BitTorrent Sequential Streaming: stream any episode directly into mpv or Memento over a local HTTP 206 Range server with smart piece prioritization (MKV header + Cues seek index first), custom Nyaa mirror support, and immediate peer cleanup on playback stop.",
      },
      {
        type: "new",
        text: "Full Catalog & Voice Actor (Seiyuu) Discovery (/discover): browse the entire AniList catalog with infinite scroll, multi-filters (Genre, Format, Status, Season, Year, Sort), and search any Voice Actor or Staff member to explore all anime they worked on.",
      },
      {
        type: "new",
        text: "Jimaku.cc Japanese Subtitle Fetcher & Automatic Subtitle Retiming: fetch Japanese subtitles (.srt/.ass/.zip packs) directly from Jimaku.cc and automatically synchronize subtitle timing against your video's reference subtitle cues or MKV tracks — hot-loading straight into mpv or Memento.",
      },
      {
        type: "new",
        text: "Smart Subtitle Matcher & Characters / Voice Cast: link or rename mixed-format external subtitle files (including absolute episode numbering like S02E25 → S02E01) with a live preview, and browse full Character, Japanese Seiyuu, and Staff credits on every anime page.",
      },
    ],
  },
  {
    version: "0.4",
    title: "AniList Two-Way Sync, Airing Calendar & Advanced Filters",
    date: "October 2026",
    summary:
      "A major milestone release: connect your AniList account with safe two-way sync and offline auto-scrobbling, browse the weekly Seasonal Airing Calendar with live episode countdowns, filter and sort your library by genres, tags, studios, and progress, plus a comprehensive visual layout hardening pass.",
    items: [
      {
        type: "new",
        text: "AniList Two-Way Sync & Auto-Scrobble: sign in via browser OAuth or token in Settings → Integrations, import your Watching/Planning/Completed lists, and auto-scrobble episodes when they hit your watched threshold — with an offline queue that syncs automatically when you reconnect.",
      },
      {
        type: "new",
        text: "Seasonal Airing Calendar: a new Calendar tab in the top bar showing a Mon–Sun weekly broadcast schedule in your local timezone, live episode countdowns, Today highlights, and instant filtering between all seasonal anime, shows in your library, and your AniList list.",
      },
      {
        type: "new",
        text: "Airing & New-Episode Notifications: optional in-app notifications when a tracked seasonal episode airs or when the live folder watcher ingests a newly downloaded episode.",
      },
      {
        type: "new",
        text: "Advanced Library Filtering & Sorting: filter your library by multiple genres, non-spoiler tags, animation studios, airing status, format, and watch status, with quick removable filter pills and multi-criteria sorting (including Watch Progress and ascending/descending order).",
      },
      {
        type: "improvement",
        text: "Visual Bug Sweep & Layout Hardening: long English, Romaji, and Japanese episode/series titles now wrap and clamp cleanly across episode rows, badges, poster cards, Continue Watching panels, search modals, and narrow (~900px) windows.",
      },
    ],
  },
  {
    version: "0.3.7",
    title: "Settings, Backups & Diagnostics",
    date: "October 2026",
    summary: "A reorganized Settings page, full backup & restore of your library data, a built-in diagnostics report, and a fix for the app hanging on first launch.",
    items: [
      {
        type: "fix",
        text: "Kura no longer freezes for a few seconds when it first opens: slow or disconnected library drives and player detection now run in the background.",
      },
      {
        type: "improvement",
        text: "Redesigned Settings: a sidebar with categories, plus a search box (Ctrl+F) that jumps straight to any setting.",
      },
      {
        type: "new",
        text: "Full Backup & Restore: export your settings, libraries, watch history, progress, track preferences and manual matches to a file, then preview and choose what to restore.",
      },
      {
        type: "improvement",
        text: "Automatic safety backups now include watch history and progress, and are restored automatically if your database is ever lost.",
      },
      {
        type: "new",
        text: "Diagnostics in Settings → About: copy a system report (API keys redacted), re-detect players, and view the recent log. Kura now keeps a log file for troubleshooting.",
      },
      {
        type: "improvement",
        text: "Kura now opens maximized instead of in a small window in the middle of the screen.",
      },
      {
        type: "fix",
        text: "The \"Up next\" badge no longer breaks the layout on episodes with very long titles.",
      },
    ],
  },
  {
    version: "0.3.6",
    title: "Smarter Skip Button",
    date: "October 2026",
    summary: "A bug-fix release: the skip button no longer mistakes prologues for openings, and it got a proper redesign.",
    items: [
      {
        type: "fix",
        text: "The Skip Opening button no longer appears during prologues and cold opens, and pressing it can't skip the prologue anymore.",
      },
      {
        type: "fix",
        text: "AniSkip timestamps are matched to your file's exact length, so TV and Blu-ray versions with different prologue lengths no longer get the wrong skip times.",
      },
      {
        type: "fix",
        text: "Better detection of openings from chapters: prologue, recap and teaser chapters are ignored, and implausibly short or long \"openings\" are rejected.",
      },
      {
        type: "improvement",
        text: "Redesigned skip button: a rounded Kura-styled pill with a TAB key hint that turns coral on hover, and stays crisp at any window size.",
      },
      {
        type: "fix",
        text: "Kura's messages inside MPV (now playing, subtitle delay, skipped) show properly styled text instead of raw formatting codes.",
      },
    ],
  },
  {
    version: "0.3.5",
    title: "MPV Detection Fix",
    date: "October 2026",
    summary: "A small bug-fix release: Kura reliably finds MPV after installing it with the built-in installer, and macOS and Linux builds are back.",
    items: [
      {
        type: "fix",
        text: "macOS and Linux downloads are available again (v0.3.4 failed to build on those platforms).",
      },
      {
        type: "fix",
        text: "MPV installed through the built-in winget installer is now detected (it installs to \"C:\\Program Files\\MPV Player\", which Kura wasn't checking).",
      },
      {
        type: "fix",
        text: "Newly installed players are found without logging out: Kura now reads the up-to-date PATH from the registry instead of the copy it started with.",
      },
      {
        type: "improvement",
        text: "Player detection also checks Windows' installed-apps list and WinGet package folders, so MPV, VLC and MPC are found whichever installer you used.",
      },
      {
        type: "fix",
        text: "The installer no longer sends you to the MPV website when winget reports MPV as \"already installed\".",
      },
    ],
  },
  {
    version: "0.3.4",
    title: "Interactive Skip Overlay & Settings Persistence",
    date: "October 2026",
    summary:
      "Direct interactive Skip OP/ED button in MPV with mouse click & hover tracking, frame-accurate AniSkip API integration, smart cold open detection, What's New popup, and bulletproof settings persistence.",
    items: [
      {
        type: "new",
        text: "Interactive Skip OP/ED button right on top of MPV: hover over the overlay or press Tab / Click to effortlessly skip openings and endings.",
      },
      {
        type: "new",
        text: "Frame-accurate AniSkip API integration with smart ~90s duration verification that prevents cold opens and prologues from being mistaken for the opening.",
      },
      {
        type: "new",
        text: "Automatic Settings & Library Persistence: fixed WiX upgrade GUID and added automated JSON backups so your settings, libraries, and theme survive all updates.",
      },
      {
        type: "new",
        text: "What's New changelog popup on new version launches, accessible anytime from Settings.",
      },
      {
        type: "improvement",
        text: "First-launch MPV setup assistant with automatic Chocolatey / Winget installer detection and custom Kura anime theme.",
      },
      {
        type: "improvement",
        text: "Enhanced player detection supporting Windows App Paths registry and custom executable paths.",
      },
      {
        type: "fix",
        text: "Fixed settings reverting to default and prompt showing repeatedly when installing new updates.",
      },
      {
        type: "fix",
        text: "Fixed chapter scan fallback so non-opening scenes aren't skipped.",
      },
    ],
  },
  {
    version: "0.3.3",
    title: "Social & Real-Time Tracking",
    date: "October 2026",
    summary:
      "Discord Rich Presence (RPC), background live folder watcher with debounced file locks, and targeted sub-10ms episode ingestion.",
    items: [
      {
        type: "new",
        text: "Discord Rich Presence (RPC) showing your anime title, episode number, cover art, and elapsed time.",
      },
      {
        type: "new",
        text: "Live folder watcher (auto-rescan) with recursive file-lock debounce that auto-ingests newly downloaded or renamed episodes in real time.",
      },
      {
        type: "improvement",
        text: "Interactive Play launch indicators transitioning from loader to animated equalizer badge.",
      },
    ],
  },
  {
    version: "0.3.2",
    title: "ClearArt & Scanner Stability",
    date: "October 2026",
    summary: "ClearLogos from Fanart.tv, deep metadata rebuilds, and scanner stability improvements.",
    items: [
      {
        type: "new",
        text: "ClearArt & Logo rendering: transparent anime titles on detail banners with Fanart.tv API integration.",
      },
      {
        type: "new",
        text: "Rebuild Metadata tool in Settings to clean up cached artwork and re-sync high-resolution logos.",
      },
      {
        type: "improvement",
        text: "Optimized episode scanner memory usage and enhanced AniList rate limit backoff.",
      },
      {
        type: "fix",
        text: "Fixed image placeholder flickering on high-DPI displays.",
      },
    ],
  },
  {
    version: "0.3.1",
    title: "Language & Audio Defaults",
    date: "October 2026",
    summary: "Granular audio and subtitle track preferences with per-show overrides.",
    items: [
      {
        type: "new",
        text: "Default audio and subtitle language selectors with smart fallback presets (e.g. Japanese + English subs).",
      },
      {
        type: "new",
        text: "Per-show audio/subtitle track override controls in anime detail pages.",
      },
      {
        type: "improvement",
        text: "Automatic track selection when launching videos in external players.",
      },
    ],
  },
  {
    version: "0.3.0",
    title: "Multi-Season & Extras",
    date: "October 2026",
    summary: "Multi-season franchise resolution, chronological ordering, and extras / bonus playback UI.",
    items: [
      {
        type: "new",
        text: "Plex/Jellyfin style multi-season bridging connecting sequels, OVAs, and spin-offs.",
      },
      {
        type: "new",
        text: "Extras and bonus video detection: trailers, clean NCOP/NCEDs, and interview clips.",
      },
      {
        type: "improvement",
        text: "Expanded cinematic backdrop header with rich episode grid view.",
      },
    ],
  },
];
