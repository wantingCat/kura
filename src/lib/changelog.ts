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
    version: "0.3.3",
    title: "The Immersion, Skip Overlay & Precision Update",
    date: "October 2026",
    badge: "Current",
    summary:
      "Direct interactive Skip OP/ED button in MPV, frame-accurate AniSkip integration, smart cold open detection, live folder watcher, and bulletproof settings persistence.",
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
        text: "Automatic Settings & Library Persistence: your preferences, libraries, and theme are now preserved across app updates and reinstalls.",
      },
      {
        type: "new",
        text: "What's New changelog popup on new version launches, accessible anytime from Settings.",
      },
      {
        type: "new",
        text: "Live folder watcher (auto-rescan) with recursive file-lock debounce that auto-ingests newly downloaded or renamed episodes in real time.",
      },
      {
        type: "new",
        text: "Discord Rich Presence (RPC) showing your anime title, episode number, cover art, and elapsed time.",
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
