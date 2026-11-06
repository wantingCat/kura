import {
  api,
  events,
  type AnilistAccount,
  type BuiltinPlayerState,
  type Library,
  type MediaCard,
  type PlaybackEvent,
  type PlayerKind,
  type ScanProgress,
  type TorrentSessionStatus,
  type UnmatchedGroup,
} from "./api";
import { updater } from "./updater.svelte";

export interface PlaybackPrefs {
  /** Empty = not chosen yet (the backend then uses the built-in player). */
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
  anilistId?: number;
}

export interface AppAlert {
  id: string;
  anilistId: number;
  title: string;
  episode: string | number;
  kind: "episode" | "jimaku_sub" | "episode_and_sub" | string;
  message: string;
  at: number;
  read: boolean;
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
    autoplay: true,
    mpcPort: 13579,
    audioLang: "jpn",
    subLang: "jpn",
    subFallback: "eng",
  });
  autoUpdate = $state(true);
  airingNotifications = $state(true);
  minimizeToTray = $state(true);
  anilist = $state<AnilistAccount | null>(null);
  folderWatcher = $state(true);
  fanartApiKey = $state("");
  jimakuApiKey = $state("");
  jimakuAutoRetime = $state(true);
  /** "local" = Offline Library Mode, "streaming" = Streaming + Hybrid Mode. */
  appMode = $state<"local" | "streaming">("local");
  /** "stream" = Stream Only (Temp Cache), "hybrid" = Stream & Save to Library, "download" = Download to Library First. */
  torrentAction = $state<"stream" | "hybrid" | "download">("stream");
  /** Preferred resolution for auto-pick ("1080p", "720p", "2160p", or "ask"). */
  torrentQuality = $state<"1080p" | "720p" | "2160p" | "ask">("1080p");
  torrentAutoSubs = $state(true);
  torrentSeeding = $state(true);
  nyaaUrl = $state("");
  torrentSessions = $state<TorrentSessionStatus[]>([]);
  discord = $state<DiscordPrefs>({
    enabled: true,
    spoilers: false,
    buttons: true,
    clientId: "",
  });
  launching = $state<LaunchingEpisode | null>(null);
  /** Latest playback event while a player is being tracked. */
  nowPlaying = $state<PlaybackEvent | null>(null);
  /** Built-in player state when active. */
  builtinPlayer = $state<BuiltinPlayerState | null>(null);
  toasts = $state<Toast[]>([]);
  alerts = $state<AppAlert[]>([]);

  #refreshTimer: ReturnType<typeof setTimeout> | null = null;
  #launchTimer: ReturnType<typeof setTimeout> | null = null;
  #initialised = false;
  #needsRefreshOnWake = false;
  #toastId = 0;

  async init() {
    if (this.#initialised) return;
    this.#initialised = true;
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", () => {
        if (!document.hidden && this.#needsRefreshOnWake) {
          this.#needsRefreshOnWake = false;
          this.scheduleRefresh();
        }
      });
    }
    if (typeof localStorage !== "undefined") {
      const localTheme = localStorage.getItem("kura-theme") as AppTheme | null;
      if (localTheme === "manga" || localTheme === "comic") {
        this.theme = localTheme;
        this.applyTheme(this.theme);
      }
      try {
        const savedAlerts = localStorage.getItem("kura-alerts");
        if (savedAlerts) {
          this.alerts = JSON.parse(savedAlerts);
        }
      } catch {}
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
    await events.onAiringNotification((n) => {
      this.recordAlert(n);
      this.toast(n.message, "info", 10000, n.anilistId);
      if (typeof window !== "undefined" && "Notification" in window) {
        if (Notification.permission === "granted") {
          try {
            new Notification(n.title, { body: n.message });
          } catch {}
        } else if (Notification.permission === "default") {
          Notification.requestPermission().then((perm) => {
            if (perm === "granted") {
              try {
                new Notification(n.title, { body: n.message });
              } catch {}
            }
          }).catch(() => {});
        }
      }
    });
    await events.onTorrentProgress((s) => {
      if (s.state === "cancelled") {
        this.torrentSessions = this.torrentSessions.filter((x) => x.sessionId !== s.sessionId);
        return;
      }
      const idx = this.torrentSessions.findIndex((x) => x.sessionId === s.sessionId);
      if (idx >= 0) {
        this.torrentSessions[idx] = s;
      } else {
        this.torrentSessions = [s, ...this.torrentSessions];
      }
      if (s.state === "error" && s.error) {
        this.toast(`Torrent error: ${s.error}`, "error", 7000);
      } else if (s.state === "completed" && s.mode !== "stream") {
        this.scheduleRefresh();
      }
    });
    api.listTorrentSessions().then((list) => (this.torrentSessions = list)).catch(() => {});
    await events.onAnilistChanged((acct) => {
      const wasConnected = !!this.anilist?.connected;
      this.anilist = acct;
      if (!wasConnected && acct.connected) {
        this.toast(`Connected to AniList as ${acct.userName ?? "user"}!`, "info", 6000);
      }
      this.scheduleRefresh();
    });
    api.getAnilistAccount().then((a) => (this.anilist = a)).catch(() => {});
    await events.onBuiltinPlayerState((s) => {
      this.builtinPlayer = s;
      if (s.active) {
        this.launching = null;
        if (this.#launchTimer) clearTimeout(this.#launchTimer);
        if (typeof document !== "undefined") {
          document.documentElement.classList.add("builtin-player-active");
        }
      } else {
        if (typeof document !== "undefined") {
          document.documentElement.classList.remove("builtin-player-active");
        }
        this.scheduleRefresh();
      }
    });
    await events.onPlayback((p) => {
      if (p.state === "tracking") {
        this.nowPlaying = p;
        if (this.launching?.anilistId === p.anilistId && this.launching?.epKey === p.epKey) {
          this.launching = null;
          if (this.#launchTimer) clearTimeout(this.#launchTimer);
        }
      } else if (p.state === "stopped") {
        for (const s of this.torrentSessions) {
          if (s.anilistId === p.anilistId && s.epKey === p.epKey && (s.mode === "stream" || s.mode === "hybrid")) {
            api.cancelTorrentSession(s.sessionId).catch(() => {});
          }
        }
        this.torrentSessions = this.torrentSessions.filter(
          (s) => !(s.anilistId === p.anilistId && s.epKey === p.epKey && (s.mode === "stream" || s.mode === "hybrid")),
        );
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
    await this.loadPrefs();
    await this.refresh();
    if (this.autoUpdate) setTimeout(() => updater.check(), 3000);
  }

  /** (Re)load preferences from the backend — at startup and after a settings import. */
  async loadPrefs() {
    try {
      const prefs = await api.getPrefs();
      if (prefs.theme === "comic" || prefs.theme === "manga") {
        this.theme = prefs.theme;
        this.applyTheme(this.theme);
      }
      this.groupSeasons = prefs.group_seasons !== "0";
      this.autoUpdate = prefs.auto_update !== "0";
      this.folderWatcher = prefs.folder_watcher !== "0";
      this.airingNotifications = prefs.airing_notifications !== "0";
      this.minimizeToTray = prefs.minimize_to_tray !== "0";
      this.fanartApiKey = prefs.fanart_api_key ?? "";
      this.jimakuApiKey = prefs.jimaku_api_key ?? "";
      this.jimakuAutoRetime = prefs.jimaku_auto_retime !== "0";
      this.appMode = prefs.app_mode === "streaming" ? "streaming" : "local";
      this.torrentAction =
        prefs.torrent_action === "hybrid" || prefs.torrent_action === "download"
          ? prefs.torrent_action
          : "stream";
      this.torrentQuality =
        prefs.torrent_quality === "720p" || prefs.torrent_quality === "2160p" || prefs.torrent_quality === "ask"
          ? prefs.torrent_quality
          : "1080p";
      this.torrentAutoSubs = prefs.torrent_auto_subs !== "0";
      this.torrentSeeding = prefs.torrent_seeding !== "0";
      this.nyaaUrl = prefs.nyaa_url ?? "";
      const num = (v: string | undefined, d: number) => (v !== undefined && !isNaN(Number(v)) ? Number(v) : d);
      this.playback = {
        player: (prefs.player as PlayerKind) ?? "",
        playerPath: prefs.player_path ?? "",
        threshold: Math.min(100, Math.max(50, num(prefs.watched_threshold, 90))),
        resume: prefs.resume !== "0",
        autoplay: prefs.autoplay !== "0",
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
  }

  scheduleRefresh() {
    if (typeof document !== "undefined" && document.hidden && this.loaded) {
      this.#needsRefreshOnWake = true;
      return;
    }
    if (this.#refreshTimer) clearTimeout(this.#refreshTimer);
    this.#refreshTimer = setTimeout(() => this.refresh(), 300);
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

  async setAiringNotifications(on: boolean) {
    this.airingNotifications = on;
    await api.setPref("airing_notifications", on ? "1" : "0");
  }

  async setMinimizeToTray(on: boolean) {
    this.minimizeToTray = on;
    await api.setPref("minimize_to_tray", on ? "1" : "0");
  }

  async setFolderWatcher(on: boolean) {
    this.folderWatcher = on;
    await api.setPref("folder_watcher", on ? "1" : "0");
  }

  async setFanartApiKey(key: string) {
    this.fanartApiKey = key.trim();
    await api.setPref("fanart_api_key", this.fanartApiKey);
  }

  async setJimakuApiKey(key: string) {
    this.jimakuApiKey = key.trim();
    await api.setPref("jimaku_api_key", this.jimakuApiKey);
  }

  async setJimakuAutoRetime(on: boolean) {
    this.jimakuAutoRetime = on;
    await api.setPref("jimaku_auto_retime", on ? "1" : "0");
  }

  async setAppMode(mode: "local" | "streaming") {
    this.appMode = mode;
    await api.setPref("app_mode", mode);
    await this.refresh();
  }

  async setTorrentAction(action: "stream" | "hybrid" | "download") {
    this.torrentAction = action;
    await api.setPref("torrent_action", action);
  }

  async setTorrentQuality(quality: "1080p" | "720p" | "2160p" | "ask") {
    this.torrentQuality = quality;
    await api.setPref("torrent_quality", quality);
  }

  async setTorrentAutoSubs(on: boolean) {
    this.torrentAutoSubs = on;
    await api.setPref("torrent_auto_subs", on ? "1" : "0");
  }

  async setTorrentSeeding(on: boolean) {
    this.torrentSeeding = on;
    await api.setPref("torrent_seeding", on ? "1" : "0");
  }

  async setNyaaUrl(url: string) {
    const clean = url.trim().replace(/\/+$/, "");
    this.nyaaUrl = clean;
    await api.setPref("nyaa_url", clean);
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
    if (this.builtinPlayer?.active) {
      if (anilistId !== undefined && epKey !== undefined) {
        return this.builtinPlayer.anilistId === anilistId && this.builtinPlayer.epKey === epKey;
      }
      if (anilistId !== undefined) {
        return this.builtinPlayer.anilistId === anilistId;
      }
      return true;
    }
    if (!this.nowPlaying) return false;
    if (anilistId !== undefined && epKey !== undefined) {
      return this.nowPlaying.anilistId === anilistId && this.nowPlaying.epKey === epKey;
    }
    if (anilistId !== undefined) {
      return this.nowPlaying.anilistId === anilistId;
    }
    return true;
  }

  toast(text: string, kind: Toast["kind"] = "info", ms = 5000, anilistId?: number) {
    const id = ++this.#toastId;
    this.toasts = [...this.toasts, { id, kind, text, anilistId }];
    setTimeout(() => this.dismissToast(id), ms);
  }

  dismissToast(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  recordAlert(n: { anilistId: number; title: string; episode: string | number; kind?: string; message: string }) {
    const id = `${n.anilistId}-${n.episode}-${n.kind ?? "episode"}-${Date.now()}`;
    const next: AppAlert = {
      id,
      anilistId: n.anilistId,
      title: n.title,
      episode: n.episode,
      kind: n.kind ?? "episode",
      message: n.message,
      at: Date.now(),
      read: false,
    };
    this.alerts = [next, ...this.alerts].slice(0, 40);
    this.#saveAlerts();
  }

  markAllAlertsRead() {
    this.alerts = this.alerts.map((a) => ({ ...a, read: true }));
    this.#saveAlerts();
  }

  clearAlerts() {
    this.alerts = [];
    this.#saveAlerts();
  }

  #saveAlerts() {
    if (typeof localStorage !== "undefined") {
      try {
        localStorage.setItem("kura-alerts", JSON.stringify(this.alerts));
      } catch {}
    }
  }

  get unreadAlertCount() {
    return this.alerts.filter((a) => !a.read).length;
  }

  get reviewCount() {
    return this.unmatched.length + this.cards.filter((c) => c.needsReview).length;
  }
}

export const app = new AppStore();
