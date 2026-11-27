import {
  api,
  events,
  type AnilistAccount,
  type Anime4kPreset,
  type BuiltinPlayerState,
  type InstalledExtension,
  type Library,
  type MediaCard,
  type MediaDetail,
  type PlaybackEvent,
  type PlayerKind,
  type ScanProgress,
  type TorrentSessionStatus,
  type UnmatchedGroup,
} from "./api";
import { detailsModal } from "./detailsModal.svelte";
import {
  ensureCandidateMagnet,
  registerInstalledExtensionsGetter,
  searchInstalledTorrentExtensions,
} from "./extensions/host";
import {
  clearCachedMediaDetails,
  getCachedOrLoadMediaDetail,
  registerMediaDetailHooks,
} from "./mediaDetail.svelte";
import {
  autoPlayWebEpisode,
  preloadNextWebEpisode,
} from "./onlinestream.svelte";
import { updater } from "./updater.svelte";

interface PlaybackPrefs {
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

interface DiscordPrefs {
  enabled: boolean;
  spoilers: boolean;
  buttons: boolean;
  clientId: string;
}

interface LaunchingEpisode {
  anilistId: number;
  epKey: string;
}

interface Toast {
  id: number;
  kind: "info" | "warn" | "error";
  text: string;
  anilistId?: number;
}

interface AppAlert {
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
  /** Tokyo Night Japanese accent layer (日本語アクセント): native titles, JP labels, kanji. */
  japaneseAccents = $state(true);
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
  /** Default stream backend for unowned episodes ("web" = Direct Soft-Sub Web Stream, "torrent" = BitTorrent Swarm). */
  streamSource = $state<"web" | "torrent">("web");
  /** Enforce clean soft-sub streams only (skip burned-in hardsub servers unless manually overridden). */
  streamSoftSubOnly = $state(true);
  /** Default Anime4K real-time GPU shader preset in native libmpv. */
  anime4kPreset = $state<Anime4kPreset>("off");
  /** Auto-select release profile preferences for torrent scoring. */
  torrentGroups = $state("");
  torrentCodec = $state<"any" | "hevc" | "avc" | "av1">("any");
  torrentMinSeeders = $state(0);
  torrentExclude = $state("");
  extensionMarketplaceUrl = $state("");
  installedExtensions = $state<InstalledExtension[]>([]);
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
  #extensionsLoaded = false;
  #needsRefreshOnWake = false;
  #toastId = 0;

  get extensionsLoaded(): boolean {
    return this.#extensionsLoaded;
  }

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
    if (typeof document !== "undefined") {
      delete document.documentElement.dataset.theme;
    }
    if (typeof localStorage !== "undefined") {
      try {
        localStorage.removeItem("kura-theme");
      } catch {}
      const localJa = localStorage.getItem("kura-ja-accents");
      if (localJa === "0" || localJa === "1") {
        this.japaneseAccents = localJa === "1";
      }
      this.#applyJapaneseAccents();
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
          Notification.requestPermission()
            .then((perm) => {
              if (perm === "granted") {
                try {
                  new Notification(n.title, { body: n.message });
                } catch {}
              }
            })
            .catch(() => {});
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
    api
      .listTorrentSessions()
      .then((list) => (this.torrentSessions = Array.isArray(list) ? list : []))
      .catch(() => {});
    await events.onAnilistChanged((acct) => {
      const wasConnected = !!this.anilist?.connected;
      this.anilist = acct;
      if (!wasConnected && acct.connected) {
        this.toast(`Connected to AniList as ${acct.userName ?? "user"}!`, "info", 6000);
      }
      this.scheduleRefresh();
    });
    api
      .getAnilistAccount()
      .then((a) => (this.anilist = a))
      .catch(() => {});
    await events.onBuiltinPlayerState((s) => {
      const wasActive = Boolean(this.builtinPlayer?.active);
      this.builtinPlayer = s;
      if (s.active) {
        this.launching = null;
        if (this.#launchTimer) clearTimeout(this.#launchTimer);
      } else if (wasActive) {
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
    await events.onPreloadNextEpisode((ev) => {
      if (ev.streamKind === "web") {
        void preloadNextWebEpisode(ev.anilistId, ev.nextEpisode, this.streamSoftSubOnly);
      } else if (ev.streamKind === "torrent") {
        void api.preloadNextTorrentEpisode(ev.anilistId, ev.nextEpisode).catch(() => {});
      }
    });
    await events.onPlayNextStreamEpisode(async (ev) => {
      if (ev.streamKind === "web") {
        try {
          const detail = await getCachedOrLoadMediaDetail(ev.anilistId);
          this.launching = { anilistId: ev.anilistId, epKey: String(ev.nextEpisode) };
          const picked = await autoPlayWebEpisode(detail, ev.nextEpisode, this.streamSoftSubOnly);
          this.toast(
            `Playing Ep ${ev.nextEpisode} via ${picked.providerName} (${picked.server})…`,
            "info",
            4000,
            ev.anilistId,
          );
        } catch (e) {
          this.launching = null;
          this.toast(`Next episode stream failed: ${e instanceof Error ? e.message : String(e)}`, "warn", 7000);
        }
      } else if (ev.streamKind === "torrent") {
        try {
          this.launching = { anilistId: ev.anilistId, epKey: String(ev.nextEpisode) };
          const [nativeRes, detailRes] = await Promise.allSettled([
            api.searchEpisodeTorrents(ev.anilistId, ev.nextEpisode, null),
            getCachedOrLoadMediaDetail(ev.anilistId),
          ]);
          const nativeCands = nativeRes.status === "fulfilled" ? nativeRes.value : [];
          let jsCands: typeof nativeCands = [];
          if (detailRes.status === "fulfilled") {
            jsCands = await searchInstalledTorrentExtensions(
              detailRes.value,
              ev.nextEpisode,
              this.torrentQuality,
            ).catch(() => []);
          }
          const cands = [...nativeCands, ...jsCands].sort((a, b) => b.score - a.score);
          const best =
            cands.find((c) => c.isBestRelease) ??
            cands.find((c) => c.seeders > 0) ??
            cands[0];
          if (!best) throw new Error(`No torrent release found for Episode ${ev.nextEpisode}`);
          const resolvedBest = await ensureCandidateMagnet(best);
          await api.startTorrentSession({
            anilistId: ev.anilistId,
            epKey: String(ev.nextEpisode),
            episodeNum: ev.nextEpisode,
            candidate: resolvedBest,
            modeOverride: this.torrentAction === "download" ? "stream" : this.torrentAction,
          });
        } catch (e) {
          this.launching = null;
          this.toast(`Next episode torrent failed: ${e instanceof Error ? e.message : String(e)}`, "warn", 7000);
        }
      }
    });
    this.scanning = await api.isScanning();
    await this.loadPrefs();
    await this.refreshExtensions();
    await this.refresh();
    if (this.autoUpdate) setTimeout(() => updater.check(), 3000);
  }

  /** (Re)load preferences from the backend — at startup and after a settings import. */
  async loadPrefs() {
    try {
      const prefs = await api.getPrefs();
      this.groupSeasons = prefs.group_seasons !== "0";
      if (prefs.ja_accents === "0" || prefs.ja_accents === "1") {
        this.japaneseAccents = prefs.ja_accents === "1";
        this.#applyJapaneseAccents();
      }
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
      this.streamSource = prefs.stream_source === "torrent" ? "torrent" : "web";
      this.streamSoftSubOnly = prefs.stream_softsub_only !== "0";
      this.anime4kPreset =
        prefs.anime4k_preset === "a_fast" ||
        prefs.anime4k_preset === "a_hq" ||
        prefs.anime4k_preset === "b_soft" ||
        prefs.anime4k_preset === "c_upscale"
          ? prefs.anime4k_preset
          : "off";
      this.torrentGroups = prefs.torrent_groups ?? "";
      this.torrentCodec =
        prefs.torrent_codec === "hevc" || prefs.torrent_codec === "avc" || prefs.torrent_codec === "av1"
          ? prefs.torrent_codec
          : "any";
      this.torrentMinSeeders = Math.max(0, Number(prefs.torrent_min_seeders ?? "0") || 0);
      this.torrentExclude = prefs.torrent_exclude ?? "";
      this.extensionMarketplaceUrl = prefs.extension_marketplace_url ?? "";
      const num = (v: string | undefined, d: number) => (v !== undefined && !isNaN(Number(v)) ? Number(v) : d);
      const rawPlayer = prefs.player ?? "";
      const validPlayer: PlayerKind | "" =
        rawPlayer === "builtin" ||
        rawPlayer === "system" ||
        rawPlayer === "mpv" ||
        rawPlayer === "vlc" ||
        rawPlayer === "mpc"
          ? rawPlayer
          : "";
      this.playback = {
        player: validPlayer,
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
    clearCachedMediaDetails();
    const [libraries, cards, unmatched] = await Promise.all([
      api.listLibraries(),
      api.getLibrary(null),
      api.getUnmatched(),
    ]);
    clearCachedMediaDetails();
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

  async setGroupSeasons(on: boolean) {
    this.groupSeasons = on;
    await api.setPref("group_seasons", on ? "1" : "0");
  }

  /** Toggle the Japanese accent layer (日本語アクセント). */
  async setJapaneseAccents(on: boolean) {
    this.japaneseAccents = on;
    this.#applyJapaneseAccents();
    await api.setPref("ja_accents", on ? "1" : "0");
  }

  #applyJapaneseAccents() {
    if (typeof document !== "undefined") {
      document.documentElement.classList.toggle("ja-off", !this.japaneseAccents);
    }
    if (typeof localStorage !== "undefined") {
      try {
        localStorage.setItem("kura-ja-accents", this.japaneseAccents ? "1" : "0");
      } catch {}
    }
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

  async refreshExtensions() {
    try {
      this.installedExtensions = await api.listExtensions();
      this.#extensionsLoaded = true;
    } catch {
      this.installedExtensions = [];
    }
  }

  get hasEnabledWebStreamProvider(): boolean {
    return this.installedExtensions.some(
      (e) => e.enabled && e.extType === "onlinestream-provider",
    );
  }

  async setStreamSource(source: "web" | "torrent") {
    this.streamSource = source;
    await api.setPref("stream_source", source);
  }

  async setStreamSoftSubOnly(on: boolean) {
    this.streamSoftSubOnly = on;
    await api.setPref("stream_softsub_only", on ? "1" : "0");
  }

  async setAnime4kPreset(preset: Anime4kPreset) {
    this.anime4kPreset = preset;
    await api.setPref("anime4k_preset", preset);
    if (this.builtinPlayer?.active) {
      await api.builtinPlayerCommand({ type: "set_shader_preset", preset }).catch(() => {});
    }
  }

  async setTorrentGroups(groups: string) {
    this.torrentGroups = groups.trim();
    await api.setPref("torrent_groups", this.torrentGroups);
  }

  async setTorrentCodec(codec: "any" | "hevc" | "avc" | "av1") {
    this.torrentCodec = codec;
    await api.setPref("torrent_codec", codec);
  }

  async setTorrentMinSeeders(minSeeders: number) {
    const clean = Math.max(0, Math.floor(minSeeders || 0));
    this.torrentMinSeeders = clean;
    await api.setPref("torrent_min_seeders", String(clean));
  }

  async setTorrentExclude(exclude: string) {
    this.torrentExclude = exclude.trim();
    await api.setPref("torrent_exclude", this.torrentExclude);
  }

  async setExtensionMarketplaceUrl(url: string) {
    const clean = url.trim();
    this.extensionMarketplaceUrl = clean;
    await api.setPref("extension_marketplace_url", clean);
  }

  /**
   * Smart primary stream launcher for unowned episodes:
   * - If `streamSource === "web"` and an `onlinestream-provider` extension is enabled, resolves
   *   the best soft-sub stream and launches `libmpv` directly (with `.vtt` -> Jimaku retiming).
   * - Otherwise tells the caller which picker dialog to open (`"open_web_picker"` or `"open_torrent_picker"`).
   */
  async playStreamEpisode(
    detail: MediaDetail,
    episodeNum: number,
  ): Promise<"launched" | "open_web_picker" | "open_torrent_picker"> {
    if (this.streamSource === "torrent") {
      return "open_torrent_picker";
    }
    if (!this.hasEnabledWebStreamProvider) {
      await this.refreshExtensions();
    }
    if (!this.hasEnabledWebStreamProvider) {
      return "open_web_picker";
    }

    const epKey = String(episodeNum);
    if (this.#launchTimer) clearTimeout(this.#launchTimer);
    this.launching = { anilistId: detail.anilistId, epKey };
    this.#launchTimer = setTimeout(() => {
      if (this.launching?.anilistId === detail.anilistId && this.launching?.epKey === epKey) {
        this.launching = null;
      }
    }, 18000);

    try {
      const picked = await autoPlayWebEpisode(detail, episodeNum, this.streamSoftSubOnly);
      this.toast(
        `Streaming Ep ${episodeNum} via ${picked.providerName} (${picked.server})…`,
        "info",
        4000,
        detail.anilistId,
      );
      return "launched";
    } catch (e) {
      this.launching = null;
      if (this.#launchTimer) clearTimeout(this.#launchTimer);
      const msg = e instanceof Error ? e.message : String(e);
      this.toast(msg.replace(/^[A-Z_]+:\s*/, ""), "warn", 7000, detail.anilistId);
      return "open_web_picker";
    }
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
    if (!path) {
      try {
        const detail = await getCachedOrLoadMediaDetail(anilistId);
        const epNum = Number(epKey.replace(/^\D+/, "")) || 1;
        const action = await this.playStreamEpisode(detail, epNum);
        if (action !== "launched") {
          detailsModal.open(anilistId);
        }
      } catch (e) {
        this.toast(String(e), "error", 7000);
      }
      return;
    }
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

registerInstalledExtensionsGetter(() =>
  app.extensionsLoaded ? app.installedExtensions : null,
);
registerMediaDetailHooks({
  refresh: () => app.refresh(),
  toast: (text, kind, ms) => app.toast(text, kind, ms),
});
