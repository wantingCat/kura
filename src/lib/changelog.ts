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
    version: "0.3.6",
    title: "Smarter Skip Button",
    date: "October 2026",
    badge: "Latest",
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
