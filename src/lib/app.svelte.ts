import {
  api,
  events,
  type Library,
  type MediaCard,
  type PlaybackEvent,
  type PlayerKind,
  type ScanProgress,
  type UnmatchedGroup,
} from "./api";
import { updater } from "./updater.svelte";

export interface PlaybackPrefs {
  /** Empty = not chosen yet (the backend then uses the first detected player). */
  player: PlayerKind | "";
  playerPath: string;
  /** Percent of the episode after which it counts as watched (50–100). */
  threshold: number;
  resume: boolean;
  autoplay: boolean;
  mpcPort: number;
  audioLang: string;
  subLang: string;
  subFallback: string;
}

export interface DiscordPrefs {
  enabled: boolean;
  spoilers: boolean;
  buttons: boolean;
  clientId: string;
}

export interface LaunchingEpisode {
  anilistId: number;
  epKey: string;
}

export type AppTheme = "comic" | "manga";

export interface Toast {
  id: number;
  kind: "info" | "warn" | "error";
  text: string;
}

class AppStore {
  libraries = $state<Library[]>([]);
  cards = $state<MediaCard[]>([]);
  unmatched = $state<UnmatchedGroup[]>([]);
  loaded = $state(false);
  scanning = $state(false);
  progress = $state<ScanProgress | null>(null);
  lastError = $state<string | null>(null);
  /** Bumped on every library change so pages can re-fetch their own data. */
  version = $state(0);
  /** Show one card per franchise (seasons as tabs) instead of one per AniList entry. */
  groupSeasons = $state(true);
  /** Current visual theme ("comic" = dark plum/coral, "manga" = light sumi/newsprint). */
  theme = $state<AppTheme>("comic");
  /** Library search, driven by the top bar. */
  query = $state("");
  playback = $state<PlaybackPrefs>({
    player: "",
    playerPath: "",
    threshold: 90,
    resume: true,
    autoplay: false,
    mpcPort: 13579,
    audioLang: "jpn",
    subLang: "jpn",
    subFallback: "eng",
  });
  autoUpdate = $state(true);
  fanartApiKey = $state("");
  discord = $state<DiscordPrefs>({
    enabled: true,
    spoilers: false,
    buttons: true,
    clientId: "",
  });
  launching = $state<LaunchingEpisode | null>(null);
  /** Latest playback event while a player is being tracked. */
  nowPlaying = $state<PlaybackEvent | null>(null);
  toasts = $state<Toast[]>([]);

  #refreshTimer: ReturnType<typeof setTimeout> | null = null;
  #launchTimer: ReturnType<typeof setTimeout> | null = null;
  #initialised = false;
  #toastId = 0;

  async init() {
    if (this.#initialised) return;
    this.#initialised = true;
    if (typeof localStorage !== "undefined") {
      const localTheme = localStorage.getItem("kura-theme") as AppTheme | null;
      if (localTheme === "manga" || localTheme === "comic") {
        this.theme = localTheme;
        this.applyTheme(this.theme);
      }
    }
    await events.onLibraryChanged(() => this.scheduleRefresh());
    await events.onScanProgress((p) => {
      this.scanning = p.phase !== "done";
      this.progress = p;
    });
    await events.onScanFinished(() => {
      this.scanning = false;
      setTimeout(() => {
        if (!this.scanning) this.progress = null;
      }, 2500);
      this.scheduleRefresh();
    });
    await events.onScanError((msg) => (this.lastError = msg));
    await events.onPlayback((p) => {
      if (p.state === "tracking") {
        this.nowPlaying = p;
        if (this.launching?.anilistId === p.anilistId && this.launching?.epKey === p.epKey) {
          this.launching = null;
          if (this.#launchTimer) clearTimeout(this.#launchTimer);
        }
      } else if (p.state === "stopped") {
        this.nowPlaying = null;
        if (this.launching?.anilistId === p.anilistId && this.launching?.epKey === p.epKey) {
          this.launching = null;
        }
      } else {
        this.nowPlaying = null;
        this.launching = null;
        if (p.hint) this.toast(p.hint, "warn", 9000);
      }
    });
    this.scanning = await api.isScanning();
    try {
      const prefs = await api.getPrefs();
      if (prefs.theme === "manga" || prefs.theme === "comic") {
        this.theme = prefs.theme;
        this.applyTheme(this.theme);
      }
      this.groupSeasons = prefs.group_seasons !== "0";
      this.autoUpdate = prefs.auto_update !== "0";
      this.fanartApiKey = prefs.fanart_api_key ?? "";
      const num = (v: string | undefined, d: number) => (v !== undefined && !isNaN(Number(v)) ? Number(v) : d);
      this.playback = {
        player: (prefs.player as PlayerKind) ?? "",
        playerPath: prefs.player_path ?? "",
        threshold: Math.min(100, Math.max(50, num(prefs.watched_threshold, 90))),
        resume: prefs.resume !== "0",
        autoplay: prefs.autoplay === "1",
        mpcPort: num(prefs.mpc_port, 13579),
        audioLang: prefs.audio_lang ?? "jpn",
        subLang: prefs.sub_lang ?? "jpn",
        subFallback: prefs.sub_fallback ?? "eng",
      };
      this.discord = {
        enabled: prefs.discord_rpc !== "0",
        spoilers: prefs.discord_spoilers === "1",
        buttons: prefs.discord_buttons !== "0",
        clientId: prefs.discord_client_id ?? "",
      };
    } catch {
      /* keep defaults */
    }
    await this.refresh();
    if (this.autoUpdate) setTimeout(() => updater.check(), 3000);
  }

  scheduleRefresh() {
    if (this.#refreshTimer) clearTimeout(this.#refreshTimer);
    this.#refreshTimer = setTimeout(() => this.refresh(), 250);
  }

  async refresh() {
    const [libraries, cards, unmatched] = await Promise.all([
      api.listLibraries(),
      api.getLibrary(null),
      api.getUnmatched(),
    ]);
    this.libraries = libraries;
    this.cards = cards;
    this.unmatched = unmatched;
    this.loaded = true;
    this.version++;
  }

  async scan(libraryId: number | null = null) {
    this.scanning = true;
    this.lastError = null;
    this.progress = { phase: "discover", current: 0, total: 0, message: "Starting scan…" };
    await api.startScan(libraryId);
  }

  async setTheme(t: AppTheme) {
    this.theme = t;
    this.applyTheme(t);
    await api.setPref("theme", t);
  }

  applyTheme(t: AppTheme) {
    if (typeof document !== "undefined") {
      if (t === "manga") {
        document.documentElement.dataset.theme = "manga";
      } else {
        delete document.documentElement.dataset.theme;
      }
      const metaTheme = document.querySelector('meta[name="theme-color"]');
      if (metaTheme) {
        metaTheme.setAttribute("content", t === "manga" ? "#f5f2eb" : "#1d1415");
      }
      const metaScheme = document.querySelector('meta[name="color-scheme"]');
      if (metaScheme) {
        metaScheme.setAttribute("content", t === "manga" ? "light" : "dark");
      }
    }
    if (typeof localStorage !== "undefined") {
      try {
        localStorage.setItem("kura-theme", t);
      } catch {}
    }
  }

  async setGroupSeasons(on: boolean) {
    this.groupSeasons = on;
    await api.setPref("group_seasons", on ? "1" : "0");
  }

  async setAutoUpdate(on: boolean) {
    this.autoUpdate = on;
    await api.setPref("auto_update", on ? "1" : "0");
  }

  async setFanartApiKey(key: string) {
    this.fanartApiKey = key.trim();
    await api.setPref("fanart_api_key", this.fanartApiKey);
  }

  async setPlayback<K extends keyof PlaybackPrefs>(key: K, value: PlaybackPrefs[K]) {
    this.playback[key] = value;
    const names: Record<keyof PlaybackPrefs, string> = {
      player: "player",
      playerPath: "player_path",
      threshold: "watched_threshold",
      resume: "resume",
      autoplay: "autoplay",
      mpcPort: "mpc_port",
      audioLang: "audio_lang",
      subLang: "sub_lang",
      subFallback: "sub_fallback",
    };
    const v = typeof value === "boolean" ? (value ? "1" : "0") : String(value);
    await api.setPref(names[key], v);
  }

  async setDiscord<K extends keyof DiscordPrefs>(key: K, value: DiscordPrefs[K]) {
    this.discord[key] = value;
    const names: Record<keyof DiscordPrefs, string> = {
      enabled: "discord_rpc",
      spoilers: "discord_spoilers",
      buttons: "discord_buttons",
      clientId: "discord_client_id",
    };
    const v = typeof value === "boolean" ? (value ? "1" : "0") : String(value);
    await api.setPref(names[key], v);
  }

  /** Play an episode with the configured player; errors become a toast. */
  async play(anilistId: number, epKey: string, path: string) {
    if (this.#launchTimer) clearTimeout(this.#launchTimer);
    this.launching = { anilistId, epKey };
    this.#launchTimer = setTimeout(() => {
      if (this.launching?.anilistId === anilistId && this.launching?.epKey === epKey) {
        this.launching = null;
      }
    }, 12000);

    try {
      await api.playEpisode(anilistId, epKey, path);
    } catch (e) {
      this.launching = null;
      if (this.#launchTimer) clearTimeout(this.#launchTimer);
      this.toast(String(e), "error", 8000);
    }
  }

  isLaunching(anilistId?: number, epKey?: string): boolean {
    if (!this.launching) return false;
    if (anilistId !== undefined && epKey !== undefined) {
      return this.launching.anilistId === anilistId && this.launching.epKey === epKey;
    }
    if (anilistId !== undefined) {
      return this.launching.anilistId === anilistId;
    }
    return true;
  }

  isPlaying(anilistId?: number, epKey?: string): boolean {
    if (!this.nowPlaying) return false;
    if (anilistId !== undefined && epKey !== undefined) {
      return this.nowPlaying.anilistId === anilistId && this.nowPlaying.epKey === epKey;
    }
    if (anilistId !== undefined) {
      return this.nowPlaying.anilistId === anilistId;
    }
    return true;
  }

  toast(text: string, kind: Toast["kind"] = "info", ms = 5000) {
    const id = ++this.#toastId;
    this.toasts = [...this.toasts, { id, kind, text }];
    setTimeout(() => this.dismissToast(id), ms);
  }

  dismissToast(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  get reviewCount() {
    return this.unmatched.length + this.cards.filter((c) => c.needsReview).length;
  }
}

export const app = new AppStore();
